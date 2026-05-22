//! Draft storage for unsigned NKBIP-01 publications
//!
//! Stores draft publications (30040/30041 events) locally as JSON files
//! before they are signed and published to relays.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tree::state::ComposeState;

/// Result type for draft operations
pub type Result<T> = std::result::Result<T, DraftError>;

/// Errors that can occur during draft operations
#[derive(Debug, thiserror::Error)]
pub enum DraftError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Draft not found: {0}")]
    NotFound(String),
}

/// A stored draft publication with unsigned events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPublication {
    /// Unique identifier for this draft
    pub draft_id: String,
    /// Publication title
    pub title: String,
    /// Creation timestamp (unix seconds)
    pub created_at: u64,
    /// Last modified timestamp (unix seconds)
    pub modified_at: u64,
    /// Unsigned 30040 index event
    pub index_event: serde_json::Value,
    /// Unsigned 30041 section events
    pub section_events: Vec<serde_json::Value>,
    /// Original compose state for resuming editing
    pub compose_state: DraftComposeState,
}

/// Serializable version of ComposeState for drafts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftComposeState {
    pub title: String,
    pub tags: Vec<DraftTagEntry>,
    pub sections: Vec<DraftSectionCompose>,
    /// Stable opaque nanoid d-tag for the publication's 30040.
    /// `None` on legacy drafts saved before the field existed; on load,
    /// a fresh nanoid is minted at first emission. New drafts always
    /// carry one so re-publishing keeps the addressable identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
}

/// Serializable tag entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftTagEntry {
    pub name: String,
    pub value: String,
}

/// Serializable section compose state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftSectionCompose {
    pub title: String,
    pub content: String,
    pub tags: Vec<DraftTagEntry>,
    /// Heading depth (defaults to 2 on legacy drafts). 3+ triggers
    /// nested 30040/30041 emission at publish time.
    #[serde(default = "default_section_level")]
    pub level: u8,
    /// Stable opaque nanoid d-tag for this section's 30041. Defaults to
    /// `None` on legacy drafts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d_tag: Option<String>,
}

fn default_section_level() -> u8 {
    2
}

impl DraftPublication {
    /// Get the d-tag for the publication
    pub fn d_tag(&self) -> String {
        ComposeState::generate_d_tag(&self.title)
    }
}

/// Storage for draft publications
pub struct DraftStore {
    data_dir: PathBuf,
}

impl DraftStore {
    /// Create a new DraftStore with the given data directory
    pub fn new(data_dir: &Path) -> Result<Self> {
        let drafts_dir = data_dir.join("drafts");
        fs::create_dir_all(&drafts_dir)?;
        Ok(DraftStore { data_dir: drafts_dir })
    }

    /// Save a draft from compose state.
    ///
    /// Takes `&mut` because `publication_d_tag` / `section_d_tag` lazily
    /// mint stable nanoid d-tags on first call — the draft has to persist
    /// those values so resuming the draft (or re-publishing it) uses the
    /// same addressable identity.
    pub fn save_draft(&self, compose: &mut ComposeState) -> Result<String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Generate draft ID from title and timestamp
        let d_tag = compose.publication_d_tag();
        let draft_id = format!("{}-{}", d_tag, now);

        // Build unsigned 30041 events for each section first — that mints
        // any pending section d-tags so the index can reference them.
        let section_events: Vec<serde_json::Value> = (0..compose.sections.len())
            .map(|i| self.build_section_event(compose, i, now))
            .collect();

        // Build the unsigned 30040 event (now that section d-tags exist).
        let index_event = self.build_index_event(compose, now);

        // Convert compose state to serializable form. By now the
        // build_section_event / build_index_event calls above have
        // minted any pending d-tags on the live ComposeState, so we
        // can clone them into the draft for stable identity on resume.
        let compose_state = DraftComposeState {
            title: compose.title.clone(),
            d_tag: compose.d_tag.clone(),
            tags: compose
                .tags
                .iter()
                .map(|t| DraftTagEntry {
                    name: t.name.clone(),
                    value: t.value.clone(),
                })
                .collect(),
            sections: compose
                .sections
                .iter()
                .map(|s| DraftSectionCompose {
                    title: s.title.clone(),
                    content: s.content.clone(),
                    level: s.level,
                    d_tag: s.d_tag.clone(),
                    tags: s
                        .tags
                        .iter()
                        .map(|t| DraftTagEntry {
                            name: t.name.clone(),
                            value: t.value.clone(),
                        })
                        .collect(),
                })
                .collect(),
        };

