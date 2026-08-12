//! Network mode and fetch activity tracking
//!
//! Provides an engine-level Online/Offline toggle and a ring buffer
//! of recent relay fetch activity so users can see exactly when
//! the engine is talking to relays.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

const MAX_LOG_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// Fetch reason — task-local cause attribution
// ---------------------------------------------------------------------------

tokio::task_local! {
    /// The human reason for relay fetches issued inside the current task —
    /// "Resolve wiki links", "Search: k:30818 …", "Profile prefetch". Set by
    /// API handlers via [`with_fetch_reason`]; read by [`NetworkActivity::
    /// begin_fetch`] so every FetchRecord carries its cause without threading
    /// a parameter through every fetch layer. Fetches spawned into detached
    /// tasks fall back to `None` (attribute those at their spawn site).
    static FETCH_REASON: String;
}

/// Run `f` with `reason` attached to every relay fetch it performs.
pub async fn with_fetch_reason<F>(reason: impl Into<String>, f: F) -> F::Output
where
    F: std::future::Future,
{
    FETCH_REASON.scope(reason.into(), f).await
}

/// The reason attached to the current task, if any.
pub fn current_fetch_reason() -> Option<String> {
    FETCH_REASON.try_with(|r| r.clone()).ok()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The engine's stance toward relay fetches.
///
/// - `Auto` — fetch from relays automatically (the former "online").
/// - `Confirm` — every user-initiated relay fetch emits an intent the
///   UI must approve before the engine proceeds (the former "offline",
///   reframed: instead of silently suppressing, the engine asks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Auto,
    Confirm,
}

impl std::str::FromStr for NetworkMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            // `online`/`offline` are accepted as aliases so configs and
            // clients written before the rename keep working.
            "auto" | "online" => Ok(NetworkMode::Auto),
            "confirm" | "offline" => Ok(NetworkMode::Confirm),
            _ => Err(format!("Unknown network mode: {s}. Valid: auto, confirm")),
        }
    }
}

