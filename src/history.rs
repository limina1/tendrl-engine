//! Reading / listening history — the local "where was I" per publication.
//!
//! One entry per publication coordinate (`30040:pubkey:d`), holding the
//! last section the reader showed and the last narration position the
//! player reached. Pure local state in `<data_dir>/history.json` (same
//! pattern as `relays.json` / `blossom.json`): never a Nostr event, never
//! broadcast. The reader re-opens a book at `section`; the player resumes
//! at `audio`. Newest-first; capped so the file stays small.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_ENTRIES: usize = 200;

/// Last section shown in the reader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReadPlace {
    /// `30041:pubkey:d` (or a nested `30040:…` the reader focused into).
    pub coord: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Last narration position the player reached.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListenPlace {
    /// Section coordinate the recording narrates.
    pub coord: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Kind-32176 event id of the recording that was playing.
    pub recording_id: String,
    #[serde(default)]
    pub piece: u32,
    #[serde(default)]
    pub position_secs: f64,
    #[serde(default)]
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    /// `30040:pubkey:d`
    pub coord: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Publication author (hex pubkey).
    #[serde(default)]
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<ReadPlace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ListenPlace>,
    /// Unix seconds of the last update (read or listen).
    pub updated_at: u64,
}

/// A partial update: fields present replace the stored ones; `section`
/// and `audio` are left alone when absent and cleared with the flags.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HistoryUpdate {
    pub coord: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub section: Option<ReadPlace>,
    #[serde(default)]
    pub audio: Option<ListenPlace>,
    #[serde(default)]
    pub clear_section: bool,
    #[serde(default)]
    pub clear_audio: bool,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Apply `up` to `entries` in place; returns the resulting entry. Pure.
pub fn apply(entries: &mut Vec<HistoryEntry>, up: HistoryUpdate, at: u64) -> HistoryEntry {
    let mut entry = match entries.iter().position(|e| e.coord == up.coord) {
        Some(i) => entries.remove(i),
        None => HistoryEntry {
            coord: up.coord.clone(),
            title: None,
            author: String::new(),
            section: None,
            audio: None,
            updated_at: at,
        },
    };
    if let Some(t) = up.title {
        entry.title = Some(t);
    }
    if let Some(a) = up.author {
        entry.author = a;
    }
    if up.clear_section {
        entry.section = None;
    } else if let Some(s) = up.section {
        entry.section = Some(s);
    }
    if up.clear_audio {
        entry.audio = None;
    } else if let Some(a) = up.audio {
        entry.audio = Some(a);
    }
    entry.updated_at = at;
    entries.insert(0, entry.clone());
    entries.truncate(MAX_ENTRIES);
    entry
}

static WRITE: Mutex<()> = Mutex::new(());

/// `<data_dir>/history.json`
#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("history.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Vec<HistoryEntry> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self, entries: &[HistoryEntry]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(entries)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Upsert under the process-wide write lock (reader + player both write).
    pub fn update(&self, up: HistoryUpdate) -> Result<HistoryEntry> {
        let _g = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let mut entries = self.load();
        let entry = apply(&mut entries, up, now());
        self.save(&entries)?;
        Ok(entry)
    }

    pub fn remove(&self, coord: &str) -> Result<bool> {
        let _g = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        let mut entries = self.load();
        let before = entries.len();
        entries.retain(|e| e.coord != coord);
        let changed = entries.len() != before;
        if changed {
            self.save(&entries)?;
        }
        Ok(changed)
    }

    pub fn clear(&self) -> Result<()> {
        let _g = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        self.save(&[])
    }
}

pub mod handlers {
    use super::*;
    use crate::api::ChatAppState;
    use crate::error::EngineError;
    use axum::extract::State;
    use axum::response::Json;
    use serde_json::{json, Value};

    type R = std::result::Result<Json<Value>, EngineError>;

    fn store(st: &ChatAppState) -> HistoryStore {
        HistoryStore::new(st.engine.data_dir())
    }

