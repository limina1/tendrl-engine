//! Paid model providers for the in-engine agent loop: **ppq.ai** and the
//! **routstr** network.
//!
//! The engine owns the accounts. Keys live in `<data_dir>/providers.json`
//! (mode 0600) — on Android that is the app's private storage, on desktop the
//! user's data dir — never in TOML, never in a Nostr event, never in the
//! browser. The API hands out *redacted* keys plus a one-shot reveal.
//!
//! Both providers speak the OpenAI chat-completions wire format, so one
//! adapter ([`crate::llm::OpenAiCompatProvider`]) serves both; what differs is
//! the account: ppq is a `credit_id` + `sk-` key funded over Lightning,
//! routstr is a Cashu token turned into an `sk-` key on one *node* found via
//! kind-38421 announcements (the central `api.routstr.com` is dead — discovery
//! is mandatory, and Cashu is the only on-ramp on live nodes).
//!
//! Shapes below were verified live on 2026-09-22 (`docs/paid-providers.org`,
//! `scripts/probe-paid-providers.py`).

use crate::error::{EngineError, Result};
use crate::llm::{
    AgentMessage, ContentBlock, LLMProvider, OpenAiCompatProvider, StopReason, ToolDefinition,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub const PPQ_BASE: &str = "https://api.ppq.ai";
pub const ROUTSTR_ANNOUNCE_KIND: u64 = 38421;
pub const ROUTSTR_RELAYS: &[&str] = &["wss://relay.routstr.com", "wss://nos.lol"];
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);
const MAX_NODES_PROBED: usize = 24;

/// Model families known to drive tool calling well. routstr nodes publish no
/// capability data, so this heuristic sorts the picker and seeds "likely".
const TOOL_FAMILIES: &[&str] = &[
    "claude", "gpt-", "gemini", "glm", "deepseek", "qwen", "mistral", "grok", "kimi", "minimax",
    "o3", "o4",
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Ppq,
    Routstr,
}

impl ProviderKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ppq" => Some(Self::Ppq),
            "routstr" => Some(Self::Routstr),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ppq => "ppq",
            Self::Routstr => "routstr",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ppq => "ppq.ai",
            Self::Routstr => "routstr",
        }
    }
}

/// A cached balance reading. `unit` is `usd` (ppq) or `msat` (routstr).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Balance {
    pub unit: String,
    pub amount: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_spent: Option<f64>,
    pub fetched_at: u64,
    /// Set when the last refresh failed; the previous amount is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A routstr node as announced (kind 38421) and described by `/v1/info`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeRef {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub mints: Vec<String>,
}

/// Discovery result for one node: the ref plus what the probe saw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    #[serde(flatten)]
    pub node: NodeRef,
    pub reachable: bool,
    pub model_count: usize,
    pub latency_ms: u64,
    /// The node lists at least one model from a tool-capable family.
    pub likely_tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTest {
    Ok,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PpqAccount {
    pub credit_id: String,
    pub api_key: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<Balance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutstrAccount {
    pub node: NodeRef,
    pub api_key: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<Balance>,
    /// Tool-call self-test outcome per model id on this node.
    #[serde(default)]
    pub tool_tests: BTreeMap<String, ToolTest>,
}

/// What the last agent turn cost, for the "running on" line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCost {
    pub provider: ProviderKind,
    pub model: String,
    /// In the provider's unit; `None` when the balance delta was unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub unit: String,
    pub tool_calls: usize,
    pub seconds: f64,
    pub at: u64,
}

fn default_ceiling_usd() -> f64 {
    0.25
}
fn default_ceiling_sats() -> f64 {
    250.0
}

/// The persisted file. `active` names the service the agent loop runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<ProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ppq: Option<PpqAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routstr: Option<RoutstrAccount>,
    /// Per-turn spend ceiling for ppq (USD) — a turn below it asks first.
    #[serde(default = "default_ceiling_usd")]
    pub ceiling_usd: f64,
    /// Per-turn spend ceiling for routstr (sats).
    #[serde(default = "default_ceiling_sats")]
    pub ceiling_sats: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn: Option<TurnCost>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            active: None,
            ppq: None,
            routstr: None,
            ceiling_usd: default_ceiling_usd(),
            ceiling_sats: default_ceiling_sats(),
            last_turn: None,
        }
    }
}

/// Why a turn cannot start: emitted to the UI as a `needs_funding` event.
#[derive(Debug, Clone, Serialize)]
pub struct FundingNeed {
    pub provider: ProviderKind,
    pub balance: f64,
    pub unit: String,
    pub ceiling: f64,
    /// The other service, if it is configured and funded — a switch target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternative: Option<ProviderKind>,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct ProviderStore {
    path: PathBuf,
}

impl ProviderStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("providers.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<ProviderState> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s).map_err(|e| {
                EngineError::Config(format!("providers.json: {e} (fix or delete the file)"))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ProviderState::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, state: &ProviderState) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        restrict_permissions(&tmp);
        std::fs::rename(&tmp, &self.path)?;
        restrict_permissions(&self.path);
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

// ---------------------------------------------------------------------------
// Registry — the live state + the provider clients
// ---------------------------------------------------------------------------

