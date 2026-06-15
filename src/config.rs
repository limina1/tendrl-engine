//! Configuration parsing
//!
//! Handles TOML configuration file loading and defaults.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3030
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to nostrdb data directory
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

pub fn default_data_dir() -> String {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        let path = Path::new(&data_home).join("nostr-engine").join("nostrdb");
        return path.to_string_lossy().to_string();
    }

    if let Some(home) = std::env::var_os("HOME") {
        let path = Path::new(&home)
            .join(".local")
            .join("share")
            .join("nostr-engine")
            .join("nostrdb");
        return path.to_string_lossy().to_string();
    }

    "./nostr_engine_data/nostrdb".to_string()
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
        }
    }
}

/// A relay set with URLs and the event kinds to fetch from them.
///
/// `urls` is the live working list — sourced from `relays.json` at runtime,
/// not from TOML. `kinds` stays code-defaulted because it's a per-purpose
/// fetch filter (e.g. "kind 0/3/10002 from general relays") and is not the
/// kind of thing users edit from the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelaySet {
    /// Relay WebSocket URLs
    pub urls: Vec<String>,
    /// Event kinds to fetch (empty = all kinds)
    #[serde(default)]
    pub kinds: Vec<u64>,
}

/// Relay configuration.
///
/// `general` / `publish` / `fetch` keep their `Vec<String>` shape because
/// the rest of the engine reads them by reference. They are **no longer**
/// deserialized from TOML — the TOML carries only `initial_relays` (the
/// first-boot seed) plus `timeout_ms` and `authors`. At runtime, the URL
/// fields are filled in from `<data_dir>/relays.json` by the engine; the
/// `kinds` come from these compiled-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// Bootstrap-only relay list. Used **once** when no `relays.json`
    /// exists yet, to seed all three working sets. After that, the JSON
    /// file is authoritative and this field is ignored.
    #[serde(default)]
    pub initial_relays: Vec<String>,
    /// General relays for profile info, metadata, notes. URLs filled at
    /// runtime from `relays.json`; `kinds` defaulted in code.
    #[serde(skip, default = "default_general")]
    pub general: RelaySet,
    /// Relays to publish events to (push only, kinds not used for fetch).
    /// URLs filled at runtime from `relays.json`.
    #[serde(skip, default = "default_publish")]
    pub publish: RelaySet,
    /// Relays to fetch publications/sections from. URLs filled at runtime
    /// from `relays.json`; `kinds` defaulted in code.
    #[serde(skip, default = "default_fetch")]
    pub fetch: RelaySet,
    /// Aggregator/broadcast relays (nostr.land etc.) — never targeted by
    /// auto-publish. Reserved for explicit per-event opt-in once routing
    /// lands. See `project-relay-classes` memory.
    #[serde(skip, default = "default_broadcast")]
    pub broadcast: RelaySet,
    /// NIP-50 search-capable relays. Two-tier: `default` joins (or
    /// replaces with `exclusive_search`) the primary `~:` fan-out;
    /// `fallback` is consulted only when the primary returns zero.
    /// URLs filled at runtime from `relays.json`.
    #[serde(skip, default)]
    pub search: crate::relay_store::DiscoveryClass,
    /// Indexer / discovery relays (purplepag.es etc.). Same two-tier
    /// shape as `search`. Used for kind 0 / 10002 / metadata lookups.
    #[serde(skip, default)]
    pub indexer: crate::relay_store::DiscoveryClass,
    /// Per-class `exclusive` toggle. When ON, the discovery class
    /// REPLACES the read relays for its lookup type rather than
    /// joining them. Filled from `relays.json` at runtime.
    #[serde(skip, default)]
    pub exclusive: crate::relay_store::ExclusiveFlags,
    /// User-defined named relay sets (NIP-51 kind 30002). Loaded from
    /// `relays.json` at runtime; never deserialized from TOML.
    #[serde(skip, default)]
    pub named_sets: Vec<crate::relay_store::NamedRelaySet>,
    /// Authors to follow — fetch their events from fetch relays (npub or hex)
    #[serde(default)]
    pub authors: Vec<String>,
    /// Request timeout in milliseconds
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_general() -> RelaySet {
    RelaySet {
        urls: Vec::new(),
        kinds: vec![0, 3, 10002, 30023, 30818, 30817],
    }
}

