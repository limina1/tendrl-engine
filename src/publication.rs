//! Publication Engine - Interface-agnostic handling of composed articles
//!
//! Provides high-level abstractions for working with NKBIP-01 publications:
//! - Kind 30040: Publication indexes
//! - Kind 30041: Publication sections
//!
//! Features:
//! - Hierarchical loading (index → sections → nested indexes)
//! - Section forking and versioning
//! - Slot-in support for alternate/older content
//! - Tree diffing for version comparison

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Kind constants for NKBIP-01 publications
pub const KIND_PUBLICATION_INDEX: u64 = 30040;
pub const KIND_PUBLICATION_SECTION: u64 = 30041;

/// Address identifier for replaceable events (kind:pubkey:d-tag)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NAddr {
    pub kind: u64,
    pub pubkey: String,
    pub d_tag: String,
}

impl NAddr {
    pub fn new(kind: u64, pubkey: &str, d_tag: &str) -> Self {
        Self {
            kind,
            pubkey: pubkey.to_string(),
            d_tag: d_tag.to_string(),
        }
    }

    /// Parse from "kind:pubkey:d-tag" format
    pub fn from_a_tag(a_tag: &str) -> Option<Self> {
        let parts: Vec<&str> = a_tag.split(':').collect();
        if parts.len() >= 3 {
            let kind = parts[0].parse().ok()?;
            Some(Self {
                kind,
                pubkey: parts[1].to_string(),
                d_tag: parts[2..].join(":"), // d-tag may contain colons
            })
        } else {
            None
        }
    }

    /// Convert to "kind:pubkey:d-tag" format
    pub fn to_a_tag(&self) -> String {
        format!("{}:{}:{}", self.kind, self.pubkey, self.d_tag)
    }

    /// Short display format: "kind:abcd...wxyz:d-tag"
    /// Shows first 4 and last 4 characters of pubkey for readability
    pub fn short_format(&self) -> String {
        let short_pubkey = if self.pubkey.len() > 8 {
            format!(
                "{}...{}",
                &self.pubkey[..4],
                &self.pubkey[self.pubkey.len() - 4..]
            )
        } else {
            self.pubkey.clone()
        };
        format!("{}:{}:{}", self.kind, short_pubkey, self.d_tag)
    }
}

/// Load status for lazy-loadable content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum LoadStatus<T> {
    /// Not yet attempted to load
    Pending,
    /// Currently loading
    Loading,
    /// Successfully loaded
    Loaded { data: T },
    /// Load failed
    Failed { error: String },
}

impl<T> LoadStatus<T> {
    pub fn is_loaded(&self) -> bool {
        matches!(self, LoadStatus::Loaded { .. })
    }

    pub fn data(&self) -> Option<&T> {
        match self {
            LoadStatus::Loaded { data } => Some(data),
            _ => None,
        }
    }
}

/// A section within a publication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    /// The section's address
    pub addr: NAddr,
    /// The raw event (if loaded)
    pub event: LoadStatus<Value>,
    /// Extracted title from tags
    pub title: Option<String>,
    /// Content (markdown/asciidoc)
    pub content: Option<String>,
    /// Position in parent's ordering
    pub position: usize,
    /// Alternative versions of this section (for forking/versioning)
    #[serde(default)]
    pub alternates: Vec<SectionVersion>,
}

/// An alternate version of a section (for forking/slotting)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionVersion {
    /// The event for this version
    pub event: Value,
    /// When this version was created
    pub created_at: u64,
    /// Author of this version
    pub author: String,
    /// Version label if present
    pub version: Option<String>,
}

/// A complete publication with its sections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    /// The publication's address
    pub addr: NAddr,
    /// The index event (kind 30040)
    pub index: LoadStatus<Value>,
    /// Extracted metadata
    pub title: Option<String>,
    pub summary: Option<String>,
    pub image: Option<String>,
    pub author_pubkey: String,
    pub author_name: Option<String>,
    pub version: Option<String>,
    pub created_at: u64,
    /// Ordered list of sections
    pub sections: Vec<Section>,
    /// Nested publications (kind 30040 referenced in a-tags)
    pub nested: Vec<Publication>,
    /// Whether this is a root publication or nested
    pub is_root: bool,
}

