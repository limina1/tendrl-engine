//! Draft storage for unsigned NKBIP-01 publications
//!
//! Stores draft publications (30040/30041 events) locally as JSON files
//! before they are signed and published to relays.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::publication::compose::ComposeState;

/// Monotonic per-process suffix that makes draft ids unique even for two saves
/// landing in the same millisecond (so no version snapshot is ever overwritten).
static NEXT_DRAFT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    /// Creation timestamp (unix **milliseconds** — finer than the events'
    /// second-granularity created_at so rapid versions stay distinct + ordered).
    pub created_at: u64,
    /// Last modified timestamp (unix milliseconds).
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
    /// Output kind. `None`/`30040` = NKBIP-01 publication; any other kind marks
    /// an atomic draft (blog 30023, wiki 30818, custom). Persisted so resuming
    /// reopens the composer in the right mode. Legacy drafts default to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<u32>,
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
    /// Transclude slot target (naddr/coordinate to a 30040/30041). When set this
    /// item is a slot, not authored content. `None` on legacy drafts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
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
        let dur = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        // Nostr event timestamps are SECONDS; draft metadata (id + created/
        // modified) uses MILLISECONDS so rapid same-second saves stay distinct,
        // ordered, and individually time-stamped versions rather than colliding.
        let now_secs = dur.as_secs();
        let now_ms = dur.as_millis() as u64;

        // The draft id is `<d-tag>-<millis>-<seq>`: millisecond granularity plus
        // a process-lifetime counter, so two saves in the same millisecond still
        // get distinct ids and neither version snapshot is overwritten.
        let d_tag = compose.publication_d_tag();
        let seq = NEXT_DRAFT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let draft_id = format!("{}-{}-{}", d_tag, now_ms, seq);

        // Build unsigned 30041 events for each section first — that mints
        // any pending section d-tags so the index can reference them.
        let section_events: Vec<serde_json::Value> = (0..compose.sections.len())
            .map(|i| self.build_section_event(compose, i, now_secs))
            .collect();

        // Build the unsigned 30040 event (now that section d-tags exist).
        let index_event = self.build_index_event(compose, now_secs);

        // Convert compose state to serializable form. By now the
        // build_section_event / build_index_event calls above have
        // minted any pending d-tags on the live ComposeState, so we
        // can clone them into the draft for stable identity on resume.
        let compose_state = DraftComposeState {
            title: compose.title.clone(),
            d_tag: compose.d_tag.clone(),
            kind: compose.kind,
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
                    slot: s.slot_coord.clone(),
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
            created_at: now_ms,
            modified_at: now_ms,
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
            pub_tags.push(json!(["T", ComposeState::generate_d_tag(&compose.title)]));
        }

        // Add custom tags, plus `N` twins for any `author` tags
        let custom_tags = ComposeState::tags_to_nostr_format(&compose.tags);
        for tag_vec in &custom_tags {
            pub_tags.push(json!(tag_vec));
        }
        for tag_vec in ComposeState::author_n_tags(&custom_tags) {
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
            "sig": crate::identity::placeholder_sig()
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
            section_tags.push(json!(["T", ComposeState::generate_d_tag(&section.title)]));
        }

        // Add section-specific tags, plus `N` twins for any `author` tags
        let custom_tags = ComposeState::tags_to_nostr_format(&section.tags);
        for tag_vec in &custom_tags {
            section_tags.push(json!(tag_vec));
        }
        for tag_vec in ComposeState::author_n_tags(&custom_tags) {
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
            "sig": crate::identity::placeholder_sig()
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
        use crate::publication::compose::{TagEntry, SectionCompose as SC};

        let mut compose = ComposeState::new();
        compose.title = draft.title.clone();
        // Re-seed identity so a resumed draft re-publishes onto the
        // same addressable events. Legacy drafts (d_tag == None) will
        // mint fresh nanoids on next emission.
        compose.d_tag = draft.d_tag.clone();
        compose.kind = draft.kind;
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

// ============================================================================
// Version diff — compare two draft snapshots of the same publication
// ============================================================================
//
// Saves of one publication (same d-tag) accumulate as timestamped snapshots.
// The composer's Saved-drafts list groups them and shows, for each older
// version, how it differs from the latest. Sections are matched by `T` (title
// slug) — the same key republish-diff uses — so per-section d-tags needn't be
// threaded. Computed from the stored compose state; the events carry the same
// information.

/// A before/after change of a single scalar field (e.g. the title).
#[derive(Debug, Clone, Serialize)]
pub struct FieldChange {
    pub old: String,
    pub new: String,
}

/// Set-based tag diff: which `(name, value)` pairs were added / removed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TagDiff {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<(String, String)>,
}