impl std::fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkMode::Auto => write!(f, "auto"),
            NetworkMode::Confirm => write!(f, "confirm"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchTrigger {
    BackgroundSync,
    UserAction,
    LocalFirst,
    FetchAlways,
    ProfilePrefetch,
}

impl std::fmt::Display for FetchTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchTrigger::BackgroundSync => write!(f, "background_sync"),
            FetchTrigger::UserAction => write!(f, "user_action"),
            FetchTrigger::LocalFirst => write!(f, "local_first"),
            FetchTrigger::FetchAlways => write!(f, "fetch_always"),
            FetchTrigger::ProfilePrefetch => write!(f, "profile_prefetch"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRecord {
    pub id: u64,
    pub relay: String,
    pub filter_summary: String,
    pub event_count: usize,
    pub duration_ms: u64,
    pub trigger: FetchTrigger,
    pub timestamp: u64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The human cause of this fetch ("Resolve wiki links", "Search: …"),
    /// captured from the task-local set by [`with_fetch_reason`]. `None` for
    /// fetches whose call path never attached one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// An in-flight relay fetch — the live counterpart of [`FetchRecord`],
/// listed in [`NetworkStatus::active`] so the UI can show (and kill) what
/// the engine is pulling right now.
#[derive(Debug, Clone, Serialize)]
pub struct ActiveFetchInfo {
    pub id: u64,
    pub relay: String,
    pub filter_summary: String,
    pub trigger: FetchTrigger,
    /// Unix seconds when the fetch started.
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NetworkStatus {
    pub mode: NetworkMode,
    /// `false` until the user has made an explicit first-run network-mode
    /// choice. The frontend shows the one-time "choose your default mode"
    /// modal — and suppresses the cold-cache feed fetch — while this is false.
    pub mode_chosen: bool,
    pub active_fetches: u64,
    pub total_events_fetched: u64,
    pub last_fetch_timestamp: u64,
    /// In-flight fetches, oldest first — each row is individually killable
    /// via POST /network/fetch-kill.
    pub active: Vec<ActiveFetchInfo>,
    pub recent: Vec<FetchRecord>,
}

// ---------------------------------------------------------------------------
// NetworkActivity — the shared tracker
// ---------------------------------------------------------------------------

pub struct NetworkActivity {
    mode: AtomicBool, // true = auto
    /// Whether the user has made an explicit first-run mode choice. Seeded
    /// from `[network] mode_chosen` in config.toml; flipped true (and
    /// persisted) the first time the mode is set through the UI.
    mode_chosen: AtomicBool,
    log: Mutex<VecDeque<FetchRecord>>,
    active_fetches: AtomicU64,
    /// Live registry of in-flight fetches: metadata for the status endpoint
    /// plus each fetch's cancellation token for the kill switch.
    active: Mutex<HashMap<u64, (ActiveFetchInfo, CancellationToken)>>,
    next_id: AtomicU64,
    total_events_fetched: AtomicU64,
    last_fetch_timestamp: AtomicU64,
    /// Broadcast of fetch-operation events to SSE subscribers.
    events: broadcast::Sender<FetchEvent>,
    /// Operations awaiting a confirm decision, keyed by operation_id.
    pending: tokio::sync::Mutex<HashMap<String, oneshot::Sender<ConfirmDecision>>>,
}

impl NetworkActivity {
    pub fn new(initial_mode: NetworkMode) -> Self {
        let (events, _) = broadcast::channel(FETCH_EVENT_CAP);
        Self {
            mode: AtomicBool::new(matches!(initial_mode, NetworkMode::Auto)),
            mode_chosen: AtomicBool::new(false),
            log: Mutex::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)),
            active_fetches: AtomicU64::new(0),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            total_events_fetched: AtomicU64::new(0),
            last_fetch_timestamp: AtomicU64::new(0),
            events,
            pending: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn is_auto(&self) -> bool {
        self.mode.load(Ordering::Relaxed)
    }

    pub fn mode(&self) -> NetworkMode {
        if self.is_auto() {
            NetworkMode::Auto
        } else {
            NetworkMode::Confirm
        }
    }

    pub fn set_mode(&self, mode: NetworkMode) {
        self.mode
            .store(matches!(mode, NetworkMode::Auto), Ordering::Relaxed);
    }

    /// Whether the user has made an explicit first-run mode choice.
    pub fn mode_chosen(&self) -> bool {
        self.mode_chosen.load(Ordering::Relaxed)
    }

    /// Record whether the mode has been explicitly chosen (seeded from config
    /// at boot, set true on the first UI-driven mode change).
    pub fn set_mode_chosen(&self, chosen: bool) {
        self.mode_chosen.store(chosen, Ordering::Relaxed);
    }

    pub fn active_count(&self) -> u64 {
        self.active_fetches.load(Ordering::Relaxed)
    }

    /// Begin tracking a fetch. Returns a guard that records the result
    /// on completion. The guard decrements active_fetches on Drop even
    /// if the future is cancelled.
    pub fn begin_fetch(
        self: &std::sync::Arc<Self>,
        relay: &str,
        filter_summary: String,
        trigger: FetchTrigger,
    ) -> FetchGuard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.active_fetches.fetch_add(1, Ordering::Relaxed);
        let reason = current_fetch_reason();
        let token = CancellationToken::new();
        if let Ok(mut active) = self.active.lock() {
            active.insert(
                id,
                (
                    ActiveFetchInfo {
                        id,
                        relay: relay.to_string(),
                        filter_summary: filter_summary.clone(),
                        trigger: trigger.clone(),
                        started_at: now_unix(),
                        reason: reason.clone(),
                    },
                    token.clone(),
                ),
            );
        }
        FetchGuard {
            activity: std::sync::Arc::clone(self),
            id,
            relay: relay.to_string(),
            filter_summary,
            trigger,
            reason,
            token,
            start: Instant::now(),
            completed: false,
        }
    }

    /// Snapshot of in-flight fetches, oldest first.
    pub fn active_fetch_list(&self) -> Vec<ActiveFetchInfo> {
        let mut list: Vec<ActiveFetchInfo> = self
            .active
            .lock()
            .map(|a| a.values().map(|(info, _)| info.clone()).collect())
            .unwrap_or_default();
        list.sort_by_key(|f| f.id);
        list
    }

    /// Kill one in-flight fetch by id. Returns whether it was found live.
    pub fn kill_fetch(&self, id: u64) -> bool {
        match self.active.lock() {
            Ok(active) => match active.get(&id) {
                Some((_, token)) => {
                    token.cancel();
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }

    /// Kill every in-flight fetch. Returns how many were signalled.
    pub fn kill_all_fetches(&self) -> usize {
        match self.active.lock() {
            Ok(active) => {
                for (_, token) in active.values() {
                    token.cancel();
                }
                active.len()
            }
            Err(_) => 0,
        }
    }

    fn remove_active(&self, id: u64) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&id);
        }
    }

    fn record(&self, record: FetchRecord) {
        self.total_events_fetched
            .fetch_add(record.event_count as u64, Ordering::Relaxed);
        self.last_fetch_timestamp
            .store(record.timestamp, Ordering::Relaxed);

        if let Ok(mut log) = self.log.lock() {
            if log.len() >= MAX_LOG_ENTRIES {
                log.pop_front();
            }
            log.push_back(record);
        }
    }

    pub fn status(&self) -> NetworkStatus {
        let recent = if let Ok(log) = self.log.lock() {
            log.iter().rev().cloned().collect()
        } else {
            vec![]
        };
        NetworkStatus {
            mode: self.mode(),
            mode_chosen: self.mode_chosen(),
            active_fetches: self.active_fetches.load(Ordering::Relaxed),
            total_events_fetched: self.total_events_fetched.load(Ordering::Relaxed),
            last_fetch_timestamp: self.last_fetch_timestamp.load(Ordering::Relaxed),
            active: self.active_fetch_list(),
            recent,
        }
    }

    /// Subscribe to the fetch-event stream — one receiver per SSE client.
    pub fn subscribe_fetch_events(&self) -> broadcast::Receiver<FetchEvent> {
        self.events.subscribe()
    }

    /// Emit a fetch event. A send error only means no subscribers — the
    /// events are advisory, so that's fine.
    fn emit(&self, ev: FetchEvent) {
        let _ = self.events.send(ev);
    }

    /// Resolve a pending confirm intent with the UI's decision. Returns
    /// false when no operation is awaiting that id (already resolved or
    /// timed out).
    pub async fn resolve_confirm(&self, decision: ConfirmDecision) -> bool {
        let waiter = self.pending.lock().await.remove(&decision.operation_id);
        match waiter {
            Some(tx) => tx.send(decision).is_ok(),
            None => false,
        }
    }

    /// Open a fetch operation. Emits an `Intent`; in Auto mode returns
    /// immediately, in Confirm mode registers a oneshot and awaits the
    /// UI's decision (`CONFIRM_TIMEOUT` → treated as cancelled).
    pub async fn begin_operation(
        self: &std::sync::Arc<Self>,
        pattern: FetchPattern,
        label: String,
        steps: Vec<String>,
        relays: Vec<String>,
    ) -> std::result::Result<FetchOperation, FetchCancelled> {
        self.begin_operation_with_summary(pattern, label, steps, relays, None)
            .await
    }

    /// Like `begin_operation` but also carries a structured
    /// `RequestSummary` (filters + composition + DSL sentence) on the
    /// emitted `Intent`. The UI uses this for the expandable toast and
    /// the confirm modal preview. New callers should prefer this form;
    /// `begin_operation` is the back-compat shim for sites that haven't
    /// yet built a summary.
    pub async fn begin_operation_with_summary(
        self: &std::sync::Arc<Self>,
        pattern: FetchPattern,
        label: String,
        steps: Vec<String>,
        relays: Vec<String>,
        summary: Option<RequestSummary>,
    ) -> std::result::Result<FetchOperation, FetchCancelled> {
        let operation_id = next_operation_id();
        let needs_confirmation = !self.is_auto();
        self.emit(FetchEvent::Intent {
            operation_id: operation_id.clone(),
            pattern,
            label,
            steps,
            relays: relays.clone(),
            needs_confirmation,
            summary,
        });

        let chosen = if needs_confirmation {
            let rx = {
                let (tx, rx) = oneshot::channel();
                self.pending.lock().await.insert(operation_id.clone(), tx);
                rx
            };
            match tokio::time::timeout(CONFIRM_TIMEOUT, rx).await {
                Ok(Ok(d)) if d.approved => d.relays.unwrap_or(relays),
                _ => {
                    self.pending.lock().await.remove(&operation_id);
                    self.emit(FetchEvent::Failed {
                        operation_id,
                        error: "cancelled".into(),
                    });
                    return Err(FetchCancelled);
                }
            }
        } else {
            relays
        };

        Ok(FetchOperation {
            activity: std::sync::Arc::clone(self),
            operation_id,
            relays: chosen,
        })
    }

    /// Open a publish operation. Mirrors `begin_operation` but emits a
    /// `PublishIntent` event (carries the event IDs being published so
    /// the UI can correlate per-relay `Accepted`/`Rejected` status).
    /// Same Confirm-mode gating as fetches.
    pub async fn begin_publish_operation(
        self: &std::sync::Arc<Self>,
        label: String,
        relays: Vec<String>,
        event_ids: Vec<String>,
        summary: Option<RequestSummary>,
        manifest: Option<PublishManifest>,
    ) -> std::result::Result<PublishOperation, FetchCancelled> {
        let operation_id = next_operation_id();
        let needs_confirmation = !self.is_auto();
        self.emit(FetchEvent::PublishIntent {
            operation_id: operation_id.clone(),
            label,
            relays: relays.clone(),
            event_ids,
            needs_confirmation,
            summary,
            manifest,
        });

        let chosen = if needs_confirmation {
            let rx = {
                let (tx, rx) = oneshot::channel();
                self.pending.lock().await.insert(operation_id.clone(), tx);
                rx
            };
            match tokio::time::timeout(CONFIRM_TIMEOUT, rx).await {
                Ok(Ok(d)) if d.approved => d.relays.unwrap_or(relays),
                _ => {
                    self.pending.lock().await.remove(&operation_id);
                    self.emit(FetchEvent::Failed {
                        operation_id,
                        error: "cancelled".into(),
                    });
                    return Err(FetchCancelled);
                }
            }
        } else {
            relays
        };

        Ok(PublishOperation {
            activity: std::sync::Arc::clone(self),
            operation_id,
            relays: chosen,
        })
    }
}

// ---------------------------------------------------------------------------
// FetchGuard — RAII tracker for individual fetches
// ---------------------------------------------------------------------------

pub struct FetchGuard {
    activity: std::sync::Arc<NetworkActivity>,
    id: u64,
    relay: String,
    filter_summary: String,
    trigger: FetchTrigger,
    reason: Option<String>,
    token: CancellationToken,
    start: Instant,
    completed: bool,
}

impl FetchGuard {
    /// Resolves when this fetch is killed (POST /network/fetch-kill) —
    /// `select!` it against the relay work.
    pub async fn cancelled(&self) {
        self.token.cancelled().await
    }

    pub fn complete(mut self, event_count: usize) {
        self.completed = true;
        self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
        self.activity.remove_active(self.id);
        let record = FetchRecord {
            id: self.id,
            relay: self.relay.clone(),
            filter_summary: self.filter_summary.clone(),
            event_count,
            duration_ms: self.start.elapsed().as_millis() as u64,
            trigger: self.trigger.clone(),
            timestamp: now_unix(),
            success: true,
            error: None,
            reason: self.reason.clone(),
        };
        self.activity.record(record);
    }

    pub fn fail(mut self, error: String) {
        self.completed = true;
        self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
        self.activity.remove_active(self.id);
        let record = FetchRecord {
            id: self.id,
            relay: self.relay.clone(),
            filter_summary: self.filter_summary.clone(),
            event_count: 0,
            duration_ms: self.start.elapsed().as_millis() as u64,
            trigger: self.trigger.clone(),
            timestamp: now_unix(),
            success: false,
            error: Some(error),
            reason: self.reason.clone(),
        };
        self.activity.record(record);
    }
}

impl Drop for FetchGuard {
    fn drop(&mut self) {
        if !self.completed {
            // Future was cancelled — still decrement active count
            self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
            self.activity.remove_active(self.id);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Produce a human-readable summary of NIP-01 filters for the activity log.
pub fn summarize_filters(filters: &[Value]) -> String {
    let mut parts = Vec::new();
    for f in filters {
        if let Some(kinds) = f.get("kinds").and_then(|v| v.as_array()) {
            let ks: Vec<String> = kinds.iter().filter_map(|k| k.as_u64().map(|n| n.to_string())).collect();
            if !ks.is_empty() {
                parts.push(format!("kinds=[{}]", ks.join(",")));
            }
        }
        if let Some(authors) = f.get("authors").and_then(|v| v.as_array()) {
            if !authors.is_empty() {
                parts.push(format!("authors={}", authors.len()));
            }
        }
        if let Some(ids) = f.get("ids").and_then(|v| v.as_array()) {
            if !ids.is_empty() {
                parts.push(format!("ids={}", ids.len()));
            }
        }
        // Every tag filter, generically — the old #d-only special case left
        // e.g. a wiki lookup's `#T` half rendering as a bare kinds=[30040,
        // 30041], which read as "why is it fetching sections?" in the panel.
        if let Some(obj) = f.as_object() {
            for (key, value) in obj {
                if let Some(values) = key.strip_prefix('#').and(value.as_array()) {
                    if !values.is_empty() {
                        parts.push(format!("{}={}", key, values.len()));
                    }
                }
            }
        }
        if let Some(limit) = f.get("limit").and_then(|v| v.as_u64()) {
            parts.push(format!("limit={}", limit));
        }
    }
    if parts.is_empty() {
        "no filters".to_string()
    } else {
        parts.join(" ")
    }
}

// ---------------------------------------------------------------------------
// Fetch operations — engine-driven confirm + progress channel
// ---------------------------------------------------------------------------

/// Capacity of the fetch-event broadcast buffer. A subscriber that
/// falls this far behind drops intermediate events — fine, since each
/// event carries the full state the UI needs.
const FETCH_EVENT_CAP: usize = 256;

/// How long the engine waits for the UI to answer a confirm intent
/// before treating the operation as cancelled.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// The shape of a user-initiated fetch operation — drives the canned
/// step description shown to the user before they approve it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchPattern {
    Event,
    Publication,
    Thread,
    Search,
    Profile,
    Custom,
}

/// Which class of relay a fetch/publish member targets — used as the
/// `phase` tag on `RelayStatus` events so the UI can group per-relay
/// status under the right execution stage. Includes the dot-notation
/// values from the DSL surface (`indexer.default`, `search.fallback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[serde(rename = "read")] Read,
    #[serde(rename = "write")] Write,
    #[serde(rename = "publish")] Publish,
    #[serde(rename = "broadcast")] Broadcast,
    #[serde(rename = "search.default")] SearchDefault,
    #[serde(rename = "search.fallback")] SearchFallback,
    #[serde(rename = "indexer.default")] IndexerDefault,
    #[serde(rename = "indexer.fallback")] IndexerFallback,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Read => "read",
            Phase::Write => "write",
            Phase::Publish => "publish",
            Phase::Broadcast => "broadcast",
            Phase::SearchDefault => "search.default",
            Phase::SearchFallback => "search.fallback",
            Phase::IndexerDefault => "indexer.default",
            Phase::IndexerFallback => "indexer.fallback",
        }
    }
}

/// Per-relay status snapshot emitted as a relay transitions through the
/// fetch/publish lifecycle. The UI uses these to drive the per-relay
/// dots in the expandable toast view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayStatusValue {
    /// WebSocket dialing or REQ sent, awaiting first event/EOSE.
    Connecting,
    /// EOSE received from this relay (NIP-01 end-of-stored-events).
    Eose { event_count: usize },
    /// Connection or protocol error.
    Error { msg: String },
    /// Hit the operation timeout before EOSE / OK.
    Timeout,
    /// Publish path — relay sent `["OK", id, true, ...]`.
    Accepted,
    /// Publish path — relay sent `["OK", id, false, msg]`.
    Rejected { msg: String },
}

/// A subset of NIP-01 filter fields, expressed structurally so the UI
/// can render each clause as a row in the expanded toast view. Not a
/// re-implementation of nostrdb's filter type — just the shape needed
/// for display + the confirm modal. All fields optional; absence
/// means "no constraint on this dimension".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NipFilter {
    #[serde(skip_serializing_if = "Option::is_none")] pub kinds: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub since: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub until: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub search: Option<String>,
    /// Generic tag filters (`#e`, `#p`, `#d`, `#a`, etc.) → list of values.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub tags: std::collections::BTreeMap<String, Vec<String>>,
}