fn default_publish() -> RelaySet {
    RelaySet {
        urls: Vec::new(),
        kinds: vec![],
    }
}

fn default_fetch() -> RelaySet {
    RelaySet {
        urls: Vec::new(),
        kinds: vec![0, 30040, 30041, 30023, 30818, 30817, 9802],
    }
}

fn default_broadcast() -> RelaySet {
    RelaySet {
        urls: Vec::new(),
        kinds: vec![],
    }
}

fn default_initial_relays() -> Vec<String> {
    crate::relay::DEFAULT_RELAYS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_timeout_ms() -> u64 {
    15000
}

/// Read the legacy `[relay.fetch] urls`, `[relay.general] urls`,
/// `[relay.publish] urls` arrays from a parsed TOML doc, returning the
/// **union** (de-duplicated, in insertion order) so a pre-migration
/// config still produces a usable bootstrap seed when `initial_relays`
/// is absent.
fn legacy_relay_urls(raw: &toml::Table) -> Vec<String> {
    let relay = match raw.get("relay").and_then(|v| v.as_table()) {
        Some(t) => t,
        None => return Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for section in ["fetch", "general", "publish"] {
        let urls = relay
            .get(section)
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("urls"))
            .and_then(|v| v.as_array());
        if let Some(arr) = urls {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if seen.insert(s.to_string()) {
                        out.push(s.to_string());
                    }
                }
            }
        }
    }
    out
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            initial_relays: default_initial_relays(),
            general: default_general(),
            publish: default_publish(),
            fetch: default_fetch(),
            broadcast: default_broadcast(),
            search: crate::relay_store::DiscoveryClass::default(),
            indexer: crate::relay_store::DiscoveryClass::default(),
            exclusive: crate::relay_store::ExclusiveFlags::default(),
            named_sets: Vec::new(),
            authors: Vec::new(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl RelayConfig {
    /// Get all unique relay URLs across all working sets.
    pub fn all_urls(&self) -> Vec<String> {
        let mut urls = std::collections::HashSet::new();
        for u in &self.general.urls {
            urls.insert(u.clone());
        }
        for u in &self.publish.urls {
            urls.insert(u.clone());
        }
        for u in &self.fetch.urls {
            urls.insert(u.clone());
        }
        urls.into_iter().collect()
    }

    /// Borrow the mutable URL list for a working-set name. Accepts the
    /// flat names (`general` / `publish` / `fetch` / `broadcast`) and the
    /// dotted discovery-class tiers (`search.default` / `search.fallback`
    /// / `indexer.default` / `indexer.fallback`); returns `None` for any
    /// other name so callers can ignore stray UI input.
    ///
    /// Single source of truth for the set-name → field routing on
    /// `RelayConfig` (used by `Engine::add_relay` / `remove_relay`). It
    /// mirrors `RelaySets::get_mut`, which routes the same names on the
    /// persistence-side struct; the two can't be one function because the
    /// flat sets here are `RelaySet { urls, kinds }`, not bare `Vec`.
    pub fn urls_mut(&mut self, set: &str) -> Option<&mut Vec<String>> {
        match set {
            "general" => Some(&mut self.general.urls),
            "publish" => Some(&mut self.publish.urls),
            "fetch" => Some(&mut self.fetch.urls),
            "broadcast" => Some(&mut self.broadcast.urls),
            "search.default" => Some(&mut self.search.default),
            "search.fallback" => Some(&mut self.search.fallback),
            "indexer.default" => Some(&mut self.indexer.default),
            "indexer.fallback" => Some(&mut self.indexer.fallback),
            _ => None,
        }
    }

    /// Apply persisted URL sets from `relays.json` onto this config. The
    /// `kinds` come from this struct's defaults — only URLs are layered in.
    /// Called by the engine on startup after loading the state file.
    pub fn apply_persisted(&mut self, sets: &crate::relay_store::RelaySets) {
        self.general.urls = sets.general.clone();
        self.publish.urls = sets.publish.clone();
        self.fetch.urls = sets.fetch.clone();
        self.broadcast.urls = sets.broadcast.clone();
        self.search = sets.search.clone();
        self.indexer = sets.indexer.clone();
        self.exclusive = sets.exclusive.clone();
        self.named_sets = sets.named.clone();
    }

    /// Resolve author list to hex pubkeys (handles npub and hex)
    pub fn authors_hex(&self) -> Vec<String> {
        self.authors
            .iter()
            .filter_map(|a| {
                if a.starts_with("npub1") {
                    crate::identity::decode_npub(a).ok()
                } else if a.len() == 64 && hex::decode(a).is_ok() {
                    Some(a.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Identity configuration — signer *preferences* only (no keys, no pubkeys).
/// Identity itself lives at runtime: the user signs via NIP-07 or a pasted
/// ncryptsec, the assistant via a pasted nsec/ncryptsec persisted in the OS
/// keyring — never in config. A stale `[identity]` table from an older config
/// (with `pubkey` / `assistant` / `ncryptsec`) parses fine and is ignored
/// (serde ignores unknown fields).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityConfig {
    /// Last-chosen signing source — `"engine"` or `"nip07"`. Persisted by
    /// Save Settings so a reload reconnects to the same signer. Engine boots
    /// as `"engine"` regardless; the web re-registers a NIP-07 signer when
    /// this is set and `window.nostr` is reachable. Kept so the engine knows
    /// which signer to reattach to on startup.
    #[serde(default)]
    pub source: Option<String>,
    /// Auto-lock the engine key after this many minutes of inactivity. `0`
    /// (default) = never. Only the engine source holds a decrypted secret to
    /// lock — a NIP-07 signer keeps its own key, so this is a no-op there.
    #[serde(default)]
    pub lock_timeout_minutes: u64,
}

/// Embedding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Enable embedding index
    #[serde(default)]
    pub enabled: bool,
    /// Embedding model. Resolved to a fastembed model code; the in-process ONNX
    /// backend is the only backend.
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Embedding dimensions (must match model)
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
    /// Custom index path (defaults to {data_dir parent}/vectors.*)
    #[serde(default)]
    pub index_path: Option<String>,
    /// Automatically embed new events of the configured kinds on retrieval
    /// (relay fetch) and publishing. When false, embedding only happens via
    /// the manual sync/reindex actions. Defaults on.
    #[serde(default = "default_auto_embed")]
    pub auto_embed: bool,
    /// Event kinds eligible for semantic embedding. Defaults to
    /// `embedding::DEFAULT_EMBED_KINDS`; editable from the UI's Embedding
    /// settings panel (POST /api/v1/embed/config) and persisted here so the
    /// selection survives restart and the background auto-embed pass honors it.
    #[serde(default = "default_embed_kinds")]
    pub embed_kinds: Vec<u16>,
}

fn default_embedding_model() -> String {
    "all-MiniLM-L6-v2".to_string()
}
fn default_dimensions() -> usize {
    384
}
fn default_embed_kinds() -> Vec<u16> {
    crate::embedding::DEFAULT_EMBED_KINDS.to_vec()
}
fn default_auto_embed() -> bool {
    true
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_embedding_model(),
            dimensions: default_dimensions(),
            index_path: None,
            auto_embed: default_auto_embed(),
            embed_kinds: default_embed_kinds(),
        }
    }
}

/// AI / LLM provider configuration (the `[ai]` block).
///
/// Carries the provider selection and the agent/chat model. The API key is
/// **never** stored here — it comes from `ANTHROPIC_API_KEY` in the environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    /// Active provider: `"claude"` (default) or `"noop"` (echo, testing).
    #[serde(default = "default_ai_provider")]
    pub provider: String,
    /// Chat/agent model id. Defaults to a strong model — agentic tool
    /// selection is where the capability gap shows, so this is intentionally
    /// not the cheap chat model.
    #[serde(default = "default_ai_model")]
    pub model: String,
    /// Hard cap on tool round-trips within a single agent turn (runaway guard).
    #[serde(default = "default_max_tool_turns")]
    pub max_tool_turns: usize,
    /// Explicit enabled-tool allowlist (tool names). `None` ⇒ the default
    /// "everything but publish" policy. Persisted by the AI Tools settings tab.
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    /// Path to a Markdown file prepended to the agent's system prompt, re-read
    /// each turn so edits apply live. Relative paths resolve against the config
    /// file's directory. Defaults to `prompt.md`.
    #[serde(default)]
    pub system_prompt_path: Option<String>,
}

fn default_ai_provider() -> String {
    "claude".to_string()
}
fn default_ai_model() -> String {
    "claude-sonnet-4-6".to_string()
}
fn default_max_tool_turns() -> usize {
    25
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: default_ai_provider(),
            model: default_ai_model(),
            max_tool_turns: default_max_tool_turns(),
            enabled_tools: None,
            system_prompt_path: None,
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Server settings
    #[serde(default)]
    pub server: ServerConfig,
    /// Database settings
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Relay settings
    #[serde(default)]
    pub relay: RelayConfig,
    /// Identity settings
    #[serde(default)]
    pub identity: IdentityConfig,
    /// Embedding settings
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// Documents folder settings
    #[serde(default)]
    pub documents: DocumentsConfig,
    /// Network mode settings
    #[serde(default)]
    pub network: NetworkConfig,
    /// Editor UI defaults (line numbers, vim mode, insert behaviour)
    #[serde(default)]
    pub editor: EditorConfig,
    /// Composer UI defaults (edit mode, sync mode, button labels)
    #[serde(default)]
    pub compose: ComposeConfig,
    /// AI / LLM provider settings (`[ai]` block)
    #[serde(default)]
    pub ai: AiConfig,
}

/// Editor surface defaults. Mirrors the in-memory state in
/// `web/src/lib/state.svelte.ts`; persisted via the settings page's
/// "Save settings" so it survives across reloads / fresh machines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorConfig {
    #[serde(default = "default_line_numbers")]
    pub line_numbers: bool,
    #[serde(default = "default_vim_mode")]
    pub vim_mode: bool,
    /// `"cursor"` (insert at caret) or `"append"` (append at bottom).
    #[serde(default = "default_insert_mode")]
    pub insert_mode: String,
}

fn default_line_numbers() -> bool {
    true
}
fn default_vim_mode() -> bool {
    true
}
fn default_insert_mode() -> String {
    "cursor".to_string()
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            line_numbers: default_line_numbers(),
            vim_mode: default_vim_mode(),
            insert_mode: default_insert_mode(),
        }
    }
}

