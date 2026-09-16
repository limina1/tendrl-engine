//! Engine-owned context set — the curated reference notes the assistant
//! works from.
//!
//! The set is the source of truth: the web overlays its routing flags and
//! renders the set; the in-app agent and external harnesses (MCP) read and
//! write it directly. Persistence lives under `<data_dir>/context/`:
//! `active.json` is the live set (survives a restart), `sets/<id>.json` are
//! named snapshots. Decision:
//! `docs/zettel/decision-context-set-engine-owned.org`.
//!
//! Pure data + a small file store; no nostrdb access here. Resolving an
//! event into an entry (title/content/tags from a stored event) is the
//! caller's job — `tools::resolve_context_entry` — so this module stays
//! IO-free apart from its own JSON files.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Where an entry came from. `Literal` entries (pasted text, an assistant
/// idea, a chat fragment) have no event behind them and never dedupe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextSource {
    Event {
        id: String,
    },
    Addr {
        kind: u64,
        pubkey: String,
        d_tag: String,
        /// The exact version captured, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        event_id: Option<String>,
    },
    Literal,
}

impl ContextSource {
    /// Two sources name the same thing: same event id, same coordinate, or
    /// an event id that is the captured version of a coordinate. Literals
    /// are never the same as anything.
    pub fn same_as(&self, other: &ContextSource) -> bool {
        use ContextSource::*;
        match (self, other) {
            (Event { id: a }, Event { id: b }) => a == b,
            (
                Addr {
                    kind: k1,
                    pubkey: p1,
                    d_tag: d1,
                    ..
                },
                Addr {
                    kind: k2,
                    pubkey: p2,
                    d_tag: d2,
                    ..
                },
            ) => k1 == k2 && p1 == p2 && d1 == d2,
            (
                Event { id },
                Addr {
                    event_id: Some(e), ..
                },
            )
            | (
                Addr {
                    event_id: Some(e), ..
                },
                Event { id },
            ) => id == e,
            _ => false,
        }
    }
}

/// Who put the entry in the set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOrigin {
    User,
    Assistant,
    Chat,
    Import,
}

/// One reference note. `content` is what the model sees (the user may edit
/// it in place); `original` is the snapshot at entry time so divergence is
/// visible and reversible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEntry {
    pub id: String,
    pub source: ContextSource,
    pub title: String,
    pub content: String,
    pub original: String,
    /// Raw event tags when sourced from an event — the composer needs them
    /// when a context entry is routed into a section.
    #[serde(default)]
    pub tags: Vec<Vec<String>>,
    pub origin: ContextOrigin,
    /// Unix seconds.
    pub added_at: u64,
}

impl ContextEntry {
    pub fn new(
        source: ContextSource,
        title: impl Into<String>,
        content: impl Into<String>,
        tags: Vec<Vec<String>>,
        origin: ContextOrigin,
    ) -> Self {
        let content = content.into();
        Self {
            id: format!("cx_{}", nanoid::nanoid!(10)),
            source,
            title: title.into(),
            original: content.clone(),
            content,
            tags,
            origin,
            added_at: now_secs(),
        }
    }

    pub fn literal(
        title: impl Into<String>,
        content: impl Into<String>,
        origin: ContextOrigin,
    ) -> Self {
        Self::new(ContextSource::Literal, title, content, Vec::new(), origin)
    }

    /// The user (or assistant) changed the text since it was captured.
    pub fn modified(&self) -> bool {
        self.content != self.original
    }
}

/// Summary row for the named-set listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSetSummary {
    pub id: String,
    pub name: String,
    pub entry_count: usize,
    pub updated_at: u64,
}

/// The set itself. The active set has id `"active"` and no name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSet {
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub entries: Vec<ContextEntry>,
    #[serde(default)]
    pub updated_at: u64,
}

impl ContextSet {
    pub const ACTIVE_ID: &'static str = "active";

