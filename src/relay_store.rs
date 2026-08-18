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
pub const CURRENT_SEED_VERSION: u32 = 1;

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
    /// Per-relay **resolve kinds**: the event kinds a relay is claimed to
    /// serve, keyed by normalized URL. Specificity layered on top of the
    /// read/write/broadcast policy — it governs *resolution* (targeted
    /// retrieval of a known coordinate/entity), not general traffic.
    ///
    /// A claim is exclusive per kind: resolving kind K goes to the relays
    /// listing K, whatever their policy. When no relay lists K, resolution
    /// falls back to the read set — so an empty map is exactly today's
    /// behavior. A URL with claims but no policy is a resolution-only
    /// target: never in feeds, searches, or profile lookups.
    ///
    /// See `docs/zettel/idea-relay-kind-routing.org`.
    #[serde(default)]
    pub resolve_kinds: std::collections::BTreeMap<String, Vec<u64>>,
    /// Inactive relays: URL → the set names it belonged to when it was
    /// deactivated (`"general"`, `"fetch"`, `"publish"`, `"broadcast"`,
    /// `"search.default"`, `"search.fallback"`, `"indexer.default"`,
    /// `"indexer.fallback"`). Presence in this map IS the inactive state:
    /// the URL is removed from those working sets (so every engine
    /// consumer skips it with zero filtering) and reactivation re-adds it
    /// to exactly the recorded sets. This is what lets the user park a
    /// relay — keep its URL and role configuration — without deleting it.
    /// Claims (`resolve_kinds`) are retained but ignored while inactive.
    #[serde(default)]
    pub inactive: std::collections::BTreeMap<String, Vec<String>>,
    /// Highest default-seed generation this file has absorbed. Lets new
    /// engine defaults reach EXISTING installs additively: on load, a
    /// file whose stamp is behind `CURRENT_SEED_VERSION` gets the newer
    /// defaults merged in (never overwriting user edits) and the stamp
    /// bumped. Files predating the field deserialize as 0.
    #[serde(default)]
    pub seed_version: u32,
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
        let default_indexers: Vec<String> = crate::relay::DEFAULT_INDEXERS
            .iter()
            .map(|s| s.to_string())
            .collect();
        let default_search: Vec<String> = crate::relay::DEFAULT_SEARCH
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut resolve_kinds = std::collections::BTreeMap::new();
        for (url, kinds) in crate::relay::DEFAULT_RESOLVE_CLAIMS {
            let key = crate::relay_url::normalize_relay_url(url);
            if !key.is_empty() {
                resolve_kinds.insert(key, kinds.to_vec());
            }
        }
        Self {
            general: normalized.clone(),
            fetch: normalized,
            publish,
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
            resolve_kinds,
            inactive: std::collections::BTreeMap::new(),
            seed_version: CURRENT_SEED_VERSION,
            exclusive: ExclusiveFlags::default(),
        }
    }

    /// Merge defaults that postdate this file's `seed_version` — the
    /// additive path by which new engine defaults reach existing
    /// installs. Never removes or reorders anything the user has; a
    /// default the user deleted after a previous upgrade won't return
    /// unless a LATER seed generation re-adds it. Returns true when
    /// anything changed (caller persists).
    pub fn apply_default_upgrades(&mut self) -> bool {
        if self.seed_version >= CURRENT_SEED_VERSION {
            return false;
        }
        // Generation 1 (2026-08-18): theforest + thecitadel + mercury
        // read defaults, thecitadel as a publish default, mercury's
        // publication/wiki resolve claims.
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
                self.resolve_kinds.entry(key).or_insert_with(|| kinds.to_vec());
            }
        }
        // The search class flattened from a two-tier section to a
        // per-relay toggle backed by `search.default`; fold any
        // fallback-tier members in so they stay visible. (No behavior
        // change: the fallback tier never had a consumer.)
        let fb = std::mem::take(&mut self.search.fallback);
        for url in fb {
            if !self.search.default.contains(&url) {
                self.search.default.push(url);
            }
        }
        self.seed_version = CURRENT_SEED_VERSION;
        true
    }

    /// The kinds claimed by `url` (normalized), or an empty slice.
    pub fn kinds_for(&self, url: &str) -> &[u64] {
        let key = crate::relay_url::normalize_relay_url(url);
        self.resolve_kinds.get(&key).map_or(&[], |v| v.as_slice())
    }

    /// Every relay claiming `kind`, in insertion-stable (sorted-key) order.
    /// Empty means nothing claims it — the caller falls back to the read
    /// set, which is what makes an unconfigured install a no-op. Parked
    /// (inactive) relays keep their claims on file but never claim here.
    pub fn relays_claiming_kind(&self, kind: u64) -> Vec<String> {
        self.resolve_kinds
            .iter()
            .filter(|(url, kinds)| kinds.contains(&kind) && !self.inactive.contains_key(*url))
            .map(|(url, _)| url.clone())
            .collect()
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
            self.resolve_kinds.remove(&key);
            return;
        }
        let mut k = kinds.to_vec();
        k.sort_unstable();
        k.dedup();
        self.resolve_kinds.insert(key, k);
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
        // Claim keys are URLs too — canonicalize them so a hand-edited
        // `relays.json` with a trailing slash still matches lookups.
        if !self.resolve_kinds.is_empty() {
            let claims = std::mem::take(&mut self.resolve_kinds);
            for (url, mut kinds) in claims {
                let key = crate::relay_url::normalize_relay_url(&url);
                if key.is_empty() || kinds.is_empty() {
                    continue;
                }
                kinds.sort_unstable();
                kinds.dedup();
                self.resolve_kinds
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

    /// Every membership list `set_active` walks when parking/unparking —
    /// the flat sets plus both tiers of each discovery class.
    pub const MEMBERSHIP_SETS: &'static [&'static str] = &[
        "general",
        "fetch",
        "publish",
        "broadcast",
        "search.default",
        "search.fallback",
        "indexer.default",
        "indexer.fallback",
    ];

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

/// The mutually-exclusive sibling tier for a discovery-class set name,
/// or `None` for a flat/unknown name. Adding a URL to one tier strips it
/// from its sibling so a URL is never in both `default` and `fallback`
/// of the same class. Single source of truth shared by
/// `RelayStore::add` (on `RelaySets`) and `Engine::add_relay` (on
/// `RelayConfig`).
pub(crate) fn discovery_sibling(set: &str) -> Option<&'static str> {
    match set {
        "search.default" => Some("search.fallback"),
        "search.fallback" => Some("search.default"),
        "indexer.default" => Some("indexer.fallback"),
        "indexer.fallback" => Some("indexer.default"),
        _ => None,
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
        if let Some(s) = discovery_sibling(set) {
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

    /// Replace `url`'s resolve-kind claims and persist. Passing an empty
    /// slice clears them. Returns whether the stored claims changed.
    ///
    /// Claims are deliberately independent of set membership: a relay may
    /// carry claims with no read/write/broadcast policy at all (the
    /// resolution-only target), so `remove` does *not* clear them — only
    /// an explicit call here does.
    pub fn set_kinds(&self, sets: &mut RelaySets, url: &str, kinds: &[u64]) -> Result<bool> {
        let key = crate::relay_url::normalize_relay_url(url);
        let before = sets.resolve_kinds.get(&key).cloned();
        sets.set_kinds(url, kinds);
        let changed = before.as_deref() != sets.resolve_kinds.get(&key).map(|v| v.as_slice());
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
    fn resolve_kinds_round_trip_and_clear() {
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
        assert!(store.load().unwrap().resolve_kinds.is_empty());
    }

    #[test]
    fn claims_survive_set_removal() {
        // A relay may be a resolution-only target: claims with no
        // read/write/broadcast membership. Leaving a set must not
        // silently drop them.
        let tmp = TempDir::new().unwrap();
        let store = RelayStore::new(tmp.path()).unwrap();
        let mut sets = RelaySets::default();

        store.add(&mut sets, "fetch", "wss://docs.example").unwrap();
        store
            .set_kinds(&mut sets, "wss://docs.example", &[30818])
            .unwrap();
        store.remove(&mut sets, "fetch", "wss://docs.example").unwrap();

        let reloaded = store.load().unwrap();
        assert!(reloaded.fetch.is_empty());
        assert_eq!(reloaded.kinds_for("wss://docs.example"), &[30818]);
        assert_eq!(
            reloaded.relays_claiming_kind(30818),
            vec!["wss://docs.example".to_string()]
        );
        // Nothing claims a kind nobody listed — the caller falls back to
        // the read set, which is what keeps an unconfigured install a no-op.
        assert!(reloaded.relays_claiming_kind(1).is_empty());
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
        // Mercury claims the publication/wiki kinds; stamp is current.
        assert_eq!(
            sets.kinds_for("wss://mercury-relay.imwald.eu/relay"),
            &[30040, 30041, 30818]
        );
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
        // Idempotent: the stamp gates a second pass.
        assert!(!sets.apply_default_upgrades());
    }

    #[test]
    fn set_active_parks_and_restores_memberships() {
        let mut sets = RelaySets::default();
        let url = "wss://parked.example".to_string();
        sets.fetch.push(url.clone());
        sets.publish.push(url.clone());
        sets.search.default.push(url.clone());
        sets.set_kinds(&url, &[30818]);

        // Park: gone from every working set, memberships recorded,
        // claims retained on file but no longer claiming.
        assert!(sets.set_active(&url, false));
        assert!(sets.fetch.is_empty());
        assert!(sets.publish.is_empty());
        assert!(sets.search.default.is_empty());
        assert_eq!(
            sets.inactive.get(&url).map(|v| v.as_slice()),
            Some(&["fetch".to_string(), "publish".to_string(), "search.default".to_string()][..])
        );
        assert_eq!(sets.kinds_for(&url), &[30818]);
        assert!(sets.relays_claiming_kind(30818).is_empty());
        // Idempotent in both directions.
        assert!(!sets.set_active(&url, false));

        // Unpark: exactly the recorded memberships come back.
        assert!(sets.set_active(&url, true));
        assert_eq!(sets.fetch, vec![url.clone()]);
        assert_eq!(sets.publish, vec![url.clone()]);
        assert_eq!(sets.search.default, vec![url.clone()]);
        assert!(sets.inactive.is_empty());
        assert_eq!(sets.relays_claiming_kind(30818), vec![url.clone()]);
        assert!(!sets.set_active(&url, true));
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