/// Composer surface defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// `"full"` or `"plain"`.
    #[serde(default = "default_compose_mode")]
    pub default_mode: String,
    /// `"reactive"` or `"explicit"`.
    #[serde(default = "default_sync_mode")]
    pub sync_mode: String,
    /// `"icon"` or `"text"`.
    #[serde(default = "default_button_labels")]
    pub button_labels: String,
}

fn default_compose_mode() -> String {
    "full".to_string()
}
fn default_sync_mode() -> String {
    "reactive".to_string()
}
fn default_button_labels() -> String {
    "icon".to_string()
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            default_mode: default_compose_mode(),
            sync_mode: default_sync_mode(),
            button_labels: default_button_labels(),
        }
    }
}

/// Network mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// "auto" or "confirm" (legacy "online"/"offline" still parse)
    #[serde(default = "default_network_mode")]
    pub mode: String,
    /// Whether the user has made an explicit first-run network-mode choice.
    /// Defaults false so a fresh install raises the one-time "choose your
    /// default mode" modal before any relay fetch. Flipped true (and
    /// persisted) the first time the mode is set through the UI.
    #[serde(default)]
    pub mode_chosen: bool,
}

fn default_network_mode() -> String {
    // Confirm-first by default: a fresh / zero-config boot gates relay
    // fetches behind the approval modal rather than auto-fetching. Local-first
    // is the deliberate posture; opt into "auto" explicitly.
    "confirm".to_string()
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: default_network_mode(),
            mode_chosen: false,
        }
    }
}