impl Publication {
    /// Extract metadata from a publication index event
    ///
    /// Note: Publication index events (kind 30040) should not have content.
    /// Events with non-empty content are rejected as invalid per NKBIP-01.
    pub fn from_event(event: &Value, is_root: bool) -> Result<Self> {
        // Defensive: 30040 events should not have content
        if let Some(content) = event.get("content").and_then(|v| v.as_str()) {
            if !content.trim().is_empty() {
                return Err(EngineError::InvalidFilter(
                    "Publication index (30040) should not have content".into(),
                ));
            }
        }

        let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let pubkey = event
            .get("pubkey")
            .and_then(|v| v.as_str())
            .ok_or_else(|| EngineError::InvalidFilter("Missing pubkey".into()))?;
        let created_at = event.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);

        let tags = event.get("tags").and_then(|v| v.as_array());

        let mut d_tag = id.to_string(); // fallback to id
        let mut title = None;
        let mut summary = None;
        let mut image = None;
        let mut version = None;
        let mut section_addrs = Vec::new();

        if let Some(tags) = tags {
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    let tag_name = arr.first().and_then(|v| v.as_str());
                    let tag_value = arr.get(1).and_then(|v| v.as_str());

                    match tag_name {
                        Some("d") => d_tag = tag_value.unwrap_or(&d_tag).to_string(),
                        Some("title") | Some("name") => title = tag_value.map(String::from),
                        Some("summary") | Some("description") => summary = tag_value.map(String::from),
                        Some("image") | Some("thumb") => image = tag_value.map(String::from),
                        Some("version") => version = tag_value.map(String::from),
                        Some("a") => {
                            if let Some(addr_str) = tag_value {
                                if let Some(addr) = NAddr::from_a_tag(addr_str) {
                                    section_addrs.push(addr);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let addr = NAddr::new(KIND_PUBLICATION_INDEX, pubkey, &d_tag);

        // Create section stubs for each a-tag
        let sections: Vec<Section> = section_addrs
            .into_iter()
            .enumerate()
            .filter(|(_, a)| a.kind == KIND_PUBLICATION_SECTION)
            .map(|(i, a)| Section {
                addr: a,
                event: LoadStatus::Pending,
                title: None,
                content: None,
                position: i,
                alternates: Vec::new(),
            })
            .collect();

        Ok(Publication {
            addr,
            index: LoadStatus::Loaded {
                data: event.clone(),
            },
            title,
            summary,
            image,
            author_pubkey: pubkey.to_string(),
            author_name: None,
            version,
            created_at,
            sections,
            nested: Vec::new(),
            is_root,
        })
    }

    /// Get total section count (including nested)
    pub fn section_count(&self) -> usize {
        let own = self.sections.len();
        let nested: usize = self.nested.iter().map(|p| p.section_count()).sum();
        own + nested
    }

    /// Get all loaded section contents as a flat list
    pub fn all_content(&self) -> Vec<&str> {
        let mut contents = Vec::new();
        for section in &self.sections {
            if let Some(content) = &section.content {
                contents.push(content.as_str());
            }
        }
        for nested in &self.nested {
            contents.extend(nested.all_content());
        }
        contents
    }
}

/// Table of contents entry for navigation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocEntry {
    /// The address of this entry
    pub addr: NAddr,
    /// Display title
    pub title: String,
    /// Nesting depth (0 = top level)
    pub depth: usize,
    /// Whether content is loaded
    pub loaded: bool,
    /// Whether this is a nested publication (30040) vs section (30041)
    pub is_publication: bool,
    /// Children entries (for tree view)
    pub children: Vec<TocEntry>,
}

/// Publication engine extension for the main Engine
pub struct PublicationEngine<'a> {
    engine: &'a Engine,
}

impl<'a> PublicationEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self { engine }
    }

    /// Load a publication by its address
    pub async fn load_publication(
        &self,
        addr: &NAddr,
        policy: FetchPolicy,
    ) -> Result<Publication> {
        let event = self
            .engine
            .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, policy)
            .await?
            .ok_or_else(|| EngineError::Database("Publication not found".into()))?;

