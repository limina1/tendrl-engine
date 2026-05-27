//! Persistent runtime relay sets.
//!
//! The engine keeps three working relay sets — `general`, `fetch`, `publish` —
//! and they're mutable from the UI. Historically they lived in `config.toml`
//! under `[relay.general]` / `[relay.fetch]` / `[relay.publish]`. That made
//! "edit a relay" mean "rewrite the user's config file", and it conflated the
//! bootstrap seed list with the live working set.
//!
//! This module is the new home for the live sets. They are persisted as a
//! plain JSON file at `<data_dir>/relays.json`, mirroring the `DraftStore`
//! pattern. The TOML config only carries `initial_relays`, used **once** on
//! first boot to seed the file. After that the JSON file is authoritative —
//! `config.toml` is never rewritten by relay UI actions.
//!
//! Philosophy: tendrl is a deliberate-publishing client. Relay sets are local
//! workbench state, not Nostr events. This module never publishes anything;
//! every `add` / `remove` is a local file write.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Result type for relay-store operations.
pub type Result<T> = std::result::Result<T, RelayStoreError>;

/// Errors that can occur loading or persisting the relay store.
#[derive(Debug, thiserror::Error)]
pub enum RelayStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Unknown relay set: {0}")]
    UnknownSet(String),
}

/// The three working relay URL lists. Serialized as
/// `{"general":[…],"fetch":[…],"publish":[…]}` in `relays.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelaySets {
    #[serde(default)]
    pub general: Vec<String>,
    #[serde(default)]
    pub fetch: Vec<String>,
    #[serde(default)]
    pub publish: Vec<String>,
    /// Aggregator relays (nostr.land etc.) — never auto-targeted by
    /// publish. Reserved for explicit per-event "also broadcast" opt-in
    /// once per-event-kind routing lands. See [[project-relay-classes]].
    #[serde(default)]
    pub broadcast: Vec<String>,
    /// Search-capable relays (NIP-50). Used by `~:` semantic queries.
    #[serde(default)]
    pub search: Vec<String>,
    /// Indexer / discovery relays (purplepag.es etc.). Fallback tier
    /// for kind 0 / 10002 lookups when the read set misses.
    #[serde(default)]
    pub indexer: Vec<String>,
    /// User-defined named relay sets — NIP-51 kind 30002. Thematic
    /// groupings ("research", "friends-only", "high-priority") that
    /// can be published as kind 30002 for sharing or backup. Orthogonal
    /// to the functional classes above: a relay can be in any
    /// combination of classes AND any number of named sets.
    #[serde(default)]
    pub named: Vec<NamedRelaySet>,
}

/// A NIP-51 kind 30002 relay set — user-named grouping of relays.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamedRelaySet {
    /// `d` tag value — stable identifier for replaceable-event addressing.
    /// Auto-generated on create; never renamed.
    pub d_tag: String,
    /// Human-readable title (`title` tag value). Editable.
    pub title: String,
    /// Member relay URLs (published as `r` tags on kind 30002).
    pub urls: Vec<String>,
}

impl RelaySets {
    /// Seed read/write/general sets from the bootstrap `initial_relays`
    /// list. `broadcast` is deliberately left empty: aggregators are
    /// opt-in, not auto-populated from the initial seed.
    pub fn seed_from_initial(initial: &[String]) -> Self {
        let normalized = normalize_dedup(initial);
        Self {
            general: normalized.clone(),
            fetch: normalized.clone(),
            publish: normalized,
            broadcast: Vec::new(),
            search: Vec::new(),
            indexer: Vec::new(),
            named: Vec::new(),
        }
    }

    /// Normalize every URL in each set and drop duplicates. Idempotent —
    /// safe to call on already-normalized sets. Called after `load()` so
    /// any legacy disk content gets canonicalized on next save.
    fn canonicalize(&mut self) {
        self.general = normalize_dedup(&self.general);
        self.fetch = normalize_dedup(&self.fetch);
        self.publish = normalize_dedup(&self.publish);
        self.broadcast = normalize_dedup(&self.broadcast);
        self.search = normalize_dedup(&self.search);
        self.indexer = normalize_dedup(&self.indexer);
        for s in &mut self.named {
            s.urls = normalize_dedup(&s.urls);
        }
    }

    /// Borrow the URL list for a named set. Returns `None` for unknown set
    /// names so callers can ignore stray UI input rather than panic.
    pub fn get(&self, set: &str) -> Option<&[String]> {
        match set {
            "general" => Some(&self.general),
            "fetch" => Some(&self.fetch),
            "publish" => Some(&self.publish),
            "broadcast" => Some(&self.broadcast),
            "search" => Some(&self.search),
            "indexer" => Some(&self.indexer),
            _ => None,
        }
    }

    fn get_mut(&mut self, set: &str) -> Result<&mut Vec<String>> {
        match set {
            "general" => Ok(&mut self.general),
            "fetch" => Ok(&mut self.fetch),
            "publish" => Ok(&mut self.publish),
            "broadcast" => Ok(&mut self.broadcast),
            "search" => Ok(&mut self.search),
            "indexer" => Ok(&mut self.indexer),
            other => Err(RelayStoreError::UnknownSet(other.to_string())),
        }
    }
}