impl TagDiff {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// One section's status + changes between two versions, matched by title slug.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionVersionDiff {
    pub title: String,
    /// Title slug — the match key.
    pub t: String,
    /// `matched` (in both), `added` (only in the newer), `removed` (only in the
    /// older). Heading depth is carried so the UI can indent nested sub-indexes.
    pub status: String,
    pub level: u8,
    /// Matched only: the section's content differs between the two versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_changed: Option<bool>,
    /// Matched only: the heading level (nesting) differs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_changed: Option<bool>,
    /// Matched only: per-section tag adds/removes.
    #[serde(skip_serializing_if = "TagDiff::is_empty")]
    pub tags: TagDiff,
}

/// The diff of one draft version (`from`) against another (`to`, typically the
/// latest): the 30040-level changes (title + index tags) plus the contained
/// 30041 sections, each annotated matched/added/removed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDiff {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_changed: Option<FieldChange>,
    #[serde(skip_serializing_if = "TagDiff::is_empty")]
    pub index_tags: TagDiff,
    pub sections: Vec<SectionVersionDiff>,
}

fn tag_set_diff(from: &[DraftTagEntry], to: &[DraftTagEntry]) -> TagDiff {
    use std::collections::HashSet;
    let pair = |t: &DraftTagEntry| (t.name.clone(), t.value.clone());
    let from_set: HashSet<(String, String)> = from.iter().map(pair).collect();
    let to_set: HashSet<(String, String)> = to.iter().map(pair).collect();
    TagDiff {
        added: to
            .iter()
            .map(pair)
            .filter(|p| !from_set.contains(p))
            .collect(),
        removed: from
            .iter()
            .map(pair)
            .filter(|p| !to_set.contains(p))
            .collect(),
    }
}