pub struct ProviderRegistry {
    store: ProviderStore,
    state: RwLock<ProviderState>,
    http: reqwest::Client,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `sk-aJv…(25)` — enough to recognise a key, never enough to use it.
pub fn redact(key: &str) -> String {
    let n = key.chars().count();
    let head: String = key.chars().take(6).collect();
    format!("{head}…({n})")
}

fn is_tool_family(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    TOOL_FAMILIES.iter().any(|f| id.contains(f))
}

fn normalize_base(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

impl ProviderRegistry {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let store = ProviderStore::new(data_dir);
        let state = store.load()?;
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("tendrl-engine")
            .build()
            .map_err(|e| EngineError::Other(format!("http client: {e}")))?;
        Ok(Self {
            store,
            state: RwLock::new(state),
            http,
        })
    }

    pub fn snapshot(&self) -> ProviderState {
        self.state.read().unwrap().clone()
    }

    fn mutate<R>(&self, f: impl FnOnce(&mut ProviderState) -> R) -> Result<R> {
        let mut guard = self.state.write().unwrap();
        let r = f(&mut guard);
        self.store.save(&guard)?;
        Ok(r)
    }

    // ----------------------------------------------------------------- view

    /// The API view: redacted keys, cached balances, the active selection.
    pub fn view(&self) -> Value {
        let s = self.snapshot();
        let ppq = s.ppq.as_ref().map(|a| {
            json!({
                "credit_id": a.credit_id,
                "api_key": redact(&a.api_key),
                "created_at": a.created_at,
                "model": a.model,
                "balance": a.balance,
                "status": status_of(a.balance.as_ref(), s.ceiling_usd),
                "web_search": true,
            })
        });
        let routstr = s.routstr.as_ref().map(|a| {
            json!({
                "node": a.node,
                "api_key": redact(&a.api_key),
                "created_at": a.created_at,
                "model": a.model,
                "balance": a.balance,
                "status": status_of(a.balance.as_ref(), s.ceiling_sats * 1000.0),
                "tool_tests": a.tool_tests,
                "web_search": false,
            })
        });
        json!({
            "active": s.active,
            "ppq": ppq,
            "routstr": routstr,
            "ceiling_usd": s.ceiling_usd,
            "ceiling_sats": s.ceiling_sats,
            "last_turn": s.last_turn,
            "configured": s.ppq.is_some() || s.routstr.is_some(),
            "path": self.store.path(),
        })
    }

    pub fn reveal(&self, kind: ProviderKind) -> Result<Value> {
        let s = self.snapshot();
        match kind {
            ProviderKind::Ppq => s
                .ppq
                .map(|a| json!({ "api_key": a.api_key, "credit_id": a.credit_id }))
                .ok_or_else(|| EngineError::NotFound("no ppq account".into())),
            ProviderKind::Routstr => s
                .routstr
                .map(|a| json!({ "api_key": a.api_key, "node_url": a.node.url }))
                .ok_or_else(|| EngineError::NotFound("no routstr key".into())),
        }
    }

    // ------------------------------------------------------------ selection

    pub fn select(&self, kind: ProviderKind, model: Option<String>) -> Result<()> {
        self.mutate(|s| {
            match kind {
                ProviderKind::Ppq => {
                    let a = s
                        .ppq
                        .as_mut()
                        .ok_or_else(|| EngineError::NotFound("no ppq account".into()))?;
                    if let Some(m) = model {
                        a.model = Some(m);
                    }
                }
                ProviderKind::Routstr => {
                    let a = s
                        .routstr
                        .as_mut()
                        .ok_or_else(|| EngineError::NotFound("no routstr key".into()))?;
                    if let Some(m) = model {
                        a.model = Some(m);
                    }
                }
            }
            s.active = Some(kind);
            Ok::<(), EngineError>(())
        })?
    }

    pub fn set_ceilings(&self, usd: Option<f64>, sats: Option<f64>) -> Result<()> {
        self.mutate(|s| {
            if let Some(v) = usd {
                s.ceiling_usd = v.max(0.0);
            }
            if let Some(v) = sats {
                s.ceiling_sats = v.max(0.0);
            }
        })
    }

    pub fn forget(&self, kind: ProviderKind) -> Result<()> {
        self.mutate(|s| {
            match kind {
                ProviderKind::Ppq => s.ppq = None,
                ProviderKind::Routstr => s.routstr = None,
            }
            if s.active == Some(kind) {
                s.active = if s.ppq.is_some() {
                    Some(ProviderKind::Ppq)
                } else if s.routstr.is_some() {
                    Some(ProviderKind::Routstr)
                } else {
                    None
                };
            }
        })
    }

    /// The provider the agent loop should run on right now, if a service is
    /// active. `None` ⇒ fall back to the boot-time `[ai]` provider.
    pub fn active_provider(&self) -> Option<(Arc<dyn LLMProvider>, String, ProviderKind)> {
        let s = self.snapshot();
        match s.active? {
            ProviderKind::Ppq => {
                let a = s.ppq?;
                let model = a.model.clone().unwrap_or_else(|| "claude-sonnet-5".into());
                Some((
                    Arc::new(OpenAiCompatProvider::new(
                        "ppq",
                        format!("{PPQ_BASE}/v1"),
                        a.api_key.clone(),
                        model.clone(),
                    )),
                    model,
                    ProviderKind::Ppq,
                ))
            }
            ProviderKind::Routstr => {
                let a = s.routstr?;
                let model = a.model.clone()?;
                Some((
                    Arc::new(OpenAiCompatProvider::new(
                        "routstr",
                        format!("{}/v1", normalize_base(&a.node.url)),
                        a.api_key.clone(),
                        model.clone(),
                    )),
                    model,
                    ProviderKind::Routstr,
                ))
            }
        }
    }

    /// Refresh the active balance and decide whether a turn may start.
    pub async fn funding_need(&self) -> Option<FundingNeed> {
        let kind = self.snapshot().active?;
        let _ = self.refresh_balance(kind).await;
        let s = self.snapshot();
        let (balance, unit, ceiling) = match kind {
            ProviderKind::Ppq => {
                let a = s.ppq.as_ref()?;
                let b = a.balance.as_ref()?;
                (b.amount, "usd", s.ceiling_usd)
            }
            ProviderKind::Routstr => {
                let a = s.routstr.as_ref()?;
                let b = a.balance.as_ref()?;
                (b.amount / 1000.0, "sat", s.ceiling_sats)
            }
        };
        if balance >= ceiling {
            return None;
        }
        let alternative = match kind {
            ProviderKind::Ppq => s
                .routstr
                .as_ref()
                .filter(|a| a.balance.as_ref().map(|b| b.amount > 0.0).unwrap_or(false))
                .map(|_| ProviderKind::Routstr),
            ProviderKind::Routstr => s
                .ppq
                .as_ref()
                .filter(|a| a.balance.as_ref().map(|b| b.amount > 0.0).unwrap_or(false))
                .map(|_| ProviderKind::Ppq),
        };
        Some(FundingNeed {
            provider: kind,
            balance,
            unit: unit.into(),
            ceiling,
            alternative,
        })
    }

    /// Called after an agent turn: refresh the balance and record the delta
    /// as the turn's cost.
    pub async fn record_turn(&self, kind: ProviderKind, model: String, tool_calls: usize, seconds: f64) {
        let before = self.current_balance(kind);
        let _ = self.refresh_balance(kind).await;
        let after = self.current_balance(kind);
        let (cost, unit) = match kind {
            ProviderKind::Ppq => (
                before.zip(after).map(|(b, a)| (b - a).max(0.0)),
                "usd",
            ),
            ProviderKind::Routstr => (
                before.zip(after).map(|(b, a)| ((b - a) / 1000.0).max(0.0)),
                "sat",
            ),
        };
        let _ = self.mutate(|s| {
            s.last_turn = Some(TurnCost {
                provider: kind,
                model,
                cost,
                unit: unit.into(),
                tool_calls,
                seconds,
                at: now(),
            })
        });
    }

    fn current_balance(&self, kind: ProviderKind) -> Option<f64> {
        let s = self.snapshot();
        match kind {
            ProviderKind::Ppq => s.ppq?.balance.map(|b| b.amount),
            ProviderKind::Routstr => s.routstr?.balance.map(|b| b.amount),
        }
    }

    pub async fn refresh_balance(&self, kind: ProviderKind) -> Result<Balance> {
        match kind {
            ProviderKind::Ppq => self.ppq_refresh_balance().await,
            ProviderKind::Routstr => self.routstr_refresh_balance().await,
        }
    }

    // ------------------------------------------------------------------ http

    async fn call(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        let mut req = self.http.request(method, url);
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| EngineError::Relay(format!("{url}: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));
        Ok((status, value))
    }

    fn fail(what: &str, status: u16, body: &Value) -> EngineError {
        let msg = body["error"]["message"]
            .as_str()
            .or_else(|| body["detail"]["error"]["message"].as_str())
            .or_else(|| body["detail"].as_str())
            .or_else(|| body["message"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| body.to_string());
        if status == 401 || status == 403 {
            EngineError::Auth(format!("{what}: {msg}"))
        } else if status == 404 {
            EngineError::NotFound(format!("{what}: {msg}"))
        } else if (400..500).contains(&status) {
            EngineError::BadRequest(format!("{what}: {msg}"))
        } else {
            EngineError::Relay(format!("{what}: HTTP {status}: {msg}"))
        }
    }

    // ------------------------------------------------------------------- ppq

    fn ppq(&self) -> Result<PpqAccount> {
        self.snapshot()
            .ppq
            .ok_or_else(|| EngineError::NotFound("no ppq account — set one up first".into()))
    }

    /// `POST /accounts/create` (no auth) → credit_id + api_key. Refuses to
    /// overwrite an existing account (forget it first).
    pub async fn ppq_setup(&self) -> Result<Value> {
        if self.snapshot().ppq.is_some() {
            return Err(EngineError::BadRequest(
                "a ppq account already exists — forget it before creating another".into(),
            ));
        }
        let (status, body) = self
            .call(reqwest::Method::POST, &format!("{PPQ_BASE}/accounts/create"), &[], Some(json!({})))
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq create account", status, &body));
        }
        let credit_id = body["credit_id"].as_str().unwrap_or("").to_string();
        let api_key = body["api_key"].as_str().unwrap_or("").to_string();
        if credit_id.is_empty() || api_key.is_empty() {
            return Err(EngineError::Relay(
                "ppq create account: response missing credit_id/api_key".into(),
            ));
        }
        self.mutate(|s| {
            s.ppq = Some(PpqAccount {
                credit_id,
                api_key,
                created_at: now(),
                model: Some("claude-sonnet-5".into()),
                balance: Some(Balance {
                    unit: "usd".into(),
                    amount: body["balance"].as_f64().unwrap_or(0.0),
                    fetched_at: now(),
                    ..Default::default()
                }),
            });
            if s.active.is_none() {
                s.active = Some(ProviderKind::Ppq);
            }
        })?;
        Ok(self.view())
    }

    /// Adopt a key created elsewhere. Validated with one balance call.
    pub async fn ppq_adopt(&self, api_key: String, credit_id: String) -> Result<Value> {
        let api_key = api_key.trim().to_string();
        let credit_id = credit_id.trim().to_string();
        if !api_key.starts_with("sk-") || credit_id.is_empty() {
            return Err(EngineError::BadRequest(
                "ppq needs an sk- key and its credit id".into(),
            ));
        }
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("{PPQ_BASE}/credits/balance"),
                &[("Authorization", format!("Bearer {api_key}"))],
                Some(json!({ "credit_id": credit_id })),
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq key check", status, &body));
        }
        let amount = body["balance"].as_f64().unwrap_or(0.0);
        self.mutate(|s| {
            s.ppq = Some(PpqAccount {
                credit_id,
                api_key,
                created_at: now(),
                model: Some("claude-sonnet-5".into()),
                balance: Some(Balance {
                    unit: "usd".into(),
                    amount,
                    fetched_at: now(),
                    ..Default::default()
                }),
            });
            if s.active.is_none() {
                s.active = Some(ProviderKind::Ppq);
            }
        })?;
        Ok(self.view())
    }

    pub async fn ppq_refresh_balance(&self) -> Result<Balance> {
        let a = self.ppq()?;
        let res = self
            .call(
                reqwest::Method::POST,
                &format!("{PPQ_BASE}/credits/balance"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                Some(json!({ "credit_id": a.credit_id })),
            )
            .await;
        let (spent_today, spent_all) = self.ppq_spend_summary(&a).await;
        let balance = match res {
            Ok((status, body)) if (200..300).contains(&status) => Balance {
                unit: "usd".into(),
                amount: body["balance"].as_f64().unwrap_or(0.0),
                reserved: spent_today,
                total_spent: spent_all,
                fetched_at: now(),
                error: None,
            },
            Ok((status, body)) => Balance {
                error: Some(Self::fail("ppq balance", status, &body).to_string()),
                ..a.balance.clone().unwrap_or_default()
            },
            Err(e) => Balance {
                error: Some(e.to_string()),
                ..a.balance.clone().unwrap_or_default()
            },
        };
        let out = balance.clone();
        self.mutate(|s| {
            if let Some(acc) = s.ppq.as_mut() {
                acc.balance = Some(balance);
            }
        })?;
        if let Some(e) = &out.error {
            return Err(EngineError::Relay(e.clone()));
        }
        Ok(out)
    }

    /// (spent today, spent all time) in USD from the key list — best effort.
    /// Stored in `reserved` / `total_spent` so the card can show spend without
    /// a second endpoint.
    async fn ppq_spend_summary(&self, a: &PpqAccount) -> (Option<f64>, Option<f64>) {
        let res = self
            .call(
                reqwest::Method::GET,
                &format!("{PPQ_BASE}/keys"),
                &[("x-credit-id", a.credit_id.clone())],
                None,
            )
            .await;
        let Ok((status, body)) = res else { return (None, None) };
        if !(200..300).contains(&status) {
            return (None, None);
        }
        let keys = body["data"].as_array().cloned().unwrap_or_default();
        let period: f64 = keys
            .iter()
            .filter_map(|k| k["current_period_usage_usd"].as_f64())
            .sum();
        let all: f64 = keys
            .iter()
            .filter_map(|k| k["total_usage_all_time_usd"].as_f64())
            .sum();
        (Some(period), Some(all))
    }

    /// Chat models that advertise `tools`, with USD per-1M pricing.
    pub async fn ppq_models(&self) -> Result<Value> {
        let (status, body) = self
            .call(reqwest::Method::GET, &format!("{PPQ_BASE}/v1/models"), &[], None)
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq models", status, &body));
        }
        let selected = self.snapshot().ppq.and_then(|a| a.model);
        let mut models: Vec<Value> = body["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                m["supported_parameters"]
                    .as_array()
                    .map(|p| p.iter().any(|x| x.as_str() == Some("tools")))
                    .unwrap_or(false)
            })
            .filter(|m| {
                let modality = m["architecture"]["modality"].as_str().unwrap_or("");
                modality.is_empty() || modality.ends_with("->text")
            })
            .map(|m| {
                let id = m["id"].as_str().unwrap_or("").to_string();
                json!({
                    "id": id,
                    "name": m["name"],
                    "context_length": m["context_length"],
                    "input_per_1m": m["pricing"]["input_per_1M_tokens"],
                    "output_per_1m": m["pricing"]["output_per_1M_tokens"],
                    "tools": true,
                    "likely_tools": true,
                    "popular": m["popular"].as_bool().unwrap_or(false),
                    "selected": Some(&id) == selected.as_ref(),
                })
            })
            .collect();
        models.sort_by(|a, b| {
            b["popular"]
                .as_bool()
                .cmp(&a["popular"].as_bool())
                .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        Ok(json!({ "provider": "ppq", "models": models }))
    }

    /// `POST /topup/create/btc-lightning` → a bolt11 the user pays. Min 100 sats.
    pub async fn ppq_create_invoice(&self, amount_sats: u64) -> Result<Value> {
        if amount_sats < 100 {
            return Err(EngineError::BadRequest("ppq minimum top-up is 100 sats".into()));
        }
        let a = self.ppq()?;
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("{PPQ_BASE}/topup/create/btc-lightning"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                Some(json!({ "amount": amount_sats, "currency": "SATS" })),
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq invoice", status, &body));
        }
        Ok(json!({
            "invoice_id": body["invoice_id"],
            "bolt11": body["lightning_invoice"],
            "amount_sats": body["amount"],
            "expires_at": body["expires_at"],
            "created_at": body["created_at"],
            "checkout_url": body["checkout_url"],
        }))
    }

    /// `GET /topup/status/{id}`. `paid` is true once the status leaves `New`
    /// (ppq reports `Settled`/`Processing`); a paid invoice refreshes balance.
    pub async fn ppq_invoice_status(&self, invoice_id: &str) -> Result<Value> {
        let a = self.ppq()?;
        let (status, body) = self
            .call(
                reqwest::Method::GET,
                &format!("{PPQ_BASE}/topup/status/{invoice_id}"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq invoice status", status, &body));
        }
        let st = body["status"].as_str().unwrap_or("").to_string();
        let expires_at = body["expires_at"].as_u64().unwrap_or(0);
        let paid = !st.is_empty()
            && !st.eq_ignore_ascii_case("new")
            && !st.eq_ignore_ascii_case("expired")
            && !st.eq_ignore_ascii_case("invalid");
        let expired = st.eq_ignore_ascii_case("expired") || (expires_at > 0 && now() > expires_at && !paid);
        let mut balance = None;
        if paid {
            balance = self.ppq_refresh_balance().await.ok();
        }
        Ok(json!({
            "invoice_id": invoice_id,
            "status": st,
            "paid": paid,
            "expired": expired,
            "amount_paid": body["amount_paid"],
            "expires_at": expires_at,
            "balance": balance,
        }))
    }

    pub async fn ppq_history(&self) -> Result<Value> {
        let a = self.ppq()?;
        let (status, body) = self
            .call(
                reqwest::Method::GET,
                &format!("{PPQ_BASE}/queries/history?page_count=50"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("ppq history", status, &body));
        }
        let rows: Vec<Value> = body["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                json!({
                    "at": r["timestamp"],
                    "model": r["model"],
                    "input": r["input_count"],
                    "output": r["output_count"],
                    "cost": r["price_in_usd"],
                    "unit": "usd",
                })
            })
            .collect();
        Ok(json!({ "provider": "ppq", "rows": rows }))
    }

    // --------------------------------------------------------------- routstr

    fn routstr(&self) -> Result<RoutstrAccount> {
        self.snapshot()
            .routstr
            .ok_or_else(|| EngineError::NotFound("no routstr key — pick a node first".into()))
    }

    /// Discover nodes: kind-38421 announcements from the routstr relays,
    /// then probe each https node's `/v1/info` + `/v1/models` concurrently.
    /// The relay REQ is Confirm-gated like any other fetch.
    pub async fn routstr_discover(&self, engine: &crate::engine::Engine) -> Result<Value> {
        let relays: Vec<String> = ROUTSTR_RELAYS.iter().map(|s| s.to_string()).collect();
        let op = engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Custom,
                "routstr: discover provider nodes".into(),
                vec![format!("Query kind {ROUTSTR_ANNOUNCE_KIND} announcements")],
                relays,
            )
            .await
            .map_err(|_| EngineError::Other("relay fetch declined".into()))?;
        let chosen = op.relays().to_vec();
        let filter = json!({ "kinds": [ROUTSTR_ANNOUNCE_KIND], "limit": 300 });
        let auth = engine.relay_auth();
        let fetches = chosen.iter().map(|url| {
            let filter = filter.clone();
            let auth = auth.clone();
            async move {
                tokio::time::timeout(
                    Duration::from_secs(12),
                    crate::relay::fetch_with_filters(engine.ndb(), url, &[filter], &auth),
                )
                .await
            }
        });
        let outcomes = futures::future::join_all(fetches).await;
        let mut announced: BTreeMap<String, NodeRef> = BTreeMap::new();
        let mut newest: BTreeMap<String, u64> = BTreeMap::new();
        let mut event_count = 0usize;
        for o in outcomes {
            let Ok(Ok(outcome)) = o else { continue };
            for ev in outcome.events {
                event_count += 1;
                let tags = ev["tags"].as_array().cloned().unwrap_or_default();
                let tag = |name: &str| {
                    tags.iter().find_map(|t| {
                        let t = t.as_array()?;
                        (t.first()?.as_str()? == name).then(|| t.get(1)?.as_str().map(str::to_string))?
                    })
                };
                let Some(url) = tag("u") else { continue };
                let url = normalize_base(&url);
                if !url.starts_with("https://") {
                    continue;
                }
                let created = ev["created_at"].as_u64().unwrap_or(0);
                if newest.get(&url).map(|c| *c >= created).unwrap_or(false) {
                    continue;
                }
                newest.insert(url.clone(), created);
                announced.insert(
                    url.clone(),
                    NodeRef {
                        url,
                        name: tag("name"),
                        description: tag("description"),
                        pubkey: ev["pubkey"].as_str().map(str::to_string),
                        ..Default::default()
                    },
                );
            }
        }
        op.complete(event_count);

        let mut nodes: Vec<NodeRef> = announced.into_values().collect();
        nodes.truncate(MAX_NODES_PROBED);
        let probes = nodes.into_iter().map(|n| self.routstr_probe(n));
        let mut health: Vec<NodeHealth> = futures::future::join_all(probes).await;
        health.sort_by(|a, b| {
            b.reachable
                .cmp(&a.reachable)
                .then_with(|| b.likely_tools.cmp(&a.likely_tools))
                .then_with(|| a.latency_ms.cmp(&b.latency_ms))
        });
        let current = self.snapshot().routstr.map(|a| a.node.url);
        Ok(json!({
            "announced": health.len(),
            "reachable": health.iter().filter(|h| h.reachable).count(),
            "current": current,
            "nodes": health,
        }))
    }

