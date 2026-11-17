//! The feed's kind registry — which document kinds the feed lists, how each
//! one reads, and the user's choices over them.
//!
//! One table ([`FEED_KINDS`]) instead of `30040` hard-coded through the feed:
//! the listing (`PublicationEngine::list_root_publications`) queries the
//! enabled kinds, the web renders a row's kind chip and its count column
//! from the same entries (`GET /api/v1/feed/kinds`), and adding a kind is one
//! row here. Preferences live in `<data_dir>/feed.json` — local view state,
//! never a Nostr event.
//!
//! **Presence check** (`require_profile`): a kind-0 profile is a cheap
//! signal of a regular presence — an author with none is far likelier to be
//! spam. When on, the feed lists only authors whose kind 0 is in the local
//! store. Either way the listing asks the engine to pull the missing kind 0s
//! of the authors it saw ([`ProfileCooldown`] keeps that from re-asking the
//! relays for the same profile-less key on every refresh); when they land,
//! the ingest announcement (kinds `[0]`) re-reads the feed and the newly
//! profiled authors appear.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One kind the feed can list.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FeedKind {
    pub kind: u16,
    /// Singular label for a row's chip ("article").
    pub label: &'static str,
    /// Plural, for the checkable list ("articles").
    pub plural: &'static str,
    /// One line for the list row: what this kind is.
    pub description: &'static str,
    pub default_on: bool,
    /// An index of sections (30040): a row shows its section count, an
    /// index with no children is junk and skipped, and an index another
    /// listed index points at is a child, not a root. A single document
    /// (30023 / 30818 / 30817) has none of that.
    pub has_sections: bool,
}

/// The registry. Order = the order of the toggles in the feed.
pub const FEED_KINDS: &[FeedKind] = &[
    FeedKind {
        kind: 30040,
        label: "publication",
        plural: "publications",
        description: "NKBIP-01 publication indexes — books, papers, kastens",
        default_on: true,
        has_sections: true,
    },
    FeedKind {
        kind: 30041,
        label: "section",
        plural: "standalone sections",
        description: "NKBIP-01 sections no listed publication contains",
        default_on: false,
        has_sections: false,
    },
    FeedKind {
        kind: 30023,
        label: "article",
        plural: "articles",
        description: "NIP-23 long-form articles — high volume on most relays",
        default_on: true,
        has_sections: false,
    },
    FeedKind {
        kind: 30818,
        label: "wiki",
        plural: "wiki pages",
        description: "NIP-54 wiki articles",
        default_on: true,
        has_sections: false,
    },
    FeedKind {
        kind: 30817,
        label: "spec",
        plural: "specs",
        description: "Specifications (NIP / NKBIP drafts)",
        default_on: false,
        has_sections: false,
    },
];

pub fn feed_kind(kind: u16) -> Option<&'static FeedKind> {
    FEED_KINDS.iter().find(|k| k.kind == kind)
}

/// The user's feed choices. Missing kinds take the registry default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeedPrefs {
    #[serde(default)]
    pub kinds: BTreeMap<u16, bool>,
    /// List only authors with a kind-0 profile in the local store.
    #[serde(default)]
    pub require_profile: bool,
}