/// Diff two draft compose snapshots. Describes the changes to go `from` → `to`;
/// for the "version vs latest" view the caller passes `from` = the older
/// version, `to` = the latest. Sections are matched by `generate_d_tag(title)`.
pub fn diff_draft_versions(from: &DraftComposeState, to: &DraftComposeState) -> VersionDiff {
    use std::collections::HashMap;

    let title_changed = (from.title != to.title).then(|| FieldChange {
        old: from.title.clone(),
        new: to.title.clone(),
    });

    let slug = ComposeState::generate_d_tag;
    let from_by_t: HashMap<String, &DraftSectionCompose> =
        from.sections.iter().map(|s| (slug(&s.title), s)).collect();
    let to_by_t: HashMap<String, &DraftSectionCompose> =
        to.sections.iter().map(|s| (slug(&s.title), s)).collect();

    let mut sections = Vec::new();

    // Follow the newer version's order for matched + added sections.
    for s in &to.sections {
        let t = slug(&s.title);
        if let Some(prev) = from_by_t.get(&t) {
            sections.push(SectionVersionDiff {
                title: s.title.clone(),
                t: t.clone(),
                status: "matched".into(),
                level: s.level,
                content_changed: Some(prev.content.trim() != s.content.trim()),
                level_changed: Some(prev.level != s.level),
                tags: tag_set_diff(&prev.tags, &s.tags),
            });
        } else {
            sections.push(SectionVersionDiff {
                title: s.title.clone(),
                t,
                status: "added".into(),
                level: s.level,
                content_changed: None,
                level_changed: None,
                tags: TagDiff::default(),
            });
        }
    }
    // Sections the older version had that the newer dropped.
    for s in &from.sections {
        let t = slug(&s.title);
        if !to_by_t.contains_key(&t) {
            sections.push(SectionVersionDiff {
                title: s.title.clone(),
                t,
                status: "removed".into(),
                level: s.level,
                content_changed: None,
                level_changed: None,
                tags: TagDiff::default(),
            });
        }
    }

    VersionDiff {
        title_changed,
        index_tags: tag_set_diff(&from.tags, &to.tags),
        sections,
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
        use crate::publication::compose::{TagEntry, SectionCompose as SC};

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

    /// A publication draft carries no explicit kind (None == 30040); an atomic
    /// draft persists its kind so resuming reopens the composer in the right
    /// mode, and the round-trip back to ComposeState restores it.
    #[test]
    fn test_draft_persists_atomic_kind() {
        use crate::publication::compose::SectionCompose as SC;

        let temp_dir = TempDir::new().unwrap();
        let store = DraftStore::new(temp_dir.path()).unwrap();

        // Publication draft: kind stays None.
        let mut pub_compose = create_test_compose();
        let pub_id = store.save_draft(&mut pub_compose).unwrap();
        assert_eq!(store.load_draft(&pub_id).unwrap().compose_state.kind, None);

        // Atomic (NIP-54 wiki) draft: kind round-trips through save → load →
        // back into ComposeState.
        let mut wiki = ComposeState::new();
        wiki.title = "Wiki Subject".to_string();
        wiki.kind = Some(30818);
        wiki.sections.push(SC {
            title: "Wiki Subject".to_string(),
            content: "A wiki body.".to_string(),
            ..Default::default()
        });
        let wiki_id = store.save_draft(&mut wiki).unwrap();

        let loaded = store.load_draft(&wiki_id).unwrap();
        assert_eq!(loaded.compose_state.kind, Some(30818));
        let resumed: ComposeState = (&loaded.compose_state).into();
        assert_eq!(resumed.kind, Some(30818));
    }

    /// Saving a draft mints d-tags on the in-memory compose state, and
    /// the persisted DraftComposeState carries them forward. Round-tripping
    /// through `From<&DraftComposeState> for ComposeState` must preserve
    /// identity so a resumed draft re-publishes the same addressable
    /// events.
    #[test]
    fn test_draft_round_trip_preserves_d_tags_and_levels() {
        use crate::publication::compose::SectionCompose as SC;

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

    fn dcs(title: &str, sections: &[(&str, &str, u8)], tags: &[(&str, &str)]) -> DraftComposeState {
        DraftComposeState {
            title: title.to_string(),
            d_tag: Some("fixed-dtag".to_string()),
            kind: None,
            tags: tags
                .iter()
                .map(|(n, v)| DraftTagEntry {
                    name: n.to_string(),
                    value: v.to_string(),
                })
                .collect(),
            sections: sections
                .iter()
                .map(|(t, c, lvl)| DraftSectionCompose {
                    title: t.to_string(),
                    content: c.to_string(),
                    tags: vec![],
                    level: *lvl,
                    d_tag: None,
                    slot: None,
                })
                .collect(),
        }
    }

    #[test]
    fn version_diff_title_sections_and_tags() {
        let from = dcs(
            "Old Title",
            &[("Intro", "old intro", 2), ("Dropped", "gone later", 2)],
            &[("t", "rust")],
        );
        let to = dcs(
            "New Title",
            &[("Intro", "NEW intro", 2), ("Results", "fresh", 3)],
            &[("t", "rust"), ("t", "nostr")],
        );

        let diff = diff_draft_versions(&from, &to);

        // 30040-level
        let tc = diff.title_changed.expect("title changed");
        assert_eq!((tc.old.as_str(), tc.new.as_str()), ("Old Title", "New Title"));
        assert_eq!(diff.index_tags.added, vec![("t".into(), "nostr".into())]);
        assert!(diff.index_tags.removed.is_empty());

        // Sections: Intro matched+changed, Results added (lvl 3), Dropped removed.
        let intro = diff.sections.iter().find(|s| s.t == "intro").unwrap();
        assert_eq!(intro.status, "matched");
        assert_eq!(intro.content_changed, Some(true));
        assert_eq!(intro.level_changed, Some(false));

        let results = diff.sections.iter().find(|s| s.t == "results").unwrap();
        assert_eq!(results.status, "added");
        assert_eq!(results.level, 3);

        let dropped = diff.sections.iter().find(|s| s.t == "dropped").unwrap();
        assert_eq!(dropped.status, "removed");
    }

    #[test]
    fn version_diff_identical_is_quiet() {
        let v = dcs("Same", &[("A", "body", 2)], &[("t", "x")]);
        let diff = diff_draft_versions(&v, &v);
        assert!(diff.title_changed.is_none());
        assert!(diff.index_tags.is_empty());
        assert_eq!(diff.sections.len(), 1);
        assert_eq!(diff.sections[0].status, "matched");
        assert_eq!(diff.sections[0].content_changed, Some(false));
        assert!(diff.sections[0].tags.is_empty());
    }

    #[test]
    fn version_diff_section_tag_changes() {
        let mut from = dcs("P", &[("S", "body", 2)], &[]);
        from.sections[0].tags = vec![DraftTagEntry {
            name: "author".into(),
            value: "alice".into(),
        }];
        let mut to = dcs("P", &[("S", "body", 2)], &[]);
        to.sections[0].tags = vec![DraftTagEntry {
            name: "author".into(),
            value: "bob".into(),
        }];
        let diff = diff_draft_versions(&from, &to);
        let s = &diff.sections[0];
        assert_eq!(s.content_changed, Some(false));
        assert_eq!(s.tags.added, vec![("author".into(), "bob".into())]);
        assert_eq!(s.tags.removed, vec![("author".into(), "alice".into())]);
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