/// One execution stage of a fetch/publish composition. Members of a
/// stage fire concurrently (relay fan-out). Stages run in order; the
/// next stage only starts when the previous returns zero events
/// (sequential — typical fallback), UNLESS `start_delay_ms > 0`, in
/// which case it begins partway through the previous stage (overlap
/// — search default + delayed fallback).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseStage {
    /// Human label rendered in the DSL sentence — `"primary"`,
    /// `"fallback"`, `"delayed-fallback"`, etc.
    pub label: String,
    /// `(phase, relays)` pairs fired concurrently. With `exclusive=off`
    /// + a default discovery class, this can be `[(Read, …), (IndexerDefault, …)]`.
    pub members: Vec<(Phase, Vec<String>)>,
    /// Start-of-stage delay relative to the prior stage (0 = wait for
    /// the prior to return zero events; >0 = overlapping start used by
    /// search Δ).
    pub start_delay_ms: u64,
}

/// How the relay-set fan-out is composed. Always a sequence of stages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompositionShape {
    pub phases: Vec<PhaseStage>,
}

/// Structured summary of a relay request — the formal-language form.
/// Travels with `Intent` and `PublishIntent` events; the UI renders the
/// canonical `dsl` string in toasts, the `filters` block in the
/// expanded view, and the `composition` shape in the confirm modal.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestSummary {
    /// NIP-01 filters as sent on the wire (one per REQ subscription).
    pub filters: Vec<NipFilter>,
    /// How per-phase relay sets compose into an execution plan.
    pub composition: CompositionShape,
    /// Canonical DSL sentence — what the toast renders collapsed.
    /// `RequestSummary::to_dsl` produces it; `from_dsl` parses it.
    /// (Round-trip lands in Phase 6 — for now this is a best-effort
    /// human-readable string built by the call site that opens the op.)
    pub dsl: String,
}