/// A registry entry with the user's choice applied (the wire shape).
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveFeedKind {
    #[serde(flatten)]
    pub kind: FeedKind,
    pub on: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedKindsView {
    pub kinds: Vec<EffectiveFeedKind>,
    pub require_profile: bool,
}

impl FeedPrefs {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("feed.json")
    }

    /// `<data_dir>/feed.json`; missing → defaults, corrupt → reported + defaults.
    pub fn load(data_dir: &Path) -> Self {
        match std::fs::read_to_string(Self::path(data_dir)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                tracing::warn!("feed.json: {e}; using the defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn is_on(&self, k: &FeedKind) -> bool {
        self.kinds.get(&k.kind).copied().unwrap_or(k.default_on)
    }

    /// The kinds the feed lists, in registry order. May be empty — every
    /// box unchecked is an empty feed, said so in the UI, not a fallback.
    pub fn enabled_kinds(&self) -> Vec<u16> {
        FEED_KINDS.iter().filter(|k| self.is_on(k)).map(|k| k.kind).collect()
    }

    /// Check exactly `kinds` (every other registry kind unchecked) — the
    /// checkable list's all / none / invert in one write. A kind outside
    /// the registry is refused.
    pub fn set_only(&mut self, kinds: &[u16]) -> Result<()> {
        if let Some(bad) = kinds.iter().find(|k| feed_kind(**k).is_none()) {
            return Err(crate::error::EngineError::BadRequest(format!("kind {bad} is not a feed kind")));
        }
        for k in FEED_KINDS {
            self.kinds.insert(k.kind, kinds.contains(&k.kind));
        }
        Ok(())
    }

    /// Set a kind on/off; a kind outside the registry is refused.
    pub fn set_kind(&mut self, kind: u16, on: bool) -> Result<()> {
        if feed_kind(kind).is_none() {
            return Err(crate::error::EngineError::BadRequest(format!("kind {kind} is not a feed kind")));
        }
        self.kinds.insert(kind, on);
        Ok(())
    }

    pub fn view(&self) -> FeedKindsView {
        FeedKindsView {
            kinds: FEED_KINDS.iter().map(|k| EffectiveFeedKind { kind: *k, on: self.is_on(k) }).collect(),
            require_profile: self.require_profile,
        }
    }
}

/// How long a pubkey whose kind 0 was asked for is left alone. Long enough
/// that a feed re-read on every ingest doesn't re-ask the relays for a
/// profile-less key; short enough that a newly profiled author shows up the
/// same session.
const PROFILE_RETRY: Duration = Duration::from_secs(20 * 60);

/// Process-wide memory of recently requested kind-0s.
pub struct ProfileCooldown {
    asked: Mutex<HashMap<String, Instant>>,
}

impl ProfileCooldown {
    pub fn new() -> Self {
        Self { asked: Mutex::new(HashMap::new()) }
    }

    /// The subset of `pubkeys` not asked for within the cooldown; marks them
    /// asked now.
    pub fn take_due(&self, pubkeys: Vec<String>) -> Vec<String> {
        self.take_due_at(pubkeys, Instant::now())
    }

    fn take_due_at(&self, pubkeys: Vec<String>, now: Instant) -> Vec<String> {
        let mut asked = self.asked.lock().unwrap_or_else(|p| p.into_inner());
        asked.retain(|_, t| now.duration_since(*t) < PROFILE_RETRY);
        let mut due = Vec::new();
        for pk in pubkeys {
            if !asked.contains_key(&pk) {
                asked.insert(pk.clone(), now);
                due.push(pk);
            }
        }
        due
    }
}

impl Default for ProfileCooldown {
    fn default() -> Self {
        Self::new()
    }
}

/// The feed's cooldown.
pub static FEED_PROFILE_COOLDOWN: std::sync::LazyLock<ProfileCooldown> = std::sync::LazyLock::new(ProfileCooldown::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_toggles() {
        let mut p = FeedPrefs::default();
        assert_eq!(p.enabled_kinds(), vec![30040, 30023, 30818]);
        p.set_kind(30817, true).unwrap();
        p.set_kind(30023, false).unwrap();
        assert_eq!(p.enabled_kinds(), vec![30040, 30818, 30817]);
        p.set_kind(30023, true).unwrap();
        p.set_kind(30817, false).unwrap();
        assert!(p.set_kind(1, true).is_err(), "not a feed kind");
        for k in FEED_KINDS {
            p.set_kind(k.kind, false).unwrap();
        }
        assert!(p.enabled_kinds().is_empty(), "every box unchecked = an empty feed");
        p.set_only(&[30818, 30041]).unwrap();
        assert_eq!(p.enabled_kinds(), vec![30041, 30818], "registry order");
        assert!(p.set_only(&[1]).is_err());
        assert_eq!(p.enabled_kinds(), vec![30041, 30818], "a refused set changes nothing");
    }

    #[test]
    fn prefs_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = FeedPrefs::load(dir.path());
        assert_eq!(p, FeedPrefs::default());
        p.require_profile = true;
        p.set_kind(30818, false).unwrap();
        p.save(dir.path()).unwrap();
        assert_eq!(FeedPrefs::load(dir.path()), p);
        let v = p.view();
        assert!(v.require_profile);
        assert!(!v.kinds.iter().find(|k| k.kind.kind == 30818).unwrap().on);
        std::fs::write(FeedPrefs::path(dir.path()), "{ nope").unwrap();
        assert_eq!(FeedPrefs::load(dir.path()), FeedPrefs::default());
    }

    #[test]
    fn cooldown_asks_once_per_window() {
        let c = ProfileCooldown::new();
        let t0 = Instant::now();
        assert_eq!(c.take_due_at(vec!["a".into(), "b".into()], t0), vec!["a", "b"]);
        assert_eq!(c.take_due_at(vec!["a".into(), "c".into()], t0 + Duration::from_secs(60)), vec!["c"]);
        assert_eq!(c.take_due_at(vec!["a".into()], t0 + PROFILE_RETRY + Duration::from_secs(1)), vec!["a"]);
    }
}