    /// Probe one node: `/v1/info` + `/v1/models`, with a short timeout.
    pub async fn routstr_probe(&self, mut node: NodeRef) -> NodeHealth {
        let base = normalize_base(&node.url);
        let started = std::time::Instant::now();
        let info = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.call(reqwest::Method::GET, &format!("{base}/v1/info"), &[], None),
        )
        .await;
        let models = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.call(reqwest::Method::GET, &format!("{base}/v1/models"), &[], None),
        )
        .await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let mut error = None;
        match info {
            Ok(Ok((200, body))) => {
                node.name = body["name"].as_str().map(str::to_string).or(node.name);
                node.description = body["description"].as_str().map(str::to_string).or(node.description);
                node.npub = body["npub"].as_str().map(str::to_string);
                node.version = body["version"].as_str().map(str::to_string);
                node.mints = body["mints"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
            }
            Ok(Ok((status, _))) => error = Some(format!("info: HTTP {status}")),
            Ok(Err(e)) => error = Some(e.to_string()),
            Err(_) => error = Some("info: timeout".into()),
        }
        let (model_count, likely_tools) = match models {
            Ok(Ok((200, body))) => {
                let ids: Vec<String> = body["data"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                (ids.len(), ids.iter().any(|id| is_tool_family(id)))
            }
            Ok(Ok((status, _))) => {
                error.get_or_insert(format!("models: HTTP {status}"));
                (0, false)
            }
            Ok(Err(e)) => {
                error.get_or_insert(e.to_string());
                (0, false)
            }
            Err(_) => {
                error.get_or_insert("models: timeout".into());
                (0, false)
            }
        };
        NodeHealth {
            reachable: error.is_none() && model_count > 0,
            node,
            model_count,
            latency_ms,
            likely_tools,
            error,
        }
    }

    /// Turn a Cashu token into a key on `node_url`
    /// (`GET /v1/balance/create?initial_balance_token=…`).
    pub async fn routstr_create(&self, node_url: String, cashu_token: String) -> Result<Value> {
        let token = cashu_token.trim().to_string();
        if !token.starts_with("cashu") {
            return Err(EngineError::BadRequest(
                "that is not a Cashu token (expected it to start with `cashuA` / `cashuB`)".into(),
            ));
        }
        let health = self
            .routstr_probe(NodeRef {
                url: normalize_base(&node_url),
                ..Default::default()
            })
            .await;
        if !health.reachable {
            return Err(EngineError::Relay(format!(
                "node not reachable: {}",
                health.error.unwrap_or_default()
            )));
        }
        let base = normalize_base(&node_url);
        let (status, body) = self
            .call(
                reqwest::Method::GET,
                &format!("{base}/v1/balance/create?initial_balance_token={}", urlenc(&token)),
                &[],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr create key", status, &body));
        }
        let api_key = body["api_key"].as_str().unwrap_or("").to_string();
        if api_key.is_empty() {
            return Err(EngineError::Relay("routstr create key: no api_key in response".into()));
        }
        self.mutate(|s| {
            s.routstr = Some(RoutstrAccount {
                node: health.node.clone(),
                api_key,
                created_at: now(),
                model: None,
                balance: None,
                tool_tests: BTreeMap::new(),
            });
            if s.active.is_none() {
                s.active = Some(ProviderKind::Routstr);
            }
        })?;
        let _ = self.routstr_refresh_balance().await;
        Ok(self.view())
    }

    /// Adopt an existing routstr key on a node. Validated with `/v1/balance/info`.
    pub async fn routstr_adopt(&self, node_url: String, api_key: String) -> Result<Value> {
        let api_key = api_key.trim().to_string();
        if !(api_key.starts_with("sk-") || api_key.starts_with("cashu")) {
            return Err(EngineError::BadRequest(
                "routstr needs an sk- key (or a cashu token) and the node URL".into(),
            ));
        }
        let health = self
            .routstr_probe(NodeRef {
                url: normalize_base(&node_url),
                ..Default::default()
            })
            .await;
        if !health.reachable {
            return Err(EngineError::Relay(format!(
                "node not reachable: {}",
                health.error.unwrap_or_default()
            )));
        }
        let base = normalize_base(&node_url);
        let (status, body) = self
            .call(
                reqwest::Method::GET,
                &format!("{base}/v1/balance/info"),
                &[("Authorization", format!("Bearer {api_key}"))],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr key check", status, &body));
        }
        let balance = balance_from_routstr(&body);
        self.mutate(|s| {
            s.routstr = Some(RoutstrAccount {
                node: health.node.clone(),
                api_key,
                created_at: now(),
                model: None,
                balance: Some(balance),
                tool_tests: BTreeMap::new(),
            });
            if s.active.is_none() {
                s.active = Some(ProviderKind::Routstr);
            }
        })?;
        Ok(self.view())
    }

    pub async fn routstr_refresh_balance(&self) -> Result<Balance> {
        let a = self.routstr()?;
        let base = normalize_base(&a.node.url);
        let res = self
            .call(
                reqwest::Method::GET,
                &format!("{base}/v1/balance/info"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                None,
            )
            .await;
        let balance = match res {
            Ok((status, body)) if (200..300).contains(&status) => balance_from_routstr(&body),
            Ok((status, body)) => Balance {
                error: Some(Self::fail("routstr balance", status, &body).to_string()),
                ..a.balance.clone().unwrap_or_default()
            },
            Err(e) => Balance {
                error: Some(e.to_string()),
                ..a.balance.clone().unwrap_or_default()
            },
        };
        let out = balance.clone();
        self.mutate(|s| {
            if let Some(acc) = s.routstr.as_mut() {
                acc.balance = Some(balance);
            }
        })?;
        if let Some(e) = &out.error {
            return Err(EngineError::Relay(e.clone()));
        }
        Ok(out)
    }

    /// The node's chat models with sats pricing and what we know about tool
    /// calling (self-test result, family heuristic). Nodes publish no
    /// capability data, so nothing is hidden — the UI shows the marks.
    pub async fn routstr_models(&self) -> Result<Value> {
        let a = self.routstr()?;
        let base = normalize_base(&a.node.url);
        let (status, body) = self
            .call(reqwest::Method::GET, &format!("{base}/v1/models"), &[], None)
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr models", status, &body));
        }
        let mut models: Vec<Value> = body["data"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|m| {
                let modality = m["architecture"]["modality"].as_str().unwrap_or("");
                modality.is_empty() || modality.ends_with("->text")
            })
            .map(|m| {
                let id = m["id"].as_str().unwrap_or("").to_string();
                let sp = &m["sats_pricing"];
                let per_1k = |k: &str| sp[k].as_f64().map(|v| v * 1000.0);
                json!({
                    "id": id,
                    "name": m["name"],
                    "context_length": m["context_length"],
                    "sat_prompt_per_1k": per_1k("prompt"),
                    "sat_completion_per_1k": per_1k("completion"),
                    "max_cost_sat": sp["max_cost"],
                    "likely_tools": is_tool_family(&id),
                    "tool_test": a.tool_tests.get(&id),
                    "selected": Some(&id) == a.model.as_ref(),
                })
            })
            .collect();
        models.sort_by(|x, y| {
            let rank = |m: &Value| match (m["tool_test"].as_str(), m["likely_tools"].as_bool()) {
                (Some("ok"), _) => 0,
                (None, Some(true)) => 1,
                (None, _) => 2,
                (Some(_), _) => 3,
            };
            rank(x)
                .cmp(&rank(y))
                .then_with(|| x["id"].as_str().cmp(&y["id"].as_str()))
        });
        Ok(json!({
            "provider": "routstr",
            "node": a.node,
            "models": models,
            "note": "This node doesn't say which of its models can call tools. Pick one you know supports tool calling — Claude, GPT, Gemini, GLM, DeepSeek and Qwen families usually do; small or older chat models often don't.",
        }))
    }

    pub async fn routstr_topup(&self, cashu_token: String) -> Result<Value> {
        let a = self.routstr()?;
        let token = cashu_token.trim().to_string();
        if !token.starts_with("cashu") {
            return Err(EngineError::BadRequest("that is not a Cashu token".into()));
        }
        let base = normalize_base(&a.node.url);
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("{base}/v1/balance/topup"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                Some(json!({ "cashu_token": token })),
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr top up", status, &body));
        }
        let balance = self.routstr_refresh_balance().await.ok();
        Ok(json!({ "ok": true, "response": body, "balance": balance }))
    }

    /// Drain the key back into a Cashu token. The token is returned ONCE and
    /// never stored — the caller shows it for the user to paste into a wallet.
    pub async fn routstr_refund(&self) -> Result<Value> {
        let a = self.routstr()?;
        let base = normalize_base(&a.node.url);
        let (status, body) = self
            .call(
                reqwest::Method::POST,
                &format!("{base}/v1/balance/refund"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr withdraw", status, &body));
        }
        let token = body["token"]
            .as_str()
            .or_else(|| body["cashu_token"].as_str())
            .map(str::to_string);
        let _ = self.routstr_refresh_balance().await;
        Ok(json!({
            "token": token,
            "sats": body["sats"].as_f64().or_else(|| body["amount"].as_f64()),
            "response_keys": body.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
        }))
    }

    pub async fn routstr_history(&self) -> Result<Value> {
        let a = self.routstr()?;
        let base = normalize_base(&a.node.url);
        let (status, body) = self
            .call(
                reqwest::Method::GET,
                &format!("{base}/v1/balance/history"),
                &[("Authorization", format!("Bearer {}", a.api_key))],
                None,
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(Self::fail("routstr history", status, &body));
        }
        let rows: Vec<Value> = body["transactions"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|t| {
                json!({
                    "at": t["created_at"],
                    "type": t["type"],
                    "source": t["source"],
                    "amount": t["amount"],
                    "unit": t["unit"],
                    "mint": t["mint_url"],
                })
            })
            .collect();
        Ok(json!({ "provider": "routstr", "rows": rows }))
    }

    // ------------------------------------------------------------ self-test

    /// One tiny request with a single trivial tool. `Ok` when the model
    /// returns a tool call; the outcome is remembered per node + model.
    pub async fn tool_self_test(&self, kind: ProviderKind, model: &str) -> Result<Value> {
        let provider: Arc<dyn LLMProvider> = match kind {
            ProviderKind::Ppq => {
                let a = self.ppq()?;
                Arc::new(OpenAiCompatProvider::new(
                    "ppq",
                    format!("{PPQ_BASE}/v1"),
                    a.api_key,
                    model.to_string(),
                ))
            }
            ProviderKind::Routstr => {
                let a = self.routstr()?;
                Arc::new(OpenAiCompatProvider::new(
                    "routstr",
                    format!("{}/v1", normalize_base(&a.node.url)),
                    a.api_key,
                    model.to_string(),
                ))
            }
        };
        let tools = vec![ToolDefinition {
            name: "ping".into(),
            description: "Call this tool to confirm that tool calling works. Takes one word.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "word": { "type": "string" } },
                "required": ["word"],
                "additionalProperties": false
            }),
        }];
        let messages = vec![
            AgentMessage::System("You are being tested for tool-calling support.".into()),
            AgentMessage::User("Call the `ping` tool now with the word \"pong\". Do not answer in text.".into()),
        ];
        let started = std::time::Instant::now();
        let out = provider.run_turn(&messages, &tools, Some(model)).await;
        let (ok, detail) = match &out {
            Ok(t) => {
                let called = t
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { name, .. } if name == "ping"));
                let detail = if called {
                    "tool call returned".to_string()
                } else {
                    let text: String = t
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!(
                        "no tool call ({}): {}",
                        match t.stop_reason {
                            StopReason::EndTurn => "end_turn",
                            StopReason::ToolUse => "tool_use",
                            StopReason::MaxTokens => "max_tokens",
                            StopReason::Other(_) => "other",
                        },
                        text.chars().take(160).collect::<String>()
                    )
                };
                (called, detail)
            }
            Err(e) => (false, e.to_string()),
        };
        if kind == ProviderKind::Routstr {
            self.mutate(|s| {
                if let Some(a) = s.routstr.as_mut() {
                    a.tool_tests.insert(
                        model.to_string(),
                        if ok { ToolTest::Ok } else { ToolTest::Failed },
                    );
                }
            })?;
        }
        Ok(json!({
            "provider": kind,
            "model": model,
            "ok": ok,
            "detail": detail,
            "seconds": started.elapsed().as_secs_f64(),
        }))
    }
}

