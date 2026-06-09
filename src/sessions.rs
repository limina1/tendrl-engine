//! Local storage for saved chat sessions.
//!
//! Mirrors `drafts::DraftStore`: each saved conversation is a JSON file under
//! `<data_dir>/sessions/`. This is tendrl's *own* chat persistence — distinct
//! from `claude_sessions` (which reads external Claude Code transcripts).

use crate::error::{EngineError, Result};
use crate::llm::ContentBlock;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One persisted conversation fragment (role + text, plus structured agent
/// blocks when it came from the tool-calling loop).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedFragment {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<ContentBlock>>,
}

/// A persisted injected-context note (title + content).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedNote {
    pub title: String,
    pub content: String,
}

/// A full saved chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSession {
    pub id: String,
    pub title: String,
    pub created_at: u64, // unix millis
    pub modified_at: u64,
    pub fragments: Vec<SavedFragment>,
    #[serde(default)]
    pub context: Vec<SavedNote>,
}

/// Lightweight listing entry (no fragment bodies).
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub created_at: u64,
    pub modified_at: u64,
    pub message_count: usize,
}

/// File-backed session store at `<data_dir>/sessions/`.
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("sessions");
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", sanitize_id(id)))
    }

    pub fn save(&self, session: &SavedSession) -> Result<()> {
        let json = serde_json::to_string_pretty(session)?;
        fs::write(self.path(&session.id), json)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<SessionSummary>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<SavedSession>(&text) {
                    out.push(SessionSummary {
                        id: s.id,
                        title: s.title,
                        created_at: s.created_at,
                        modified_at: s.modified_at,
                        message_count: s.fragments.len(),
                    });
                }
            }
        }
        // Newest first.
        out.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(out)
    }

    pub fn load(&self, id: &str) -> Result<SavedSession> {
        let text = fs::read_to_string(self.path(id))
            .map_err(|_| EngineError::NotFound(format!("session '{id}' not found")))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let p = self.path(id);
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok(())
    }
}

/// Current unix time in milliseconds.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Derive a human title from the first user message (fallback: "Untitled chat").
pub fn derive_title(fragments: &[SavedFragment]) -> String {
    fragments
        .iter()
        .find(|f| f.role == "user")
        .map(|f| {
            let t: String = f.content.chars().take(60).collect();
            let t = t.trim();
            if t.is_empty() {
                "Untitled chat".to_string()
            } else {
                t.to_string()
            }
        })
        .unwrap_or_else(|| "Untitled chat".to_string())
}

/// Kebab-case slug from a title (alphanumerics, collapsed dashes, capped).
pub fn slug(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let joined = mapped
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    joined.chars().take(40).collect()
}

/// Keep ids filesystem-safe (defensive — ids are engine-generated).
fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_title() {
        assert_eq!(
            slug("Find publications about X!"),
            "find-publications-about-x"
        );
        let frags = vec![
            SavedFragment {
                role: "system".into(),
                content: "sys".into(),
                blocks: None,
            },
            SavedFragment {
                role: "user".into(),
                content: "  Draft an intro section  ".into(),
                blocks: None,
            },
        ];
        assert_eq!(derive_title(&frags), "Draft an intro section");
    }

    #[test]
    fn save_list_load_delete_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path()).unwrap();
        let s = SavedSession {
            id: "123-test".into(),
            title: "Test".into(),
            created_at: 1,
            modified_at: 2,
            fragments: vec![SavedFragment {
                role: "user".into(),
                content: "hi".into(),
                blocks: None,
            }],
            context: vec![],
        };
        store.save(&s).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].message_count, 1);
        let loaded = store.load("123-test").unwrap();
        assert_eq!(loaded.title, "Test");
        store.delete("123-test").unwrap();
        assert!(store.list().unwrap().is_empty());
    }
}
