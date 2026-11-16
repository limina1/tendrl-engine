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

/// Current default-seed generation. Bump when new engine defaults
/// should reach existing installs; add the corresponding merge step in
/// [`RelaySets::apply_default_upgrades`].
pub const CURRENT_SEED_VERSION: u32 = 2;

/// The kinds a former *indexer* relay (purplepag.es etc.) is asked for
/// now that the indexer class is gone: an indexer is just a search relay
/// narrowed to profile metadata and relay lists.
pub const INDEXER_KINDS: &[u64] = &[0, 3, 10002, 10007, 10088, 30002];

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
    /// Relays tendrl authenticates to (NIP-42) when they challenge.
    /// Opt-in per relay: a relay outside this set has its `AUTH`
    /// challenge ignored and its `auth-required` refusal recorded as
    /// status — the user's pubkey is never revealed to a relay they did
    /// not pick. Local-only like every set here; never published.
    /// See `docs/nip42-relay-auth-plan.org`.
    #[serde(default)]
    pub auth: Vec<String>,
    /// Search relays — a per-relay flag like read/write. A search relay
    /// joins the read set for every query intent whose kinds it serves
    /// (see `search_kinds`); with no kinds listed it serves any kind.
    /// Backward-compat: the old `{"default": […], "fallback": […]}`
    /// two-tier shape deserializes into the union.
    #[serde(default, deserialize_with = "deserialize_flat_or_tiers")]
    pub search: Vec<String>,
    /// Legacy `indexer` class, read only so
    /// [`RelaySets::apply_default_upgrades`] can fold it into `search`
    /// + `search_kinds`. Never written back.
    #[serde(
        default,
        rename = "indexer",
        skip_serializing,
        deserialize_with = "deserialize_flat_or_tiers"
    )]
    pub legacy_indexer: Vec<String>,
    /// User-defined named relay sets — NIP-51 kind 30002. Thematic
    /// groupings ("research", "friends-only", "high-priority") that
    /// can be published as kind 30002 for sharing or backup. Orthogonal
    /// to the functional classes above: a relay can be in any
    /// combination of classes AND any number of named sets.
    #[serde(default)]
    pub named: Vec<NamedRelaySet>,
    /// Per-relay **search kinds**: the event kinds a search relay is
    /// asked for, keyed by normalized URL. They narrow the `search` flag:
    /// a fetch for kind K goes to the read relays **plus** every search
    /// relay listing K (a union — nothing is excluded). A search relay
    /// with no entry serves any kind; a filter naming no kind reaches
    /// only those unrestricted search relays. Kinds stay on file when
    /// the search flag is off, but are ignored.
    ///
    /// (Formerly `resolve_kinds`, an exclusive per-kind claim.)
    /// See `docs/zettel/idea-relay-kind-routing.org`.
    #[serde(default, alias = "resolve_kinds")]
    pub search_kinds: std::collections::BTreeMap<String, Vec<u64>>,
    /// Inactive relays: URL → the set names it belonged to when it was
    /// deactivated (`"general"`, `"fetch"`, `"publish"`, `"broadcast"`,
    /// `"auth"`, `"search"`). Presence in this map IS the inactive state:
    /// the URL is removed from those working sets (so every engine
    /// consumer skips it with zero filtering) and reactivation re-adds it
    /// to exactly the recorded sets. This is what lets the user park a
    /// relay — keep its URL and role configuration — without deleting it.
    /// Search kinds are retained but ignored while inactive.
    #[serde(default)]
    pub inactive: std::collections::BTreeMap<String, Vec<String>>,
    /// Highest default-seed generation this file has absorbed. Lets new
    /// engine defaults reach EXISTING installs additively: on load, a
    /// file whose stamp is behind `CURRENT_SEED_VERSION` gets the newer
    /// defaults merged in (never overwriting user edits) and the stamp
    /// bumped. Files predating the field deserialize as 0.
    #[serde(default)]
    pub seed_version: u32,
}

/// Backward-compat deserialization for a flat relay list that used to be
/// a two-tier `{"default": [...], "fallback": [...]}` class: accepts
/// both shapes and yields the union, default tier first.
fn deserialize_flat_or_tiers<'de, D>(d: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        Flat(Vec<String>),
        Tiers {
            #[serde(default)]
            default: Vec<String>,
            #[serde(default)]
            fallback: Vec<String>,
        },
    }
    Ok(match Form::deserialize(d)? {
        Form::Flat(urls) => urls,
        Form::Tiers { default, mut fallback } => {
            let mut out = default;
            out.append(&mut fallback);
            out
        }
    })
}

