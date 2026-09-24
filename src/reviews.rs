//! Agent reviews of merge requests — local records, never events.
//!
//! `review_ask` (the assistant, over MCP or in-app) reads the same diff the
//! review screen shows and writes what it found: findings, a
//! recommendation, a note. That is advice to the owner, so it is a *tool
//! result kept as a record* under `<data_dir>/reviews/<818 id>.json` — the
//! assistant never comments on someone else's merge request under its own
//! key, and nothing here can be broadcast. The review screen shows the
//! record beside the diff; merge, decline, relink and the 818 itself stay
//! the user's press (`docs/zettel/decision-merge-requests-818.org`).

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What the agent recommends after reading the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Recommendation {
    Merge,
    Decline,
    EditFirst,
}

impl std::str::FromStr for Recommendation {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, String> {
        match s.trim() {
            "merge" => Ok(Self::Merge),
            "decline" => Ok(Self::Decline),
            "edit-first" | "edit_first" | "edit" => Ok(Self::EditFirst),
            other => Err(format!("recommendation must be merge, decline or edit-first (got `{other}`)")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// The kind-818 event id this review is about.
    pub id: String,
    pub findings: Vec<String>,
    pub recommendation: Recommendation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Who wrote it — `assistant` for the agent tools.
    pub by: String,
    pub created_at: i64,
}

/// `<data_dir>/reviews/<818 id>.json`, one record per merge request (a
/// second review replaces the first).
pub struct ReviewStore {
    dir: PathBuf,
}

impl ReviewStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("reviews");
        std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("reviews dir: {e}")))?;
        Ok(Self { dir })
    }

    fn valid_id(id: &str) -> bool {
        id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn save(&self, review: &Review) -> Result<()> {
        if !Self::valid_id(&review.id) {
            return Err(EngineError::BadRequest(format!("bad merge-request id `{}`", review.id)));
        }
        let path = self.path(&review.id);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(review)?)
            .map_err(|e| EngineError::Other(format!("write review: {e}")))?;
        std::fs::rename(&tmp, &path).map_err(|e| EngineError::Other(format!("commit review: {e}")))?;
        Ok(())
    }

    /// The review for an 818, or `None` when nobody has written one.
    pub fn load(&self, id: &str) -> Result<Option<Review>> {
        if !Self::valid_id(id) {
            return Err(EngineError::BadRequest(format!("bad merge-request id `{id}`")));
        }
        match std::fs::read_to_string(self.path(id)) {
            Ok(raw) => Ok(Some(serde_json::from_str(&raw)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(EngineError::Other(format!("read review: {e}"))),
        }
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !Self::valid_id(id) {
            return Err(EngineError::BadRequest(format!("bad merge-request id `{id}`")));
        }
        std::fs::remove_file(self.path(id)).map_err(|_| EngineError::NotFound(format!("review {id}")))
    }

    /// Every review, newest first. Unreadable files are skipped.
    pub fn list(&self) -> Result<Vec<Review>> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Ok(out) };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&p) {
                if let Ok(r) = serde_json::from_str::<Review>(&raw) {
                    out.push(r);
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_round_trips_and_a_second_one_replaces_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = ReviewStore::new(dir.path()).unwrap();
        let id = "a".repeat(64);
        assert!(store.load(&id).unwrap().is_none());
        let r = Review { id: id.clone(), findings: vec!["2 sections touched".into()], recommendation: Recommendation::Merge, note: None, by: "assistant".into(), created_at: 10 };
        store.save(&r).unwrap();
        assert_eq!(store.load(&id).unwrap().unwrap(), r);
        let r2 = Review { recommendation: Recommendation::EditFirst, note: Some("the glossary slot is wrong".into()), created_at: 11, ..r.clone() };
        store.save(&r2).unwrap();
        assert_eq!(store.load(&id).unwrap().unwrap().recommendation, Recommendation::EditFirst);
        assert_eq!(store.list().unwrap().len(), 1);
        store.delete(&id).unwrap();
        assert!(store.load(&id).unwrap().is_none());
        assert!(store.save(&Review { id: "short".into(), ..r2 }).is_err(), "an id must be the 818's 64-hex id");
        assert_eq!("edit-first".parse::<Recommendation>().unwrap(), Recommendation::EditFirst);
        assert!("maybe".parse::<Recommendation>().is_err());
    }
}