    pub fn active() -> Self {
        Self {
            id: Self::ACTIVE_ID.to_string(),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn find(&self, id: &str) -> Option<&ContextEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn find_by_source(&self, source: &ContextSource) -> Option<&ContextEntry> {
        self.entries.iter().find(|e| e.source.same_as(source))
    }

    /// Add an entry. An entry whose source is already in the set is not
    /// added twice: the existing id comes back with `false`.
    pub fn add(&mut self, entry: ContextEntry) -> (String, bool) {
        if let Some(existing) = self.find_by_source(&entry.source) {
            return (existing.id.clone(), false);
        }
        let id = entry.id.clone();
        self.entries.push(entry);
        self.touch();
        (id, true)
    }

    /// Edit title and/or content in place. Returns false for an unknown id.
    pub fn update(&mut self, id: &str, title: Option<String>, content: Option<String>) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        if let Some(t) = title {
            e.title = t;
        }
        if let Some(c) = content {
            e.content = c;
        }
        self.touch();
        true
    }

    /// Restore an entry's content to its captured snapshot.
    pub fn reset(&mut self, id: &str) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        e.content = e.original.clone();
        self.touch();
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        let removed = self.entries.len() != before;
        if removed {
            self.touch();
        }
        removed
    }

    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.touch();
        }
    }

    pub fn summary(&self) -> ContextSetSummary {
        ContextSetSummary {
            id: self.id.clone(),
            name: self.name.clone().unwrap_or_default(),
            entry_count: self.entries.len(),
            updated_at: self.updated_at,
        }
    }

    fn touch(&mut self) {
        self.updated_at = now_secs();
    }
}

/// JSON files under `<data_dir>/context/`.
pub struct ContextStore {
    dir: PathBuf,
}

impl ContextStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("context");
        fs::create_dir_all(dir.join("sets")).map_err(io_err)?;
        Ok(Self { dir })
    }

    fn active_path(&self) -> PathBuf {
        self.dir.join("active.json")
    }

    fn set_path(&self, id: &str) -> Result<PathBuf> {
        if id.is_empty()
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(EngineError::BadRequest(format!(
                "invalid context set id: {id}"
            )));
        }
        Ok(self.dir.join("sets").join(format!("{id}.json")))
    }

    /// The live set, or `None` on a fresh data dir.
    pub fn load_active(&self) -> Result<Option<ContextSet>> {
        let path = self.active_path();
        if !path.exists() {
            return Ok(None);
        }
        let mut set: ContextSet = read_json(&path)?;
        set.id = ContextSet::ACTIVE_ID.to_string();
        Ok(Some(set))
    }

    pub fn save_active(&self, set: &ContextSet) -> Result<()> {
        write_json(&self.active_path(), set)
    }

    pub fn list_sets(&self) -> Result<Vec<ContextSetSummary>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(self.dir.join("sets")).map_err(io_err)? {
            let entry = entry.map_err(io_err)?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(set) = read_json::<ContextSet>(&path) {
                out.push(set.summary());
            }
        }
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(out)
    }

    /// Snapshot a set under a name. The stored copy gets its own id.
    pub fn save_set(&self, set: &ContextSet, name: &str) -> Result<ContextSetSummary> {
        let name = name.trim();
        if name.is_empty() {
            return Err(EngineError::BadRequest("a context set needs a name".into()));
        }
        let mut copy = set.clone();
        copy.id = format!("cs_{}", nanoid::nanoid!(10));
        copy.name = Some(name.to_string());
        copy.updated_at = now_secs();
        write_json(&self.set_path(&copy.id)?, &copy)?;
        Ok(copy.summary())
    }

    pub fn load_set(&self, id: &str) -> Result<ContextSet> {
        let path = self.set_path(id)?;
        if !path.exists() {
            return Err(EngineError::NotFound(format!("context set {id} not found")));
        }
        read_json(&path)
    }

    pub fn delete_set(&self, id: &str) -> Result<()> {
        let path = self.set_path(id)?;
        if !path.exists() {
            return Err(EngineError::NotFound(format!("context set {id} not found")));
        }
        fs::remove_file(path).map_err(io_err)
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(io_err)?;
    serde_json::from_str(&text).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))
}