fn status_of(balance: Option<&Balance>, ceiling_in_unit: f64) -> &'static str {
    match balance {
        None => "unfunded",
        Some(b) if b.error.is_some() => "offline",
        Some(b) if b.amount <= 0.0 => "unfunded",
        Some(b) if b.amount < ceiling_in_unit => "low",
        Some(_) => "funded",
    }
}

fn balance_from_routstr(body: &Value) -> Balance {
    Balance {
        unit: "msat".into(),
        amount: body["balance"].as_f64().unwrap_or(0.0),
        reserved: body["reserved"].as_f64(),
        total_spent: body["total_spent"].as_f64(),
        fetched_at: now(),
        error: None,
    }
}

fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// HTTP handlers (mounted by server.rs on the chat router's state)
// ---------------------------------------------------------------------------

pub mod handlers {
    use super::*;
    use crate::api::ChatAppState;
    use axum::extract::{Path as AxPath, State};
    use axum::Json;

    type R = std::result::Result<Json<Value>, EngineError>;

    fn kind(s: &str) -> Result<ProviderKind> {
        ProviderKind::parse(s)
            .ok_or_else(|| EngineError::BadRequest(format!("unknown provider `{s}`")))
    }

    pub async fn state(State(st): State<ChatAppState>) -> Json<Value> {
        Json(st.providers.view())
    }