/// Per-event entry in a `PublishManifest` — the rows the confirm modal
/// lists (collapsed by default). Identifies each event by kind + id and,
/// where present, its `title` / `d` tag so the user reads titles, not
/// hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishEntry {
    pub event_id: String,
    pub kind: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
}

/// Plain-language description of *what* a publish replicates — the
/// "function / procedure" the confirm modal renders instead of the raw
/// event JSON (which the user can already inspect elsewhere). Travels on
/// `PublishIntent`. For an NKBIP-01 publication this captures the
/// index/section shape (kinds 30040 / 30041) and whether the tree is
/// nested; for a bare broadcast it's a single-entry manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PublishManifest {
    /// `(kind, count)` pairs, ascending by kind for stable display.
    pub kind_counts: Vec<(u64, usize)>,
    /// Total events being published.
    pub total: usize,
    /// kind-30040 count (publication indices).
    pub index_count: usize,
    /// kind-30041 count (publication sections).
    pub section_count: usize,
    /// True when more than one 30040 index is present — a nested tree
    /// rather than a flat index + sections.
    pub nested: bool,
    /// One row per event, for the modal's collapsible list.
    pub entries: Vec<PublishEntry>,
}

impl PublishManifest {
    /// Build from the JSON events about to be broadcast. Reads `kind`,
    /// `id`, and the `title` / `d` tags off each event — display only,
    /// no protocol semantics.
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a Value>) -> Self {
        let mut m = PublishManifest::default();
        let mut counts: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
        for e in events {
            let kind = e.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
            let event_id = e
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let (title, d_tag) = entry_tags(e);
            *counts.entry(kind).or_insert(0) += 1;
            m.total += 1;
            match kind {
                30040 => m.index_count += 1,
                30041 => m.section_count += 1,
                _ => {}
            }
            m.entries.push(PublishEntry {
                event_id,
                kind,
                title,
                d_tag,
            });
        }
        m.nested = m.index_count > 1;
        m.kind_counts = counts.into_iter().collect();
        m
    }
}

/// Pull the `title` and `d` tag values off an event JSON, if present.
fn entry_tags(e: &Value) -> (Option<String>, Option<String>) {
    let mut title = None;
    let mut d_tag = None;
    if let Some(tags) = e.get("tags").and_then(|v| v.as_array()) {
        for t in tags {
            let Some(arr) = t.as_array() else { continue };
            let name = arr.first().and_then(|v| v.as_str());
            let val = arr.get(1).and_then(|v| v.as_str()).map(str::to_string);
            match name {
                Some("title") => title = val,
                Some("d") => d_tag = val,
                _ => {}
            }
        }
    }
    (title, d_tag)
}

/// Project a raw NIP-01 filter JSON into the structured `NipFilter`
/// for `RequestSummary`. Unknown / non-matching fields are dropped —
/// this is for display, not protocol round-tripping. Used by Phase-4
/// fetch helpers that build a summary alongside the filter they send.
pub fn nip_filter_from_json(f: &Value) -> NipFilter {
    let mut out = NipFilter::default();
    if let Some(arr) = f.get("kinds").and_then(|v| v.as_array()) {
        out.kinds = Some(arr.iter().filter_map(|v| v.as_u64()).collect());
    }
    if let Some(arr) = f.get("authors").and_then(|v| v.as_array()) {
        out.authors = Some(
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        );
    }
    if let Some(arr) = f.get("ids").and_then(|v| v.as_array()) {
        out.ids = Some(
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
        );
    }
    if let Some(v) = f.get("since").and_then(|v| v.as_i64()) {
        out.since = Some(v);
    }
    if let Some(v) = f.get("until").and_then(|v| v.as_i64()) {
        out.until = Some(v);
    }
    if let Some(v) = f.get("limit").and_then(|v| v.as_u64()) {
        out.limit = Some(v);
    }
    if let Some(s) = f.get("search").and_then(|v| v.as_str()) {
        out.search = Some(s.to_string());
    }
    if let Some(obj) = f.as_object() {
        for (k, v) in obj {
            if let Some(tag) = k.strip_prefix('#') {
                if let Some(arr) = v.as_array() {
                    let vals: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    out.tags.insert(tag.to_string(), vals);
                }
            }
        }
    }
    out
}