    /// `GET /api/v1/history` — newest first.
    pub async fn list(State(st): State<ChatAppState>) -> R {
        let s = store(&st);
        Ok(Json(json!({ "entries": s.load(), "path": s.path() })))
    }

    /// `POST /api/v1/history` — upsert one publication's place.
    pub async fn update(State(st): State<ChatAppState>, Json(up): Json<HistoryUpdate>) -> R {
        if up.coord.is_empty() {
            return Err(EngineError::BadRequest("coord required".into()));
        }
        let entry = store(&st).update(up)?;
        Ok(Json(json!({ "entry": entry })))
    }

    #[derive(Deserialize)]
    pub struct RemoveReq {
        pub coord: String,
    }

    /// `POST /api/v1/history/remove` `{coord}`; `{coord:"*"}` clears all.
    pub async fn remove(State(st): State<ChatAppState>, Json(req): Json<RemoveReq>) -> R {
        let s = store(&st);
        if req.coord == "*" {
            s.clear()?;
            return Ok(Json(json!({ "removed": true })));
        }
        Ok(Json(json!({ "removed": s.remove(&req.coord)? })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(coord: &str) -> HistoryUpdate {
        HistoryUpdate {
            coord: coord.into(),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_moves_to_front_and_merges() {
        let mut v = Vec::new();
        apply(&mut v, HistoryUpdate { title: Some("A".into()), ..up("30040:p:a") }, 1);
        apply(&mut v, HistoryUpdate { title: Some("B".into()), ..up("30040:p:b") }, 2);
        let e = apply(
            &mut v,
            HistoryUpdate {
                section: Some(ReadPlace { coord: "30041:p:s1".into(), event_id: None, title: Some("§1".into()) }),
                ..up("30040:p:a")
            },
            3,
        );
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].coord, "30040:p:a");
        assert_eq!(e.title.as_deref(), Some("A"), "title kept when the update omits it");
        assert_eq!(e.section.as_ref().unwrap().coord, "30041:p:s1");
        assert_eq!(e.updated_at, 3);
    }

    #[test]
    fn audio_and_section_are_independent() {
        let mut v = Vec::new();
        apply(
            &mut v,
            HistoryUpdate {
                audio: Some(ListenPlace {
                    coord: "30041:p:s2".into(),
                    title: None,
                    recording_id: "r".into(),
                    piece: 0,
                    position_secs: 12.5,
                    duration_secs: 100.0,
                }),
                ..up("30040:p:a")
            },
            1,
        );
        let e = apply(
            &mut v,
            HistoryUpdate {
                section: Some(ReadPlace { coord: "30041:p:s3".into(), event_id: None, title: None }),
                ..up("30040:p:a")
            },
            2,
        );
        assert_eq!(e.audio.as_ref().unwrap().position_secs, 12.5);
        assert_eq!(e.section.as_ref().unwrap().coord, "30041:p:s3");
        let e = apply(&mut v, HistoryUpdate { clear_audio: true, ..up("30040:p:a") }, 3);
        assert!(e.audio.is_none());
        assert!(e.section.is_some());
    }

    #[test]
    fn capped() {
        let mut v = Vec::new();
        for i in 0..(MAX_ENTRIES + 5) {
            apply(&mut v, up(&format!("30040:p:{i}")), i as u64);
        }
        assert_eq!(v.len(), MAX_ENTRIES);
        assert_eq!(v[0].coord, format!("30040:p:{}", MAX_ENTRIES + 4));
    }

    #[test]
    fn store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tendrl-history-{}", std::process::id()));
        let s = HistoryStore::new(&dir);
        s.update(HistoryUpdate { title: Some("T".into()), ..up("30040:p:x") }).unwrap();
        assert_eq!(s.load()[0].title.as_deref(), Some("T"));
        assert!(s.remove("30040:p:x").unwrap());
        assert!(s.load().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