/// Documents folder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentsConfig {
    /// Path to documents folder
    #[serde(default = "default_documents_path")]
    pub path: String,
}

fn default_documents_path() -> String {
    "./docs".to_string()
}

impl Default for DocumentsConfig {
    fn default() -> Self {
        Self {
            path: default_documents_path(),
        }
    }
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)
            .map_err(|e| EngineError::Config(format!("Failed to parse config: {}", e)))?;
        // Backward compatibility: if a config.toml still has the legacy
        // `[relay.fetch] urls = [...]` / `[relay.general]` / `[relay.publish]`
        // sections but no `initial_relays`, fall back to the legacy URLs
        // as the bootstrap seed. Stops a quiet data-loss footgun where
        // an older config would silently produce an empty relays.json.
        if config.relay.initial_relays.is_empty() {
            if let Ok(raw) = content.parse::<toml::Table>() {
                let legacy = legacy_relay_urls(&raw);
                if !legacy.is_empty() {
                    tracing::info!(
                        "config.toml has no `initial_relays`; falling back to legacy [relay.*] urls ({} unique) for bootstrap. Consider migrating to `initial_relays = [...]` under `[relay]`.",
                        legacy.len()
                    );
                    config.relay.initial_relays = legacy;
                }
            }
        }
        Ok(config)
    }

    /// Load configuration from file if it exists, otherwise use defaults
    pub fn load_or_default(path: Option<&Path>) -> Self {
        if let Some(p) = path {
            if p.exists() {
                match Self::from_file(p) {
                    Ok(config) => return config,
                    Err(e) => {
                        tracing::warn!("Failed to load config from {:?}: {}", p, e);
                    }
                }
            }
        }
        Self::default()
    }

    /// Get the bind address as a string
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_identity_table_parses_and_is_ignored() {
        // An older config.toml with the now-removed identity *data* fields must
        // still parse (no deny_unknown_fields), keep the signer preferences,
        // and silently drop pubkey/assistant/ncryptsec.
        let toml_text = r#"
[identity]
pubkey = "npub1abc"
assistant = "npub1def"
ncryptsec = "ncryptsec1ghi"
source = "nip07"
lock_timeout_minutes = 5
"#;
        let config: Config = toml::from_str(toml_text).expect("stale [identity] should parse");
        assert_eq!(config.identity.source.as_deref(), Some("nip07"));
        assert_eq!(config.identity.lock_timeout_minutes, 5);
    }

    #[test]
    fn legacy_relay_urls_unions_three_sections_dedup_preserve_order() {
        let toml_text = r#"
[relay]
timeout_ms = 15000

[relay.fetch]
urls = ["wss://a", "wss://b"]

[relay.general]
urls = ["wss://b", "wss://c"]

[relay.publish]
urls = ["wss://c", "wss://d"]
"#;
        let raw: toml::Table = toml_text.parse().unwrap();
        let urls = legacy_relay_urls(&raw);
        assert_eq!(urls, vec!["wss://a", "wss://b", "wss://c", "wss://d"]);
    }

    #[test]
    fn legacy_relay_urls_returns_empty_when_no_legacy_sections() {
        let toml_text = "[relay]\ninitial_relays = [\"wss://x\"]\n";
        let raw: toml::Table = toml_text.parse().unwrap();
        assert!(legacy_relay_urls(&raw).is_empty());
    }

    #[test]
    fn from_file_seeds_initial_from_legacy_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[relay]
timeout_ms = 15000

[relay.fetch]
urls = ["wss://legacy-a", "wss://legacy-b"]

[relay.publish]
urls = ["ws://localhost:3334"]
"#,
        )
        .unwrap();
        let cfg = Config::from_file(&path).expect("load");
        assert_eq!(
            cfg.relay.initial_relays,
            vec![
                "wss://legacy-a".to_string(),
                "wss://legacy-b".to_string(),
                "ws://localhost:3334".to_string()
            ],
            "legacy [relay.*] urls should seed initial_relays when it's absent"
        );
    }

    #[test]
    fn from_file_keeps_explicit_initial_relays_over_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[relay]
initial_relays = ["wss://explicit"]

[relay.fetch]
urls = ["wss://legacy"]
"#,
        )
        .unwrap();
        let cfg = Config::from_file(&path).expect("load");
        // Explicit initial_relays wins; legacy is ignored when initial_relays
        // is non-empty (the fallback only kicks in for the empty case).
        assert_eq!(cfg.relay.initial_relays, vec!["wss://explicit".to_string()]);
    }
}