        let draft = DraftPublication {
            draft_id: draft_id.clone(),
            title: compose.title.clone(),
            created_at: now,
            modified_at: now,
            index_event,
            section_events,
            compose_state,
        };

        // Save to file
        let path = self.draft_path(&draft_id);
        let json = serde_json::to_string_pretty(&draft)?;
        fs::write(&path, json)?;

        Ok(draft_id)
    }

    /// Load a draft by ID
    pub fn load_draft(&self, draft_id: &str) -> Result<DraftPublication> {
        let path = self.draft_path(draft_id);
        if !path.exists() {
            return Err(DraftError::NotFound(draft_id.to_string()));
        }
        let content = fs::read_to_string(&path)?;
        let draft: DraftPublication = serde_json::from_str(&content)?;
        Ok(draft)
    }

    /// List all drafts
    pub fn list_drafts(&self) -> Result<Vec<DraftPublication>> {
        let mut drafts = Vec::new();

        if !self.data_dir.exists() {
            return Ok(drafts);
        }

        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(draft) = serde_json::from_str::<DraftPublication>(&content) {
                        drafts.push(draft);
                    }
                }
            }
        }

        // Sort by modified_at descending (newest first)
        drafts.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));

        Ok(drafts)
    }

    /// Delete a draft by ID
    pub fn delete_draft(&self, draft_id: &str) -> Result<()> {
        let path = self.draft_path(draft_id);
        if !path.exists() {
            return Err(DraftError::NotFound(draft_id.to_string()));
        }
        fs::remove_file(&path)?;
        Ok(())
    }

    /// Get the file path for a draft
    fn draft_path(&self, draft_id: &str) -> PathBuf {
        self.data_dir.join(format!("{}.json", draft_id))
    }

    /// Build an unsigned 30040 index event
    fn build_index_event(&self, compose: &mut ComposeState, timestamp: u64) -> serde_json::Value {
        use serde_json::json;

        let pub_d_tag = compose.publication_d_tag();

        // Placeholder pubkey (64 zeros)
        let placeholder_pubkey = "0".repeat(64);

        // Build section `a` tag references
        let mut section_a_tags: Vec<serde_json::Value> = Vec::new();
        for i in 0..compose.sections.len() {
            let section_d_tag = compose.section_d_tag(i);
            section_a_tags.push(json!(["a", format!("30041:{}:{}", placeholder_pubkey, section_d_tag), ""]));
        }

        // Build publication tags
        let mut pub_tags: Vec<serde_json::Value> = vec![json!(["d", &pub_d_tag])];

        if !compose.title.is_empty() {
            // `title` = display; `T` = indexable title for search/discovery.
            pub_tags.push(json!(["title", &compose.title]));
            pub_tags.push(json!(["T", &compose.title]));
        }

        // Add custom tags
        for tag_vec in ComposeState::tags_to_nostr_format(&compose.tags) {
            pub_tags.push(json!(tag_vec));
        }

        // Add section references
        pub_tags.extend(section_a_tags);

        // Add auto-update tag
        pub_tags.push(json!(["auto-update", compose.auto_update.as_str()]));

        // Build placeholder event ID from d-tag + timestamp
        let placeholder_id = format!(
            "{:0>64}",
            format!("{:x}", Self::simple_hash(&format!("{}{}", pub_d_tag, timestamp)))
        );

        json!({
            "id": placeholder_id,
            "pubkey": placeholder_pubkey,
            "created_at": timestamp,
            "kind": 30040,
            "tags": pub_tags,
            "content": "",
            "sig": "0".repeat(128)
        })
    }

    /// Build an unsigned 30041 section event
    fn build_section_event(&self, compose: &mut ComposeState, section_idx: usize, timestamp: u64) -> serde_json::Value {
        use serde_json::json;

        // Mint the d-tag before taking an immutable borrow of the section
        // for its title/content (avoids overlapping mutable+immutable refs).
        let section_d_tag = compose.section_d_tag(section_idx);
        let section = match compose.sections.get(section_idx) {
            Some(s) => s,
            None => return json!({}),
        };

        // Placeholder pubkey (64 zeros)
        let placeholder_pubkey = "0".repeat(64);

        // Build section tags
        let mut section_tags: Vec<serde_json::Value> = vec![json!(["d", &section_d_tag])];

        if !section.title.is_empty() {
            // `title` = display; `T` = indexable title for search/discovery.
            section_tags.push(json!(["title", &section.title]));
            section_tags.push(json!(["T", &section.title]));
        }

        // Add section-specific tags
        for tag_vec in ComposeState::tags_to_nostr_format(&section.tags) {
            section_tags.push(json!(tag_vec));
        }

        // Build placeholder event ID from d-tag + timestamp
        let placeholder_id = format!(
            "{:0>64}",
            format!("{:x}", Self::simple_hash(&format!("{}{}", section_d_tag, timestamp)))
        );

        json!({
            "id": placeholder_id,
            "pubkey": placeholder_pubkey,
            "created_at": timestamp,
            "kind": 30041,
            "tags": section_tags,
            "content": &section.content,
            "sig": "0".repeat(128)
        })
    }

    /// Simple hash function for generating placeholder IDs
    fn simple_hash(input: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        input.hash(&mut hasher);
        hasher.finish()
    }
}