/// Write via a temp file + rename so a crash mid-write never truncates the
/// live set.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| EngineError::Other(format!("serialize: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)
}

fn io_err(e: std::io::Error) -> EngineError {
    EngineError::Other(format!("context store: {e}"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(d: &str) -> ContextSource {
        ContextSource::Addr {
            kind: 30041,
            pubkey: "ab".repeat(32),
            d_tag: d.into(),
            event_id: None,
        }
    }

    fn entry(src: ContextSource, title: &str, body: &str) -> ContextEntry {
        ContextEntry::new(src, title, body, vec![], ContextOrigin::User)
    }

    #[test]
    fn add_dedupes_by_source_but_not_literals() {
        let mut set = ContextSet::active();
        let (a, new_a) = set.add(entry(addr("x"), "X", "body"));
        let (b, new_b) = set.add(entry(addr("x"), "X again", "body2"));
        assert!(new_a && !new_b);
        assert_eq!(a, b);
        assert_eq!(set.len(), 1);
        set.add(ContextEntry::literal("i", "idea", ContextOrigin::Chat));
        set.add(ContextEntry::literal("i", "idea", ContextOrigin::Chat));
        assert_eq!(set.len(), 3, "literals never dedupe");
    }

    #[test]
    fn event_id_matches_captured_version_of_addr() {
        let mut set = ContextSet::active();
        set.add(entry(
            ContextSource::Addr {
                kind: 30041,
                pubkey: "ab".repeat(32),
                d_tag: "x".into(),
                event_id: Some("ff".repeat(32)),
            },
            "X",
            "b",
        ));
        let (_, added) = set.add(entry(
            ContextSource::Event {
                id: "ff".repeat(32),
            },
            "X",
            "b",
        ));
        assert!(!added);
    }

    #[test]
    fn update_reset_remove() {
        let mut set = ContextSet::active();
        let (id, _) = set.add(entry(addr("x"), "X", "orig"));
        assert!(set.update(&id, None, Some("edited".into())));
        assert!(set.find(&id).unwrap().modified());
        assert!(set.reset(&id));
        assert!(!set.find(&id).unwrap().modified());
        assert!(!set.update("nope", Some("t".into()), None));
        assert!(set.remove(&id));
        assert!(!set.remove(&id));
        assert!(set.is_empty());
    }

    #[test]
    fn store_round_trips_active_and_named_sets() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path()).unwrap();
        assert!(store.load_active().unwrap().is_none());

        let mut set = ContextSet::active();
        set.add(ContextEntry::new(
            addr("x"),
            "X",
            "body",
            vec![vec!["title".into(), "X".into()]],
            ContextOrigin::User,
        ));
        store.save_active(&set).unwrap();
        let back = store.load_active().unwrap().unwrap();
        assert_eq!(back.entries, set.entries);
        assert_eq!(back.id, ContextSet::ACTIVE_ID);

        let sum = store.save_set(&set, "reading list").unwrap();
        assert_eq!(sum.name, "reading list");
        assert_eq!(sum.entry_count, 1);
        let listed = store.list_sets().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, sum.id);
        let loaded = store.load_set(&sum.id).unwrap();
        assert_eq!(loaded.entries, set.entries);
        assert_eq!(loaded.name.as_deref(), Some("reading list"));
        store.delete_set(&sum.id).unwrap();
        assert!(store.list_sets().unwrap().is_empty());
        assert!(store.load_set(&sum.id).is_err());
        assert!(store.set_path("../etc").is_err());
    }

    #[test]
    fn serde_shape_is_web_friendly() {
        let e = ContextEntry::new(addr("x"), "X", "b", vec![], ContextOrigin::Assistant);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["source"]["type"], "addr");
        assert_eq!(v["source"]["kind"], 30041);
        assert_eq!(v["origin"], "assistant");
        assert!(v["source"].get("event_id").is_none());
        let l = serde_json::to_value(ContextEntry::literal("t", "c", ContextOrigin::Chat)).unwrap();
        assert_eq!(l["source"]["type"], "literal");
    }
}
