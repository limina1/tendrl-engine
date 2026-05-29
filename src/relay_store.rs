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

/// The working relay URL lists. Serialized as
/// `{"general":[…],"fetch":[…],"publish":[…],"search":{"default":[…],"fallback":[…]},…}`
/// in `relays.json`.
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
    /// Search-capable relays (NIP-50). Split into `default` (joined
    /// into every primary `~:` fan-out) and `fallback` (consulted only
    /// when default returns zero / errors / times out). When
    /// `exclusive=true` at the engine level, `default` REPLACES read
    /// relays for `~:` queries rather than joining them.
    /// Backward-compat: a flat `Vec<String>` deserializes into
    /// `default` with empty `fallback`.
    #[serde(default, deserialize_with = "deserialize_discovery_class")]
    pub search: DiscoveryClass,
    /// Indexer / discovery relays (purplepag.es etc.). Same shape as
    /// `search`: `default` joins (or replaces, with `exclusive`) the
    /// primary read fan-out for kind 0 / 10002 lookups; `fallback`
    /// kicks in only on a default-miss.
    #[serde(default, deserialize_with = "deserialize_discovery_class")]
    pub indexer: DiscoveryClass,
    /// User-defined named relay sets — NIP-51 kind 30002. Thematic
    /// groupings ("research", "friends-only", "high-priority") that
    /// can be published as kind 30002 for sharing or backup. Orthogonal
    /// to the functional classes above: a relay can be in any
    /// combination of classes AND any number of named sets.
    #[serde(default)]
    pub named: Vec<NamedRelaySet>,
    /// Per-class `exclusive` toggle for discovery classes. When ON for
    /// a class, the engine bypasses read relays entirely for that
    /// lookup type — primary = `class.default` only, fallback =
    /// `class.fallback` only. Keyed by class name (`"search"`,
    /// `"indexer"`). Default OFF for both = read relays are the
    /// primary floor.
    #[serde(default)]
    pub exclusive: ExclusiveFlags,
}

/// Two-tier membership within a discovery class (search / indexer).
///
/// `default` URLs join (or replace, with `exclusive`) the primary
/// fan-out. `fallback` URLs are only consulted when the primary
/// returned zero events. The two are mutually exclusive *per URL per
/// class*: a relay is either default OR fallback for `search`, not
/// both. Across classes the modes are independent — the same URL can
/// be `search.default` AND `indexer.fallback` simultaneously.
///
/// Empty `default` AND empty `fallback` for a class = OFF, no extra
/// fan-out for that lookup type. The read relays handle everything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryClass {
    #[serde(default)]
    pub default: Vec<String>,
    #[serde(default)]
    pub fallback: Vec<String>,
}

impl DiscoveryClass {
    pub fn is_empty(&self) -> bool {
        self.default.is_empty() && self.fallback.is_empty()
    }

    /// True if `url` appears in either tier.
    pub fn contains(&self, url: &str) -> bool {
        self.default.iter().any(|u| u == url)
            || self.fallback.iter().any(|u| u == url)
    }

    /// Strip `url` from both tiers. Returns true if anything was
    /// removed. Used by `add` to enforce mutual exclusion: switching a
    /// URL from default → fallback (or back) requires first removing
    /// it from its current tier.
    pub fn remove_url(&mut self, url: &str) -> bool {
        let before_d = self.default.len();
        let before_f = self.fallback.len();
        self.default.retain(|u| u != url);
        self.fallback.retain(|u| u != url);
        self.default.len() != before_d || self.fallback.len() != before_f
    }
}

/// Backward-compat deserialization: accepts both the new
/// `{"default": [...], "fallback": [...]}` shape AND a legacy flat
/// `[...]` array. Flat arrays are promoted into `.default`, leaving
/// `.fallback` empty — preserving prior behavior so older
/// `relays.json` files keep working.
fn deserialize_discovery_class<'de, D>(d: D) -> std::result::Result<DiscoveryClass, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        Flat(Vec<String>),
        Structured {
            #[serde(default)]
            default: Vec<String>,
            #[serde(default)]
            fallback: Vec<String>,
        },
    }
    Ok(match Form::deserialize(d)? {
        Form::Flat(urls) => DiscoveryClass {
            default: urls,
            fallback: Vec::new(),
        },
        Form::Structured { default, fallback } => DiscoveryClass { default, fallback },
    })
}