/// Convert a DraftComposeState back to ComposeState for editing
impl From<&DraftComposeState> for ComposeState {
    fn from(draft: &DraftComposeState) -> Self {
        use crate::tree::state::{TagEntry, SectionCompose as SC};

        let mut compose = ComposeState::new();
        compose.title = draft.title.clone();
        // Re-seed identity so a resumed draft re-publishes onto the
        // same addressable events. Legacy drafts (d_tag == None) will
        // mint fresh nanoids on next emission.
        compose.d_tag = draft.d_tag.clone();
        compose.tags = draft
            .tags
            .iter()
            .map(|t| TagEntry {
                name: t.name.clone(),
                value: t.value.clone(),
            })
            .collect();
        compose.sections = draft
            .sections
            .iter()
            .map(|s| SC {
                title: s.title.clone(),
                content: s.content.clone(),
                level: s.level,
                d_tag: s.d_tag.clone(),
                tags: s
                    .tags
                    .iter()
                    .map(|t| TagEntry {
                        name: t.name.clone(),
                        value: t.value.clone(),
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        compose
    }
}

/// Tracks locally-created publications that haven't been published to relays yet.
/// Stores a simple list of a-tag addresses (kind:pubkey:d_tag) in a JSON file.
pub struct LocalPublicationTracker {
    file_path: PathBuf,
}

impl LocalPublicationTracker {
    /// Create a new tracker, storing data in the given directory
    pub fn new(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        let file_path = data_dir.join("local_publications.json");
        Ok(Self { file_path })
    }

    /// Mark a publication as locally created (not yet published to relays)
    pub fn mark_local(&self, a_tag: &str) -> Result<()> {
        let mut tags = self.load_tags()?;
        if !tags.contains(&a_tag.to_string()) {
            tags.push(a_tag.to_string());
            self.save_tags(&tags)?;
        }
        Ok(())
    }

    /// Remove a publication from local tracking (after publishing to relays)
    pub fn mark_published(&self, a_tag: &str) -> Result<()> {
        let mut tags = self.load_tags()?;
        tags.retain(|t| t != a_tag);
        self.save_tags(&tags)?;
        Ok(())
    }

    /// Check if a publication is marked as locally created
    pub fn is_local(&self, a_tag: &str) -> bool {
        self.load_tags()
            .map(|tags| tags.contains(&a_tag.to_string()))
            .unwrap_or(false)
    }

    /// Get all locally-created publication a-tags
    pub fn list_local(&self) -> Vec<String> {
        self.load_tags().unwrap_or_default()
    }

    fn load_tags(&self) -> Result<Vec<String>> {
        if !self.file_path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.file_path)?;
        let tags: Vec<String> = serde_json::from_str(&content)?;
        Ok(tags)
    }

    fn save_tags(&self, tags: &[String]) -> Result<()> {
        let content = serde_json::to_string_pretty(tags)?;
        fs::write(&self.file_path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_compose() -> ComposeState {
        use crate::tree::state::{TagEntry, SectionCompose as SC};

        let mut compose = ComposeState::new();
        compose.title = "Test Publication".to_string();
        compose.tags.push(TagEntry {
            name: "tags".to_string(),
            value: "rust, nostr".to_string(),
        });
        compose.sections.push(SC {
            title: "Chapter 1".to_string(),
            content: "This is the first chapter content.".to_string(),
            ..Default::default()
        });
        compose.sections.push(SC {
            title: "Chapter 2".to_string(),
            content: "This is the second chapter content.".to_string(),
            ..Default::default()
        });
        compose
    }

    #[test]
    fn test_save_and_load_draft() {
        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        let mut compose = create_test_compose();
        let draft_id = store.save_draft(&mut compose).unwrap();

        let loaded = store.load_draft(&draft_id).unwrap();
        assert_eq!(loaded.title, "Test Publication");
        assert_eq!(loaded.section_events.len(), 2);
        assert_eq!(loaded.compose_state.sections.len(), 2);
    }

    /// Saving a draft mints d-tags on the in-memory compose state, and
    /// the persisted DraftComposeState carries them forward. Round-tripping
    /// through `From<&DraftComposeState> for ComposeState` must preserve
    /// identity so a resumed draft re-publishes the same addressable
    /// events.
    #[test]
    fn test_draft_round_trip_preserves_d_tags_and_levels() {
        use crate::tree::state::SectionCompose as SC;

        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        // Build a nested compose: one level-2 section + a level-3 child.
        let mut compose = ComposeState::new();
        compose.title = "Nested Test".to_string();
        compose.sections.push(SC {
            title: "Outer".to_string(),
            content: "outer body".to_string(),
            level: 2,
            ..Default::default()
        });
        compose.sections.push(SC {
            title: "Inner".to_string(),
            content: "inner body".to_string(),
            level: 3,
            ..Default::default()
        });

        let draft_id = store.save_draft(&mut compose).unwrap();
        let original_pub_d = compose.d_tag.clone().expect("pub d-tag minted on save");
        let original_section_ds: Vec<String> = compose
            .sections
            .iter()
            .map(|s| s.d_tag.clone().expect("section d-tag minted on save"))
            .collect();

        // Load and convert back to a live ComposeState.
        let loaded = store.load_draft(&draft_id).unwrap();
        let restored: ComposeState = (&loaded.compose_state).into();

        assert_eq!(restored.d_tag.as_deref(), Some(original_pub_d.as_str()));
        assert_eq!(restored.sections.len(), 2);
        assert_eq!(restored.sections[0].level, 2);
        assert_eq!(restored.sections[1].level, 3);
        assert_eq!(
            restored.sections[0].d_tag.as_deref(),
            Some(original_section_ds[0].as_str())
        );
        assert_eq!(
            restored.sections[1].d_tag.as_deref(),
            Some(original_section_ds[1].as_str())
        );
    }

    #[test]
    fn test_list_drafts() {
        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        let mut compose1 = create_test_compose();
        store.save_draft(&mut compose1).unwrap();

        let mut compose2 = create_test_compose();
        compose2.title = "Another Publication".to_string();
        store.save_draft(&mut compose2).unwrap();

        let drafts = store.list_drafts().unwrap();
        assert_eq!(drafts.len(), 2);
    }

    #[test]
    fn test_delete_draft() {
        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        let mut compose = create_test_compose();
        let draft_id = store.save_draft(&mut compose).unwrap();

        assert!(store.load_draft(&draft_id).is_ok());
        store.delete_draft(&draft_id).unwrap();
        assert!(store.load_draft(&draft_id).is_err());
    }

    #[test]
    fn test_draft_events_structure() {
        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        let mut compose = create_test_compose();
        let draft_id = store.save_draft(&mut compose).unwrap();
        let loaded = store.load_draft(&draft_id).unwrap();

        // Check index event structure
        assert_eq!(loaded.index_event["kind"], 30040);
        assert_eq!(loaded.index_event["content"], "");

        // Check section event structure
        assert_eq!(loaded.section_events[0]["kind"], 30041);
        assert_eq!(
            loaded.section_events[0]["content"],
            "This is the first chapter content."
        );
    }
}