/// The search relays that serve a fetch for `kinds` — the pure rule
/// behind `Engine::search_relays_for_kinds`. `kinds = None` is a filter
/// naming no kind (an id lookup, a `#e` thread): only unrestricted
/// search relays qualify. Order follows `search`.
pub fn search_relays_matching(
    search: &[String],
    search_kinds: &std::collections::BTreeMap<String, Vec<u64>>,
    kinds: Option<&[u64]>,
) -> Vec<String> {
    search
        .iter()
        .filter(|url| match search_kinds.get(*url) {
            None => true,
            Some(listed) if listed.is_empty() => true,
            Some(listed) => kinds.is_some_and(|ks| ks.iter().any(|k| listed.contains(k))),
        })
        .cloned()
        .collect()
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
    /// list, and seed the search set with the engine's well-known
    /// profile/relay-list relays (`crate::relay::DEFAULT_INDEXERS`,
    /// narrowed to [`INDEXER_KINDS`]) plus the default search-kind
    /// relays, so a fresh install can find profiles without manual
    /// configuration. Broadcast stays empty — aggregators are opt-in.
    ///
    /// Only applies on **first boot** (no `relays.json` yet); later
    /// loads honor the user's stored state, including explicit-empty.
    pub fn seed_from_initial(initial: &[String]) -> Self {
        let normalized = normalize_dedup(initial);
        // Publish gets the same seed minus the read-only defaults —
        // content sources (theforest, mercury) shouldn't become publish
        // targets just by being in the bootstrap list.
        let read_only: Vec<String> = crate::relay::DEFAULT_READ_ONLY
            .iter()
            .map(|u| crate::relay_url::normalize_relay_url(u))
            .collect();
        let publish: Vec<String> = normalized
            .iter()
            .filter(|u| !read_only.contains(u))
            .cloned()
            .collect();
        let mut sets = Self {
            general: normalized.clone(),
            fetch: normalized,
            publish,
            seed_version: CURRENT_SEED_VERSION,
            ..Self::default()
        };
        for url in crate::relay::DEFAULT_SEARCH {
            sets.add_search_relay(url, &[]);
        }
        for (url, kinds) in crate::relay::DEFAULT_RESOLVE_CLAIMS {
            sets.add_search_relay(url, kinds);
        }
        for url in crate::relay::DEFAULT_INDEXERS {
            sets.add_search_relay(url, INDEXER_KINDS);
        }
        sets
    }

    /// Put `url` in the search set and merge `kinds` into its search
    /// kinds (empty = leave it unrestricted / as it is). Used by the
    /// seed and the legacy migrations; no-op on an empty URL.
    fn add_search_relay(&mut self, url: &str, kinds: &[u64]) {
        let key = crate::relay_url::normalize_relay_url(url);
        if key.is_empty() {
            return;
        }
        if let Some(memberships) = self.inactive.get_mut(&key) {
            // Parked: record the role so reactivation restores it.
            if !memberships.iter().any(|m| m == "search") {
                memberships.push("search".to_string());
            }
        } else if !self.search.contains(&key) {
            self.search.push(key.clone());
        }
        if !kinds.is_empty() {
            let entry = self.search_kinds.entry(key).or_default();
            entry.extend_from_slice(kinds);
            entry.sort_unstable();
            entry.dedup();
        }
    }

    /// Merge defaults that postdate this file's `seed_version` — the
    /// additive path by which new engine defaults (and model changes)
    /// reach existing installs. Never removes or reorders anything the
    /// user has; a default the user deleted after a previous upgrade
    /// won't return unless a LATER seed generation re-adds it. Returns
    /// true when anything changed (caller persists).
    pub fn apply_default_upgrades(&mut self) -> bool {
        if self.seed_version >= CURRENT_SEED_VERSION {
            return false;
        }
        if self.seed_version < 1 {
            // Generation 1 (2026-08-18): theforest + thecitadel + mercury
            // read defaults, thecitadel as a publish default, mercury's
            // publication/wiki kinds.
            let read_only: Vec<String> = crate::relay::DEFAULT_READ_ONLY
                .iter()
                .map(|u| crate::relay_url::normalize_relay_url(u))
                .collect();
            for url in crate::relay::DEFAULT_RELAYS {
                let key = crate::relay_url::normalize_relay_url(url);
                if key.is_empty() {
                    continue;
                }
                for list in [&mut self.general, &mut self.fetch] {
                    if !list.contains(&key) {
                        list.push(key.clone());
                    }
                }
                if !read_only.contains(&key) && !self.publish.contains(&key) {
                    self.publish.push(key.clone());
                }
            }
            for (url, kinds) in crate::relay::DEFAULT_RESOLVE_CLAIMS {
                let key = crate::relay_url::normalize_relay_url(url);
                if !key.is_empty() {
                    self.search_kinds.entry(key).or_insert_with(|| kinds.to_vec());
                }
            }
        }
        if self.seed_version < 2 {
            // Generation 2 (2026-11-09): one flat relay list with five
            // flags. The indexer class is gone — its relays become
            // search relays narrowed to profile / relay-list kinds — and
            // the old "resolve kinds" are now search kinds, which only
            // count on a search relay, so every relay carrying kinds
            // gets the flag. Parked relays have their recorded roles
            // renamed the same way.
            for memberships in self.inactive.values_mut() {
                for m in memberships.iter_mut() {
                    if m.starts_with("search.") {
                        *m = "search".to_string();
                    }
                }
                let mut seen = Vec::new();
                memberships.retain(|m| {
                    let fresh = !seen.contains(m);
                    seen.push(m.clone());
                    fresh
                });
            }
            let parked_indexers: Vec<String> = self
                .inactive
                .iter()
                .filter(|(_, ms)| ms.iter().any(|m| m.starts_with("indexer.")))
                .map(|(url, _)| url.clone())
                .collect();
            for memberships in self.inactive.values_mut() {
                memberships.retain(|m| !m.starts_with("indexer."));
            }
            for url in std::mem::take(&mut self.legacy_indexer)
                .into_iter()
                .chain(parked_indexers)
            {
                self.add_search_relay(&url, INDEXER_KINDS);
            }
            let with_kinds: Vec<String> = self.search_kinds.keys().cloned().collect();
            for url in with_kinds {
                self.add_search_relay(&url, &[]);
            }
        }
        self.seed_version = CURRENT_SEED_VERSION;
        true
    }

    /// The search kinds listed for `url` (normalized), or an empty slice.
    pub fn kinds_for(&self, url: &str) -> &[u64] {
        let key = crate::relay_url::normalize_relay_url(url);
        self.search_kinds.get(&key).map_or(&[], |v| v.as_slice())
    }

    /// The search relays serving a fetch for `kinds` — see
    /// [`search_relays_matching`]. Parked relays are in no working set,
    /// so they never match.
    pub fn search_relays_for_kinds(&self, kinds: Option<&[u64]>) -> Vec<String> {
        search_relays_matching(&self.search, &self.search_kinds, kinds)
    }

    /// Replace `url`'s claims. An empty list drops the entry entirely so
    /// the map never accumulates empty vectors (and `relays.json` stays
    /// readable). Kinds are deduped and sorted for a stable file.
    pub fn set_kinds(&mut self, url: &str, kinds: &[u64]) {
        let key = crate::relay_url::normalize_relay_url(url);
        if key.is_empty() {
            return;
        }
        if kinds.is_empty() {
            self.search_kinds.remove(&key);
            return;
        }
        let mut k = kinds.to_vec();
        k.sort_unstable();
        k.dedup();
        self.search_kinds.insert(key, k);
    }

    /// Normalize every URL in each set and drop duplicates. Idempotent —
    /// safe to call on already-normalized sets. Called after `load()` so
    /// any legacy disk content gets canonicalized on next save.
    fn canonicalize(&mut self) {
        self.general = normalize_dedup(&self.general);
        self.fetch = normalize_dedup(&self.fetch);
        self.publish = normalize_dedup(&self.publish);
        self.broadcast = normalize_dedup(&self.broadcast);
        self.auth = normalize_dedup(&self.auth);
        self.search = normalize_dedup(&self.search);
        self.legacy_indexer = normalize_dedup(&self.legacy_indexer);
        for s in &mut self.named {
            s.urls = normalize_dedup(&s.urls);
        }
        // Claim keys are URLs too — canonicalize them so a hand-edited
        // `relays.json` with a trailing slash still matches lookups.
        if !self.search_kinds.is_empty() {
            let claims = std::mem::take(&mut self.search_kinds);
            for (url, mut kinds) in claims {
                let key = crate::relay_url::normalize_relay_url(&url);
                if key.is_empty() || kinds.is_empty() {
                    continue;
                }
                kinds.sort_unstable();
                kinds.dedup();
                self.search_kinds
                    .entry(key)
                    .and_modify(|existing| {
                        existing.extend_from_slice(&kinds);
                        existing.sort_unstable();
                        existing.dedup();
                    })
                    .or_insert(kinds);
            }
        }
        // Inactive keys are URLs too — same canonicalization as claims.
        if !self.inactive.is_empty() {
            let parked = std::mem::take(&mut self.inactive);
            for (url, sets) in parked {
                let key = crate::relay_url::normalize_relay_url(&url);
                if key.is_empty() {
                    continue;
                }
                self.inactive.entry(key).or_insert(sets);
            }
        }
    }

    /// Every membership list `set_active` walks when parking/unparking.
    pub const MEMBERSHIP_SETS: &'static [&'static str] =
        &["general", "fetch", "publish", "broadcast", "auth", "search"];

    /// Park (`active=false`) or unpark (`active=true`) a relay.
    /// Deactivating strips `url` from every working set and records the
    /// memberships in `inactive` — so all engine consumers skip it with
    /// no filtering anywhere — while the row (URL, roles, claims)
    /// survives for later reactivation. Activating re-adds the URL to
    /// exactly the recorded sets and drops the entry. A membership-less
    /// park is valid (a claims-only relay can be parked too). Returns
    /// true when state changed.
    pub fn set_active(&mut self, url: &str, active: bool) -> bool {
        let key = crate::relay_url::normalize_relay_url(url);
        if key.is_empty() {
            return false;
        }
        if active {
            let Some(memberships) = self.inactive.remove(&key) else {
                return false;
            };
            for set in memberships {
                if let Ok(list) = self.get_mut(&set) {
                    if !list.contains(&key) {
                        list.push(key.clone());
                    }
                }
            }
            true
        } else {
            if self.inactive.contains_key(&key) {
                return false;
            }
            let mut memberships = Vec::new();
            for set in Self::MEMBERSHIP_SETS {
                let list = self.get_mut(set).expect("membership set names are valid");
                let before = list.len();
                list.retain(|u| u != &key);
                if list.len() != before {
                    memberships.push(set.to_string());
                }
            }
            self.inactive.insert(key, memberships);
            true
        }
    }

    /// Borrow the URL list for a set (`"general"`, `"fetch"`,
    /// `"publish"`, `"broadcast"`, `"auth"`, `"search"`). Returns `None`
    /// for unknown names so callers can ignore stray UI input rather
    /// than panic.
    pub fn get(&self, set: &str) -> Option<&[String]> {
        match set {
            "general" => Some(&self.general),
            "fetch" => Some(&self.fetch),
            "publish" => Some(&self.publish),
            "broadcast" => Some(&self.broadcast),
            "auth" => Some(&self.auth),
            "search" => Some(&self.search),
            _ => None,
        }
    }

    fn get_mut(&mut self, set: &str) -> Result<&mut Vec<String>> {
        match set {
            "general" => Ok(&mut self.general),
            "fetch" => Ok(&mut self.fetch),
            "publish" => Ok(&mut self.publish),
            "broadcast" => Ok(&mut self.broadcast),
            "auth" => Ok(&mut self.auth),
            "search" => Ok(&mut self.search),
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

    /// Replace `url`'s search kinds and persist. Passing an empty slice
    /// clears them. Returns whether the stored kinds changed.
    ///
    /// Kinds are stored independently of set membership: turning the
    /// search flag off keeps them on file (ignored) so turning it back
    /// on restores the narrowing — only an explicit call here clears.
    pub fn set_kinds(&self, sets: &mut RelaySets, url: &str, kinds: &[u64]) -> Result<bool> {
        let key = crate::relay_url::normalize_relay_url(url);
        let before = sets.search_kinds.get(&key).cloned();
        sets.set_kinds(url, kinds);
        let changed = before.as_deref() != sets.search_kinds.get(&key).map(|v| v.as_slice());
        if changed {
            self.save(sets)?;
        }
        Ok(changed)
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
    fn search_set_round_trips_through_disk() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        assert!(store.add(&mut sets, "search", "wss://search-a").unwrap());
        assert!(!store.add(&mut sets, "search", "wss://search-a").unwrap());
        // The tiered and indexer set names are gone.
        assert!(store.add(&mut sets, "search.default", "wss://x").is_err());
        assert!(store.add(&mut sets, "indexer.default", "wss://x").is_err());

        let reloaded = store.load().unwrap();
        assert_eq!(reloaded.search, vec!["wss://search-a".to_string()]);
    }

    #[test]
    fn legacy_tiers_and_indexers_migrate_to_search_relays() {
        // A pre-generation-2 file: two-tier search, an indexer class
        // (one tiered, so both shapes are covered), an exclusive flag,
        // a kind claim on a relay with no search role, and a parked
        // relay that was an indexer.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let legacy_json = r#"{
            "general": [], "fetch": ["wss://read.example"], "publish": [],
            "search": { "default": ["wss://s1.example"], "fallback": ["wss://s2.example"] },
            "indexer": { "default": ["wss://purplepag.es"], "fallback": ["wss://idx2.example"] },
            "exclusive": { "search": true, "indexer": false },
            "resolve_kinds": { "wss://docs.example": [30818] },
            "inactive": { "wss://parked.example": ["fetch", "indexer.default", "search.fallback"] },
            "seed_version": 1
        }"#;
        std::fs::write(store.path(), legacy_json).unwrap();

        let mut loaded = store.load().unwrap();
        assert!(loaded.apply_default_upgrades());
        for url in [
            "wss://s1.example",
            "wss://s2.example",
            "wss://purplepag.es",
            "wss://idx2.example",
            "wss://docs.example",
        ] {
            assert!(loaded.search.contains(&url.to_string()), "{url} is a search relay");
        }
        // Former indexers are narrowed to profile / relay-list kinds;
        // former search relays stay unrestricted; the claim is kept.
        assert_eq!(loaded.kinds_for("wss://purplepag.es"), INDEXER_KINDS);
        assert_eq!(loaded.kinds_for("wss://idx2.example"), INDEXER_KINDS);
        assert!(loaded.kinds_for("wss://s1.example").is_empty());
        assert_eq!(loaded.kinds_for("wss://docs.example"), &[30818]);
        // The parked relay keeps its roles under the new names.
        assert_eq!(
            loaded.inactive.get("wss://parked.example").map(|v| v.as_slice()),
            Some(&["fetch".to_string(), "search".to_string()][..])
        );
        assert_eq!(loaded.kinds_for("wss://parked.example"), INDEXER_KINDS);
        assert!(!loaded.search.contains(&"wss://parked.example".to_string()));

        // The rewritten file has no trace of the old classes.
        store.save(&loaded).unwrap();
        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains("indexer"));
        assert!(!raw.contains("exclusive"));
        assert!(!raw.contains("resolve_kinds"));
        let mut again = store.load().unwrap();
        assert!(!again.apply_default_upgrades());
        assert_eq!(again.search, loaded.search);
    }

    #[test]
    fn search_relays_match_by_kind_as_a_union() {
        let mut sets = RelaySets::default();
        for url in ["wss://any.example", "wss://profiles.example", "wss://docs.example"] {
            sets.search.push(url.to_string());
        }
        sets.set_kinds("wss://profiles.example", &[0, 10002]);
        sets.set_kinds("wss://docs.example", &[30040, 30818]);
        // Kinds on a relay without the search flag are ignored.
        sets.set_kinds("wss://off.example", &[0]);

        let urls = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            sets.search_relays_for_kinds(Some(&[0])),
            urls(&["wss://any.example", "wss://profiles.example"])
        );
        // Any listed kind matches — a multi-kind filter reaches both.
        assert_eq!(
            sets.search_relays_for_kinds(Some(&[0, 30818])),
            urls(&["wss://any.example", "wss://profiles.example", "wss://docs.example"])
        );
        // An unlisted kind, and a filter naming no kind, reach only the
        // unrestricted search relays.
        assert_eq!(sets.search_relays_for_kinds(Some(&[1])), urls(&["wss://any.example"]));
        assert_eq!(sets.search_relays_for_kinds(None), urls(&["wss://any.example"]));
    }

    #[test]
    fn auth_set_round_trips_and_survives_parking() {
        // The NIP-42 opt-in set is an ordinary membership set: persisted,
        // normalized, addressable by name, and kept across park/unpark.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        assert!(store.add(&mut sets, "auth", "wss://Members.Relay/").unwrap());
        assert!(!store.add(&mut sets, "auth", "wss://Members.Relay/").unwrap());
        let loaded = store.load().unwrap();
        assert_eq!(loaded.auth, vec!["wss://members.relay".to_string()]);
        assert_eq!(loaded.get("auth"), Some(&["wss://members.relay".to_string()][..]));

        // Park: stripped from the set, membership recorded; unpark restores.
        let mut sets = loaded;
        store.add(&mut sets, "general", "wss://members.relay").unwrap();
        assert!(sets.set_active("wss://members.relay", false));
        assert!(sets.auth.is_empty());
        let recorded = sets.inactive.get("wss://members.relay").unwrap();
        assert!(recorded.contains(&"auth".to_string()));
        assert!(recorded.contains(&"general".to_string()));
        assert!(sets.set_active("wss://members.relay", true));
        assert_eq!(sets.auth, vec!["wss://members.relay".to_string()]);

        assert!(store.remove(&mut sets, "auth", "wss://members.relay").unwrap());
        assert!(store.load().unwrap().auth.is_empty());
    }

    #[test]
    fn legacy_file_without_auth_loads_empty_auth() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        std::fs::write(store.path(), r#"{"general":["wss://a"],"fetch":[],"publish":[]}"#).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.auth.is_empty());
        assert_eq!(loaded.general, vec!["wss://a".to_string()]);
    }

    #[test]
    fn search_kinds_round_trip_and_clear() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        // Unsorted + duplicated input normalizes on the way in.
        assert!(store
            .set_kinds(&mut sets, "wss://docs.example", &[30041, 30818, 30040, 30818])
            .unwrap());
        assert_eq!(
            store.load().unwrap().kinds_for("wss://docs.example"),
            &[30040, 30041, 30818]
        );
        // Idempotent — same claims, no rewrite.
        assert!(!store
            .set_kinds(&mut sets, "wss://docs.example", &[30040, 30041, 30818])
            .unwrap());

        // Empty clears the entry rather than storing an empty vec.
        assert!(store.set_kinds(&mut sets, "wss://docs.example", &[]).unwrap());
        assert!(store.load().unwrap().search_kinds.is_empty());
    }

    #[test]
    fn search_kinds_survive_turning_search_off() {
        // Kinds are kept on file when the flag goes off, so turning
        // search back on restores the narrowing — but they don't route
        // anything meanwhile.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        store.add(&mut sets, "search", "wss://docs.example").unwrap();
        store
            .set_kinds(&mut sets, "wss://docs.example", &[30818])
            .unwrap();
        assert_eq!(
            sets.search_relays_for_kinds(Some(&[30818])),
            vec!["wss://docs.example".to_string()]
        );
        store.remove(&mut sets, "search", "wss://docs.example").unwrap();

        let reloaded = store.load().unwrap();
        assert!(reloaded.search.is_empty());
        assert_eq!(reloaded.kinds_for("wss://docs.example"), &[30818]);
        assert!(reloaded.search_relays_for_kinds(Some(&[30818])).is_empty());
    }

    #[test]
    fn claim_keys_canonicalize_on_load() {
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let legacy = r#"{
            "resolve_kinds": { "WSS://Docs.Example/": [30818] }
        }"#;
        std::fs::write(store.path(), legacy).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.kinds_for("wss://docs.example"), &[30818]);
    }

    #[test]
    fn seed_excludes_read_only_defaults_from_publish() {
        let initial: Vec<String> = crate::relay::DEFAULT_RELAYS
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sets = RelaySets::seed_from_initial(&initial);
        // Read side carries everything…
        assert!(sets.fetch.contains(&"wss://theforest.nostr1.com".to_string()));
        assert!(sets.fetch.contains(&"wss://mercury-relay.imwald.eu/relay".to_string()));
        assert!(sets.fetch.contains(&"wss://thecitadel.nostr1.com".to_string()));
        // …publish drops the read-only sources but keeps thecitadel.
        assert!(!sets.publish.contains(&"wss://theforest.nostr1.com".to_string()));
        assert!(!sets.publish.contains(&"wss://mercury-relay.imwald.eu/relay".to_string()));
        assert!(sets.publish.contains(&"wss://thecitadel.nostr1.com".to_string()));
        // Mercury is a search relay for the publication/wiki kinds, the
        // profile relays for profile / relay-list kinds; stamp is current.
        assert_eq!(
            sets.kinds_for("wss://mercury-relay.imwald.eu/relay"),
            &[30040, 30041, 30818]
        );
        assert!(sets.search.contains(&"wss://mercury-relay.imwald.eu/relay".to_string()));
        assert_eq!(sets.kinds_for("wss://purplepag.es"), INDEXER_KINDS);
        assert!(sets
            .search_relays_for_kinds(Some(&[0]))
            .contains(&"wss://purplepag.es".to_string()));
        assert_eq!(sets.seed_version, CURRENT_SEED_VERSION);
    }

    #[test]
    fn default_upgrades_merge_additively_into_old_files() {
        // A pre-seed_version file: user has their own relay and their own
        // claim on a kind mercury also claims.
        let legacy = r#"{
            "general": ["wss://mine.example"],
            "fetch": ["wss://mine.example"],
            "publish": ["wss://mine.example"],
            "resolve_kinds": { "wss://mywiki.example": [30818] }
        }"#;
        let mut sets: RelaySets = serde_json::from_str(legacy).unwrap();
        sets.canonicalize();
        assert_eq!(sets.seed_version, 0);

        assert!(sets.apply_default_upgrades());
        // User entries untouched, new defaults appended.
        assert_eq!(sets.fetch[0], "wss://mine.example");
        assert!(sets.fetch.contains(&"wss://theforest.nostr1.com".to_string()));
        assert!(sets.publish.contains(&"wss://thecitadel.nostr1.com".to_string()));
        assert!(!sets.publish.contains(&"wss://theforest.nostr1.com".to_string()));
        // Mercury's claim arrives WITHOUT displacing the user's own claim.
        assert_eq!(sets.kinds_for("wss://mywiki.example"), &[30818]);
        assert_eq!(
            sets.kinds_for("wss://mercury-relay.imwald.eu/relay"),
            &[30040, 30041, 30818]
        );
        // Generation 2: a relay carrying kinds becomes a search relay.
        assert!(sets.search.contains(&"wss://mywiki.example".to_string()));
        // Idempotent: the stamp gates a second pass.
        assert!(!sets.apply_default_upgrades());
    }

    #[test]
    fn set_active_parks_and_restores_memberships() {
        let mut sets = RelaySets::default();
        let url = "wss://parked.example".to_string();
        sets.fetch.push(url.clone());
        sets.publish.push(url.clone());
        sets.search.push(url.clone());
        sets.set_kinds(&url, &[30818]);

        // Park: gone from every working set, memberships recorded,
        // search kinds retained on file but no longer routing.
        assert!(sets.set_active(&url, false));
        assert!(sets.fetch.is_empty());
        assert!(sets.publish.is_empty());
        assert!(sets.search.is_empty());
        assert_eq!(
            sets.inactive.get(&url).map(|v| v.as_slice()),
            Some(&["fetch".to_string(), "publish".to_string(), "search".to_string()][..])
        );
        assert_eq!(sets.kinds_for(&url), &[30818]);
        assert!(sets.search_relays_for_kinds(Some(&[30818])).is_empty());
        // Idempotent in both directions.
        assert!(!sets.set_active(&url, false));

        // Unpark: exactly the recorded memberships come back.
        assert!(sets.set_active(&url, true));
        assert_eq!(sets.fetch, vec![url.clone()]);
        assert_eq!(sets.publish, vec![url.clone()]);
        assert_eq!(sets.search, vec![url.clone()]);
        assert!(sets.inactive.is_empty());
        assert_eq!(sets.search_relays_for_kinds(Some(&[30818])), vec![url.clone()]);
        assert!(!sets.set_active(&url, true));
    }
}