/// Best-effort DSL string for a raw-JSON (filters + composition)
/// pair. Phase-4 callers still pass `&[Value]` rather than the typed
/// `Vec<NipFilter>` they could now build, so this shim turns each
/// JSON filter into a `NipFilter` and delegates to
/// `RequestSummary::to_dsl`. New call sites should construct
/// `RequestSummary` directly and call `.to_dsl()` on it.
/// Compact human-readable "what" for a REQ's filters — the relay-fetch
/// log lines' answer to "fetched WHAT from that relay?". A clean
/// coordinate lookup (one kind + one author + `#d` values) renders as
/// naddr(s) so the log names the actual entity; anything else renders
/// as the search-DSL clause string, which pastes straight back into
/// the search box. Capped so a bulk section backfill (dozens of `#d`
/// values) can't flood a log line.
pub fn describe_filters_for_log(filters: &[Value]) -> String {
    filters
        .iter()
        .map(describe_filter_for_log)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn describe_filter_for_log(f: &Value) -> String {
    let nf = nip_filter_from_json(f);
    if let (Some(kinds), Some(authors)) = (&nf.kinds, &nf.authors) {
        if kinds.len() == 1 && authors.len() == 1 {
            if let (Ok(kind), Some(ds)) = (u32::try_from(kinds[0]), nf.tags.get("d")) {
                let mut naddrs: Vec<String> = ds
                    .iter()
                    .take(2)
                    .filter_map(|d| crate::nip19::encode_naddr(kind, &authors[0], d, &[]).ok())
                    .collect();
                // Only when every shown d-tag encoded — a bad pubkey
                // falls through to the DSL form instead of logging a
                // half-empty naddr list.
                if !naddrs.is_empty() && naddrs.len() == ds.len().min(2) {
                    if ds.len() > naddrs.len() {
                        naddrs.push(format!("+{} more", ds.len() - naddrs.len()));
                    }
                    return naddrs.join(" ");
                }
            }
        }
    }
    let mut s = filter_to_dsl_clauses(&nf);
    const MAX_LEN: usize = 220;
    if s.len() > MAX_LEN {
        let mut cut = MAX_LEN;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    if s.is_empty() {
        "(empty filter)".to_string()
    } else {
        s
    }
}

pub fn dsl_for_composition(filters: &[Value], composition: &CompositionShape) -> String {
    let summary = RequestSummary {
        filters: filters.iter().map(nip_filter_from_json).collect(),
        composition: composition.clone(),
        dsl: String::new(),
    };
    summary.to_dsl()
}

/// Try to parse a single phase name (the surface form of `Phase`).
fn parse_phase(s: &str) -> Option<Phase> {
    match s {
        "read" => Some(Phase::Read),
        "write" => Some(Phase::Write),
        "publish" => Some(Phase::Publish),
        "broadcast" => Some(Phase::Broadcast),
        "search.default" => Some(Phase::SearchDefault),
        "search.fallback" => Some(Phase::SearchFallback),
        "indexer.default" => Some(Phase::IndexerDefault),
        "indexer.fallback" => Some(Phase::IndexerFallback),
        _ => None,
    }
}

impl RequestSummary {
    /// Render this summary as a canonical DSL sentence — the formal
    /// language form the user can read in the toast and paste back
    /// into a query box. Round-trips with [`Self::from_dsl`] for the
    /// subset both sides understand (`k:` / `by:` / `~:"…"` /
    /// `limit:` / `since:` / `until:` / `#tag:` / `id:` on the
    /// filter side; `via:` / `then:` / `also:` on the composition
    /// side).
    pub fn to_dsl(&self) -> String {
        // Render each filter as its own clause group. Multi-filter
        // REQs (e.g. the feed-init that piggybacks kind 0 alongside
        // kind 30040) join with ` | ` — matches the existing search
        // grammar's union operator (CompoundQuery). Single-filter
        // requests render without the pipe, so backward compatibility
        // with the old round-trip tests holds.
        let filter_strs: Vec<String> = self
            .filters
            .iter()
            .map(filter_to_dsl_clauses)
            .filter(|s| !s.is_empty())
            .collect();
        let mut sentence = filter_strs.join(" | ");

        // Composition: `via:` for the first stage, `then:` for each
        // subsequent stage. `also:Δms` for stages with a non-zero
        // start_delay_ms. Composition applies to the whole REQ, so
        // it trails the union rather than attaching to any one
        // filter.
        let mut comp_parts: Vec<String> = Vec::new();
        for (i, stage) in self.composition.phases.iter().enumerate() {
            let keyword = if i == 0 {
                "via"
            } else if stage.start_delay_ms > 0 {
                "also"
            } else {
                "then"
            };
            let phases: Vec<&str> = stage.members.iter().map(|(p, _)| p.as_str()).collect();
            if phases.is_empty() {
                continue;
            }
            if keyword == "also" {
                comp_parts.push(format!(
                    "also:{} Δ{}",
                    phases.join(","),
                    stage.start_delay_ms
                ));
            } else {
                comp_parts.push(format!("{}:{}", keyword, phases.join(",")));
            }
        }
        if !comp_parts.is_empty() {
            if !sentence.is_empty() {
                sentence.push(' ');
            }
            sentence.push_str(&comp_parts.join(" "));
        }
        sentence
    }

    /// Parse a DSL sentence back into a `RequestSummary`. The
    /// inverse of [`Self::to_dsl`] for the subset both sides
    /// understand. Unknown tokens are silently skipped — round-trip
    /// preserves what the parser knows; unknowns become a hint that
    /// the grammar should be extended.
    ///
    /// On parse, `dsl` is set to the input string (preserved
    /// verbatim) so the UI can display exactly what the user typed.
    pub fn from_dsl(s: &str) -> Self {
        // Multi-filter REQs join their filter clauses with ` | `. We
        // accumulate clauses into a filters vector; encountering a
        // `|` token finalizes the current filter and starts a new one.
        // Composition tokens (via/then/also/Δ) ALWAYS go to the
        // composition struct regardless of where they appear — the
        // composition is per-REQ, not per-filter.
        let mut filters: Vec<NipFilter> = Vec::new();
        let mut filter = NipFilter::default();
        let mut filter_has_content = false;
        let mut phases: Vec<PhaseStage> = Vec::new();

        for tok in tokenize_dsl(s) {
            let tok = tok.as_str();
            if tok == "|" {
                if filter_has_content {
                    filters.push(std::mem::take(&mut filter));
                    filter_has_content = false;
                }
                continue;
            }
            // Filter clauses — each sets filter_has_content so the
            // `|` finalizer (and end-of-input finalizer) know there's
            // a filter to push.
            if let Some(rest) = tok.strip_prefix("k:") {
                let kinds: Vec<u64> = rest
                    .split(',')
                    .filter_map(|s| s.parse::<u64>().ok())
                    .collect();
                if !kinds.is_empty() {
                    filter.kinds = Some(kinds);
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("by:") {
                let authors: Vec<String> = rest.split(',').map(|s| s.to_string()).collect();
                if !authors.is_empty() {
                    filter.authors = Some(authors);
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("id:") {
                filter
                    .ids
                    .get_or_insert_with(Vec::new)
                    .push(rest.to_string());
                filter_has_content = true;
                continue;
            }
            if let Some(rest) = tok.strip_prefix("limit:") {
                if let Ok(n) = rest.parse::<u64>() {
                    filter.limit = Some(n);
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("since:") {
                if let Ok(t) = rest.parse::<i64>() {
                    filter.since = Some(t);
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("until:") {
                if let Ok(t) = rest.parse::<i64>() {
                    filter.until = Some(t);
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("~:") {
                // `~:"…"` strip surrounding quotes if present
                let q = rest.trim_matches('"');
                if !q.is_empty() {
                    filter.search = Some(q.to_string());
                    filter_has_content = true;
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix('#') {
                if let Some((tag, val)) = rest.split_once(':') {
                    filter
                        .tags
                        .entry(tag.to_string())
                        .or_default()
                        .push(val.to_string());
                    filter_has_content = true;
                }
                continue;
            }

            // Composition clauses
            if let Some(rest) = tok.strip_prefix("via:") {
                let members = parse_phase_list(rest);
                if !members.is_empty() {
                    phases.push(PhaseStage {
                        label: "primary".into(),
                        members,
                        start_delay_ms: 0,
                    });
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("then:") {
                let members = parse_phase_list(rest);
                if !members.is_empty() {
                    phases.push(PhaseStage {
                        label: "fallback".into(),
                        members,
                        start_delay_ms: 0,
                    });
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix("also:") {
                // also:phase[,phase] — the Δms delta comes as the next
                // whitespace-separated token (`Δ500`). Without it, the
                // overlap starts immediately (delay 0).
                let members = parse_phase_list(rest);
                if !members.is_empty() {
                    phases.push(PhaseStage {
                        label: "delayed-fallback".into(),
                        members,
                        start_delay_ms: 0,
                    });
                }
                continue;
            }
            if let Some(rest) = tok.strip_prefix('Δ') {
                // Δms — attach to the previous stage if it's still
                // marked "delayed-fallback" and currently has no
                // delay.
                if let Ok(ms) = rest.parse::<u64>() {
                    if let Some(last) = phases.last_mut() {
                        if last.label == "delayed-fallback" && last.start_delay_ms == 0 {
                            last.start_delay_ms = ms;
                        }
                    }
                }
                continue;
            }
            // Unknown token — silently skipped. A later sub-phase
            // can collect these into a diagnostics list.
        }

        // Finalize the trailing filter (the one after the last `|`,
        // or the only one when there's no `|`).
        if filter_has_content {
            filters.push(filter);
        } else if filters.is_empty() {
            // No filter clauses at all — push the default so callers
            // still see a one-entry vec for the common code path.
            filters.push(filter);
        }

        let composition = CompositionShape { phases };
        let mut summary = Self {
            filters,
            composition,
            dsl: s.to_string(),
        };
        // Re-render so `dsl` is the canonical form, not the user's
        // input (caller can still read the input via the param).
        summary.dsl = summary.to_dsl();
        summary
    }
}

/// Render a single `NipFilter` as a space-separated clause string
/// (no `via:`/`then:` — those are composition, not filter, and live
/// outside the filter group in the DSL). Field order matches the
/// `to_dsl` layout: event-shape first (kinds, authors, ids, tags),
/// then query controls (search, limit, time bounds).
fn filter_to_dsl_clauses(f: &NipFilter) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(kinds) = &f.kinds {
        if !kinds.is_empty() {
            let ks: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
            parts.push(format!("k:{}", ks.join(",")));
        }
    }
    if let Some(authors) = &f.authors {
        if !authors.is_empty() {
            parts.push(format!("by:{}", authors.join(",")));
        }
    }
    if let Some(ids) = &f.ids {
        for id in ids {
            parts.push(format!("id:{}", id));
        }
    }
    for (tag, vals) in &f.tags {
        for v in vals {
            parts.push(format!("#{}:{}", tag, v));
        }
    }
    if let Some(s) = &f.search {
        parts.push(format!("~:\"{}\"", s));
    }
    if let Some(n) = f.limit {
        parts.push(format!("limit:{}", n));
    }
    if let Some(t) = f.since {
        parts.push(format!("since:{}", t));
    }
    if let Some(t) = f.until {
        parts.push(format!("until:{}", t));
    }
    parts.join(" ")
}

/// Parse a comma-separated `phase[,phase,…]` list into a member vector
/// with an empty relays list (engine fills in the actual URLs from
/// the live relay config when executing).
fn parse_phase_list(s: &str) -> Vec<(Phase, Vec<String>)> {
    s.split(',')
        .filter_map(parse_phase)
        .map(|p| (p, Vec::new()))
        .collect()
}

/// Split a DSL string into whitespace-separated tokens, but treat the
/// run of characters between matched `"` quotes as a single token —
/// `~:"local first"` stays in one piece instead of fracturing on the
/// space.
fn tokenize_dsl(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    for c in input.chars() {
        if c == '"' {
            in_quote = !in_quote;
            current.push(c);
            continue;
        }
        if c.is_whitespace() && !in_quote {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(c);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Events streamed to the UI for every user-initiated relay operation
/// — fetches AND publishes. `Intent`/`PublishIntent` open an operation,
/// `Progress` narrates, `RelayStatus` carries per-relay updates within
/// it, `Completed`/`Failed` close it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FetchEvent {
    Intent {
        operation_id: String,
        pattern: FetchPattern,
        label: String,
        steps: Vec<String>,
        relays: Vec<String>,
        /// True in Confirm mode — the UI must POST a decision before the
        /// engine proceeds. False in Auto mode — informational only.
        needs_confirmation: bool,
        /// Structured request representation — filters + composition +
        /// DSL sentence. Optional during incremental rollout: legacy
        /// `begin_operation` callers emit Intent with `summary: None`;
        /// new callers via `begin_operation_with_summary` populate it.
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<RequestSummary>,
    },
    /// Publish counterpart to `Intent` — opens a publish/broadcast op.
    /// Carries the IDs being published so the UI can correlate per-relay
    /// `Accepted`/`Rejected` status.
    PublishIntent {
        operation_id: String,
        label: String,
        relays: Vec<String>,
        event_ids: Vec<String>,
        needs_confirmation: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<RequestSummary>,
        /// Plain-language description of what's being published — kinds,
        /// index/section shape, per-event titles. The confirm modal
        /// renders this instead of dumping raw JSON.
        #[serde(skip_serializing_if = "Option::is_none")]
        manifest: Option<PublishManifest>,
    },
    Progress {
        operation_id: String,
        label: String,
        done: usize,
        total: Option<usize>,
    },
    /// Per-relay status update within an open operation. The UI keys
    /// off `(operation_id, relay)` to update the matching row in the
    /// expanded toast view.
    RelayStatus {
        operation_id: String,
        relay: String,
        phase: Phase,
        status: RelayStatusValue,
    },
    Completed {
        operation_id: String,
        event_count: usize,
    },
    Failed {
        operation_id: String,
        error: String,
    },
}

/// The UI's reply to an `Intent`, delivered via
/// `POST /api/v1/network/fetch-confirm`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmDecision {
    pub operation_id: String,
    pub approved: bool,
    /// When present, the relay set the user picked in the modal —
    /// overrides the relays the engine proposed.
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

/// Returned by `begin_operation` when the user declined the fetch (or
/// didn't answer within `CONFIRM_TIMEOUT`). Callers degrade gracefully
/// — typically by returning local-only results.
#[derive(Debug, Clone, Copy)]
pub struct FetchCancelled;

impl std::fmt::Display for FetchCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "fetch cancelled by user")
    }
}

fn next_operation_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("op-{micros:013x}-{n:08x}")
}

/// Handle for an approved (or Auto-mode) fetch operation. Emits progress
/// and the terminal event; carries the relay set to actually fetch from.
pub struct FetchOperation {
    activity: std::sync::Arc<NetworkActivity>,
    operation_id: String,
    relays: Vec<String>,
}

impl FetchOperation {
    pub fn id(&self) -> &str {
        &self.operation_id
    }

    /// The relay set to fetch from — the engine's proposal, or the
    /// user's override from the confirm modal.
    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    /// Narrate a step of the operation to the UI's progress toast.
    pub fn progress(&self, label: impl Into<String>, done: usize, total: Option<usize>) {
        self.activity.emit(FetchEvent::Progress {
            operation_id: self.operation_id.clone(),
            label: label.into(),
            done,
            total,
        });
    }

    /// Close the operation successfully.
    pub fn complete(self, event_count: usize) {
        self.activity.emit(FetchEvent::Completed {
            operation_id: self.operation_id.clone(),
            event_count,
        });
    }

    /// Close the operation with an error.
    pub fn fail(self, error: impl Into<String>) {
        self.activity.emit(FetchEvent::Failed {
            operation_id: self.operation_id.clone(),
            error: error.into(),
        });
    }

    /// Emit a per-relay status update for this operation. The UI keys
    /// off `(operation_id, relay)` to update the expandable toast's
    /// per-relay row.
    pub fn relay_status(&self, relay: impl Into<String>, phase: Phase, status: RelayStatusValue) {
        self.activity.emit(FetchEvent::RelayStatus {
            operation_id: self.operation_id.clone(),
            relay: relay.into(),
            phase,
            status,
        });
    }
}

/// Handle for an approved (or Auto-mode) publish operation. Mirror of
/// `FetchOperation` — same lifecycle (`progress` → `relay_status` →
/// `complete`/`fail`), distinguished by the `PublishIntent` event that
/// opened it. Callers turn signed events into publish actions, then
/// stream per-relay `Accepted`/`Rejected` status as each relay replies.
pub struct PublishOperation {
    activity: std::sync::Arc<NetworkActivity>,
    operation_id: String,
    relays: Vec<String>,
}

impl PublishOperation {
    pub fn id(&self) -> &str {
        &self.operation_id
    }

    /// The relay set to publish to — the engine's proposal, or the
    /// user's override from the confirm modal.
    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    /// Narrate a step. Same shape as fetch progress — the UI updates
    /// the same toast.
    pub fn progress(&self, label: impl Into<String>, done: usize, total: Option<usize>) {
        self.activity.emit(FetchEvent::Progress {
            operation_id: self.operation_id.clone(),
            label: label.into(),
            done,
            total,
        });
    }

    /// Emit a per-relay status update for this publish op. Typical
    /// flow: `Connecting` → `Accepted` (relay sent `["OK", id, true]`)
    /// or `Rejected { msg }` (relay sent `["OK", id, false, msg]`).
    pub fn relay_status(&self, relay: impl Into<String>, status: RelayStatusValue) {
        self.activity.emit(FetchEvent::RelayStatus {
            operation_id: self.operation_id.clone(),
            relay: relay.into(),
            phase: Phase::Publish,
            status,
        });
    }

    /// Close the operation successfully. `event_count` is the count of
    /// (event, relay) pairs that returned `Accepted` — i.e. how many
    /// relays accepted at least one event in the batch.
    pub fn complete(self, event_count: usize) {
        self.activity.emit(FetchEvent::Completed {
            operation_id: self.operation_id.clone(),
            event_count,
        });
    }

    /// Close the operation with an error.
    pub fn fail(self, error: impl Into<String>) {
        self.activity.emit(FetchEvent::Failed {
            operation_id: self.operation_id.clone(),
            error: error.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nip_filter(kinds: Vec<u64>, authors: Vec<&str>) -> NipFilter {
        NipFilter {
            kinds: Some(kinds),
            authors: Some(authors.into_iter().map(String::from).collect()),
            ..NipFilter::default()
        }
    }

    #[test]
    fn describe_filters_renders_coordinate_lookup_as_naddr() {
        let pubkey = "aa".repeat(32);
        let f = serde_json::json!({
            "kinds": [30041], "authors": [pubkey], "#d": ["intro"], "limit": 1
        });
        let s = describe_filters_for_log(&[f]);
        assert!(s.starts_with("naddr1"), "expected naddr, got: {s}");
        // Round-trip: the logged naddr decodes back to the coordinate.
        match crate::nip19::decode(&s).expect("logged naddr decodes") {
            crate::nip19::Decoded::Naddr { kind_int, d_tag, .. } => {
                assert_eq!(kind_int, 30041);
                assert_eq!(d_tag, "intro");
            }
            other => panic!("expected naddr, decoded {other:?}"),
        }
    }

    #[test]
    fn describe_filters_renders_generic_filter_as_dsl() {
        let f = serde_json::json!({ "kinds": [30040, 0], "limit": 50 });
        assert_eq!(describe_filters_for_log(&[f]), "k:30040,0 limit:50");
    }

    #[test]
    fn describe_filters_caps_bulk_d_tag_lists() {
        let pubkey = "bb".repeat(32);
        let ds: Vec<String> = (0..30).map(|i| format!("section-{i}")).collect();
        let f = serde_json::json!({ "kinds": [30041], "authors": [pubkey], "#d": ds });
        let s = describe_filters_for_log(&[f]);
        assert!(s.contains("+28 more"), "expected capped list, got: {s}");
    }

    fn stage(label: &str, members: Vec<(Phase, Vec<String>)>, delay_ms: u64) -> PhaseStage {
        PhaseStage {
            label: label.into(),
            members,
            start_delay_ms: delay_ms,
        }
    }

    #[test]
    fn publish_manifest_summarizes_flat_publication() {
        let events = vec![
            serde_json::json!({
                "id": "aa", "kind": 30040,
                "tags": [["d", "my-pub"], ["title", "My Publication"]]
            }),
            serde_json::json!({ "id": "bb", "kind": 30041, "tags": [["title", "Intro"]] }),
            serde_json::json!({ "id": "cc", "kind": 30041, "tags": [["title", "Body"]] }),
        ];
        let m = PublishManifest::from_events(events.iter());
        assert_eq!(m.total, 3);
        assert_eq!(m.index_count, 1);
        assert_eq!(m.section_count, 2);
        assert!(!m.nested, "single index is a flat publication");
        assert_eq!(m.kind_counts, vec![(30040, 1), (30041, 2)]);
        // title preferred, d_tag captured.
        assert_eq!(m.entries[0].title.as_deref(), Some("My Publication"));
        assert_eq!(m.entries[0].d_tag.as_deref(), Some("my-pub"));
    }

    #[test]
    fn publish_manifest_flags_nested_tree() {
        // Two indices => nested.
        let events = vec![
            serde_json::json!({ "id": "a", "kind": 30040, "tags": [] }),
            serde_json::json!({ "id": "b", "kind": 30040, "tags": [] }),
            serde_json::json!({ "id": "c", "kind": 30041, "tags": [] }),
        ];
        let m = PublishManifest::from_events(events.iter());
        assert!(m.nested);
        assert_eq!(m.index_count, 2);
    }

    #[test]
    fn to_dsl_renders_basic_profile_lookup() {
        let summary = RequestSummary {
            filters: vec![nip_filter(vec![0], vec!["dc4cd086"])],
            composition: CompositionShape {
                phases: vec![
                    stage("primary", vec![(Phase::Read, vec![])], 0),
                    stage("fallback", vec![(Phase::IndexerDefault, vec![])], 0),
                ],
            },
            dsl: String::new(),
        };
        assert_eq!(
            summary.to_dsl(),
            "k:0 by:dc4cd086 via:read then:indexer.default"
        );
    }

    #[test]
    fn from_dsl_parses_basic_profile_lookup() {
        let parsed = RequestSummary::from_dsl("k:0 by:dc4cd086 via:read then:indexer.default");
        assert_eq!(
            parsed.filters[0].kinds,
            Some(vec![0])
        );
        assert_eq!(
            parsed.filters[0].authors,
            Some(vec!["dc4cd086".to_string()])
        );
        assert_eq!(parsed.composition.phases.len(), 2);
        assert_eq!(parsed.composition.phases[0].label, "primary");
        assert_eq!(parsed.composition.phases[0].members[0].0, Phase::Read);
        assert_eq!(parsed.composition.phases[1].label, "fallback");
        assert_eq!(
            parsed.composition.phases[1].members[0].0,
            Phase::IndexerDefault
        );
    }

    #[test]
    fn dsl_round_trip_preserves_filter_and_composition() {
        let original = "k:30040,30041 by:dc4cd086 limit:50 via:read,indexer.default then:indexer.fallback";
        let parsed = RequestSummary::from_dsl(original);
        // Re-render — should match the input verbatim (modulo
        // whitespace normalization).
        assert_eq!(parsed.to_dsl(), original);
        // And the structured form matches the expected shape.
        assert_eq!(parsed.filters[0].kinds, Some(vec![30040, 30041]));
        assert_eq!(parsed.filters[0].limit, Some(50));
        assert_eq!(parsed.composition.phases.len(), 2);
        assert_eq!(parsed.composition.phases[0].members.len(), 2);
        assert_eq!(parsed.composition.phases[0].members[0].0, Phase::Read);
        assert_eq!(
            parsed.composition.phases[0].members[1].0,
            Phase::IndexerDefault
        );
    }

    #[test]
    fn dsl_parses_search_with_delayed_fallback() {
        // `also:` lays the overlap stage, and the next `Δ500` token
        // attaches the delay onto it. Together they describe
        // "search.default; 500ms later, also fan out to fallback".
        let parsed = RequestSummary::from_dsl(
            "~:\"local first\" via:search.default also:search.fallback Δ500",
        );
        assert_eq!(parsed.filters[0].search.as_deref(), Some("local first"));
        assert_eq!(parsed.composition.phases.len(), 2);
        assert_eq!(parsed.composition.phases[1].label, "delayed-fallback");
        assert_eq!(parsed.composition.phases[1].start_delay_ms, 500);
    }

    #[test]
    fn dsl_round_trip_for_delayed_fallback() {
        let original = "~:\"local first\" via:search.default also:search.fallback Δ500";
        let parsed = RequestSummary::from_dsl(original);
        assert_eq!(parsed.to_dsl(), original);
    }

    #[test]
    fn dsl_round_trip_with_tag_filter_and_time_bounds() {
        let original = "k:30040 #d:my-publication since:1700000000 until:1800000000 via:read";
        let parsed = RequestSummary::from_dsl(original);
        assert_eq!(parsed.to_dsl(), original);
        assert_eq!(
            parsed.filters[0].tags.get("d"),
            Some(&vec!["my-publication".to_string()])
        );
        assert_eq!(parsed.filters[0].since, Some(1700000000));
        assert_eq!(parsed.filters[0].until, Some(1800000000));
    }

    #[test]
    fn unknown_tokens_are_skipped_silently() {
        // Future grammar additions (`target:`, `pub`, etc.) shouldn't
        // crash older binaries — they just get dropped from the
        // structured form, and the next round-trip omits them.
        let parsed = RequestSummary::from_dsl(
            "k:0 target:wss://specific.relay pub via:read unknown:value",
        );
        assert_eq!(parsed.filters[0].kinds, Some(vec![0]));
        assert_eq!(parsed.composition.phases.len(), 1);
        assert_eq!(parsed.composition.phases[0].members[0].0, Phase::Read);
    }

    #[test]
    fn dsl_round_trip_multi_filter_with_pipe_separator() {
        // The feed-init Intent piggybacks a kind-0 filter alongside
        // the kind-30040 filter so one approval covers both. Verify
        // the DSL renders both filters joined with ` | ` and parses
        // back to the same structure.
        let original = "k:30040 by:aaa,bbb limit:200 | k:0 by:aaa,bbb limit:2 via:read";
        let parsed = RequestSummary::from_dsl(original);
        assert_eq!(parsed.to_dsl(), original);
        assert_eq!(parsed.filters.len(), 2);
        assert_eq!(parsed.filters[0].kinds, Some(vec![30040]));
        assert_eq!(parsed.filters[0].limit, Some(200));
        assert_eq!(parsed.filters[1].kinds, Some(vec![0]));
        assert_eq!(parsed.filters[1].limit, Some(2));
        assert_eq!(
            parsed.filters[0].authors,
            Some(vec!["aaa".to_string(), "bbb".to_string()])
        );
        assert_eq!(parsed.composition.phases.len(), 1);
        assert_eq!(parsed.composition.phases[0].members[0].0, Phase::Read);
    }

    #[test]
    fn parse_phase_accepts_dotted_class_names() {
        assert_eq!(parse_phase("read"), Some(Phase::Read));
        assert_eq!(parse_phase("indexer.default"), Some(Phase::IndexerDefault));
        assert_eq!(parse_phase("search.fallback"), Some(Phase::SearchFallback));
        assert_eq!(parse_phase("nonsense"), None);
        assert_eq!(parse_phase(""), None);
    }
}