    #[derive(Deserialize)]
    pub struct SelectReq {
        pub provider: String,
        #[serde(default)]
        pub model: Option<String>,
    }
    pub async fn select(State(st): State<ChatAppState>, Json(req): Json<SelectReq>) -> R {
        st.providers.select(kind(&req.provider)?, req.model)?;
        Ok(Json(st.providers.view()))
    }

    #[derive(Deserialize)]
    pub struct CeilingReq {
        #[serde(default)]
        pub ceiling_usd: Option<f64>,
        #[serde(default)]
        pub ceiling_sats: Option<f64>,
    }
    pub async fn ceiling(State(st): State<ChatAppState>, Json(req): Json<CeilingReq>) -> R {
        st.providers.set_ceilings(req.ceiling_usd, req.ceiling_sats)?;
        Ok(Json(st.providers.view()))
    }

    pub async fn forget(State(st): State<ChatAppState>, AxPath(p): AxPath<String>) -> R {
        st.providers.forget(kind(&p)?)?;
        Ok(Json(st.providers.view()))
    }

    pub async fn reveal(State(st): State<ChatAppState>, AxPath(p): AxPath<String>) -> R {
        Ok(Json(st.providers.reveal(kind(&p)?)?))
    }

    pub async fn refresh(State(st): State<ChatAppState>, AxPath(p): AxPath<String>) -> R {
        let k = kind(&p)?;
        let _ = st.providers.refresh_balance(k).await;
        Ok(Json(st.providers.view()))
    }