        Publication::from_event(&event, true)
    }

    /// Load all sections for a publication
    pub async fn load_sections(
        &self,
        pub_: &mut Publication,
        policy: FetchPolicy,
    ) -> Result<usize> {
        let mut loaded = 0;

        for section in &mut pub_.sections {
            if section.event.is_loaded() {
                continue;
            }

            section.event = LoadStatus::Loading;

            match self
                .engine
                .get_addressable(
                    section.addr.kind,
                    &section.addr.pubkey,
                    &section.addr.d_tag,
                    policy,
                )
                .await
            {
                Ok(Some(event)) => {
                    // Extract content and title from section event
                    section.content = event.get("content").and_then(|v| v.as_str()).map(String::from);

                    if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
                        for tag in tags {
                            if let Some(arr) = tag.as_array() {
                                if arr.first().and_then(|v| v.as_str()) == Some("title") {
                                    section.title = arr.get(1).and_then(|v| v.as_str()).map(String::from);
                                }
                            }
                        }
                    }

                    section.event = LoadStatus::Loaded { data: event };
                    loaded += 1;
                }
                Ok(None) => {
                    section.event = LoadStatus::Failed {
                        error: "Section not found".into(),
                    };
                }
                Err(e) => {
                    section.event = LoadStatus::Failed {
                        error: e.to_string(),
                    };
                }
            }
        }

        Ok(loaded)
    }

    /// Load a single section by index
    pub async fn load_section(
        &self,
        pub_: &mut Publication,
        index: usize,
        policy: FetchPolicy,
    ) -> Result<bool> {
        let section = pub_
            .sections
            .get_mut(index)
            .ok_or_else(|| EngineError::InvalidFilter("Section index out of bounds".into()))?;

        if section.event.is_loaded() {
            return Ok(false);
        }

        section.event = LoadStatus::Loading;

        match self
            .engine
            .get_addressable(
                section.addr.kind,
                &section.addr.pubkey,
                &section.addr.d_tag,
                policy,
            )
            .await
        {
            Ok(Some(event)) => {
                section.content = event.get("content").and_then(|v| v.as_str()).map(String::from);

                if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
                    for tag in tags {
                        if let Some(arr) = tag.as_array() {
                            if arr.first().and_then(|v| v.as_str()) == Some("title") {
                                section.title = arr.get(1).and_then(|v| v.as_str()).map(String::from);
                            }
                        }
                    }
                }

                section.event = LoadStatus::Loaded { data: event };
                Ok(true)
            }
            Ok(None) => {
                section.event = LoadStatus::Failed {
                    error: "Section not found".into(),
                };
                Ok(false)
            }
            Err(e) => {
                section.event = LoadStatus::Failed {
                    error: e.to_string(),
                };
                Err(e)
            }
        }
    }

    /// Build table of contents for a publication
    pub fn build_toc(&self, pub_: &Publication, depth: usize) -> Vec<TocEntry> {
        let mut entries = Vec::new();

        for section in &pub_.sections {
            entries.push(TocEntry {
                addr: section.addr.clone(),
                title: section
                    .title
                    .clone()
                    .unwrap_or_else(|| format!("Section {}", section.position + 1)),
                depth,
                loaded: section.event.is_loaded(),
                is_publication: false,
                children: Vec::new(),
            });
        }

        for nested in &pub_.nested {
            let children = self.build_toc(nested, depth + 1);
            entries.push(TocEntry {
                addr: nested.addr.clone(),
                title: nested
                    .title
                    .clone()
                    .unwrap_or_else(|| "Nested Publication".into()),
                depth,
                loaded: nested.index.is_loaded(),
                is_publication: true,
                children,
            });
        }

        entries
    }

    /// Query all root publications (not referenced by other 30040s)
    pub async fn list_root_publications(
        &self,
        policy: FetchPolicy,
        limit: usize,
    ) -> Result<Vec<Publication>> {
        use serde_json::json;

        // Fetch all 30040 events
        let filter = json!({
            "kinds": [KIND_PUBLICATION_INDEX],
            "limit": limit * 2  // Fetch extra to account for non-root filtering
        });

        let response = self.engine.get_events(vec![filter], policy, None).await?;

        // Build set of all addresses referenced as children
        let mut child_addrs = std::collections::HashSet::new();
        for event in &response.events {
            if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
                for tag in tags {
                    if let Some(arr) = tag.as_array() {
                        if arr.first().and_then(|v| v.as_str()) == Some("a") {
                            if let Some(addr_str) = arr.get(1).and_then(|v| v.as_str()) {
                                if let Some(addr) = NAddr::from_a_tag(addr_str) {
                                    if addr.kind == KIND_PUBLICATION_INDEX {
                                        child_addrs.insert(addr.to_a_tag());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Filter to root publications only, deduplicating by a-tag (keep newest)
        // nostrdb stores all versions of replaceable events, so we need to dedupe
        let mut by_addr: std::collections::HashMap<String, Publication> = std::collections::HashMap::new();

        for event in response.events {
            match Publication::from_event(&event, true) {
                Ok(pub_) => {
                    let own_addr = pub_.addr.to_a_tag();

                    // Skip if this is a child publication
                    if child_addrs.contains(&own_addr) {
                        continue;
                    }

                    // Keep only the newest version for each a-tag
                    match by_addr.get(&own_addr) {
                        Some(existing) if pub_.created_at <= existing.created_at => continue,
                        _ => {
                            by_addr.insert(own_addr, pub_);
                        }
                    }
                }
                Err(e) => {
                    // Log and skip invalid publications (e.g., 30040 with content)
                    let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    tracing::debug!("Skipping invalid publication {}: {}", id, e);
                }
            }
        }

        // Collect into vec and sort by created_at descending
        let mut roots: Vec<Publication> = by_addr.into_values().collect();
        roots.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Apply limit after filtering
        roots.truncate(limit);

        Ok(roots)
    }

    /// List publications created before a given timestamp (for pagination)
    pub async fn list_publications_before(
        &self,
        before_timestamp: u64,
        policy: FetchPolicy,
        limit: usize,
    ) -> Result<Vec<Publication>> {
        use serde_json::json;

        // Fetch 30040 events created before the given timestamp
        let filter = json!({
            "kinds": [KIND_PUBLICATION_INDEX],
            "until": before_timestamp - 1,  // Exclusive of the timestamp
            "limit": limit * 2  // Fetch extra to account for filtering
        });

        let response = self.engine.get_events(vec![filter], policy, None).await?;

        // Build set of all addresses referenced as children (to filter out nested pubs)
        let mut child_addrs = std::collections::HashSet::new();
        for event in &response.events {
            if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
                for tag in tags {
                    if let Some(arr) = tag.as_array() {
                        if arr.first().and_then(|v| v.as_str()) == Some("a") {
                            if let Some(addr_str) = arr.get(1).and_then(|v| v.as_str()) {
                                if let Some(addr) = NAddr::from_a_tag(addr_str) {
                                    if addr.kind == KIND_PUBLICATION_INDEX {
                                        child_addrs.insert(addr.to_a_tag());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Filter to root publications only, deduplicating by a-tag (keep newest)
        let mut by_addr: std::collections::HashMap<String, Publication> = std::collections::HashMap::new();

        for event in response.events {
            match Publication::from_event(&event, true) {
                Ok(pub_) => {
                    let own_addr = pub_.addr.to_a_tag();

                    // Skip if this is a child publication
                    if child_addrs.contains(&own_addr) {
                        continue;
                    }

                    // Keep only the newest version for each a-tag
                    match by_addr.get(&own_addr) {
                        Some(existing) if pub_.created_at <= existing.created_at => continue,
                        _ => {
                            by_addr.insert(own_addr, pub_);
                        }
                    }
                }
                Err(e) => {
                    let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    tracing::debug!("Skipping invalid publication {}: {}", id, e);
                }
            }
        }

        // Collect into vec and sort by created_at descending
        let mut roots: Vec<Publication> = by_addr.into_values().collect();
        roots.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Apply limit after filtering
        roots.truncate(limit);

        Ok(roots)
    }

    /// Find alternate versions of a section (for forking UI)
    pub async fn find_section_versions(
        &self,
        section_addr: &NAddr,
        policy: FetchPolicy,
    ) -> Result<Vec<SectionVersion>> {
        use serde_json::json;

        // Query all events with the same kind and d-tag (from any author)
        let filter = json!({
            "kinds": [section_addr.kind],
            "#d": [&section_addr.d_tag],
            "limit": 50
        });

        let response = self.engine.get_events(vec![filter], policy, None).await?;

        let versions: Vec<SectionVersion> = response
            .events
            .into_iter()
            .map(|event| {
                let created_at = event.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
                let author = event
                    .get("pubkey")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let version = event
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .and_then(|tags| {
                        tags.iter().find_map(|tag| {
                            let arr = tag.as_array()?;
                            if arr.first()?.as_str()? == "version" {
                                arr.get(1)?.as_str().map(String::from)
                            } else {
                                None
                            }
                        })
                    });

                SectionVersion {
                    event,
                    created_at,
                    author,
                    version,
                }
            })
            .collect();

        Ok(versions)
    }

    /// Slot in an alternate section version
    pub fn slot_in_version(
        &self,
        pub_: &mut Publication,
        section_index: usize,
        version: SectionVersion,
    ) -> Result<()> {
        let section = pub_
            .sections
            .get_mut(section_index)
            .ok_or_else(|| EngineError::InvalidFilter("Section index out of bounds".into()))?;

        // Save current version to alternates if loaded
        if let LoadStatus::Loaded { data } = &section.event {
            let current_created_at = data.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let current_author = data
                .get("pubkey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            section.alternates.push(SectionVersion {
                event: data.clone(),
                created_at: current_created_at,
                author: current_author,
                version: None,
            });
        }

        // Replace with new version
        section.content = version
            .event
            .get("content")
            .and_then(|v| v.as_str())
            .map(String::from);
        section.event = LoadStatus::Loaded {
            data: version.event,
        };

        Ok(())
    }
}

// --- Event Building for Local Creation ---

use crate::tree::state::{ComposeState, SectionCompose};
use sha2::{Sha256, Digest};

/// Build unsigned publication events from compose state
///
/// Returns (publication_event, section_events) as JSON values.
/// The events have proper structure with calculated IDs but placeholder signatures.
/// Use `build_signed_publication_events` for events that can be stored in nostrdb.
pub fn build_publication_events(
    compose: &ComposeState,
    pubkey: &str,
) -> (Value, Vec<Value>) {
    build_publication_events_internal(compose, pubkey, None)
}

/// Build publication events with proper Schnorr signatures.
/// This is required for events to be accepted by nostrdb.
pub fn build_signed_publication_events(
    compose: &ComposeState,
    pubkey: &str,
    secret_hex: &str,
) -> (Value, Vec<Value>) {
    build_publication_events_internal(compose, pubkey, Some(secret_hex))
}

/// Internal function to build publication events with optional signing
fn build_publication_events_internal(
    compose: &ComposeState,
    pubkey: &str,
    secret_hex: Option<&str>,
) -> (Value, Vec<Value>) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Build section events first (need their d-tags for references)
    let mut section_events = Vec::new();
    for i in 0..compose.sections.len() {
        let section_d_tag = compose.section_d_tag(i);
        let section_event = build_section_event_internal(&compose.sections[i], &section_d_tag, pubkey, timestamp, secret_hex);
        section_events.push(section_event);
    }

    // Build publication index event
    let pub_event = build_index_event_internal(compose, pubkey, timestamp, secret_hex);

    (pub_event, section_events)
}

/// Build a section (30041) event with optional signing
fn build_section_event_internal(
    section: &SectionCompose,
    d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    secret_hex: Option<&str>,
) -> Value {
    use serde_json::json;

    // Build section tags
    let mut tags: Vec<Value> = vec![json!(["d", d_tag])];

    if !section.title.is_empty() {
        tags.push(json!(["title", &section.title]));
    }

    // Add section-specific tags
    for tag_vec in ComposeState::tags_to_nostr_format(&section.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    // Build the event without id/sig first for hashing
    let event_for_hash = json!([
        0,
        pubkey,
        timestamp,
        KIND_PUBLICATION_SECTION,
        tags,
        &section.content
    ]);

    let id = calculate_event_id(&event_for_hash);

    // Sign with secret key if provided, otherwise use placeholder
    let sig = if let Some(secret) = secret_hex {
        crate::identity::sign_event_hash(&id, secret).unwrap_or_else(|_| "0".repeat(128))
    } else {
        "0".repeat(128)
    };

    json!({
        "id": id,
        "pubkey": pubkey,
        "created_at": timestamp,
        "kind": KIND_PUBLICATION_SECTION,
        "tags": tags,
        "content": &section.content,
        "sig": sig
    })
}

/// Build a publication index (30040) event with optional signing
fn build_index_event_internal(
    compose: &ComposeState,
    pubkey: &str,
    timestamp: u64,
    secret_hex: Option<&str>,
) -> Value {
    use serde_json::json;

    let pub_d_tag = compose.publication_d_tag();

    // Build publication tags
    let mut tags: Vec<Value> = vec![json!(["d", &pub_d_tag])];

    if !compose.title.is_empty() {
        tags.push(json!(["title", &compose.title]));
    }

    // Add custom tags
    for tag_vec in ComposeState::tags_to_nostr_format(&compose.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    // Add section references (a-tags)
    for i in 0..compose.sections.len() {
        let section_d_tag = compose.section_d_tag(i);
        let a_tag_value = format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, section_d_tag);
        tags.push(json!(["a", a_tag_value, ""]));
    }

    // Add auto-update tag
    tags.push(json!(["auto-update", compose.auto_update.as_str()]));

    // Build the event without id/sig first for hashing
    // Note: 30040 events MUST have empty content
    let event_for_hash = json!([
        0,
        pubkey,
        timestamp,
        KIND_PUBLICATION_INDEX,
        tags,
        ""
    ]);

    let id = calculate_event_id(&event_for_hash);

    // Sign with secret key if provided, otherwise use placeholder
    let sig = if let Some(secret) = secret_hex {
        crate::identity::sign_event_hash(&id, secret).unwrap_or_else(|_| "0".repeat(128))
    } else {
        "0".repeat(128)
    };

    json!({
        "id": id,
        "pubkey": pubkey,
        "created_at": timestamp,
        "kind": KIND_PUBLICATION_INDEX,
        "tags": tags,
        "content": "",
        "sig": sig
    })
}

/// Calculate the event ID per NIP-01
///
/// The ID is the SHA256 hash of the serialized event array:
/// [0, pubkey, created_at, kind, tags, content]
fn calculate_event_id(event_array: &Value) -> String {
    let serialized = serde_json::to_string(event_array).unwrap_or_default();
    let hash = Sha256::digest(serialized.as_bytes());
    hex::encode(hash)
}

// --- Block-aware event building ---

use crate::tree::state::{BlockKind, ComposeBlock, ComposeBlockState};

/// Build publication events from a block-based compose state.
///
/// Returns (publication_30040_event, section_30041_events).
/// Imported blocks do NOT generate 30041 events — they reference the original.
pub fn build_block_publication_events(
    state: &ComposeBlockState,
    pubkey: &str,
    secret_hex: Option<&str>,
) -> (Value, Vec<Value>) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let pub_d_tag = state.publication_d_tag();
    let mut section_events = Vec::new();
    let mut a_tags: Vec<Value> = Vec::new();

    for (i, block) in state.blocks.iter().enumerate() {
        let block_d_tag = state.block_d_tag(i);

        match &block.kind {
            BlockKind::Editable { content, .. } => {
                // Build a new 30041 event via existing internal builder
                let section = SectionCompose {
                    title: block.title.clone(),
                    content: content.clone(),
                    tags: block.tags.iter().map(|t| crate::tree::state::TagEntry {
                        name: t.name.clone(),
                        value: t.value.clone(),
                    }).collect(),
                    ..Default::default()
                };
                let event = build_section_event_internal(
                    &section, &block_d_tag, pubkey, timestamp, secret_hex,
                );
                section_events.push(event);
                a_tags.push(serde_json::json!([
                    "a",
                    format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, block_d_tag),
                    ""
                ]));
            }
            BlockKind::Imported { source_addr, .. } => {
                // No 30041 event — reference the original directly
                a_tags.push(serde_json::json!([
                    "a",
                    format!(
                        "{}:{}:{}",
                        KIND_PUBLICATION_SECTION,
                        source_addr.pubkey,
                        source_addr.d_tag
                    ),
                    ""
                ]));
            }
            BlockKind::Forked { original_addr, content, .. } => {
                let event = build_forked_section_event(
                    block, &block_d_tag, original_addr, content, pubkey, timestamp, secret_hex,
                );
                section_events.push(event);
                a_tags.push(serde_json::json!([
                    "a",
                    format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, block_d_tag),
                    ""
                ]));
            }
        }
    }

    // Build the 30040 publication index event
    let pub_event = build_block_index_event(state, &pub_d_tag, pubkey, timestamp, &a_tags, secret_hex);

    (pub_event, section_events)
}

/// Build a 30041 event for a forked section with lineage tags
fn build_forked_section_event(
    block: &ComposeBlock,
    d_tag: &str,
    original_addr: &NAddr,
    content: &str,
    pubkey: &str,
    timestamp: u64,
    secret_hex: Option<&str>,
) -> Value {
    use serde_json::json;

    let mut tags: Vec<Value> = vec![json!(["d", d_tag])];

    if !block.title.is_empty() {
        tags.push(json!(["title", &block.title]));
    }

    // Fork lineage tag — NIP-54 addressable fork marker
    tags.push(json!([
        "a",
        format!(
            "{}:{}:{}",
            KIND_PUBLICATION_SECTION, original_addr.pubkey, original_addr.d_tag
        ),
        "",
        "fork"
    ]));

    // Custom tags from block
    for tag_vec in ComposeState::tags_to_nostr_format(&block.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    let event_for_hash = json!([
        0,
        pubkey,
        timestamp,
        KIND_PUBLICATION_SECTION,
        tags,
        content
    ]);

    let id = calculate_event_id(&event_for_hash);

    let sig = if let Some(secret) = secret_hex {
        crate::identity::sign_event_hash(&id, secret).unwrap_or_else(|_| "0".repeat(128))
    } else {
        "0".repeat(128)
    };

    json!({
        "id": id,
        "pubkey": pubkey,
        "created_at": timestamp,
        "kind": KIND_PUBLICATION_SECTION,
        "tags": tags,
        "content": content,
        "sig": sig
    })
}

/// Build a 30040 publication index event for block-based composition
fn build_block_index_event(
    state: &ComposeBlockState,
    pub_d_tag: &str,
    pubkey: &str,
    timestamp: u64,
    a_tags: &[Value],
    secret_hex: Option<&str>,
) -> Value {
    use serde_json::json;

    let mut tags: Vec<Value> = vec![json!(["d", pub_d_tag])];

    if !state.title.is_empty() {
        tags.push(json!(["title", &state.title]));
    }

    // Custom tags
    for tag_vec in ComposeState::tags_to_nostr_format(&state.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    // Section references
    tags.extend(a_tags.iter().cloned());

    // Auto-update tag
    tags.push(json!(["auto-update", state.auto_update.as_str()]));

    // 30040 events MUST have empty content
    let event_for_hash = json!([
        0,
        pubkey,
        timestamp,
        KIND_PUBLICATION_INDEX,
        tags,
        ""
    ]);

    let id = calculate_event_id(&event_for_hash);

    let sig = if let Some(secret) = secret_hex {
        crate::identity::sign_event_hash(&id, secret).unwrap_or_else(|_| "0".repeat(128))
    } else {
        "0".repeat(128)
    };

    json!({
        "id": id,
        "pubkey": pubkey,
        "created_at": timestamp,
        "kind": KIND_PUBLICATION_INDEX,
        "tags": tags,
        "content": "",
        "sig": sig
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::state::{ComposeBlockState, TagEntry};

    #[test]
    fn test_naddr_parsing() {
        let addr = NAddr::from_a_tag("30041:abc123:my-section").unwrap();
        assert_eq!(addr.kind, 30041);
        assert_eq!(addr.pubkey, "abc123");
        assert_eq!(addr.d_tag, "my-section");

        // D-tag with colons
        let addr = NAddr::from_a_tag("30041:abc123:my:section:with:colons").unwrap();
        assert_eq!(addr.d_tag, "my:section:with:colons");
    }

    #[test]
    fn test_naddr_roundtrip() {
        let addr = NAddr::new(30040, "pubkey123", "my-pub");
        let tag = addr.to_a_tag();
        let parsed = NAddr::from_a_tag(&tag).unwrap();
        assert_eq!(addr, parsed);
    }

    #[test]
    fn test_naddr_short_format() {
        // Long pubkey gets abbreviated
        let addr = NAddr::new(30041, "abcdefghijklmnopqrstuvwxyz", "my-section");
        assert_eq!(addr.short_format(), "30041:abcd...wxyz:my-section");

        // Short pubkey stays as-is
        let addr = NAddr::new(30040, "abcd1234", "doc");
        assert_eq!(addr.short_format(), "30040:abcd1234:doc");

        // Very short pubkey stays as-is
        let addr = NAddr::new(30040, "abc", "doc");
        assert_eq!(addr.short_format(), "30040:abc:doc");
    }

    // --- Block publication event tests ---

    fn make_block_state_all_editable() -> ComposeBlockState {
        let mut state = ComposeBlockState::new();
        state.title = "Test Article".into();
        state.add_editable();
        if let crate::tree::state::BlockKind::Editable { ref mut content, .. } = state.blocks[0].kind {
            *content = "Hello world".into();
        }
        state.blocks[0].title = "Intro".into();
        state
    }

    #[test]
    fn test_build_block_all_editable_regression() {
        let state = make_block_state_all_editable();
        let (pub_event, section_events) = build_block_publication_events(&state, "pubkey1", None);

        assert_eq!(pub_event["kind"], 30040);
        assert_eq!(pub_event["content"], "");
        assert_eq!(section_events.len(), 1);
        assert_eq!(section_events[0]["kind"], 30041);
        assert_eq!(section_events[0]["content"], "Hello world");
    }

    #[test]
    fn test_build_block_imported_only() {
        let mut state = ComposeBlockState::new();
        state.title = "Curated".into();
        let addr = NAddr::new(30041, "alice", "ch1");
        state.add_imported(addr, "imported content".into(), "alice".into(), "Chapter 1".into());

        let (pub_event, section_events) = build_block_publication_events(&state, "pubkey1", None);

        // No section events for imported blocks
        assert!(section_events.is_empty());
        // But the 30040 should have an a-tag pointing to alice's section
        let tags = pub_event["tags"].as_array().unwrap();
        let a_tags: Vec<_> = tags.iter().filter(|t| t[0] == "a").collect();
        assert_eq!(a_tags.len(), 1);
        assert!(a_tags[0][1].as_str().unwrap().contains("alice"));
        assert!(a_tags[0][1].as_str().unwrap().contains("ch1"));
    }

    #[test]
    fn test_build_block_forked_creates_30041_with_fork_tags() {
        let mut state = ComposeBlockState::new();
        state.title = "My Fork".into();
        let addr = NAddr::new(30041, "alice", "original-section");
        state.add_imported(addr, "original text".into(), "alice".into(), "Original".into());
        state.toggle_fork(0);

        let (_, section_events) = build_block_publication_events(&state, "pubkey1", None);
        assert_eq!(section_events.len(), 1);
        assert_eq!(section_events[0]["kind"], 30041);

        // Should have fork lineage tag
        let tags = section_events[0]["tags"].as_array().unwrap();
        let fork_tags: Vec<_> = tags.iter().filter(|t| {
            t.as_array().map(|a| a.len() >= 4 && a[3] == "fork").unwrap_or(false)
        }).collect();
        assert_eq!(fork_tags.len(), 1);
        assert!(fork_tags[0][1].as_str().unwrap().contains("alice"));
    }

    #[test]
    fn test_build_block_mixed_event_count_and_a_tags() {
        let mut state = ComposeBlockState::new();
        state.title = "Mixed".into();

        // 1 editable
        state.add_editable();
        state.blocks[0].title = "My Section".into();
        if let crate::tree::state::BlockKind::Editable { ref mut content, .. } = state.blocks[0].kind {
            *content = "editable text".into();
        }

        // 1 imported
        let addr = NAddr::new(30041, "alice", "ref-1");
        state.add_imported(addr, "imported".into(), "alice".into(), "Referenced".into());

        // 1 forked
        let addr2 = NAddr::new(30041, "bob", "forked-1");
        state.add_imported(addr2, "fork source".into(), "bob".into(), "Forked".into());
        state.toggle_fork(2);

        let (pub_event, section_events) = build_block_publication_events(&state, "pubkey1", None);

        // 2 section events (editable + forked), not 3
        assert_eq!(section_events.len(), 2);

        // 3 a-tags in the 30040
        let tags = pub_event["tags"].as_array().unwrap();
        let a_tags: Vec<_> = tags.iter().filter(|t| t[0] == "a").collect();
        assert_eq!(a_tags.len(), 3);
    }

    #[test]
    fn test_build_block_forked_has_fork_a_tag() {
        let mut state = ComposeBlockState::new();
        state.title = "Fork Test".into();
        let addr = NAddr::new(30041, "alice", "orig");
        state.add_imported(addr, "text".into(), "alice".into(), "T".into());
        state.toggle_fork(0);

        let (_, section_events) = build_block_publication_events(&state, "me", None);
        let tags = section_events[0]["tags"].as_array().unwrap();

        // Should have ["a", "30041:alice:orig", "", "fork"]
        let fork_a = tags.iter().find(|t| {
            t.as_array().map(|a| a.len() >= 4 && a[3] == "fork").unwrap_or(false)
        });
        assert!(fork_a.is_some());
        let fork_a = fork_a.unwrap();
        assert_eq!(fork_a[1].as_str().unwrap(), "30041:alice:orig");
    }

    #[test]
    fn test_build_block_imported_produces_no_event() {
        let mut state = ComposeBlockState::new();
        state.title = "Import Only".into();
        state.add_imported(
            NAddr::new(30041, "x", "y"),
            "content".into(),
            "x".into(),
            "T".into(),
        );
        let (_, section_events) = build_block_publication_events(&state, "me", None);
        assert!(section_events.is_empty());
    }

    #[test]
    fn test_build_block_signed_events_structure() {
        let state = make_block_state_all_editable();
        let (pub_event, section_events) = build_block_publication_events(&state, "pubkey1", None);

        // All events should have required fields
        for event in std::iter::once(&pub_event).chain(section_events.iter()) {
            assert!(event.get("id").is_some());
            assert!(event.get("pubkey").is_some());
            assert!(event.get("created_at").is_some());
            assert!(event.get("kind").is_some());
            assert!(event.get("tags").is_some());
            assert!(event.get("content").is_some());
            assert!(event.get("sig").is_some());
        }
    }
}
