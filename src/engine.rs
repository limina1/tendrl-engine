//! Core unified query engine
//!
//! Provides a unified interface for querying events from local nostrdb
//! with optional relay backfill based on configurable fetch policies.

use crate::config::{EmbeddingConfig, RelayConfig};
use crate::embedding::{EmbeddingIndex, EmbeddingStatus};
use crate::error::{EngineError, Result};
use crate::network::{self, FetchTrigger, NetworkActivity, NetworkMode};
use crate::relay_store::{RelayStore, RelaySets};
use crate::search::{self, SearchQuery, SearchResponse};
use crate::{query, relay};
use nostrdb::{Config, IngestMetadata, Ndb};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
// Separate sync RwLock for relay_config — guards short-lived
// read/write of in-memory URL lists. Never held across .await.
use std::sync::RwLock as StdRwLock;
use tracing::{debug, info, warn};

/// Upper bound on authors fetched in one profile backfill — keeps a
/// large result set from issuing an unwieldy REQ to relays.
const PROFILE_BACKFILL_CAP: usize = 200;

/// nostrdb fetch limit for a query that must be post-filtered in memory
/// (multi-char tag / `has:` / `count:` / keyword). Such constraints are
/// not indexable, so the scan must see every candidate — this bound is
/// "effectively unbounded" for any realistic local DB while staying well
/// inside `i32` (nostrdb's query-limit type).
const EXHAUSTIVE_SCAN_LIMIT: usize = 1_000_000;

/// Fetch policy determines how the engine retrieves events
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchPolicy {
    /// Only query local nostrdb, never fetch from relays
    LocalOnly,
    /// Try local first, fetch from relays if no results (default)
    #[default]
    LocalFirst,
    /// Always fetch from relays, merge with local results
    FetchAlways,
}

impl std::str::FromStr for FetchPolicy {
    type Err = EngineError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "local_only" | "localonly" => Ok(FetchPolicy::LocalOnly),
            "local_first" | "localfirst" => Ok(FetchPolicy::LocalFirst),
            "fetch_always" | "fetchalways" => Ok(FetchPolicy::FetchAlways),
            _ => Err(EngineError::InvalidFilter(format!(
                "Unknown fetch policy: {}. Valid values: local_only, local_first, fetch_always",
                s
            ))),
        }
    }
}

/// Source information for query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySource {
    /// Number of events from local database
    pub local_count: usize,
    /// Number of events fetched from relays
    pub relay_count: usize,
}

/// Response from a query operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    /// The retrieved events
    pub events: Vec<Value>,
    /// Total count of events
    pub count: usize,
    /// Source breakdown
    pub source: QuerySource,
}

/// Persistent ignore list for hiding events from feed/search
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IgnoreList {
    /// Event IDs to ignore (hex)
    pub event_ids: std::collections::HashSet<String>,
    /// Pubkeys to ignore (hex) — all events from these authors
    pub pubkeys: std::collections::HashSet<String>,
}

impl IgnoreList {
    fn path(data_dir: &Path) -> std::path::PathBuf {
        data_dir.parent().unwrap_or(data_dir).join("ignored.json")
    }

    pub fn load(data_dir: &Path) -> Self {
        let path = Self::path(data_dir);
        if let Ok(data) = std::fs::read_to_string(&path) {
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| EngineError::Database(format!("Failed to serialize ignore list: {e}")))?;
        std::fs::write(&path, data)
            .map_err(|e| EngineError::Database(format!("Failed to write ignore list: {e}")))?;
        Ok(())
    }

    pub fn is_ignored(&self, event_id: &str, pubkey: &str) -> bool {
        self.event_ids.contains(event_id) || self.pubkeys.contains(pubkey)
    }
}

/// The main Nostr Engine
pub struct Engine {
    /// The nostrdb instance
    ndb: Arc<Ndb>,
    /// Relay configuration (general, publish, fetch sets).
    ///
    /// URL fields are mirrored from `RelayStore` at boot; thereafter the
    /// `RwLock` allows `add_relay`/`remove_relay` to mutate the live copy
    /// at runtime via `&self` (so API handlers can take effect without a
    /// restart). Read sites clone before any `.await`.
    relay_config: StdRwLock<RelayConfig>,
    /// Persistent backing for the working relay sets — writes through
    /// to `<data_dir>/relays.json` on every `add_relay` / `remove_relay`.
    relay_store: RelayStore,
    /// Data directory path
    data_dir: std::path::PathBuf,
    /// Config file path (for saving changes from UI)
    config_path: Option<std::path::PathBuf>,
    /// Configured user pubkey (hex) for resolving by:me
    my_pubkey: Option<String>,
    /// Configured assistant pubkey (hex) for resolving by:assistant
    assistant_pubkey: Option<String>,
    /// Optional embedding index for semantic search
    embedding: Option<Arc<RwLock<EmbeddingIndex>>>,
    /// Ignore list for filtering events
    ignore_list: RwLock<IgnoreList>,
    /// Documents folder path
    documents_dir: std::path::PathBuf,
    /// Sidecar URL
    sidecar_url: String,
    /// Claude Code sessions directory
    claude_sessions_dir: Option<std::path::PathBuf>,
    /// Network activity tracker (mode + fetch log)
    network: Arc<NetworkActivity>,
    /// NIP-11 relay information cache (process-wide, 1h TTL)
    nip11_cache: crate::nip11::Nip11Cache,
}

impl Engine {
    /// Create a new Engine with the specified data path
    pub fn new(data_path: &Path) -> Result<Self> {
        Self::with_relay_config(data_path, &RelayConfig::default())
    }

    /// Create a new Engine with a bootstrap relay list (test/back-compat).
    /// The relays are passed in as `initial_relays`; on first boot they
    /// seed all three working sets via the relay store.
    pub fn with_config(data_path: &Path, relays: &[&str], _timeout_ms: u64) -> Result<Self> {
        let config = RelayConfig {
            initial_relays: relays.iter().map(|s| s.to_string()).collect(),
            ..RelayConfig::default()
        };
        Self::with_relay_config(data_path, &config)
    }

    /// Create a new Engine with full relay configuration.
    ///
    /// The TOML-derived `relay_config` provides `initial_relays`, `kinds`
    /// defaults, `authors`, and `timeout_ms`. The live working URL sets
    /// (`general` / `publish` / `fetch`) come from `<data_dir>/relays.json`:
    ///
    /// - First boot (no `relays.json`): seed all three sets from
    ///   `initial_relays`, write the file, and use that as the runtime
    ///   working copy.
    /// - Subsequent boots: load the JSON file; ignore `initial_relays`
    ///   entirely. The file is the source of truth.
    pub fn with_relay_config(data_path: &Path, relay_config: &RelayConfig) -> Result<Self> {
        // Ensure the data directory exists
        std::fs::create_dir_all(data_path)?;

        let config = Config::new().set_ingester_threads(2);

        let ndb = Ndb::new(
            data_path
                .to_str()
                .ok_or_else(|| EngineError::Config("Invalid data path encoding".to_string()))?,
            &config,
        )
        .map_err(|e| EngineError::Database(format!("Failed to open nostrdb: {}", e)))?;

        info!("Opened nostrdb at {:?}", data_path);

        let ignore_list = IgnoreList::load(data_path);
        let ignored_count = ignore_list.event_ids.len() + ignore_list.pubkeys.len();
        if ignored_count > 0 {
            info!(
                "Loaded ignore list: {} events, {} pubkeys",
                ignore_list.event_ids.len(),
                ignore_list.pubkeys.len()
            );
        }

        // Layer the persisted relay store on top of the (mostly-empty)
        // TOML-derived RelayConfig. First boot seeds from initial_relays.
        let relay_store = RelayStore::new(data_path)
            .map_err(|e| EngineError::Config(format!("Failed to open relay store: {e}")))?;
        let sets = if relay_store.is_first_boot() {
            info!(
                "No {} found — seeding from initial_relays ({} URLs)",
                relay_store.path().display(),
                relay_config.initial_relays.len()
            );
            let seeded = RelaySets::seed_from_initial(&relay_config.initial_relays);
            relay_store
                .save(&seeded)
                .map_err(|e| EngineError::Config(format!("Failed to seed relay store: {e}")))?;
            seeded
        } else {
            relay_store
                .load()
                .map_err(|e| EngineError::Config(format!("Failed to load relay store: {e}")))?
        };
        let mut relay_config = relay_config.clone();
        relay_config.apply_persisted(&sets);
        info!(
            "Relays — general: {:?}, fetch: {:?}, publish: {:?}",
            relay_config.general.urls, relay_config.fetch.urls, relay_config.publish.urls
        );

        Ok(Engine {
            ndb: Arc::new(ndb),
            relay_config: StdRwLock::new(relay_config),
            relay_store,
            data_dir: data_path.to_path_buf(),
            config_path: None,
            my_pubkey: None,
            assistant_pubkey: None,
            embedding: None,
            ignore_list: RwLock::new(ignore_list),
            documents_dir: std::path::PathBuf::from("./docs"),
            sidecar_url: "http://localhost:3031".to_string(),
            claude_sessions_dir: None,
            network: Arc::new(NetworkActivity::new(NetworkMode::Auto)),
            nip11_cache: crate::nip11::Nip11Cache::new(),
        })
    }

    /// Get a reference to the underlying Ndb instance
    pub fn ndb(&self) -> &Ndb {
        &self.ndb
    }

    /// Snapshot of the current relay configuration. Owns a clone — never
    /// hand back a reference into the lock or callers might block writers.
    pub fn relay_config(&self) -> RelayConfig {
        self.relay_config.read().unwrap().clone()
    }