    pub async fn models(State(st): State<ChatAppState>, AxPath(p): AxPath<String>) -> R {
        Ok(Json(match kind(&p)? {
            ProviderKind::Ppq => st.providers.ppq_models().await?,
            ProviderKind::Routstr => st.providers.routstr_models().await?,
        }))
    }

    pub async fn ledger(State(st): State<ChatAppState>, AxPath(p): AxPath<String>) -> R {
        Ok(Json(match kind(&p)? {
            ProviderKind::Ppq => st.providers.ppq_history().await?,
            ProviderKind::Routstr => st.providers.routstr_history().await?,
        }))
    }

    #[derive(Deserialize)]
    pub struct SelfTestReq {
        pub provider: String,
        pub model: String,
    }
    pub async fn self_test(State(st): State<ChatAppState>, Json(req): Json<SelfTestReq>) -> R {
        Ok(Json(
            st.providers
                .tool_self_test(kind(&req.provider)?, &req.model)
                .await?,
        ))
    }

    // ppq
    pub async fn ppq_setup(State(st): State<ChatAppState>) -> R {
        Ok(Json(st.providers.ppq_setup().await?))
    }
    #[derive(Deserialize)]
    pub struct PpqAdoptReq {
        pub api_key: String,
        pub credit_id: String,
    }
    pub async fn ppq_adopt(State(st): State<ChatAppState>, Json(req): Json<PpqAdoptReq>) -> R {
        Ok(Json(st.providers.ppq_adopt(req.api_key, req.credit_id).await?))
    }
    #[derive(Deserialize)]
    pub struct FundReq {
        pub amount_sats: u64,
    }
    pub async fn ppq_fund(State(st): State<ChatAppState>, Json(req): Json<FundReq>) -> R {
        Ok(Json(st.providers.ppq_create_invoice(req.amount_sats).await?))
    }
    pub async fn ppq_fund_status(State(st): State<ChatAppState>, AxPath(id): AxPath<String>) -> R {
        Ok(Json(st.providers.ppq_invoice_status(&id).await?))
    }