/// `exclusive` toggles per discovery class. Drives the
/// composition-matrix interpretation in `Engine::profile_lookup` etc.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExclusiveFlags {
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub indexer: bool,
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
    /// list, and seed the discovery classes with the engine's
    /// well-known defaults (`crate::relay::DEFAULT_INDEXERS` /
    /// `DEFAULT_SEARCH`) so a fresh install has a working indexer
    /// fallback chain without manual configuration. Broadcast stays
    /// empty — aggregators are opt-in per-event.
    ///
    /// Discovery defaults only apply on **first boot** (when no
    /// `relays.json` exists yet); subsequent loads honor the user's
    /// stored state, including explicit-empty.
    pub fn seed_from_initial(initial: &[String]) -> Self {
        let normalized = normalize_dedup(initial);
        let default_indexers: Vec<String> = crate::relay::DEFAULT_INDEXERS
            .iter()
            .map(|s| s.to_string())
            .collect();
        let default_search: Vec<String> = crate::relay::DEFAULT_SEARCH
            .iter()
            .map(|s| s.to_string())
            .collect();
        Self {
            general: normalized.clone(),
            fetch: normalized.clone(),
            publish: normalized,
            broadcast: Vec::new(),
            search: DiscoveryClass {
                default: normalize_dedup(&default_search),
                fallback: Vec::new(),
            },
            indexer: DiscoveryClass {
                default: normalize_dedup(&default_indexers),
                fallback: Vec::new(),
            },
            named: Vec::new(),
            exclusive: ExclusiveFlags::default(),
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
        self.search.default = normalize_dedup(&self.search.default);
        self.search.fallback = normalize_dedup(&self.search.fallback);
        self.indexer.default = normalize_dedup(&self.indexer.default);
        self.indexer.fallback = normalize_dedup(&self.indexer.fallback);
        for s in &mut self.named {
            s.urls = normalize_dedup(&s.urls);
        }
    }

    /// Borrow the URL list for a set. Accepts both flat names
    /// (`"general"`, `"fetch"`, `"publish"`, `"broadcast"`) and dotted
    /// discovery-class names (`"search.default"`, `"search.fallback"`,
    /// `"indexer.default"`, `"indexer.fallback"`). Returns `None` for
    /// unknown names so callers can ignore stray UI input rather
    /// than panic.
    pub fn get(&self, set: &str) -> Option<&[String]> {
        match set {
            "general" => Some(&self.general),
            "fetch" => Some(&self.fetch),
            "publish" => Some(&self.publish),
            "broadcast" => Some(&self.broadcast),
            "search.default" => Some(&self.search.default),
            "search.fallback" => Some(&self.search.fallback),
            "indexer.default" => Some(&self.indexer.default),
            "indexer.fallback" => Some(&self.indexer.fallback),
            _ => None,
        }
    }

    fn get_mut(&mut self, set: &str) -> Result<&mut Vec<String>> {
        match set {
            "general" => Ok(&mut self.general),
            "fetch" => Ok(&mut self.fetch),
            "publish" => Ok(&mut self.publish),
            "broadcast" => Ok(&mut self.broadcast),
            "search.default" => Ok(&mut self.search.default),
            "search.fallback" => Ok(&mut self.search.fallback),
            "indexer.default" => Ok(&mut self.indexer.default),
            "indexer.fallback" => Ok(&mut self.indexer.fallback),
            other => Err(RelayStoreError::UnknownSet(other.to_string())),
        }
    }

    /// Borrow the `DiscoveryClass` for `"search"` or `"indexer"`.
    /// Returns `None` for any other name.
    pub fn discovery(&self, class: &str) -> Option<&DiscoveryClass> {
        match class {
            "search" => Some(&self.search),
            "indexer" => Some(&self.indexer),
            _ => None,
        }
    }

    /// Merge the well-known default discovery URLs into `default`
    /// tier for whichever class is empty. Used by the
    /// `/api/v1/restore-defaults/indexer` style endpoints that let
    /// existing users (with non-empty `relays.json` from before
    /// discovery defaults existed) opt into the same set a fresh
    /// install would get. Returns how many URLs were added across
    /// both classes.
    pub fn merge_discovery_defaults(&mut self) -> usize {
        let mut added = 0;
        for &url in crate::relay::DEFAULT_INDEXERS {
            let u = crate::relay_url::normalize_relay_url(url);
            if !self.indexer.contains(&u) {
                self.indexer.default.push(u);
                added += 1;
            }
        }
        for &url in crate::relay::DEFAULT_SEARCH {
            let u = crate::relay_url::normalize_relay_url(url);
            if !self.search.contains(&u) {
                self.search.default.push(u);
                added += 1;
            }
        }
        added
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
    ///
    /// For discovery classes (`search.default` / `search.fallback` /
    /// `indexer.default` / `indexer.fallback`) this enforces the
    /// per-URL mutual exclusion: adding to one tier strips the URL
    /// from the other tier of the same class. The user's intent
    /// "move this URL to fallback" expresses as a single `add` call;
    /// no separate `remove` needed.
    pub fn add(&self, sets: &mut RelaySets, set: &str, url: &str) -> Result<bool> {
        // Strip from the sibling tier first so the same URL never
        // appears in both default AND fallback within a class.
        let sibling = match set {
            "search.default" => Some("search.fallback"),
            "search.fallback" => Some("search.default"),
            "indexer.default" => Some("indexer.fallback"),
            "indexer.fallback" => Some("indexer.default"),
            _ => None,
        };
        if let Some(s) = sibling {
            if let Ok(list) = sets.get_mut(s) {
                list.retain(|u| u != url);
            }
        }

        let list = sets.get_mut(set)?;
        if list.iter().any(|u| u == url) {
            // Already in the target tier and (possibly) just had to
            // be stripped from the sibling — persist so the strip
            // lands on disk too.
            self.save(sets)?;
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

    #[test]
    fn discovery_class_round_trips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        assert!(store.add(&mut sets, "search.default", "wss://search-a").unwrap());
        assert!(store.add(&mut sets, "search.fallback", "wss://search-b").unwrap());
        assert!(store.add(&mut sets, "indexer.default", "wss://idx-a").unwrap());

        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.search.default, vec!["wss://search-a".to_string()]);
        assert_eq!(reloaded.search.fallback, vec!["wss://search-b".to_string()]);
        assert_eq!(reloaded.indexer.default, vec!["wss://idx-a".to_string()]);
        assert!(reloaded.indexer.fallback.is_empty());
    }

    #[test]
    fn legacy_flat_search_array_migrates_to_default() {
        // Older relays.json with `"search": ["wss://x"]` should load
        // as DiscoveryClass { default: ["wss://x"], fallback: [] }.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let legacy_json = r#"{
            "general": [],
            "fetch": [],
            "publish": [],
            "broadcast": [],
            "search": ["wss://nostr.search.example"],
            "indexer": ["wss://purplepag.es"],
            "named": []
        }"#;
        std::fs::write(store.path(), legacy_json).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.search.default,
            vec!["wss://nostr.search.example".to_string()]
        );
        assert!(loaded.search.fallback.is_empty());
        assert_eq!(loaded.indexer.default, vec!["wss://purplepag.es".to_string()]);
        assert!(loaded.indexer.fallback.is_empty());
    }

    #[test]
    fn adding_to_one_tier_strips_url_from_sibling_tier() {
        // Per-URL mutual exclusion within a discovery class.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        assert!(store.add(&mut sets, "search.default", "wss://a").unwrap());
        // Re-adding to the fallback tier moves it; default should be empty.
        store.add(&mut sets, "search.fallback", "wss://a").unwrap();

        let reloaded = store.load().unwrap();
        assert!(reloaded.search.default.is_empty());
        assert_eq!(reloaded.search.fallback, vec!["wss://a".to_string()]);
    }

    #[test]
    fn exclusive_flag_round_trips() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();
        sets.exclusive.search = true;
        store.save(&sets).unwrap();

        let reloaded = store.load().unwrap();
        assert!(reloaded.exclusive.search);
        assert!(!reloaded.exclusive.indexer);
    }
}