    /// Get fetch relay URLs (owned clone).
    pub fn relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().fetch.urls.clone()
    }

    /// Get publish relay URLs (owned clone).
    pub fn publish_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().publish.urls.clone()
    }

    /// Get general relay URLs (owned clone).
    pub fn general_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().general.urls.clone()
    }

    /// Get broadcast relay URLs (owned clone). Aggregator relays the user
    /// has opted into — NEVER auto-targeted by publish; only via explicit
    /// per-event opt-in once per-kind routing lands. See
    /// `project-relay-classes` memory + T31.
    pub fn broadcast_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().broadcast.urls.clone()
    }

    /// Get search.default relay URLs (owned clone). The primary tier
    /// for `~:` queries — joined into the fan-out (or replaces read
    /// relays when `exclusive.search` is true).
    pub fn search_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().search.default.clone()
    }

    /// Get search.fallback relay URLs (owned clone). Consulted only
    /// when the primary `~:` phase returns zero events.
    pub fn search_fallback_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().search.fallback.clone()
    }

    /// Get indexer.default relay URLs (owned clone). Discovery primary
    /// for kind 0 / 10002 / metadata lookups (joins or replaces read
    /// relays per `exclusive.indexer`).
    pub fn indexer_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().indexer.default.clone()
    }

    /// Get indexer.fallback relay URLs (owned clone). Consulted only
    /// after the indexer.default phase returns zero events.
    pub fn indexer_fallback_relays(&self) -> Vec<String> {
        self.relay_config.read().unwrap().indexer.fallback.clone()
    }

    /// Read whether a discovery class is in `exclusive` mode (bypasses
    /// read relays for that lookup type). `class` is `"search"` or
    /// `"indexer"`. Unknown classes return false.
    pub fn discovery_exclusive(&self, class: &str) -> bool {
        let cfg = self.relay_config.read().unwrap();
        match class {
            "search" => cfg.exclusive.search,
            "indexer" => cfg.exclusive.indexer,
            _ => false,
        }
    }

    /// Toggle the `exclusive` flag for a discovery class. Persists
    /// through `relays.json` like other relay mutations. Returns true
    /// if the value changed.
    pub fn set_discovery_exclusive(&self, class: &str, value: bool) -> bool {
        let snapshot = {
            let mut cfg = self.relay_config.write().unwrap();
            let slot = match class {
                "search" => &mut cfg.exclusive.search,
                "indexer" => &mut cfg.exclusive.indexer,
                _ => return false,
            };
            if *slot == value {
                return false;
            }
            *slot = value;
            self.relay_sets_snapshot_locked(&cfg)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist exclusive flag ({class}={value}): {e}");
        }
        true
    }

    /// Build the execution composition for a discovery-class lookup
    /// (`class = "indexer"` for profile / kind 10002 / addressable;
    /// `class = "search"` for `~:`). Returns a `CompositionShape`
    /// matching the per-class exclusive × default × fallback matrix in
    /// the relay-management plan:
    ///
    /// - `exclusive=OFF` + `default` empty + `fallback` empty: one
    ///   stage with `(Read, fetch_urls)`.
    /// - `exclusive=OFF` + `default` non-empty: one stage with
    ///   `[(Read, fetch_urls), (Class.Default, default_urls)]` —
    ///   fired concurrently.
    /// - `exclusive=OFF` + `fallback` non-empty: a SECOND stage with
    ///   `(Class.Fallback, fallback_urls)` after primary returns 0.
    /// - `exclusive=ON`: read relays are excluded; primary stage is
    ///   `(Class.Default, default_urls)` (or fallback if default is
    ///   empty); fallback stage follows.
    pub fn compose_discovery_phases(&self, class: &str) -> crate::network::CompositionShape {
        use crate::network::{Phase, PhaseStage};
        let cfg = self.relay_config.read().unwrap();
        let read_urls = cfg.fetch.urls.clone();
        let (default_urls, fallback_urls, default_phase, fallback_phase, exclusive) = match class {
            "search" => (
                cfg.search.default.clone(),
                cfg.search.fallback.clone(),
                Phase::SearchDefault,
                Phase::SearchFallback,
                cfg.exclusive.search,
            ),
            "indexer" => (
                cfg.indexer.default.clone(),
                cfg.indexer.fallback.clone(),
                Phase::IndexerDefault,
                Phase::IndexerFallback,
                cfg.exclusive.indexer,
            ),
            _ => {
                return crate::network::CompositionShape {
                    phases: vec![PhaseStage {
                        label: "primary".into(),
                        members: vec![(Phase::Read, read_urls)],
                        start_delay_ms: 0,
                    }],
                };
            }
        };
        drop(cfg);

        let mut phases = Vec::new();
        // Track whether the fallback set has already been consumed by
        // the primary stage (the degenerate exclusive+empty-default
        // case) so we don't re-add it as a second stage.
        let mut fallback_consumed = false;

        let primary_members: Vec<(Phase, Vec<String>)> = if exclusive {
            // Read relays bypassed entirely.
            if !default_urls.is_empty() {
                vec![(default_phase, default_urls)]
            } else if !fallback_urls.is_empty() {
                fallback_consumed = true;
                vec![(fallback_phase, fallback_urls.clone())]
            } else {
                // ON with nothing configured — degrade to read so
                // the lookup still runs.
                vec![(Phase::Read, read_urls.clone())]
            }
        } else {
            // OFF: read is the floor; default joins concurrently.
            let mut m = vec![(Phase::Read, read_urls.clone())];
            if !default_urls.is_empty() {
                m.push((default_phase, default_urls));
            }
            m
        };

        phases.push(PhaseStage {
            label: "primary".into(),
            members: primary_members,
            start_delay_ms: 0,
        });

        if !fallback_urls.is_empty() && !fallback_consumed {
            phases.push(PhaseStage {
                label: "fallback".into(),
                members: vec![(fallback_phase, fallback_urls)],
                start_delay_ms: 0,
            });
        }

        crate::network::CompositionShape { phases }
    }

    /// Execute a multi-phase fetch against a composition. Each phase
    /// fans out across its members concurrently; the next phase only
    /// fires when the previous returned zero events (sequential
    /// fallback). Per-relay `Connecting` / `Eose` / `Error` are
    /// streamed through the SSE channel so the activity-toast +
    /// modal show live status.
    pub async fn fetch_with_composition(
        &self,
        composition: &crate::network::CompositionShape,
        filters: &[Value],
        label: String,
        pattern: crate::network::FetchPattern,
        mode_confirm: bool,
    ) -> Vec<Value> {
        use crate::network::{Phase, RequestSummary, RelayStatusValue};

        // Flatten the composition's primary stage relays into the
        // initial Intent's `relays` field (back-compat for clients
        // that haven't picked up the structured summary yet).
        let primary_relays: Vec<String> = composition
            .phases
            .first()
            .map(|s| s.members.iter().flat_map(|(_, urls)| urls.clone()).collect())
            .unwrap_or_default();

        let summary = RequestSummary {
            filters: filters
                .iter()
                .map(crate::network::nip_filter_from_json)
                .collect(),
            composition: composition.clone(),
            dsl: crate::network::dsl_for_composition(filters, composition),
        };

        let op = match self
            .network
            .begin_operation_with_summary(
                pattern,
                label,
                vec![],
                primary_relays,
                Some(summary),
            )
            .await
        {
            Ok(o) => o,
            Err(_) => return Vec::new(),
        };

        let mut all_events: Vec<Value> = Vec::new();

        for stage in &composition.phases {
            // Skip subsequent phases if a prior returned events
            // (fallback semantics: only escalate on zero).
            if !all_events.is_empty() && stage.label != "primary" {
                break;
            }

            let mut stage_events: Vec<Value> = Vec::new();
            for (phase, urls) in &stage.members {
                for relay_url in urls {
                    op.relay_status(relay_url, *phase, RelayStatusValue::Connecting);
                    match self
                        .tracked_fetch_with_options(
                            relay_url,
                            filters,
                            crate::network::FetchTrigger::ProfilePrefetch,
                            mode_confirm,
                        )
                        .await
                    {
                        Ok(events) => {
                            op.relay_status(
                                relay_url,
                                *phase,
                                RelayStatusValue::Eose {
                                    event_count: events.len(),
                                },
                            );
                            stage_events.extend(events);
                        }
                        Err(e) => {
                            op.relay_status(
                                relay_url,
                                *phase,
                                RelayStatusValue::Error { msg: e.to_string() },
                            );
                        }
                    }
                }
            }
            all_events.extend(stage_events);
        }

        op.complete(all_events.len());
        // Silence unused warning if Phase is referenced only above.
        let _ = Phase::Read;
        all_events
    }

    /// Get all named relay sets (NIP-51 kind 30002 groupings). Owned
    /// clone — never hand back a reference into the lock.
    pub fn named_relay_sets(&self) -> Vec<crate::relay_store::NamedRelaySet> {
        self.relay_config.read().unwrap().named_sets.clone()
    }

    /// Create a new empty named set. `d_tag` is the stable identifier
    /// (caller-provided, e.g. a nanoid); `title` is the human label.
    /// Returns false if a set with this d_tag already exists.
    pub fn create_named_set(&self, d_tag: &str, title: &str) -> bool {
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            if rc.named_sets.iter().any(|s| s.d_tag == d_tag) {
                return false;
            }
            rc.named_sets.push(crate::relay_store::NamedRelaySet {
                d_tag: d_tag.to_string(),
                title: title.to_string(),
                urls: Vec::new(),
            });
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist named-set create ({d_tag}): {e}");
        }
        true
    }

    /// Remove a named set by `d_tag`. Returns whether anything was removed.
    pub fn delete_named_set(&self, d_tag: &str) -> bool {
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            let before = rc.named_sets.len();
            rc.named_sets.retain(|s| s.d_tag != d_tag);
            if rc.named_sets.len() == before {
                return false;
            }
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist named-set delete ({d_tag}): {e}");
        }
        true
    }

    /// Rename an existing named set's `title`. Returns whether the title
    /// actually changed.
    pub fn rename_named_set(&self, d_tag: &str, new_title: &str) -> bool {
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            let Some(s) = rc.named_sets.iter_mut().find(|s| s.d_tag == d_tag) else {
                return false;
            };
            if s.title == new_title {
                return false;
            }
            s.title = new_title.to_string();
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist named-set rename ({d_tag}): {e}");
        }
        true
    }

    /// Add a URL to a named set. Idempotent: returns false if already a
    /// member or if the set doesn't exist.
    pub fn add_to_named_set(&self, d_tag: &str, url: &str) -> bool {
        let url = crate::relay_url::normalize_relay_url(url);
        if url.is_empty() {
            return false;
        }
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            let Some(s) = rc.named_sets.iter_mut().find(|s| s.d_tag == d_tag) else {
                return false;
            };
            if s.urls.iter().any(|u| u == &url) {
                return false;
            }
            s.urls.push(url.clone());
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist add-to-named-set ({d_tag}/{url}): {e}");
        }
        true
    }

    /// Remove a URL from a named set. Returns whether anything was removed.
    pub fn remove_from_named_set(&self, d_tag: &str, url: &str) -> bool {
        let url = crate::relay_url::normalize_relay_url(url);
        if url.is_empty() {
            return false;
        }
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            let Some(s) = rc.named_sets.iter_mut().find(|s| s.d_tag == d_tag) else {
                return false;
            };
            let before = s.urls.len();
            s.urls.retain(|u| u != &url);
            if s.urls.len() == before {
                return false;
            }
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist remove-from-named-set ({d_tag}/{url}): {e}");
        }
        true
    }

    /// Helper — build a RelaySets snapshot from the *already locked*
    /// RelayConfig. Internal; callers hold the write lock when they
    /// invoke this and release it after the snapshot is built.
    fn relay_sets_snapshot_locked(&self, rc: &RelayConfig) -> RelaySets {
        RelaySets {
            general: rc.general.urls.clone(),
            fetch: rc.fetch.urls.clone(),
            publish: rc.publish.urls.clone(),
            broadcast: rc.broadcast.urls.clone(),
            search: rc.search.clone(),
            indexer: rc.indexer.clone(),
            named: rc.named_sets.clone(),
            exclusive: rc.exclusive.clone(),
        }
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Set the configured user pubkey (hex)
    pub fn set_my_pubkey(&mut self, pubkey: Option<String>) {
        self.my_pubkey = pubkey;
    }

    /// Get the configured user pubkey
    pub fn my_pubkey(&self) -> Option<&str> {
        self.my_pubkey.as_deref()
    }

    /// Set the configured assistant pubkey (hex)
    pub fn set_assistant_pubkey(&mut self, pubkey: Option<String>) {
        self.assistant_pubkey = pubkey;
    }

    /// Get the configured assistant pubkey
    pub fn assistant_pubkey(&self) -> Option<&str> {
        self.assistant_pubkey.as_deref()
    }

    /// Get the network activity tracker
    /// NIP-11 cache shared across all consumers (web, eventual Emacs).
    pub fn nip11_cache(&self) -> &crate::nip11::Nip11Cache {
        &self.nip11_cache
    }

    pub fn network(&self) -> &Arc<NetworkActivity> {
        &self.network
    }

    /// True when the engine auto-fetches from relays (Auto mode).
    pub fn is_auto(&self) -> bool {
        self.network.is_auto()
    }

    /// Set the network mode
    pub fn set_network_mode(&self, mode: NetworkMode) {
        self.network.set_mode(mode);
    }

    /// Open a user-initiated fetch operation — the confirm/auto gate.
    /// In Auto mode returns immediately; in Confirm mode emits an intent
    /// and blocks until the UI approves (or `Err(FetchCancelled)`).
    pub async fn begin_fetch_operation(
        &self,
        pattern: crate::network::FetchPattern,
        label: String,
        steps: Vec<String>,
        relays: Vec<String>,
    ) -> std::result::Result<crate::network::FetchOperation, crate::network::FetchCancelled> {
        self.network
            .begin_operation(pattern, label, steps, relays)
            .await
    }

    /// Open a publish operation. Same shape as `begin_fetch_operation`
    /// — emits a `PublishIntent` that the UI renders as a pending toast,
    /// gates on Confirm mode, returns a `PublishOperation` handle the
    /// caller drives through `relay_status` / `complete` / `fail`.
    pub async fn begin_publish_operation(
        &self,
        label: String,
        relays: Vec<String>,
        event_ids: Vec<String>,
        summary: Option<crate::network::RequestSummary>,
    ) -> std::result::Result<crate::network::PublishOperation, crate::network::FetchCancelled>
    {
        self.network
            .begin_publish_operation(label, relays, event_ids, summary)
            .await
    }

    /// Subscribe to the fetch-event stream (one receiver per SSE client).
    pub fn subscribe_fetch_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::network::FetchEvent> {
        self.network.subscribe_fetch_events()
    }

    /// Resolve a pending confirm intent with the UI's decision.
    pub async fn resolve_fetch_confirm(&self, decision: crate::network::ConfirmDecision) -> bool {
        self.network.resolve_confirm(decision).await
    }

    /// Get the ignore list
    pub fn ignore_list(&self) -> &RwLock<IgnoreList> {
        &self.ignore_list
    }

    /// Add an event ID to the ignore list
    pub async fn ignore_event(&self, event_id: &str) -> Result<()> {
        let mut list = self.ignore_list.write().await;
        list.event_ids.insert(event_id.to_string());
        list.save(&self.data_dir)
    }

    /// Add a pubkey to the ignore list
    pub async fn ignore_pubkey(&self, pubkey: &str) -> Result<()> {
        let mut list = self.ignore_list.write().await;
        list.pubkeys.insert(pubkey.to_string());
        list.save(&self.data_dir)
    }

    /// Remove an event ID from the ignore list
    pub async fn unignore_event(&self, event_id: &str) -> Result<()> {
        let mut list = self.ignore_list.write().await;
        list.event_ids.remove(event_id);
        list.save(&self.data_dir)
    }

    /// Remove a pubkey from the ignore list
    pub async fn unignore_pubkey(&self, pubkey: &str) -> Result<()> {
        let mut list = self.ignore_list.write().await;
        list.pubkeys.remove(pubkey);
        list.save(&self.data_dir)
    }

    /// Check if an event is ignored
    pub async fn is_ignored(&self, event_id: &str, pubkey: &str) -> bool {
        let list = self.ignore_list.read().await;
        list.is_ignored(event_id, pubkey)
    }

    /// Set the config file path (for saving changes from UI)
    pub fn set_config_path(&mut self, path: std::path::PathBuf) {
        self.config_path = Some(path);
    }

    /// Get the config file path
    pub fn config_path(&self) -> Option<&std::path::Path> {
        self.config_path.as_deref()
    }

    /// Add a relay URL to a working set. Mutates the in-memory `RelayConfig`
    /// and writes through to `<data_dir>/relays.json` so the change survives
    /// a restart.
    ///
    /// `set` accepts both flat names (`"general"`, `"publish"`, `"fetch"`,
    /// `"broadcast"`) and dotted discovery-class names (`"search.default"`,
    /// `"search.fallback"`, `"indexer.default"`, `"indexer.fallback"`).
    /// For discovery classes, the per-URL mutex applies: adding to one
    /// tier strips the URL from the other tier of the same class.
    ///
    /// Silently ignores unknown set names so the API endpoint can stay lax
    /// about user-supplied input (matching the previous behaviour). A
    /// write-through failure is surfaced as a `tracing::warn!` rather than
    /// an `Err` because the in-memory mutation still succeeded — callers
    /// don't currently check a Result here.
    pub fn add_relay(&self, set: &str, url: &str) -> bool {
        let url = crate::relay_url::normalize_relay_url(url);
        if url.is_empty() {
            return false;
        }
        // Mutate in-memory first; release the write lock before disk I/O.
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            // Per-URL mutual exclusion within a discovery class —
            // moving a URL from default to fallback (or vice versa) is
            // a single add() call.
            match set {
                "search.default" => rc.search.fallback.retain(|u| u != &url),
                "search.fallback" => rc.search.default.retain(|u| u != &url),
                "indexer.default" => rc.indexer.fallback.retain(|u| u != &url),
                "indexer.fallback" => rc.indexer.default.retain(|u| u != &url),
                _ => {}
            }
            let urls = match set {
                "general" => &mut rc.general.urls,
                "publish" => &mut rc.publish.urls,
                "fetch" => &mut rc.fetch.urls,
                "broadcast" => &mut rc.broadcast.urls,
                "search.default" => &mut rc.search.default,
                "search.fallback" => &mut rc.search.fallback,
                "indexer.default" => &mut rc.indexer.default,
                "indexer.fallback" => &mut rc.indexer.fallback,
                _ => return false,
            };
            if urls.iter().any(|u| u == &url) {
                // Already a member — but the sibling-strip above may
                // have changed state. Persist anyway so disk matches.
                let snap = self.relay_sets_snapshot_locked(&rc);
                drop(rc);
                if let Err(e) = self.relay_store.save(&snap) {
                    warn!("Failed to persist relay add ({set}/{url}): {e}");
                }
                return false;
            }
            urls.push(url.clone());
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist relay add ({set}/{url}): {e}");
        }
        true
    }

    /// Remove a relay URL from a working set. Mirror of `add_relay` —
    /// accepts the same flat and dotted set names; mutates the
    /// in-memory config and writes through to `relays.json`.
    pub fn remove_relay(&self, set: &str, url: &str) -> bool {
        let url = crate::relay_url::normalize_relay_url(url);
        if url.is_empty() {
            return false;
        }
        let snapshot = {
            let mut rc = self.relay_config.write().unwrap();
            let urls = match set {
                "general" => &mut rc.general.urls,
                "publish" => &mut rc.publish.urls,
                "fetch" => &mut rc.fetch.urls,
                "broadcast" => &mut rc.broadcast.urls,
                "search.default" => &mut rc.search.default,
                "search.fallback" => &mut rc.search.fallback,
                "indexer.default" => &mut rc.indexer.default,
                "indexer.fallback" => &mut rc.indexer.fallback,
                _ => return false,
            };
            let before = urls.len();
            urls.retain(|u| u != &url);
            if urls.len() == before {
                return false;
            }
            self.relay_sets_snapshot_locked(&rc)
        };
        if let Err(e) = self.relay_store.save(&snapshot) {
            warn!("Failed to persist relay remove ({set}/{url}): {e}");
        }
        true
    }

    /// Add an author to the follow list
    pub fn add_author(&mut self, author: &str) {
        let mut rc = self.relay_config.write().unwrap();
        if !rc.authors.contains(&author.to_string()) {
            rc.authors.push(author.to_string());
        }
    }

    /// Remove an author from the follow list
    pub fn remove_author(&mut self, author: &str) {
        self.relay_config.write().unwrap().authors.retain(|a| a != author);
    }

    /// Get the documents folder path
    pub fn documents_path(&self) -> &std::path::Path {
        &self.documents_dir
    }

    /// Set the documents folder path
    pub fn set_documents_path(&mut self, path: std::path::PathBuf) {
        self.documents_dir = path;
    }

    /// Get the sidecar URL
    pub fn sidecar_url(&self) -> &str {
        &self.sidecar_url
    }

    /// Set the sidecar URL
    pub fn set_sidecar_url(&mut self, url: String) {
        self.sidecar_url = url;
    }

    /// Get the Claude Code sessions directory
    pub fn claude_sessions_dir(&self) -> Option<&std::path::Path> {
        self.claude_sessions_dir.as_deref()
    }

    /// Set the Claude Code sessions directory
    pub fn set_claude_sessions_dir(&mut self, path: Option<std::path::PathBuf>) {
        self.claude_sessions_dir = path;
    }

    /// Set the initial network mode (called during startup from config)
    pub fn set_initial_network_mode(&self, mode: NetworkMode) {
        self.network.set_mode(mode);
    }

    /// Fetch from a single relay with activity tracking.
    /// Returns empty vec if engine is offline.
    pub async fn tracked_fetch(
        &self,
        relay_url: &str,
        filters: &[Value],
        trigger: FetchTrigger,
    ) -> Result<Vec<Value>> {
        self.tracked_fetch_with_options(relay_url, filters, trigger, false)
            .await
    }

    /// Variant of `tracked_fetch` that can bypass the offline-mode
    /// short-circuit. Reserved for explicit, user-initiated relay
    /// fetches — e.g. the profile-page refresh modal where the user
    /// has selected which relays to hit. Background pollers must
    /// continue using `tracked_fetch` so offline mode actually
    /// silences automatic traffic.
    pub async fn tracked_fetch_with_options(
        &self,
        relay_url: &str,
        filters: &[Value],
        trigger: FetchTrigger,
        mode_confirm: bool,
    ) -> Result<Vec<Value>> {
        if !mode_confirm && !self.is_auto() {
            return Ok(vec![]);
        }
        let summary = network::summarize_filters(filters);
        let guard = self.network.begin_fetch(relay_url, summary, trigger);
        match relay::fetch_with_filters(&self.ndb, relay_url, filters).await {
            Ok(events) => {
                let count = events.len();
                guard.complete(count);
                Ok(events)
            }
            Err(e) => {
                let msg = e.to_string();
                guard.fail(msg);
                Err(e)
            }
        }
    }

    /// Fetch from multiple relays with activity tracking.
    /// Each relay gets its own FetchRecord in the activity log.
    pub async fn tracked_fetch_multiple(
        &self,
        relays: &[String],
        filters: &[Value],
        trigger: FetchTrigger,
    ) -> Result<Vec<Value>> {
        self.tracked_fetch_multiple_with_options(relays, filters, trigger, false)
            .await
    }

    /// Bypass-aware variant of `tracked_fetch_multiple`. The
    /// search-with-relays CTA + discussions/list pass `bypass=true` so
    /// explicit user actions reach the network even when global
    /// network mode is offline. Background syncs keep using
    /// `tracked_fetch_multiple` (delegates with bypass=false).
    pub async fn tracked_fetch_multiple_with_options(
        &self,
        relays: &[String],
        filters: &[Value],
        trigger: FetchTrigger,
        mode_confirm: bool,
    ) -> Result<Vec<Value>> {
        if !mode_confirm && !self.is_auto() {
            return Ok(vec![]);
        }
        let mut all_events = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        for relay_url in relays {
            match self
                .tracked_fetch_with_options(relay_url, filters, trigger.clone(), mode_confirm)
                .await
            {
                Ok(events) => {
                    for event in events {
                        if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                            if seen_ids.insert(id.to_string()) {
                                all_events.push(event);
                            }
                        }
                    }
                }
                Err(e) => {
                    debug!("Relay {} error: {}", relay_url, e);
                }
            }
        }
        Ok(all_events)
    }

    /// Initialize the embedding index from config
    pub fn init_embedding(&mut self, config: &EmbeddingConfig) -> Result<()> {
        if !config.enabled {
            return Ok(());
        }

        let index_dir = config
            .index_path
            .as_deref()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                self.data_dir
                    .parent()
                    .unwrap_or(&self.data_dir)
                    .to_path_buf()
            });

        let index = EmbeddingIndex::load(&index_dir, config)?;
        info!(
            "Embedding index: {} vectors, model={}",
            index.len(),
            index.model()
        );
        self.embedding = Some(Arc::new(RwLock::new(index)));
        Ok(())
    }

    /// Get the embedding index (for API handlers)
    pub fn embedding_index(&self) -> Option<&Arc<RwLock<EmbeddingIndex>>> {
        self.embedding.as_ref()
    }

    /// Sync embeddings: find unembedded events, embed them, update index
    pub async fn sync_embeddings(&self) -> Result<EmbeddingStatus> {
        let emb = self
            .embedding
            .as_ref()
            .ok_or_else(|| EngineError::Config("Embedding not enabled".into()))?;

        // CPU-heavy: query 100k events, iterate to find unembedded — offload to blocking pool
        let ndb = Arc::clone(&self.ndb);
        let indexed_ids: std::collections::HashSet<String> = {
            let index = emb.read().await;
            index.all_ids().into_iter().collect()
        };

        let (total_events, to_embed) = tokio::task::spawn_blocking(move || {
            let filter = serde_json::json!({"kinds": [30041, 30023, 30818, 9802], "limit": 100000});
            let all_events = query::query_local(&ndb, &[filter]).unwrap_or_default();
            let total_events = all_events.len();

            let mut to_embed: Vec<(String, String)> = Vec::new();
            for event in &all_events {
                let event_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if event_id.is_empty() || indexed_ids.contains(event_id) {
                    continue;
                }

                let content = event.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let title = event
                    .get("tags")
                    .and_then(|t| t.as_array())
                    .and_then(|tags| {
                        tags.iter().find_map(|tag| {
                            let arr = tag.as_array()?;
                            if arr.first()?.as_str()? == "title" {
                                arr.get(1)?.as_str()
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or("");

                let text = if title.is_empty() {
                    content.to_string()
                } else if content.is_empty() {
                    title.to_string()
                } else {
                    format!("{title}\n{content}")
                };

                if text.trim().is_empty() {
                    continue;
                }

                to_embed.push((event_id.to_string(), text));
            }

            (total_events, to_embed)
        })
        .await
        .map_err(|e| EngineError::Database(format!("spawn_blocking: {e}")))?;

        if to_embed.is_empty() {
            let index = emb.read().await;
            let model = index.model().to_string();
            return Ok(EmbeddingStatus {
                enabled: true,
                indexed_count: index.len(),
                total_events,
                sidecar_available: true,
                model: Some(model),
            });
        }

        info!("Embedding {} new events", to_embed.len());

        // Batch embed — release lock between batches so status polling works
        let batch_size = 64;

        for chunk in to_embed.chunks(batch_size) {
            let texts: Vec<String> = chunk.iter().map(|(_, t)| t.clone()).collect();
            let ids: Vec<String> = chunk.iter().map(|(id, _)| id.clone()).collect();

            // Embed with read lock (only needs the backend, not mutation)
            let vectors = {
                let index = emb.read().await;
                index.embed_texts(&texts).await
            };

            match vectors {
                Ok(vectors) => {
                    // Insert with write lock
                    let mut index = emb.write().await;
                    for (id, vec) in ids.iter().zip(vectors.iter()) {
                        if let Err(e) = index.insert(id, vec) {
                            warn!("Failed to insert embedding for {}: {}", id, e);
                        }
                    }
                    // Save after each batch so progress is preserved
                    if let Err(e) = index.save() {
                        warn!("Failed to save embeddings after batch: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Batch embedding failed: {}", e);
                    // Continue with remaining batches instead of breaking
                    continue;
                }
            }
        }

        // Save once at end
        {
            let index = emb.read().await;
            index.save()?;
        }

        let index = emb.read().await;
        let model = index.model().to_string();
        let indexed_count = index.len();

        Ok(EmbeddingStatus {
            enabled: true,
            indexed_count,
            total_events,
            sidecar_available: true,
            model: Some(model),
        })
    }

    /// Fetch missing 30041 sections for all locally known 30040 indexes
    pub async fn fetch_missing_sections(&self) -> Result<(usize, usize, usize)> {
        use serde_json::json;
        use std::collections::{HashMap, HashSet};

        if !self.is_auto() {
            return Ok((0, 0, 0));
        }

        // CPU-heavy part: query indexes, extract tags, check missing — offload to blocking pool
        let ndb = Arc::clone(&self.ndb);
        let (needed_count, missing) = tokio::task::spawn_blocking(move || {
            let filter = json!({"kinds": [30040], "limit": 100000});
            let indexes = query::query_local(&ndb, &[filter]).unwrap_or_default();

            let mut needed: HashSet<(String, String)> = HashSet::new();
            for event in &indexes {
                if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
                    for tag in tags {
                        if let Some(arr) = tag.as_array() {
                            if arr.first().and_then(|v| v.as_str()) == Some("a") {
                                if let Some(addr_str) = arr.get(1).and_then(|v| v.as_str()) {
                                    let parts: Vec<&str> = addr_str.splitn(3, ':').collect();
                                    if parts.len() == 3 && parts[0] == "30041" {
                                        needed.insert((parts[1].to_string(), parts[2].to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let mut missing: Vec<(String, String)> = Vec::new();
            for (pubkey, d_tag) in &needed {
                let check =
                    json!({"kinds": [30041], "authors": [pubkey], "#d": [d_tag], "limit": 1});
                let found = query::query_local(&ndb, &[check])
                    .map(|e| !e.is_empty())
                    .unwrap_or(false);
                if !found {
                    missing.push((pubkey.clone(), d_tag.clone()));
                }
            }

            (needed.len(), missing)
        })
        .await
        .map_err(|e| EngineError::Database(format!("spawn_blocking: {e}")))?;

        if missing.is_empty() {
            return Ok((needed_count, 0, 0));
        }

        debug!(
            "fetch_missing_sections: {} referenced, {} missing",
            needed_count,
            missing.len()
        );

        // 4. Batch fetch from relays
        let mut by_pubkey: HashMap<String, Vec<String>> = HashMap::new();
        for (pubkey, d_tag) in &missing {
            by_pubkey
                .entry(pubkey.clone())
                .or_default()
                .push(d_tag.clone());
        }

        let relays = self.relays();
        let mut total_fetched = 0usize;

        for (pubkey, d_tags) in &by_pubkey {
            for chunk in d_tags.chunks(50) {
                let filter = json!({
                    "kinds": [30041],
                    "authors": [pubkey],
                    "#d": chunk,
                    "limit": chunk.len() * 2
                });

                for relay_url in &relays {
                    match self
                        .tracked_fetch(relay_url, &[filter.clone()], FetchTrigger::BackgroundSync)
                        .await
                    {
                        Ok(events) => {
                            if !events.is_empty() {
                                debug!("Fetched {} sections from {}", events.len(), relay_url);
                                total_fetched += events.len();
                                break;
                            }
                        }
                        Err(e) => {
                            debug!("Failed to fetch sections from {}: {}", relay_url, e);
                        }
                    }
                }
            }
        }

        info!(
            "fetch_missing_sections: fetched {} of {} missing sections",
            total_fetched,
            missing.len()
        );
        Ok((needed_count, missing.len(), total_fetched))
    }

    /// Clear and rebuild the entire embedding index
    pub async fn reindex_embeddings(&self) -> Result<EmbeddingStatus> {
        let emb = self
            .embedding
            .as_ref()
            .ok_or_else(|| EngineError::Config("Embedding not enabled".into()))?;

        {
            let mut index = emb.write().await;
            index.clear()?;
        }

        self.sync_embeddings().await
    }

    /// Query events using NIP-01 filters with the specified fetch policy
    pub async fn get_events(
        &self,
        filters: Vec<Value>,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
    ) -> Result<QueryResponse> {
        self.get_events_with_options(filters, policy, override_relays, false)
            .await
    }

    /// Variant of `get_events` that can bypass the offline-mode policy
    /// downgrade. Use sparingly — this is for explicit, user-initiated
    /// fetches (e.g. "Refresh discussions" buttons) where the user has
    /// asked for a relay round-trip despite the engine being offline.
    /// Automatic / background callers should keep using `get_events`.
    pub async fn get_events_with_options(
        &self,
        filters: Vec<Value>,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
        mode_confirm: bool,
    ) -> Result<QueryResponse> {
        // Snapshot the fetch URLs once so the lock is released before
        // any .await. `override_relays` keeps borrowed semantics; the
        // local Vec lives long enough for the downstream call.
        let owned_fetch: Vec<String>;
        let relays: &[String] = if let Some(o) = override_relays {
            o
        } else {
            owned_fetch = self.relays();
            &owned_fetch
        };
        let policy = if mode_confirm || self.is_auto() {
            policy
        } else {
            FetchPolicy::LocalOnly
        };

        match policy {
            FetchPolicy::LocalOnly => self.query_local_only(&filters),
            FetchPolicy::LocalFirst => {
                self.query_local_first_with_options(&filters, relays, mode_confirm)
                    .await
            }
            FetchPolicy::FetchAlways => {
                self.query_fetch_always_with_options(&filters, relays, mode_confirm)
                    .await
            }
        }
    }

    /// Get a single event by its ID
    pub async fn get_by_id(&self, event_id: &str, policy: FetchPolicy) -> Result<Option<Value>> {
        let policy = if self.is_auto() {
            policy
        } else {
            FetchPolicy::LocalOnly
        };

        // Try local first (unless FetchAlways)
        if policy != FetchPolicy::FetchAlways {
            if let Some(event) = query::query_by_id(&self.ndb, event_id)? {
                debug!("Found event {} locally", event_id);
                return Ok(Some(event));
            }
        }

        // Fetch from relays if needed
        if policy != FetchPolicy::LocalOnly {
            debug!("Fetching event {} from relays", event_id);
            let fetch_relays = self.relays();
            return relay::fetch_event_by_id(&self.ndb, &fetch_relays, event_id).await;
        }

        Ok(None)
    }

    /// Get an addressable event by kind, pubkey, and d-tag
    pub async fn get_addressable(
        &self,
        kind: u64,
        pubkey: &str,
        d_tag: &str,
        policy: FetchPolicy,
    ) -> Result<Option<Value>> {
        let policy = if self.is_auto() {
            policy
        } else {
            FetchPolicy::LocalOnly
        };

        // Try local first (unless FetchAlways)
        if policy != FetchPolicy::FetchAlways {
            if let Some(event) = query::query_addressable(&self.ndb, kind, pubkey, d_tag)? {
                debug!(
                    "Found addressable event {}:{}:{}... locally",
                    kind,
                    &pubkey.chars().take(8).collect::<String>(),
                    d_tag
                );
                return Ok(Some(event));
            }
        }

        // Fetch from relays if needed
        if policy != FetchPolicy::LocalOnly {
            debug!(
                "Fetching addressable event {}:{}:{}... from relays",
                kind,
                &pubkey.chars().take(8).collect::<String>(),
                d_tag
            );
            let fetch_relays = self.relays();
            return relay::fetch_addressable(
                &self.ndb,
                &fetch_relays,
                kind,
                pubkey,
                d_tag,
            )
            .await;
        }

        Ok(None)
    }

    /// Ingest a raw event JSON string into nostrdb
    ///
    /// The event_json should be a raw event object `{...}`. This function wraps it
    /// in the client message format `["EVENT", {...}]` that nostrdb expects for client events.
    ///
    /// Uses `client(true)` to skip signature verification for locally-created events.
    /// This allows storing unsigned/placeholder-signed events created by the compose flow.
    pub fn ingest_event(&self, event_json: &str) -> Result<()> {
        // Client events use format: ["EVENT", {...event...}] (no subscription ID)
        // Relay events use format: ["EVENT", "subid", {...event...}] (with subscription ID)
        let wrapped = format!(r#"["EVENT",{}]"#, event_json);
        debug!("Ingesting event: {}", &wrapped[..wrapped.len().min(200)]);
        self.ndb
            .process_event_with(&wrapped, IngestMetadata::new().client(true))
            .map_err(|e| {
                debug!("Ingest failed: {}", e);
                EngineError::Database(format!("Failed to ingest event: {}", e))
            })?;
        debug!("Ingest succeeded");
        Ok(())
    }

    /// Record that `event_json` (a raw signed event object `{...}`) has been
    /// seen on / broadcast to `relay_url`.
    ///
    /// Re-ingests the event with relay provenance metadata so `note.relays()`
    /// reflects it — this is how a locally-authored publication stops showing
    /// as "local-only" once it has actually been published. nostrdb dedups by
    /// event id and simply appends the relay to the existing note's relay set.
    /// The event must carry a valid signature (relay-form ingest verifies it).
    pub fn record_event_relay(&self, event_json: &str, relay_url: &str) -> Result<()> {
        // Normalize so two URLs differing in trailing slash / case /
        // default port collapse to a single chip in the provenance UI.
        let normalized = crate::relay_url::normalize_relay_url(relay_url);
        let tag_url = if normalized.is_empty() {
            relay_url
        } else {
            normalized.as_str()
        };
        let wrapped = format!(r#"["EVENT","tendrl-relay-meta",{}]"#, event_json);
        self.ndb
            .process_event_with(
                &wrapped,
                IngestMetadata::new().client(false).relay(tag_url),
            )
            .map_err(|e| EngineError::Database(format!("Failed to record relay metadata: {e}")))
    }

    /// Search for events using a structured search query.
    ///
    /// Compiles the query to NIP-01 filters, fetches events, applies text
    /// post-filtering, and builds SearchResult objects. When a `~:` semantic
    /// filter is present, queries the HNSW index for nearest neighbors.
    pub async fn search(
        &self,
        query: &SearchQuery,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
    ) -> Result<SearchResponse> {
        self.search_with_options(query, policy, override_relays, false)
            .await
    }

    /// Variant of `search` that can bypass the offline-mode policy
    /// downgrade. The web's "No events in local DB — search relays?"
    /// CTA passes `mode_confirm=true` so the engine actually performs
    /// the relay round-trip even when offline mode is set.
    pub async fn search_with_options(
        &self,
        query: &SearchQuery,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
        mode_confirm: bool,
    ) -> Result<SearchResponse> {
        let limit = query.limit.unwrap_or(100);

        // Semantic search path
        if query.needs_semantic() {
            return self.semantic_search(query, policy, override_relays).await;
        }

        let mut filters = query.to_nip01_filters();
        let has_multi_char_tag = query
            .tag_filters
            .iter()
            .any(|tf| tf.tag_name.chars().count() > 1);
        let has_has_tags = !query.has_tags.is_empty();
        let has_count_tags = !query.count_tags.is_empty();
        // A multi-char tag / `has:` / `count:` / keyword query carries a
        // constraint nostrdb cannot index — it is post-filtered in memory
        // by `query::filter_by_tags` / `filter_by_text` after the DB read.
        // For the result to be complete the scan must see *every*
        // candidate, not just a recent window. Any single-letter tag /
        // kind / author in the same query still narrows the scan via the
        // nostrdb index.
        let has_text_filter = query.text_filter.is_some();
        let needs_broad_fetch =
            has_multi_char_tag || has_has_tags || has_count_tags || has_text_filter;

        // An exhaustive scan is inherently local — a relay cannot answer a
        // multi-char-tag / keyword constraint. Run it against nostrdb only;
        // an explicit relay search (`mode_confirm`) instead keeps its given
        // policy with a bounded limit.
        let exhaustive = needs_broad_fetch && !mode_confirm;
        let (scan_policy, scan_limit) = if exhaustive {
            (FetchPolicy::LocalOnly, EXHAUSTIVE_SCAN_LIMIT)
        } else if needs_broad_fetch {
            (policy, limit.max(500))
        } else {
            (policy, limit)
        };

        // Stamp the scan limit onto every filter — or synthesize a bare
        // filter when the query had nothing NIP-01-indexable at all.
        if filters.is_empty() {
            filters = vec![serde_json::json!({ "limit": scan_limit })];
        } else {
            for f in &mut filters {
                if let Some(obj) = f.as_object_mut() {
                    obj.insert("limit".to_string(), serde_json::json!(scan_limit));
                }
            }
        }

        // A keyword/exact query runs its exhaustive scan through
        // `query_local_text`, which applies the text match at the
        // nostrdb-note level so only matches pay `note_to_json` — the
        // difference between a sub-second search and a multi-second one.
        // Other exhaustive scans (multi-char tag / has: / count:) keep
        // the generic path.
        let response = match (exhaustive, &query.text_filter) {
            (true, Some(text_filter)) => {
                let events = query::query_local_text(&self.ndb, &filters, text_filter, limit)?;
                let count = events.len();
                QueryResponse {
                    events,
                    count,
                    source: QuerySource {
                        local_count: count,
                        relay_count: 0,
                    },
                }
            }
            _ => {
                self.get_events_with_options(filters, scan_policy, override_relays, mode_confirm)
                    .await?
            }
        };

        // Multi-char tag filters (e.g. `author:Claude`) are applied here —
        // NIP-01 only indexes single-letter keys at the DB layer.
        let tag_filtered = if has_multi_char_tag {
            query::filter_by_tags(&response.events, &query.tag_filters)
        } else {
            response.events
        };

        // `has:NAME` tag-presence filters — also post-filter only.
        let has_filtered = if has_has_tags {
            query::filter_by_has_tags(&tag_filtered, &query.has_tags)
        } else {
            tag_filtered
        };

        let mut filtered = if let Some(text_filter) = &query.text_filter {
            query::filter_by_text(&has_filtered, text_filter)
        } else {
            has_filtered
        };

        // Newest-first. nostrdb returns notes recency-ordered already,
        // but a LocalFirst relay backfill appends its events unsorted —
        // and an addressable (naddr) lookup must list every stored
        // version most-recent first. Sort explicitly so order is
        // guaranteed regardless of source.
        filtered.sort_by(|a, b| {
            let ta = a.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let tb = b.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            tb.cmp(&ta)
        });

        // `count:NAME` runs AFTER all filtering so histograms reflect the
        // user's narrowing. Empty by default when no count: was requested.
        let tag_counts = if has_count_tags {
            query::count_tag_values(&filtered, &query.count_tags)
        } else {
            std::collections::HashMap::new()
        };

        let results = search::build_search_results(&filtered, limit);

        // Filter ignored events (check event_id, pubkey, and a-tag format)
        let ignore = self.ignore_list.read().await;
        let results: Vec<_> = results
            .into_iter()
            .filter(|r| {
                if ignore.is_ignored(&r.event_id, &r.author) {
                    return false;
                }
                // Also check a-tag format (used when hiding publications from feed)
                if let Some(ref addr) = r.addr {
                    let a_tag = format!("{}:{}:{}", addr.kind, addr.pubkey, addr.d_tag);
                    if ignore.event_ids.contains(&a_tag) {
                        return false;
                    }
                }
                true
            })
            .collect();
        let count = results.len();

        Ok(SearchResponse {
            results,
            profiles: vec![],
            count,
            local_count: response.source.local_count,
            relay_count: response.source.relay_count,
            doc_results: vec![],
            tag_counts,
        })
    }

    /// Search local kind-0 profiles for an author match — the "people"
    /// half of search's fan-out (see `docs/search-architecture.org`).
    /// Local-only for now; the NIP-50 relay path is a later phase.
    /// Profiles whose pubkey is on the ignore list are dropped.
    pub async fn search_profiles(&self, term: &str) -> Vec<search::ProfileResult> {
        let profiles = query::find_profiles_matching(&self.ndb, term);
        let ignore = self.ignore_list.read().await;
        profiles
            .into_iter()
            .filter(|p| !ignore.pubkeys.contains(&p.pubkey))
            .collect()
    }

    /// Ensure a kind-0 profile is cached for each of `pubkeys` — fetch
    /// the ones missing locally in a single batched REQ across every
    /// configured relay. Best-effort; returns the count of profile
    /// events ingested. Meant to run after a search/fetch pulls in
    /// events, so the authors of freshly-seen events get their metadata
    /// cached. Logs its outcome at `info` so the backfill is visible.
    ///
    /// `mode_confirm` carries the search's relay authorization: when
    /// the accompanying search was allowed to reach relays despite
    /// offline mode, the backfill rides along on the same okay.
    pub async fn backfill_missing_profiles(
        &self,
        pubkeys: Vec<String>,
        mode_confirm: bool,
    ) -> usize {
        let considered = pubkeys.len();
        // Distinct, well-formed pubkeys with no kind-0 cached locally.
        let mut seen = std::collections::HashSet::new();
        let missing: Vec<String> = pubkeys
            .into_iter()
            .filter(|pk| pk.len() == 64 && seen.insert(pk.clone()))
            .filter(|pk| !query::profile_exists(&self.ndb, pk))
            .take(PROFILE_BACKFILL_CAP)
            .collect();
        if missing.is_empty() {
            debug!(
                "Profile backfill: all {} result authors already cached",
                considered
            );
            return 0;
        }

        if !mode_confirm && !self.is_auto() {
            info!(
                "Profile backfill: {} author(s) missing a kind-0, but the engine is offline — skipped",
                missing.len()
            );
            return 0;
        }

        // Profiles can live on any configured relay — query the union of
        // the general / fetch / publish sets so an empty `general` set
        // doesn't silently disable the backfill.
        let relays = self.relay_config.read().unwrap().all_urls();
        if relays.is_empty() {
            warn!(
                "Profile backfill: {} author(s) missing a kind-0, but no relays are configured",
                missing.len()
            );
            return 0;
        }

        info!(
            "Profile backfill: fetching {} missing author profile(s) from {} relay(s)",
            missing.len(),
            relays.len()
        );
        let filter = serde_json::json!({
            "kinds": [0],
            "authors": missing,
            "limit": missing.len(),
        });
        let mut fetched = 0;
        for relay_url in &relays {
            match self
                .tracked_fetch_with_options(
                    relay_url,
                    &[filter.clone()],
                    FetchTrigger::ProfilePrefetch,
                    mode_confirm,
                )
                .await
            {
                Ok(events) => fetched += events.len(),
                Err(e) => debug!("Profile backfill from {} failed: {}", relay_url, e),
            }
        }
        info!(
            "Profile backfill: ingested {} kind-0 event(s) for {} missing author(s)",
            fetched,
            missing.len()
        );
        fetched
    }

    /// Semantic search: embed query, search HNSW, fetch events, merge scores
    async fn semantic_search(
        &self,
        query: &SearchQuery,
        _policy: FetchPolicy,
        _override_relays: Option<&[String]>,
    ) -> Result<SearchResponse> {
        let emb = self.embedding.as_ref().ok_or_else(|| {
            EngineError::Config("Semantic search requires [embedding] enabled in config".into())
        })?;

        let semantic = query.semantic_filter.as_ref().unwrap();
        let k = semantic.k;

        // Over-fetch from HNSW: the index contains both events and doc pages,
        // and kind/author/text post-filters may discard many matches.
        let has_post_filters = query.kind_filter.is_some()
            || query.author_filter.is_some()
            || query.text_filter.is_some();
        let fetch_k = if has_post_filters { k * 10 } else { k * 5 };

        // Embed the query
        let index = emb.read().await;
        let query_vec = index.embed_query(&semantic.query).await?;

        // Search HNSW
        let matches = index.search(&query_vec, fetch_k)?;
        drop(index);

        debug!(
            "HNSW returned {} matches for k={}, fetch_k={}",
            matches.len(),
            k,
            fetch_k
        );

        if matches.is_empty() {
            return Ok(SearchResponse {
                results: vec![],
                profiles: vec![],
                count: 0,
                local_count: 0,
                relay_count: 0,
                doc_results: vec![],
                tag_counts: std::collections::HashMap::new(),
            });
        }

        // Split matches into event results and doc page results
        let ignore = self.ignore_list.read().await;
        let mut results = Vec::new();
        let mut doc_results = Vec::new();
        let mut lookup_failures = 0usize;

        for (match_id, score) in matches {
            if match_id.starts_with("doc:") {
                // Document page: doc:filename:page_num
                let parts: Vec<&str> = match_id.splitn(3, ':').collect();
                if parts.len() == 3 {
                    let filename = parts[1].to_string();
                    let page_num: usize = parts[2].parse().unwrap_or(0);

                    // Load page content from the docs folder
                    let doc_path = self.documents_dir.join(&filename);
                    let content = if let Ok(pages) = self.load_doc_page(&doc_path, page_num).await {
                        pages
                    } else {
                        format!("[Page {} of {}]", page_num, filename)
                    };

                    doc_results.push(search::DocPageResult {
                        filename,
                        page_num,
                        title: None,
                        content,
                        semantic_score: score,
                    });
                }
            } else {
                // Nostr event — check ignore list
                if ignore.is_ignored(&match_id, "") {
                    continue;
                }
                let event = match query::query_by_id(&self.ndb, &match_id) {
                    Ok(Some(e)) => e,
                    Ok(None) => {
                        lookup_failures += 1;
                        continue;
                    }
                    Err(e) => {
                        debug!(
                            "query_by_id error for {}: {}",
                            &match_id[..16.min(match_id.len())],
                            e
                        );
                        lookup_failures += 1;
                        continue;
                    }
                };
                let author = event.get("pubkey").and_then(|v| v.as_str()).unwrap_or("");
                if ignore.pubkeys.contains(author) {
                    continue;
                }
                if let Some(kinds) = &query.kind_filter {
                    let event_kind = event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
                    if !kinds.contains(&event_kind) {
                        continue;
                    }
                }
                if let Some(search::AuthorFilter::Pubkeys(pks)) = &query.author_filter {
                    if !pks.iter().any(|pk| pk == author) {
                        continue;
                    }
                }
                if let Some(text_filter) = &query.text_filter {
                    if query::filter_by_text(&[event.clone()], text_filter).is_empty() {
                        continue;
                    }
                }
                let mut sr = search::build_search_results(&[event], 1);
                if let Some(mut r) = sr.pop() {
                    r.semantic_score = Some(score);
                    results.push(r);
                }
            }
        }

        if lookup_failures > 0 {
            warn!(
                "Semantic search: {} event ID lookups failed (stale index?)",
                lookup_failures
            );
        }
        debug!(
            "Semantic search: {} event results, {} doc results from HNSW matches",
            results.len(),
            doc_results.len()
        );

        results.sort_by(|a, b| {
            b.semantic_score
                .partial_cmp(&a.semantic_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(k);
        doc_results.sort_by(|a, b| {
            b.semantic_score
                .partial_cmp(&a.semantic_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        doc_results.truncate(k);

        let count = results.len();

        Ok(SearchResponse {
            results,
            profiles: vec![],
            count,
            local_count: count,
            relay_count: 0,
            doc_results,
            tag_counts: std::collections::HashMap::new(),
        })
    }

    /// Sync document embeddings: parse all docs, embed pages not yet in HNSW
    pub async fn sync_doc_embeddings(&self) -> Result<usize> {
        let emb = self
            .embedding
            .as_ref()
            .ok_or_else(|| EngineError::Config("Embedding not enabled".into()))?;

        let docs_dir = &self.documents_dir;
        if !docs_dir.exists() {
            return Ok(0);
        }

        let sidecar = &self.sidecar_url;
        let mut total_embedded = 0;

        let entries: Vec<_> = std::fs::read_dir(docs_dir)
            .map_err(|e| EngineError::Database(format!("Failed to read docs dir: {e}")))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .collect();

        for entry in entries {
            let path = entry.path();
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let supported = [
                "pdf", "docx", "epub", "html", "htm", "txt", "md", "org", "adoc",
            ];
            if !supported.contains(&ext.as_str()) {
                continue;
            }

            // Check if first page is already embedded (quick check)
            let first_key = format!("doc:{}:1", filename);
            {
                let index = emb.read().await;
                if index.contains(&first_key) {
                    continue;
                }
            }

            // Parse via sidecar
            let file_bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };

            let part = reqwest::multipart::Part::bytes(file_bytes)
                .file_name(filename.clone())
                .mime_str("application/octet-stream")
                .unwrap();
            let form = reqwest::multipart::Form::new().part("file", part);

            let resp = match reqwest::Client::new()
                .post(format!("{sidecar}/parse"))
                .multipart(form)
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("Failed to parse {}: {}", filename, e);
                    continue;
                }
            };

            let parsed: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    warn!("Invalid parse response for {}: {}", filename, e);
                    continue;
                }
            };

            let pages = match parsed.get("pages").and_then(|p| p.as_array()) {
                Some(p) => p,
                None => continue,
            };

            // Embed each page
            let mut texts = Vec::new();
            let mut keys = Vec::new();

            for page in pages {
                let page_num = page.get("page_num").and_then(|v| v.as_u64()).unwrap_or(0);
                let content = page.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let title = page.get("title").and_then(|v| v.as_str()).unwrap_or("");

                if content.trim().is_empty() {
                    continue;
                }

                let key = format!("doc:{}:{}", filename, page_num);
                let text = if title.is_empty() {
                    content.to_string()
                } else {
                    format!("{}\n{}", title, content)
                };

                let index = emb.read().await;
                if index.contains(&key) {
                    continue;
                }
                drop(index);

                keys.push(key);
                texts.push(text);
            }

            if texts.is_empty() {
                continue;
            }

            // Batch embed
            for chunk in texts.chunks(64) {
                let chunk_keys: Vec<&str> =
                    keys[..chunk.len()].iter().map(|s| s.as_str()).collect();
                let chunk_texts: Vec<String> = chunk.to_vec();

                let vectors = {
                    let index = emb.read().await;
                    index.embed_texts(&chunk_texts).await
                };

                match vectors {
                    Ok(vecs) => {
                        let mut index = emb.write().await;
                        for (key, vec) in chunk_keys.iter().zip(vecs.iter()) {
                            if let Err(e) = index.insert(key, vec) {
                                warn!("Failed to insert doc embedding {}: {}", key, e);
                            } else {
                                total_embedded += 1;
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Doc embedding batch failed: {}", e);
                        break;
                    }
                }
            }
        }

        if total_embedded > 0 {
            let index = emb.read().await;
            index.save()?;
            info!("Embedded {} document pages", total_embedded);
        }

        Ok(total_embedded)
    }

    /// Load a specific page's content from a parsed document (via sidecar)
    async fn load_doc_page(&self, doc_path: &std::path::Path, page_num: usize) -> Result<String> {
        let file_bytes = std::fs::read(doc_path)
            .map_err(|e| EngineError::Database(format!("Failed to read doc: {e}")))?;
        let filename = doc_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")
            .unwrap();
        let form = reqwest::multipart::Form::new().part("file", part);

        let parsed: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/parse", self.sidecar_url))
            .multipart(form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| EngineError::Database(format!("Sidecar parse failed: {e}")))?
            .json()
            .await
            .map_err(|e| EngineError::Database(format!("Invalid response: {e}")))?;

        let pages = parsed
            .get("pages")
            .and_then(|p| p.as_array())
            .ok_or_else(|| EngineError::Database("No pages in response".into()))?;

        for page in pages {
            let pn = page.get("page_num").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if pn == page_num {
                return Ok(page
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string());
            }
        }

        Err(EngineError::Database(format!(
            "Page {} not found",
            page_num
        )))
    }

    // ---- Private helper methods ----

    fn query_local_only(&self, filters: &[Value]) -> Result<QueryResponse> {
        let events = query::query_local(&self.ndb, filters)?;
        let count = events.len();

        Ok(QueryResponse {
            events,
            count,
            source: QuerySource {
                local_count: count,
                relay_count: 0,
            },
        })
    }

    async fn query_local_first_with_options(
        &self,
        filters: &[Value],
        relays: &[String],
        mode_confirm: bool,
    ) -> Result<QueryResponse> {
        // Try local first
        let local_events = query::query_local(&self.ndb, filters)?;
        let local_count = local_events.len();

        // Extract the requested limit from filters (use first filter's limit, default to 50)
        let requested_limit = filters
            .first()
            .and_then(|f| f.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // If we have enough local results, return them
        if local_count >= requested_limit {
            debug!(
                "Found {} local results (requested {}), returning local only",
                local_count, requested_limit
            );
            return Ok(QueryResponse {
                events: local_events,
                count: local_count,
                source: QuerySource {
                    local_count,
                    relay_count: 0,
                },
            });
        }

        // Not enough local results, fetch from relays and merge
        debug!(
            "Found {} local results (requested {}), fetching from relays",
            local_count, requested_limit
        );
        let relay_events = self
            .tracked_fetch_multiple_with_options(
                relays,
                filters,
                FetchTrigger::LocalFirst,
                mode_confirm,
            )
            .await?;
        let relay_count = relay_events.len();

        // Merge and deduplicate by event ID
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged = Vec::new();

        // Add local events first
        for event in local_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        // Add relay events — but only those that actually satisfy the
        // filters. Relays can echo back more than was asked (e.g. when
        // they don't honour an uppercase `#A` root-scope tag filter);
        // trusting raw over-returns here leaked unrelated events.
        for event in relay_events {
            if !query::event_matches_filters(&event, filters) {
                continue;
            }
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        let count = merged.len();
        Ok(QueryResponse {
            events: merged,
            count,
            source: QuerySource {
                local_count,
                relay_count,
            },
        })
    }

    async fn query_fetch_always_with_options(
        &self,
        filters: &[Value],
        relays: &[String],
        mode_confirm: bool,
    ) -> Result<QueryResponse> {
        // Fetch from relays first (this also ingests into nostrdb)
        let relay_events = self
            .tracked_fetch_multiple_with_options(
                relays,
                filters,
                FetchTrigger::FetchAlways,
                mode_confirm,
            )
            .await?;
        let relay_count = relay_events.len();

        // Now query local (which includes freshly ingested events)
        let local_events = query::query_local(&self.ndb, filters)?;
        let local_count = local_events.len();

        // Merge and deduplicate (local results may include more than relay returned due to limits)
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged = Vec::new();

        // Add local events first (they're from the DB, so authoritative)
        for event in local_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        // Add any relay events not in local (edge case: just fetched but
        // not yet queryable). Only those that satisfy the filters —
        // relays can over-return (e.g. ignoring an uppercase `#A` tag
        // filter), and those raw events must not leak into the result.
        for event in relay_events {
            if !query::event_matches_filters(&event, filters) {
                continue;
            }
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        let count = merged.len();

        Ok(QueryResponse {
            events: merged,
            count,
            source: QuerySource {
                local_count,
                relay_count,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_fetch_policy_parsing() {
        assert_eq!(
            "local_only".parse::<FetchPolicy>().unwrap(),
            FetchPolicy::LocalOnly
        );
        assert_eq!(
            "local_first".parse::<FetchPolicy>().unwrap(),
            FetchPolicy::LocalFirst
        );
        assert_eq!(
            "fetch_always".parse::<FetchPolicy>().unwrap(),
            FetchPolicy::FetchAlways
        );
        assert_eq!(
            "LocalFirst".parse::<FetchPolicy>().unwrap(),
            FetchPolicy::LocalFirst
        );
        assert!("invalid".parse::<FetchPolicy>().is_err());
    }

    #[test]
    fn test_fetch_policy_serde() {
        let policy = FetchPolicy::LocalFirst;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"local_first\"");

        let parsed: FetchPolicy = serde_json::from_str("\"fetch_always\"").unwrap();
        assert_eq!(parsed, FetchPolicy::FetchAlways);
    }

    #[tokio::test]
    async fn test_ingest_valid_signed_event() {
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use a known-valid event from nostrdb tests (kind 1 with valid signature)
        let event_json = r#"{"id": "702555e52e82cc24ad517ba78c21879f6e47a7c0692b9b20df147916ae8731a3","pubkey": "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15","created_at": 1702675561,"kind": 1,"tags": [],"content": "hello, world","sig": "2275c5f5417abfd644b7bc74f0388d70feb5d08b6f90fa18655dda5c95d013bfbc5258ea77c05b7e40e0ee51d8a2efa931dc7a0ec1db4c0a94519762c6625675"}"#;

        // Subscribe BEFORE ingesting (nostrdb processes asynchronously)
        let filter = FilterBuilder::new().kinds([1]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest the event
        let result = engine.ingest_event(event_json);
        assert!(result.is_ok(), "Ingest failed: {:?}", result.err());

        // Wait for the event to be processed (with timeout)
        let note_keys = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            engine.ndb.wait_for_notes(sub, 1),
        )
        .await
        .expect("timeout waiting for notes")
        .expect("waiting for notes");

        println!("Got {} note keys after ingest", note_keys.len());
        assert!(!note_keys.is_empty(), "No notes received from subscription");
    }

    #[tokio::test]
    async fn test_ingest_signed_event() {
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use the EXACT known-working event from nostrdb tests but change kind to 30040
        // First, let's verify the original kind 1 event works with our ingest
        let valid_event = r#"{"id": "702555e52e82cc24ad517ba78c21879f6e47a7c0692b9b20df147916ae8731a3","pubkey": "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15","created_at": 1702675561,"kind": 1,"tags": [],"content": "hello, world","sig": "2275c5f5417abfd644b7bc74f0388d70feb5d08b6f90fa18655dda5c95d013bfbc5258ea77c05b7e40e0ee51d8a2efa931dc7a0ec1db4c0a94519762c6625675"}"#;

        // Subscribe BEFORE ingesting
        let filter = FilterBuilder::new().kinds([1]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest the event
        let result = engine.ingest_event(valid_event);
        assert!(result.is_ok(), "Ingest failed: {:?}", result.err());

        // Wait for the event with timeout
        let note_keys_result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.ndb.wait_for_notes(sub, 1),
        )
        .await;

        match note_keys_result {
            Ok(Ok(keys)) => {
                println!("SUCCESS: Got {} note keys for signed event", keys.len());
                assert!(!keys.is_empty(), "No notes received");
            }
            Ok(Err(e)) => {
                panic!("Subscription error: {:?}", e);
            }
            Err(_) => {
                panic!("TIMEOUT: Signed event was not processed");
            }
        }
    }

    #[tokio::test]
    async fn test_build_and_ingest_signed_publication() {
        use crate::publication::build_signed_publication_events;
        use crate::tree::state::ComposeState;
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use the known test secret from identity tests
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";

        // Derive the pubkey
        use secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized = public_key.serialize();
        let pubkey = hex::encode(&serialized[1..33]);

        println!("Derived pubkey: {}", pubkey);

        // Create a simple publication
        use crate::tree::state::SectionCompose;
        let mut compose = ComposeState::new();
        compose.title = "Test Publication".to_string();
        compose.sections.push(SectionCompose {
            title: "Section 1".to_string(),
            content: "Hello, world!".to_string(),
            ..Default::default()
        });

        // Build signed events
        let (pub_event, section_events) =
            build_signed_publication_events(&mut compose, &pubkey, secret_hex);

        println!(
            "Publication event: {}",
            serde_json::to_string_pretty(&pub_event).unwrap()
        );
        println!(
            "Section event: {}",
            serde_json::to_string_pretty(&section_events[0]).unwrap()
        );

        // Subscribe BEFORE ingesting
        let filter = FilterBuilder::new().kinds([30040, 30041]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest section event first
        let section_json = serde_json::to_string(&section_events[0]).unwrap();
        let result = engine.ingest_event(&section_json);
        assert!(result.is_ok(), "Section ingest failed: {:?}", result.err());

        // Ingest publication event
        let pub_json = serde_json::to_string(&pub_event).unwrap();
        let result = engine.ingest_event(&pub_json);
        assert!(
            result.is_ok(),
            "Publication ingest failed: {:?}",
            result.err()
        );

        // Wait for the events with timeout
        let note_keys_result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.ndb.wait_for_notes(sub, 2),
        )
        .await;

        match note_keys_result {
            Ok(Ok(keys)) => {
                println!("SUCCESS: Got {} note keys for signed events", keys.len());
                assert!(
                    keys.len() >= 1,
                    "Expected at least 1 note, got {}",
                    keys.len()
                );
            }
            Ok(Err(e)) => {
                panic!("Subscription error: {:?}", e);
            }
            Err(_) => {
                panic!("TIMEOUT: Signed events were not processed");
            }
        }
    }

    #[tokio::test]
    async fn test_events_persist_across_restart() {
        use crate::publication::build_signed_publication_events;
        use crate::tree::state::ComposeState;
        use nostrdb::FilterBuilder;

        // Use a non-temp directory so we can reopen it
        let test_dir = std::path::PathBuf::from("target/testdbs/persistence_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(&test_dir).unwrap();

        // Known test secret
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";

        // Derive pubkey
        use secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized = public_key.serialize();
        let pubkey = hex::encode(&serialized[1..33]);

        let event_id: String;

        // PHASE 1: Create engine, ingest event, drop engine
        {
            let engine = Engine::with_config(&test_dir, &[], 1000).unwrap();

            // Create a publication
            use crate::tree::state::SectionCompose;
            let mut compose = ComposeState::new();
            compose.title = "Persistence Test".to_string();
            compose.sections.push(SectionCompose {
                title: "Section 1".to_string(),
                content: "This should persist!".to_string(),
                ..Default::default()
            });

            let (pub_event, _section_events) =
                build_signed_publication_events(&mut compose, &pubkey, secret_hex);
            event_id = pub_event.get("id").unwrap().as_str().unwrap().to_string();

            println!("Created event with ID: {}", event_id);

            // Subscribe and ingest
            let filter = FilterBuilder::new().kinds([30040]).build();
            let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

            let pub_json = serde_json::to_string(&pub_event).unwrap();
            engine.ingest_event(&pub_json).expect("ingest should work");

            // Wait for processing
            let keys = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                engine.ndb.wait_for_notes(sub, 1),
            )
            .await
            .expect("timeout")
            .expect("wait_for_notes");

            println!("Phase 1: Ingested and got {} note keys", keys.len());
            assert!(!keys.is_empty(), "Event not processed in phase 1");

            // Drop engine (should flush to disk)
            drop(engine);
            println!("Phase 1: Engine dropped");
        }

        // Small delay to ensure filesystem sync
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // PHASE 2: Reopen engine, query for the event
        {
            let engine = Engine::with_config(&test_dir, &[], 1000).unwrap();
            println!("Phase 2: Engine reopened");

            // Query for the event
            let filter = serde_json::json!({
                "kinds": [30040],
                "limit": 10
            });
            let events = query::query_local(&engine.ndb, &[filter]).unwrap();

            println!("Phase 2: Found {} events", events.len());

            // Check if our event is there
            let found = events
                .iter()
                .any(|e| e.get("id").and_then(|v| v.as_str()) == Some(&event_id));

            if !found {
                println!(
                    "Events found: {:?}",
                    events.iter().map(|e| e.get("id")).collect::<Vec<_>>()
                );
            }

            assert!(found, "Event {} not found after restart!", event_id);
            println!("SUCCESS: Event persisted across restart!");
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&test_dir);
    }

    /// Helper: build a signed event with given kind, content, tags
    fn build_test_event(kind: u64, content: &str, tags: Vec<Vec<&str>>, created_at: u64) -> String {
        use secp256k1::{PublicKey, Secp256k1, SecretKey};
        use sha2::{Digest, Sha256};

        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey = hex::encode(&public_key.serialize()[1..33]);

        let tags_json: Vec<Vec<String>> = tags
            .iter()
            .map(|t| t.iter().map(|s| s.to_string()).collect())
            .collect();

        let serial = serde_json::json!([0, pubkey, created_at, kind, tags_json, content]);
        let serial_str = serde_json::to_string(&serial).unwrap();

        let mut hasher = Sha256::new();
        hasher.update(serial_str.as_bytes());
        let hash = hasher.finalize();
        let id_hex = hex::encode(&hash);

        let sig = crate::identity::sign_event_hash(&id_hex, secret_hex).unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": id_hex,
            "pubkey": pubkey,
            "created_at": created_at,
            "kind": kind,
            "tags": tags_json,
            "content": content,
            "sig": sig
        }))
        .unwrap()
    }

    /// Helper: ingest events and wait for processing
    async fn ingest_and_wait(engine: &Engine, events: &[String]) {
        use nostrdb::FilterBuilder;

        let filter = FilterBuilder::new().limit(events.len() as u64 + 10).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        for event_json in events {
            engine.ingest_event(event_json).expect("ingest");
        }

        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            engine.ndb.wait_for_notes(sub, events.len() as u32),
        )
        .await;
    }

    #[tokio::test]
    async fn test_search_by_tag() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(
                30041,
                "Python tutorial",
                vec![vec!["t", "python"], vec!["d", "s1"]],
                1700000001,
            ),
            build_test_event(
                30041,
                "Rust guide",
                vec![vec!["t", "rust"], vec!["d", "s2"]],
                1700000002,
            ),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("t:python").unwrap();
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert!(
            !response.results.is_empty(),
            "Should find python-tagged events"
        );
        assert!(response.results.iter().all(|r| {
            r.tags.iter().any(|t| {
                t.get(0).map(|s| s.as_str()) == Some("t")
                    && t.get(1).map(|s| s.as_str()) == Some("python")
            })
        }));
    }

    #[tokio::test]
    async fn test_search_by_text_keyword() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(1, "neural network training", vec![], 1700000001),
            build_test_event(1, "hello world", vec![], 1700000002),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("neural").unwrap();
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert_eq!(response.results.len(), 1);
        assert!(response.results[0].preview.contains("neural"));
    }

    #[tokio::test]
    async fn test_search_by_kind() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(30041, "A section", vec![vec!["d", "sec1"]], 1700000001),
            build_test_event(1, "A note", vec![], 1700000002),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("k:30041").unwrap();
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert!(response.results.iter().all(|r| r.kind == 30041));
    }

    #[tokio::test]
    async fn test_search_by_id_finds_local_event() {
        // The `id:` / nevent path end-to-end: ingest an event, then look
        // it up by its event id against the local DB only.
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let event = build_test_event(1, "find me by id", vec![], 1700000001);
        let id = serde_json::from_str::<Value>(&event).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        ingest_and_wait(&engine, &[event]).await;

        let query = SearchQuery::parse(&format!("id:{}", id)).unwrap();
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert_eq!(response.results.len(), 1, "id: should find the event");
        assert_eq!(response.results[0].event_id, id);
    }

    #[tokio::test]
    async fn test_search_empty_results() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let query = SearchQuery::parse("t:nonexistent").unwrap();
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert_eq!(response.count, 0);
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn test_search_respects_limit() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events: Vec<String> = (0..10)
            .map(|i| build_test_event(1, &format!("note {}", i), vec![], 1700000000 + i))
            .collect();
        ingest_and_wait(&engine, &events).await;

        let mut query = SearchQuery::parse("note").unwrap();
        query.limit = Some(3);
        let response = engine
            .search(&query, FetchPolicy::LocalOnly, None)
            .await
            .unwrap();

        assert!(response.results.len() <= 3);
    }

    /// `record_event_relay` must persist the source relay against the event
    /// id so `note.relays(txn)` (surfaced as the `relays` field on
    /// `GET /api/v1/events/:id`) reflects it. Re-attributing the same event
    /// from a second relay appends rather than replacing — the per-event
    /// set is append-only and survives nostrdb's dedup.
    ///
    /// Mocking a real WebSocket relay round-trip would be heavy; the audit
    /// invariant is satisfied by exercising `record_event_relay` directly,
    /// since `relay.rs:fetch_with_filters` funnels every inbound EVENT
    /// through the same `IngestMetadata::relay(url)` write path.
    #[tokio::test]
    async fn test_record_event_relay_persists_provenance() {
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // A signed kind-1 event (built the same way as the other tests).
        let event_json =
            build_test_event(1, "relay provenance probe", vec![], 1_700_000_001);
        let event_value: serde_json::Value = serde_json::from_str(&event_json).unwrap();
        let event_id = event_value["id"].as_str().unwrap().to_string();

        // Subscribe before writing so we can await processing.
        let filter = FilterBuilder::new().kinds([1]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Record from relay A.
        engine
            .record_event_relay(&event_json, "wss://relay.example/")
            .expect("record relay A");
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            engine.ndb.wait_for_notes(sub, 1),
        )
        .await
        .expect("ndb timeout")
        .expect("wait_for_notes");

        let value = crate::query::query_by_id(&engine.ndb, &event_id)
            .expect("query")
            .expect("event present");
        let relays: Vec<String> = value["relays"]
            .as_array()
            .expect("relays array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        // record_event_relay normalizes the URL (strips the trailing slash)
        // before tagging the note, so provenance reflects the canonical form.
        assert!(
            relays.iter().any(|r| r == "wss://relay.example"),
            "expected relay A in provenance, got {:?}",
            relays
        );

        // Re-attribute via a second relay. The same event id must now carry
        // both — nostrdb's per-event relay set is append-only.
        engine
            .record_event_relay(&event_json, "wss://relay.other/")
            .expect("record relay B");
        // Give nostrdb a moment to process the second ingest.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let value = crate::query::query_by_id(&engine.ndb, &event_id)
            .expect("re-query")
            .expect("event still present");
        let relays: Vec<String> = value["relays"]
            .as_array()
            .expect("relays array")
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(
            relays.iter().any(|r| r == "wss://relay.example")
                && relays.iter().any(|r| r == "wss://relay.other"),
            "expected both relays after second attribution, got {:?}",
            relays
        );
    }

    /// `engine.add_relay` / `remove_relay` mutate the live in-memory
    /// `RelayConfig` (not just `relays.json`) so API-driven edits take
    /// effect for the running process without a restart. Locks in the
    /// fix that previously made the relays buffer's toggles look like
    /// they worked while the engine kept fetching from the old set.
    #[tokio::test]
    async fn test_add_and_remove_relay_mutates_in_memory_and_disk() {
        let temp_dir = tempdir().expect("temp dir");
        // Boot with no initial_relays so we can observe each add cleanly.
        let cfg = RelayConfig {
            initial_relays: Vec::new(),
            ..RelayConfig::default()
        };
        let engine = Engine::with_relay_config(temp_dir.path(), &cfg).expect("engine");

        assert!(engine.publish_relays().is_empty(), "publish should start empty");
        assert!(engine.relays().is_empty(), "fetch should start empty");

        // Add through &self (this is the API-handler path).
        assert!(engine.add_relay("publish", "wss://target.relay"));
        assert!(engine.add_relay("fetch", "wss://reader.relay"));

        // In-memory view reflects the add immediately — no restart.
        assert_eq!(engine.publish_relays(), vec!["wss://target.relay".to_string()]);
        assert_eq!(engine.relays(), vec!["wss://reader.relay".to_string()]);

        // Disk file also reflects the add — open a fresh store and read.
        let store = crate::relay_store::RelayStore::new(temp_dir.path()).expect("store");
        let sets = store.load().expect("load");
        assert_eq!(sets.publish, vec!["wss://target.relay".to_string()]);
        assert_eq!(sets.fetch, vec!["wss://reader.relay".to_string()]);

        // Duplicate add is a no-op (returns false).
        assert!(!engine.add_relay("publish", "wss://target.relay"));

        // Remove through &self mirrors add.
        assert!(engine.remove_relay("publish", "wss://target.relay"));
        assert!(engine.publish_relays().is_empty());
        let sets = store.load().expect("reload");
        assert!(sets.publish.is_empty());

        // Remove of a missing relay is a no-op.
        assert!(!engine.remove_relay("publish", "wss://never-added"));
    }
}