    // routstr
    pub async fn routstr_discover(State(st): State<ChatAppState>) -> R {
        Ok(Json(st.providers.routstr_discover(&st.engine).await?))
    }
    #[derive(Deserialize)]
    pub struct ProbeReq {
        pub node_url: String,
    }
    pub async fn routstr_probe(State(st): State<ChatAppState>, Json(req): Json<ProbeReq>) -> R {
        let h = st
            .providers
            .routstr_probe(NodeRef {
                url: normalize_base(&req.node_url),
                ..Default::default()
            })
            .await;
        Ok(Json(serde_json::to_value(h)?))
    }
    #[derive(Deserialize)]
    pub struct RoutstrCreateReq {
        pub node_url: String,
        pub cashu_token: String,
    }
    pub async fn routstr_create(State(st): State<ChatAppState>, Json(req): Json<RoutstrCreateReq>) -> R {
        Ok(Json(st.providers.routstr_create(req.node_url, req.cashu_token).await?))
    }
    #[derive(Deserialize)]
    pub struct RoutstrAdoptReq {
        pub node_url: String,
        pub api_key: String,
    }
    pub async fn routstr_adopt(State(st): State<ChatAppState>, Json(req): Json<RoutstrAdoptReq>) -> R {
        Ok(Json(st.providers.routstr_adopt(req.node_url, req.api_key).await?))
    }
    #[derive(Deserialize)]
    pub struct TopupReq {
        pub cashu_token: String,
    }
    pub async fn routstr_topup(State(st): State<ChatAppState>, Json(req): Json<TopupReq>) -> R {
        Ok(Json(st.providers.routstr_topup(req.cashu_token).await?))
    }
    pub async fn routstr_refund(State(st): State<ChatAppState>) -> R {
        Ok(Json(st.providers.routstr_refund().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_roundtrip_and_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let reg = ProviderRegistry::new(dir.path()).unwrap();
        assert!(reg.snapshot().ppq.is_none());
        reg.mutate(|s| {
            s.ppq = Some(PpqAccount {
                credit_id: "cid".into(),
                api_key: "sk-abcdefghijklmnop".into(),
                created_at: 1,
                model: None,
                balance: None,
            });
            s.active = Some(ProviderKind::Ppq);
        })
        .unwrap();
        let again = ProviderRegistry::new(dir.path()).unwrap();
        let v = again.view();
        assert_eq!(v["active"], "ppq");
        assert_eq!(v["ppq"]["api_key"], "sk-abc…(19)");
        assert_eq!(v["ppq"]["status"], "unfunded");
        assert!(v["configured"].as_bool().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("providers.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn forget_moves_active() {
        let dir = tempfile::tempdir().unwrap();
        let reg = ProviderRegistry::new(dir.path()).unwrap();
        reg.mutate(|s| {
            s.ppq = Some(PpqAccount {
                credit_id: "c".into(),
                api_key: "sk-1".into(),
                created_at: 1,
                model: None,
                balance: None,
            });
            s.routstr = Some(RoutstrAccount {
                node: NodeRef { url: "https://n".into(), ..Default::default() },
                api_key: "sk-2".into(),
                created_at: 1,
                model: Some("claude-fable-5".into()),
                balance: None,
                tool_tests: BTreeMap::new(),
            });
            s.active = Some(ProviderKind::Ppq);
        })
        .unwrap();
        reg.forget(ProviderKind::Ppq).unwrap();
        assert_eq!(reg.snapshot().active, Some(ProviderKind::Routstr));
        let (_, model, kind) = reg.active_provider().unwrap();
        assert_eq!(kind, ProviderKind::Routstr);
        assert_eq!(model, "claude-fable-5");
        reg.forget(ProviderKind::Routstr).unwrap();
        assert!(reg.snapshot().active.is_none());
        assert!(reg.active_provider().is_none());
    }

    #[test]
    fn fresh_state_has_default_ceilings() {
        let dir = tempfile::tempdir().unwrap();
        let reg = ProviderRegistry::new(dir.path()).unwrap();
        let v = reg.view();
        assert_eq!(v["ceiling_usd"], 0.25);
        assert_eq!(v["ceiling_sats"], 250.0);
    }

    #[test]
    fn status_thresholds() {
        let b = |amount: f64| Balance { unit: "usd".into(), amount, fetched_at: 0, ..Default::default() };
        assert_eq!(status_of(None, 0.25), "unfunded");
        assert_eq!(status_of(Some(&b(0.0)), 0.25), "unfunded");
        assert_eq!(status_of(Some(&b(0.1)), 0.25), "low");
        assert_eq!(status_of(Some(&b(1.0)), 0.25), "funded");
        let off = Balance { error: Some("x".into()), ..b(1.0) };
        assert_eq!(status_of(Some(&off), 0.25), "offline");
    }

    #[test]
    fn tool_family_heuristic() {
        assert!(is_tool_family("claude-fable-5"));
        assert!(is_tool_family("gemini-3-6-flash"));
        assert!(!is_tool_family("mimo-v2.6-pro-ultraspeed"));
        assert_eq!(urlenc("cashuA+/="), "cashuA%2B%2F%3D");
    }
}