/// Persisted relay sets backed by a JSON file under the engine's data dir.
///
/// Operations are synchronous and small — the file is rewritten in full on
/// every mutation. The store owns nothing in memory beyond the file path;
/// the in-memory working copy lives on `Engine` itself. This matches how
/// `DraftStore` and `LocalPublicationTracker` work in `drafts.rs`.
pub struct RelayStore {
    file_path: PathBuf,
}

impl RelayStore {
    /// Create a store rooted at `<data_dir>/relays.json`. The parent
    /// directory must already exist (the engine ensures this for the
    /// nostrdb data path on startup).
    pub fn new(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        let file_path = data_dir.join("relays.json");
        Ok(Self { file_path })
    }

    /// True when no `relays.json` exists yet — caller should seed from
    /// `initial_relays` and then `save` the result.
    pub fn is_first_boot(&self) -> bool {
        !self.file_path.exists()
    }

    /// Load relay sets from disk. Returns the empty `RelaySets` if the
    /// file is missing — callers usually want to check `is_first_boot()`
    /// first and seed.
    pub fn load(&self) -> Result<RelaySets> {
        if !self.file_path.exists() {
            return Ok(RelaySets::default());
        }
        let raw = fs::read_to_string(&self.file_path)?;
        let mut sets: RelaySets = serde_json::from_str(&raw)?;
        // Canonicalize loaded URLs so historical disk state with mixed-
        // case / trailing-slash entries collapses to one chip per relay.
        // Caller's next save will write the cleaned form.
        sets.canonicalize();
        Ok(sets)
    }

    /// Overwrite `relays.json` with the given sets. Writes pretty-printed
    /// JSON for human-readability (the file is small and edited by hand
    /// during development).
    pub fn save(&self, sets: &RelaySets) -> Result<()> {
        let content = serde_json::to_string_pretty(sets)?;
        fs::write(&self.file_path, content)?;
        Ok(())
    }

    /// Path the store writes to — exposed for logging.
    pub fn path(&self) -> &Path {
        &self.file_path
    }

    /// Add `url` to the named set if it isn't already present, then
    /// persist the result. The mutation is applied to `sets` in place so
    /// the caller's in-memory copy stays in sync with the file.
    pub fn add(&self, sets: &mut RelaySets, set: &str, url: &str) -> Result<bool> {
        let list = sets.get_mut(set)?;
        if list.iter().any(|u| u == url) {
            return Ok(false);
        }
        list.push(url.to_string());
        self.save(sets)?;
        Ok(true)
    }

    /// Remove `url` from the named set (if present) and persist. Returns
    /// whether anything was actually removed.
    pub fn remove(&self, sets: &mut RelaySets, set: &str, url: &str) -> Result<bool> {
        let list = sets.get_mut(set)?;
        let before = list.len();
        list.retain(|u| u != url);
        let changed = list.len() != before;
        if changed {
            self.save(sets)?;
        }
        Ok(changed)
    }
}

/// Run every URL through `crate::relay_url::normalize_relay_url`,
/// drop empty strings, and dedupe — preserving insertion order so the
/// resulting list reads the same way the user typed it (modulo
/// canonicalization).
fn normalize_dedup(urls: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(urls.len());
    for u in urls {
        let n = crate::relay_url::normalize_relay_url(u);
        if n.is_empty() {
            continue;
        }
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn first_boot_then_seed_persists_to_disk() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        assert!(store.is_first_boot());

        let initial = vec!["wss://foo".to_string(), "wss://bar".to_string()];
        let sets = RelaySets::seed_from_initial(&initial);
        store.save(&sets).unwrap();

        assert!(!store.is_first_boot());
        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.general, initial);
        assert_eq!(reloaded.fetch, initial);
        assert_eq!(reloaded.publish, initial);
    }

    #[test]
    fn add_and_remove_round_trip_through_disk() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        assert!(store.add(&mut sets, "fetch", "wss://a").unwrap());
        // Idempotent — adding the same URL twice returns `false`.
        assert!(!store.add(&mut sets, "fetch", "wss://a").unwrap());
        assert!(store.add(&mut sets, "publish", "wss://b").unwrap());

        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.fetch, vec!["wss://a".to_string()]);
        assert_eq!(reloaded.publish, vec!["wss://b".to_string()]);
        assert!(reloaded.general.is_empty());

        assert!(store.remove(&mut sets, "fetch", "wss://a").unwrap());
        assert!(!store.remove(&mut sets, "fetch", "wss://a").unwrap());

        let reloaded = store.load().unwrap();
        assert!(reloaded.fetch.is_empty());
        assert_eq!(reloaded.publish, vec!["wss://b".to_string()]);
    }

    #[test]
    fn unknown_set_is_an_error_not_a_panic() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();
        let err = store.add(&mut sets, "nonsense", "wss://x").unwrap_err();
        assert!(matches!(err, RelayStoreError::UnknownSet(_)));
    }
}
