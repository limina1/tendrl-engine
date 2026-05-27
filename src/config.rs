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

fn default_data_dir() -> String {
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
            authors: Vec::new(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl RelayConfig {
    /// Get all unique relay URLs across all working sets.
    pub fn all_urls(&self) -> Vec<String> {
        let mut urls = std::collections::HashSet::new();
        for u in &self.general.urls { urls.insert(u.clone()); }
        for u in &self.publish.urls { urls.insert(u.clone()); }
        for u in &self.fetch.urls { urls.insert(u.clone()); }
        urls.into_iter().collect()
    }

    /// Apply persisted URL sets from `relays.json` onto this config. The
    /// `kinds` come from this struct's defaults — only URLs are layered in.
    /// Called by the engine on startup after loading the state file.
    pub fn apply_persisted(&mut self, sets: &crate::relay_store::RelaySets) {
        self.general.urls = sets.general.clone();
        self.publish.urls = sets.publish.clone();
        self.fetch.urls = sets.fetch.clone();
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

/// Identity configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityConfig {
    /// User's public key (npub1... or hex)
    pub pubkey: Option<String>,
    /// Assistant's public key (npub1... or hex)
    pub assistant: Option<String>,
}

/// Embedding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Enable embedding index
    #[serde(default)]
    pub enabled: bool,
    /// Backend: "python" (sidecar) or "onnx" (in-process, requires --features onnx)
    #[serde(default = "default_embedding_backend")]
    pub backend: String,
    /// Python sidecar URL
    #[serde(default = "default_sidecar_url")]
    pub sidecar_url: String,
    /// Sentence transformer model name
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Embedding dimensions (must match model)
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
    /// Custom index path (defaults to {data_dir parent}/vectors.*)
    #[serde(default)]
    pub index_path: Option<String>,
    /// Automatically embed new events on ingest
    #[serde(default)]
    pub auto_embed: bool,
}

fn default_embedding_backend() -> String { "python".to_string() }
fn default_sidecar_url() -> String { "http://localhost:3031".to_string() }
fn default_embedding_model() -> String { "all-MiniLM-L6-v2".to_string() }
fn default_dimensions() -> usize { 384 }

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_embedding_backend(),
            sidecar_url: default_sidecar_url(),
            model: default_embedding_model(),
            dimensions: default_dimensions(),
            index_path: None,
            auto_embed: false,
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

fn default_line_numbers() -> bool { true }
fn default_vim_mode() -> bool { true }
fn default_insert_mode() -> String { "cursor".to_string() }

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

fn default_compose_mode() -> String { "full".to_string() }
fn default_sync_mode() -> String { "reactive".to_string() }
fn default_button_labels() -> String { "icon".to_string() }

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
}

fn default_network_mode() -> String {
    "auto".to_string()
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            mode: default_network_mode(),
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
    /// Resolve the configured pubkey to hex format (handles npub1... bech32)
    pub fn pubkey_hex(&self) -> Option<String> {
        Self::resolve_pubkey(self.identity.pubkey.as_deref())
    }

    /// Resolve the configured assistant pubkey to hex format
    pub fn assistant_pubkey_hex(&self) -> Option<String> {
        Self::resolve_pubkey(self.identity.assistant.as_deref())
    }

    fn resolve_pubkey(raw: Option<&str>) -> Option<String> {
        let raw = raw?;
        if raw.starts_with("npub1") {
            crate::identity::decode_npub(raw).ok()
        } else if raw.len() == 64 && hex::decode(raw).is_ok() {
            Some(raw.to_string())
        } else {
            None
        }
    }

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
