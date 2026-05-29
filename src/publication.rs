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

pub mod tree_emit;

/// Kind constants for NKBIP-01 publications
pub const KIND_PUBLICATION_INDEX: u64 = 30040;
pub const KIND_PUBLICATION_SECTION: u64 = 30041;

/// Mint an opaque d-tag — 21-character URL-safe random string, same shape as
/// the JS nanoid package (alphabet `A-Za-z0-9_-`). NKBIP-01 publications use
/// stable nanoid d-tags so titles can be edited without breaking addressable
/// identity, and so the same author can publish e.g. "virus" in different
/// contexts without slug collisions. Every site that needs a fresh
/// publication or section d-tag should funnel through here.
pub fn mint_d_tag() -> String {
    nanoid::nanoid!()
}

/// Kinds an NKBIP-01 index (30040) may reference as a *content leaf* — a
/// readable terminal node, as opposed to a nested 30040 index. 30041 is the
/// canonical publication section; 30023 (NIP-23 long-form) and 30818/30817
/// (NKBIP-02 wiki) are addressable content events an index can curate
/// directly as "zettels". Extend here to admit more leaf kinds.
pub const ZETTEL_KINDS: &[u64] = &[
    KIND_PUBLICATION_SECTION, // 30041
    30023,                    // NIP-23 long-form article
    30818,                    // NKBIP-02 wiki article
    30817,                    // NKBIP-02 wiki (alt)
];

/// Whether `kind` is a content leaf an index may reference (vs. a nested
/// 30040 index, or a non-publication kind that should be ignored).
pub fn is_zettel_kind(kind: u64) -> bool {
    ZETTEL_KINDS.contains(&kind)
}

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
    /// Relays the index event (kind 30040) has been seen on. Empty means the
    /// publication was written locally and has not been published/fetched.
    pub relays: Vec<String>,
    /// Whether the index event carries a real signature. `false` = an
    /// unsigned draft (placeholder all-zero signature).
    pub signed: bool,
    /// True when the index event carries a NIP-54 fork-marker tag — an
    /// `a` tag pointing at another kind-30040 publication or an `e` tag
    /// pointing at another 30040 event, with the 4th element equal to
    /// `"fork"`. Drives the `fork` provenance pill on every surface that
    /// renders this publication.
    pub forked: bool,
}

impl Publication {
    /// Extract metadata from a publication index event
    ///
    /// Note: Publication index events (kind 30040) should not have content per NKBIP-01,
    /// but in practice many events do. We accept them and ignore the content field.
    pub fn from_event(event: &Value, is_root: bool) -> Result<Self> {

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
        // NIP-54 fork marker: an `a` or `e` tag whose 4th element is the
        // literal "fork". The `a` variant points at the parent 30040
        // address (kind:pubkey:d-tag), the `e` variant at its event id.
        // Both signal lineage; either presence is enough.
        let mut forked = false;

        if let Some(tags) = tags {
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    let tag_name = arr.first().and_then(|v| v.as_str());
                    let tag_value = arr.get(1).and_then(|v| v.as_str());
                    let tag_marker = arr.get(3).and_then(|v| v.as_str());

                    match tag_name {
                        Some("d") => d_tag = tag_value.unwrap_or(&d_tag).to_string(),
                        Some("title") | Some("name") => title = tag_value.map(String::from),
                        Some("summary") | Some("description") => summary = tag_value.map(String::from),
                        Some("image") | Some("thumb") => image = tag_value.map(String::from),
                        Some("version") => version = tag_value.map(String::from),
                        Some("a") => {
                            // Fork-marker `a` tag points at the parent 30040
                            // — recognised before the section-collection
                            // path so it isn't mis-parsed as a content ref.
                            if tag_marker == Some("fork") {
                                forked = true;
                            } else if let Some(addr_str) = tag_value {
                                if let Some(addr) = NAddr::from_a_tag(addr_str) {
                                    section_addrs.push(addr);
                                }
                            }
                        }
                        Some("e") => {
                            // Fork-marker `e` tag — points at the parent
                            // 30040's event id. Non-fork `e` tags on a
                            // 30040 are uncommon but we ignore them.
                            if tag_marker == Some("fork") {
                                forked = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let addr = NAddr::new(KIND_PUBLICATION_INDEX, pubkey, &d_tag);

        // Separate a-tags: 30041 → sections, 30040 → nested publication stubs
        let mut sections = Vec::new();
        let mut nested = Vec::new();
        let mut sec_pos = 0;

        for a in section_addrs {
            // Any zettel kind (30041 + long-form/wiki) is a content leaf;
            // 30040 is a nested index; everything else is dropped.
            if is_zettel_kind(a.kind) {
                sections.push(Section {
                    addr: a,
                    event: LoadStatus::Pending,
                    title: None,
                    content: None,
                    position: sec_pos,
                    alternates: Vec::new(),
                });
                sec_pos += 1;
            } else if a.kind == KIND_PUBLICATION_INDEX {
                nested.push(Publication {
                    addr: a.clone(),
                    index: LoadStatus::Pending,
                    title: None,
                    summary: None,
                    image: None,
                    author_pubkey: a.pubkey.clone(),
                    author_name: None,
                    version: None,
                    created_at: 0,
                    sections: Vec::new(),
                    nested: Vec::new(),
                    is_root: false,
                    // Unknown until the nested index is actually loaded.
                    relays: Vec::new(),
                    signed: true,
                    forked: false,
                });
            }
        }

        // Relay provenance + signature state, threaded through from
        // `note_to_json` (relays) and the raw event (sig).
        let relays: Vec<String> = event
            .get("relays")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let signed = event
            .get("sig")
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty() && !s.chars().all(|c| c == '0'))
            .unwrap_or(false);

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
            nested,
            is_root,
            relays,
            signed,
            forked,
        })
    }

    /// Get total child count (sections + nested sub-publications)
    /// Matches notedeck behavior: counts all a-tag references
    pub fn section_count(&self) -> usize {
        self.sections.len() + self.nested.len()
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
    /// Section body, when this is a resolved 30041 leaf. `None` for nested
    /// indexes and for sections whose content has not been loaded. A depth-N
    /// tree load fills this for every section inside the depth horizon, so the
    /// reader can render the whole tree without per-section round trips.
    pub content: Option<String>,
    /// Children entries (for tree view)
    pub children: Vec<TocEntry>,
}

/// CPU-heavy publication dedup/filter/sort — runs in spawn_blocking
fn process_root_publications(
    events: Vec<serde_json::Value>,
    ignore_list: crate::engine::IgnoreList,
    limit: usize,
) -> Vec<Publication> {
    use std::collections::{HashMap, HashSet};

    // Build set of all addresses referenced as children
    let mut child_addrs = HashSet::new();
    for event in &events {
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

    let mut by_addr: HashMap<String, Publication> = HashMap::new();
    let mut skipped_child = 0usize;
    let mut skipped_empty = 0usize;
    let mut skipped_dupe = 0usize;
    let mut skipped_err = 0usize;
    let mut skipped_ignored = 0usize;

    for event in events {
        let event_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let pubkey = event.get("pubkey").and_then(|v| v.as_str()).unwrap_or("");
        if ignore_list.is_ignored(event_id, pubkey) {
            skipped_ignored += 1;
            continue;
        }
        let d_tag = event.get("tags").and_then(|t| t.as_array()).and_then(|tags| {
            tags.iter().find_map(|tag| {
                let arr = tag.as_array()?;
                if arr.first()?.as_str()? == "d" { arr.get(1)?.as_str() } else { None }
            })
        }).unwrap_or("");
        let a_tag = format!("{}:{}:{}", KIND_PUBLICATION_INDEX, pubkey, d_tag);
        if ignore_list.event_ids.contains(&a_tag) {
            skipped_ignored += 1;
            continue;
        }

        match Publication::from_event(&event, true) {
            Ok(pub_) => {
                let own_addr = pub_.addr.to_a_tag();
                if child_addrs.contains(&own_addr) {
                    skipped_child += 1;
                    continue;
                }
                if pub_.sections.is_empty() && pub_.nested.is_empty() {
                    skipped_empty += 1;
                    continue;
                }
                match by_addr.get(&own_addr) {
                    Some(existing) if pub_.created_at <= existing.created_at => {
                        skipped_dupe += 1;
                    }
                    _ => {
                        by_addr.insert(own_addr, pub_);
                    }
                }
            }
            Err(e) => {
                skipped_err += 1;
                tracing::debug!("Skipping invalid publication: {}", e);
            }
        }
    }

    tracing::info!(
        "list_root_publications: {} unique roots (skipped: {} child, {} empty, {} dupe, {} err, {} ignored)",
        by_addr.len(), skipped_child, skipped_empty, skipped_dupe, skipped_err, skipped_ignored
    );

    let mut roots: Vec<Publication> = by_addr.into_values().collect();
    roots.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.addr.d_tag.cmp(&b.addr.d_tag)));
    roots.truncate(limit);
    roots
}

/// A child reference inside a streamed [`PubLoadEvent::Index`] — enough for the
/// client to allocate a tree slot and decide whether the child counts toward
/// the load total `N`.
#[derive(Debug, Clone, Serialize)]
pub struct PubChildRef {
    pub addr: NAddr,
    /// True for a nested 30040 index, false for a content leaf.
    pub is_index: bool,
    /// True if this child's own event(s) will be streamed (within the depth
    /// horizon and not a cycle). `false` = a frontier stub: rendered and
    /// refocus-able, but it emits no events and does not count toward `N`.
    pub in_horizon: bool,
}

/// Serde default for the `signed` field on stream events. Referenced via
/// `#[serde(default = ...)]` attributes; the compiler can't see those uses
/// statically because we never deserialize PubLoadEvent in the engine
/// (only the web client does). Kept anyway so the schema round-trips
/// cleanly if a tool ever decodes the stream.
#[allow(dead_code)]
fn default_true() -> bool {
    true
}

/// One progress event in a streaming publication load (see
/// [`PublicationEngine::stream_publication_tree`]). Serialized to SSE as a
/// `{"type": ...}`-tagged JSON object.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PubLoadEvent {
    /// A 30040 index resolved. Always arrives before its children's events,
    /// so the client can allocate child slots immediately.
    Index {
        addr: NAddr,
        depth: usize,
        title: Option<String>,
        is_root: bool,
        children: Vec<PubChildRef>,
        /// Relays this index event has been seen on. Empty = local-only.
        /// Powers the reader's draft / relay-label provenance pill.
        #[serde(default)]
        relays: Vec<String>,
        /// False = unsigned draft (placeholder all-zero signature).
        #[serde(default = "default_true")]
        signed: bool,
        /// True when the index event carries a NIP-54 fork-marker tag —
        /// powers the `fork` provenance pill.
        #[serde(default)]
        forked: bool,
    },
    /// A content leaf (30041 or another `ZETTEL_KINDS` kind) resolved.
    Leaf {
        addr: NAddr,
        depth: usize,
        title: Option<String>,
        content: Option<String>,
        /// Same provenance fields as Index — surfaced on the reader outline
        /// row + the paginated/continuous section title.
        #[serde(default)]
        relays: Vec<String>,
        #[serde(default = "default_true")]
        signed: bool,
    },
    /// A node (index or leaf) failed to resolve. Counted toward `N` so the
    /// client's `i/N` still terminates cleanly.
    Error {
        addr: NAddr,
        depth: usize,
        message: String,
    },
    /// Terminal event — the in-horizon walk finished. `total` is the
    /// authoritative in-horizon node count (a checksum for the client's `N`).
    Done { total: usize },
}

/// Emit one stream event and bump the in-horizon counter. Returns `false` when
/// the receiver has been dropped (the client disconnected) — callers then
/// return, which unwinds the recursive load.
async fn emit_pub_event(
    tx: &tokio::sync::mpsc::Sender<PubLoadEvent>,
    counter: &std::sync::atomic::AtomicUsize,
    ev: PubLoadEvent,
) -> bool {
    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tx.send(ev).await.is_ok()
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

    /// Recursively load a publication and its nested sub-publications to a
    /// bounded depth.
    ///
    /// `max_depth` counts levels of *30040 nesting*: `0` loads this index and its
    /// own 30041 sections only (nested 30040s are left as `Pending` stubs); `1`
    /// additionally recurses one level into each nested index; and so on.
    /// Sections (30041) are leaves and never consume a level.
    ///
    /// Below `max_depth`, nested indexes remain `Pending` stubs — the caller can
    /// expand them later (lazily, or with another `load_publication_tree` call).
    ///
    /// Cycle-safe: a 30040 that references an ancestor (or itself) is not
    /// recursed into; its stub is left `Pending`. Sibling nested indexes at one
    /// level are fetched concurrently, bounded by `MAX_CONCURRENT_INDEX_FETCHES`.
    pub async fn load_publication_tree(
        &self,
        addr: &NAddr,
        max_depth: usize,
        policy: FetchPolicy,
    ) -> Result<Publication> {
        self.load_tree_inner(
            addr.clone(),
            max_depth,
            true,
            policy,
            std::collections::HashSet::new(),
        )
        .await
    }

    /// Recursive worker for [`load_publication_tree`]. Returns a boxed future so
    /// the async recursion has a concrete (non-infinite) type.
    fn load_tree_inner<'s>(
        &'s self,
        addr: NAddr,
        max_depth: usize,
        is_root: bool,
        policy: FetchPolicy,
        ancestors: std::collections::HashSet<NAddr>,
    ) -> futures::future::BoxFuture<'s, Result<Publication>> {
        /// Max nested indexes fetched concurrently at a single tree level.
        const MAX_CONCURRENT_INDEX_FETCHES: usize = 8;

        Box::pin(async move {
            // 1. Fetch and parse this index event.
            let event = self
                .engine
                .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, policy)
                .await?
                .ok_or_else(|| {
                    EngineError::Database(format!(
                        "Publication index not found: {}",
                        addr.to_a_tag()
                    ))
                })?;
            let mut pub_ = Publication::from_event(&event, is_root)?;

            // 2. Resolve this level's 30041 leaf sections (full load).
            if let Err(e) = self.load_sections(&mut pub_, policy).await {
                tracing::warn!(
                    "load_publication_tree: section load failed for {}: {}",
                    addr.to_a_tag(),
                    e
                );
            }

            // 3. Recurse into nested 30040 indexes, depth-bounded and cycle-guarded.
            if max_depth > 0 && !pub_.nested.is_empty() {
                // This node joins the ancestor set seen by its children.
                let mut child_ancestors = ancestors;
                child_ancestors.insert(addr.clone());

                // Children to recurse into, skipping any that point back up the path.
                let to_load: Vec<(usize, NAddr)> = pub_
                    .nested
                    .iter()
                    .enumerate()
                    .filter(|(_, stub)| !child_ancestors.contains(&stub.addr))
                    .map(|(i, stub)| (i, stub.addr.clone()))
                    .collect();

                // Fetch sibling sub-publications concurrently, bounded.
                use futures::stream::StreamExt;
                let results: Vec<(usize, Result<Publication>)> = futures::stream::iter(to_load)
                    .map(|(i, child_addr)| {
                        let ancestors = child_ancestors.clone();
                        async move {
                            (
                                i,
                                self.load_tree_inner(
                                    child_addr,
                                    max_depth - 1,
                                    false,
                                    policy,
                                    ancestors,
                                )
                                .await,
                            )
                        }
                    })
                    .buffer_unordered(MAX_CONCURRENT_INDEX_FETCHES)
                    .collect()
                    .await;

                // Attach results, replacing stubs in place (preserves a-tag order).
                for (i, result) in results {
                    match result {
                        Ok(child) => pub_.nested[i] = child,
                        Err(e) => {
                            tracing::warn!(
                                "load_publication_tree: nested index {} failed: {}",
                                pub_.nested[i].addr.to_a_tag(),
                                e
                            );
                            pub_.nested[i].index = LoadStatus::Failed {
                                error: e.to_string(),
                            };
                        }
                    }
                }
            }

            // 4. Title backfill for nested indexes left unresolved (depth
            //    horizon, cycle skip, or a failed recurse). Look the index
            //    event up in the local store only — if it's there, surface
            //    its real title; if it isn't, leave `title` empty so the TOC
            //    falls back to the d-tag as a preview. No relay traffic here:
            //    this is a cheap display nicety, not a fetch.
            for stub in pub_.nested.iter_mut() {
                if stub.index.is_loaded() {
                    continue;
                }
                if let Ok(Some(ev)) = self
                    .engine
                    .get_addressable(
                        stub.addr.kind,
                        &stub.addr.pubkey,
                        &stub.addr.d_tag,
                        FetchPolicy::LocalOnly,
                    )
                    .await
                {
                    stub.title = Publication::from_event(&ev, false).ok().and_then(|p| p.title);
                }
            }

            Ok(pub_)
        })
    }

    /// Stream a publication tree: run the same depth-bounded, cycle-guarded
    /// recursive load as [`load_publication_tree`], but emit a [`PubLoadEvent`]
    /// for every node as it resolves rather than returning the assembled tree.
    /// An SSE handler forwards the events to the client, which builds the tree
    /// incrementally and shows a per-event `i/N` counter.
    ///
    /// Cancellation is implicit: when `tx`'s receiver is dropped (the client
    /// disconnected) every `send` fails and each recursive task returns. Sends
    /// a terminal [`PubLoadEvent::Done`] and returns the in-horizon node count.
    pub async fn stream_publication_tree(
        &self,
        addr: &NAddr,
        max_depth: usize,
        policy: FetchPolicy,
        tx: tokio::sync::mpsc::Sender<PubLoadEvent>,
    ) -> usize {
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        self.stream_tree_inner(
            addr.clone(),
            0,
            max_depth,
            true,
            policy,
            std::collections::HashSet::new(),
            tx.clone(),
            counter.clone(),
        )
        .await;
        let total = counter.load(std::sync::atomic::Ordering::Relaxed);
        let _ = tx.send(PubLoadEvent::Done { total }).await;
        total
    }

    /// Recursive worker for [`stream_publication_tree`]. `depth` is this node's
    /// level from the root (root = 0); `remaining_depth` is the 30040-nesting
    /// budget left (mirrors `max_depth` in `load_tree_inner`). Returns early on
    /// the first failed `send` — that is how a client disconnect aborts the load.
    fn stream_tree_inner<'s>(
        &'s self,
        addr: NAddr,
        depth: usize,
        remaining_depth: usize,
        is_root: bool,
        policy: FetchPolicy,
        ancestors: std::collections::HashSet<NAddr>,
        tx: tokio::sync::mpsc::Sender<PubLoadEvent>,
        counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> futures::future::BoxFuture<'s, ()> {
        /// Max nested indexes streamed concurrently at a single tree level.
        const MAX_CONCURRENT_INDEX_FETCHES: usize = 8;

        Box::pin(async move {
            // 1. Fetch and parse this index event.
            let event = match self
                .engine
                .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, policy)
                .await
            {
                Ok(Some(ev)) => ev,
                Ok(None) => {
                    emit_pub_event(
                        &tx,
                        &counter,
                        PubLoadEvent::Error {
                            addr: addr.clone(),
                            depth,
                            message: "Publication index not found".into(),
                        },
                    )
                    .await;
                    return;
                }
                Err(e) => {
                    emit_pub_event(
                        &tx,
                        &counter,
                        PubLoadEvent::Error {
                            addr: addr.clone(),
                            depth,
                            message: e.to_string(),
                        },
                    )
                    .await;
                    return;
                }
            };
            let pub_ = match Publication::from_event(&event, is_root) {
                Ok(p) => p,
                Err(e) => {
                    emit_pub_event(
                        &tx,
                        &counter,
                        PubLoadEvent::Error {
                            addr: addr.clone(),
                            depth,
                            message: e.to_string(),
                        },
                    )
                    .await;
                    return;
                }
            };

            // The current node joins the ancestor set for the cycle guard.
            let mut child_ancestors = ancestors;
            child_ancestors.insert(addr.clone());

            // 2. Ordered child list: leaf sections first, then nested indexes
            //    (matching `build_toc`). A nested index is in-horizon iff there
            //    is depth budget left and it doesn't point back up the path.
            let mut children: Vec<PubChildRef> = Vec::new();
            for section in &pub_.sections {
                children.push(PubChildRef {
                    addr: section.addr.clone(),
                    is_index: false,
                    in_horizon: true,
                });
            }
            for nested in &pub_.nested {
                let will_recurse =
                    remaining_depth > 0 && !child_ancestors.contains(&nested.addr);
                children.push(PubChildRef {
                    addr: nested.addr.clone(),
                    is_index: true,
                    in_horizon: will_recurse,
                });
            }

            // 3. Emit this index — before any of its children's events.
            //    `pub_` already carries the provenance fields off the
            //    index event (Publication::from_event derives them), so
            //    the stream just clones them through.
            if !emit_pub_event(
                &tx,
                &counter,
                PubLoadEvent::Index {
                    addr: addr.clone(),
                    depth,
                    title: pub_.title.clone(),
                    is_root,
                    children,
                    relays: pub_.relays.clone(),
                    signed: pub_.signed,
                    forked: pub_.forked,
                },
            )
            .await
            {
                return; // receiver gone — abort
            }

            // 4. Resolve this index's content leaves, one event each.
            for section in &pub_.sections {
                let ev = match self
                    .engine
                    .get_addressable(
                        section.addr.kind,
                        &section.addr.pubkey,
                        &section.addr.d_tag,
                        policy,
                    )
                    .await
                {
                    Ok(Some(leaf)) => {
                        let content = leaf
                            .get("content")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let title = leaf
                            .get("tags")
                            .and_then(|v| v.as_array())
                            .and_then(|tags| {
                                tags.iter().find_map(|t| {
                                    let t = t.as_array()?;
                                    if t.first()?.as_str()? == "title" {
                                        t.get(1)?.as_str().map(String::from)
                                    } else {
                                        None
                                    }
                                })
                            });
                        // Mirror the publication-index path: extract sig +
                        // relays from the raw event JSON so the section row
                        // can render the same draft / relay-label pill the
                        // root publication does.
                        let relays: Vec<String> = leaf
                            .get("relays")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let signed = leaf
                            .get("sig")
                            .and_then(|v| v.as_str())
                            .map(|s| !s.is_empty() && !s.chars().all(|c| c == '0'))
                            .unwrap_or(false);
                        PubLoadEvent::Leaf {
                            addr: section.addr.clone(),
                            depth: depth + 1,
                            title,
                            content,
                            relays,
                            signed,
                        }
                    }
                    Ok(None) => PubLoadEvent::Error {
                        addr: section.addr.clone(),
                        depth: depth + 1,
                        message: "Section not found".into(),
                    },
                    Err(e) => PubLoadEvent::Error {
                        addr: section.addr.clone(),
                        depth: depth + 1,
                        message: e.to_string(),
                    },
                };
                if !emit_pub_event(&tx, &counter, ev).await {
                    return; // receiver gone — abort
                }
            }

            // 5. Recurse into in-horizon nested indexes, concurrency-bounded.
            if remaining_depth > 0 {
                let to_recurse: Vec<NAddr> = pub_
                    .nested
                    .iter()
                    .map(|n| n.addr.clone())
                    .filter(|a| !child_ancestors.contains(a))
                    .collect();
                if !to_recurse.is_empty() {
                    use futures::stream::StreamExt;
                    futures::stream::iter(to_recurse)
                        .map(|child_addr| {
                            let ancestors = child_ancestors.clone();
                            let tx = tx.clone();
                            let counter = counter.clone();
                            async move {
                                self.stream_tree_inner(
                                    child_addr,
                                    depth + 1,
                                    remaining_depth - 1,
                                    false,
                                    policy,
                                    ancestors,
                                    tx,
                                    counter,
                                )
                                .await
                            }
                        })
                        .buffer_unordered(MAX_CONCURRENT_INDEX_FETCHES)
                        .collect::<Vec<()>>()
                        .await;
                }
            }
        })
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
                content: section.content.clone(),
                children: Vec::new(),
            });
        }

        for nested in &pub_.nested {
            let children = self.build_toc(nested, depth + 1);
            entries.push(TocEntry {
                addr: nested.addr.clone(),
                // `title` is set whenever the index event is in the store
                // (resolved, or title-backfilled in `load_tree_inner` step 4).
                // It's empty only for an index genuinely absent from the DB —
                // there, preview with the `d` tag from the parent's `a` tag.
                title: nested
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| {
                        let d = nested.addr.d_tag.trim();
                        (!d.is_empty()).then(|| d.to_string())
                    })
                    .unwrap_or_else(|| "Nested Publication".into()),
                depth,
                loaded: nested.index.is_loaded(),
                is_publication: true,
                content: None,
                children,
            });
        }

        entries
    }

    /// Query all root publications (not referenced by other 30040s)
    ///
    /// Pass `before` timestamp for cursor-based pagination.
    pub async fn list_root_publications(
        &self,
        policy: FetchPolicy,
        limit: usize,
        before: Option<u64>,
    ) -> Result<Vec<Publication>> {
        use serde_json::json;

        // Scope to known authors to avoid processing thousands of foreign events.
        // nostrdb stores all versions of replaceable events, so we over-fetch and dedup.
        let mut authors: Vec<String> = self.engine.relay_config().authors_hex();
        if let Some(me) = self.engine.my_pubkey() {
            if !authors.contains(&me.to_string()) {
                authors.push(me.to_string());
            }
        }
        if let Some(asst) = self.engine.assistant_pubkey() {
            if !authors.contains(&asst.to_string()) {
                authors.push(asst.to_string());
            }
        }

        let query_limit = if authors.is_empty() { limit * 5 } else { limit * 10 };
        let mut filter = json!({
            "kinds": [KIND_PUBLICATION_INDEX],
            "limit": query_limit
        });
        if !authors.is_empty() {
            filter["authors"] = json!(authors);
        }
        if let Some(ts) = before {
            filter["until"] = json!(ts - 1);
        }

        // Honor the caller's policy. The web's loadFeed() retries
        // with FetchAlways when the local query returns zero (cold
        // cache after purge / fresh install) — that retry only does
        // anything if we actually fan out to relays here.
        //
        // For FetchAlways specifically, we route through
        // begin_fetch_operation first so confirm mode gets its modal
        // (and auto mode gets its activity toast). Passing
        // mode_confirm=true to get_events would BYPASS the confirm
        // step rather than trigger it; the modal flow lives in
        // begin_operation. If the user declines, we fall back to
        // LocalOnly (returns the empty result we already have).
        let response = match policy {
            FetchPolicy::FetchAlways => {
                let relays: Vec<String> = self.engine.relays();
                let label = "Feed sync — list publications".to_string();
                match self
                    .engine
                    .begin_fetch_operation(
                        crate::network::FetchPattern::Publication,
                        label,
                        Vec::new(),
                        relays,
                    )
                    .await
                {
                    Ok(op) => {
                        // User approved (or auto mode let it through).
                        // Run the actual fan-out via get_events with
                        // mode_confirm=true so the engine doesn't
                        // silently downgrade now that the user has OK'd it.
                        let chosen_relays = op.relays().to_vec();
                        let res = self
                            .engine
                            .get_events_with_options(
                                vec![filter],
                                FetchPolicy::FetchAlways,
                                Some(&chosen_relays),
                                true,
                            )
                            .await;
                        let count = res.as_ref().map(|r| r.events.len()).unwrap_or(0);
                        op.complete(count);
                        res?
                    }
                    Err(_) => {
                        // User declined / timeout. Return what's in
                        // the local cache (empty for the cold-cache
                        // case that motivated this branch).
                        self.engine
                            .get_events(vec![filter], FetchPolicy::LocalOnly, None)
                            .await?
                    }
                }
            }
            _ => {
                self.engine
                    .get_events(vec![filter], policy, None)
                    .await?
            }
        };
        tracing::debug!(
            "list_root_publications: got {} raw 30040 events from store (policy {:?})",
            response.events.len(),
            policy
        );

        // Read ignore list before entering blocking closure
        let ignore_list = self.engine.ignore_list().read().await.clone();
        let events = response.events;

        // Offload CPU-heavy dedup/filter/sort to blocking threadpool
        // so the async runtime stays responsive
        let roots = tokio::task::spawn_blocking(move || {
            process_root_publications(events, ignore_list, limit)
        })
        .await
        .map_err(|e| crate::error::EngineError::Database(format!("spawn_blocking: {e}")))?;

        Ok(roots)
    }

    /// List publications created before a given timestamp (for pagination)
    pub async fn list_publications_before(
        &self,
        before_timestamp: u64,
        _policy: FetchPolicy,
        limit: usize,
    ) -> Result<Vec<Publication>> {
        use serde_json::json;

        // Scope to known authors — same as list_root_publications
        let mut authors: Vec<String> = self.engine.relay_config().authors_hex();
        if let Some(me) = self.engine.my_pubkey() {
            if !authors.contains(&me.to_string()) {
                authors.push(me.to_string());
            }
        }
        if let Some(asst) = self.engine.assistant_pubkey() {
            if !authors.contains(&asst.to_string()) {
                authors.push(asst.to_string());
            }
        }

        let mut filter = json!({
            "kinds": [KIND_PUBLICATION_INDEX],
            "until": before_timestamp - 1,
            "limit": limit * 5
        });
        if !authors.is_empty() {
            filter["authors"] = json!(authors);
        }

        let response = self.engine.get_events(vec![filter], FetchPolicy::LocalOnly, None).await?;
        let ignore_list = self.engine.ignore_list().read().await.clone();
        let events = response.events;

        let roots = tokio::task::spawn_blocking(move || {
            process_root_publications(events, ignore_list, limit)
        })
        .await
        .map_err(|e| crate::error::EngineError::Database(format!("spawn_blocking: {e}")))?;

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
    compose: &mut ComposeState,
    pubkey: &str,
) -> (Value, Vec<Value>) {
    build_publication_events_internal(compose, pubkey, None)
}

/// Build publication events with proper Schnorr signatures using the
/// engine's in-process key material directly. Kept for the synchronous / test
/// paths that don't need to route through the SigningController; live
/// HTTP publish goes through `build_signed_publication_events_via_signer`.
pub fn build_signed_publication_events(
    compose: &mut ComposeState,
    pubkey: &str,
    secret_hex: &str,
) -> (Value, Vec<Value>) {
    build_publication_events_internal(compose, pubkey, Some(secret_hex))
}

/// Build publication events through a `Signer`. Each section + the
/// 30040 index is constructed as an `EventTemplate` and signed via
/// `signer.sign(...)`. This is the path that supports external signers
/// (NIP-07 / NIP-46): every per-event sig round-trips through whatever
/// the active source is.
///
/// `pubkey` must match the signer's active source pubkey. The signer
/// re-checks via `template.pubkey` and refuses on mismatch.
pub async fn build_signed_publication_events_via_signer(
    compose: &mut ComposeState,
    pubkey: &str,
    signer: &dyn crate::signing::Signer,
) -> std::result::Result<(Value, Vec<Value>), crate::signing::SigningError> {
    use crate::signing::EventTemplate;
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let pubkey = pubkey.to_string();
    let is_nested = compose.sections.iter().any(|s| s.level > 2);

    // Nested path: reuse the in-process emitter to build unsigned events
    // (placeholder sigs), then re-sign each through the external signer.
    // This keeps the recursive tag-graph logic in one place.
    if is_nested {
        let pub_d_tag = compose.publication_d_tag();
        let (root_unsigned, children_unsigned) = tree_emit::build_nested_publication_events(
            compose,
            &pub_d_tag,
            &pubkey,
            timestamp as u64,
            DEFAULT_PARSE_LEVEL,
            None, // placeholder sig — we re-sign below
        );

        // Re-sign each child event through the external signer (e.g.
        // NIP-07 / NIP-46). The Value already carries the correct tags
        // and content; we just need a fresh sig over the canonical hash.
        let mut signed_children = Vec::with_capacity(children_unsigned.len());
        for ev in children_unsigned {
            let signed = resign_value_through_signer(&ev, &pubkey, timestamp, signer).await?;
            signed_children.push(signed);
        }
        let pub_event =
            resign_value_through_signer(&root_unsigned, &pubkey, timestamp, signer).await?;

        return Ok((pub_event, signed_children));
    }

    // Flat path (unchanged): per-section template signed in sequence,
    // then the single 30040 index.
    let mut section_events = Vec::new();
    for i in 0..compose.sections.len() {
        let section_d_tag = compose.section_d_tag(i);
        let section = &compose.sections[i];
        let section_title = section.title.clone();
        let section_content = section.content.clone();
        let section_tags = section.tags.clone();

        let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), section_d_tag.clone()]];
        if !section_title.is_empty() {
            // `title` = display; `T` = indexable title for search/discovery.
            tags.push(vec!["title".into(), section_title.clone()]);
            tags.push(vec!["T".into(), ComposeState::generate_d_tag(&section_title)]);
        }
        for tag_vec in ComposeState::tags_to_nostr_format(&section_tags) {
            tags.push(tag_vec);
        }

        let template = EventTemplate {
            kind: KIND_PUBLICATION_SECTION as u32,
            created_at: timestamp,
            tags,
            content: section_content,
            pubkey: Some(pubkey.clone()),
        };
        let signed = signer.sign(template).await?;
        section_events.push(signed);
    }

    // Index event references each section by `a` tag.
    let pub_d_tag = compose.publication_d_tag();
    let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), pub_d_tag]];
    if !compose.title.is_empty() {
        // `title` = display; `T` = indexable title for search/discovery.
        tags.push(vec!["title".into(), compose.title.clone()]);
        tags.push(vec!["T".into(), ComposeState::generate_d_tag(&compose.title)]);
    }
    for tag_vec in ComposeState::tags_to_nostr_format(&compose.tags) {
        tags.push(tag_vec);
    }
    for i in 0..compose.sections.len() {
        let section_d_tag = compose.section_d_tag(i);
        let a_tag_value = format!("{}:{}:{}", KIND_PUBLICATION_SECTION, pubkey, section_d_tag);
        tags.push(vec!["a".into(), a_tag_value, "".into()]);
    }
    tags.push(vec!["auto-update".into(), compose.auto_update.as_str().into()]);

    let template = EventTemplate {
        kind: KIND_PUBLICATION_INDEX as u32,
        created_at: timestamp,
        tags,
        content: String::new(),
        pubkey: Some(pubkey.clone()),
    };
    let pub_event = signer.sign(template).await?;

    Ok((pub_event, section_events))
}

/// Take an event `Value` built by `tree_emit` (which has a placeholder
/// signature) and re-sign it through the external `Signer`. Used by the
/// nested-graph path of the signer-routed publish so the recursive
/// tag-emission logic lives in one place.
async fn resign_value_through_signer(
    ev: &Value,
    expected_pubkey: &str,
    created_at: i64,
    signer: &dyn crate::signing::Signer,
) -> std::result::Result<Value, crate::signing::SigningError> {
    use crate::signing::EventTemplate;

    // The Value built by tree_emit has kind/tags/content shaped exactly
    // for hashing. Convert to a template the signer can consume.
    let kind = ev.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let content = ev
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let tags: Vec<Vec<String>> = ev
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|tag| {
                    tag.as_array()
                        .map(|inner| {
                            inner
                                .iter()
                                .map(|v| v.as_str().unwrap_or("").to_string())
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();

    let template = EventTemplate {
        kind,
        created_at,
        tags,
        content,
        pubkey: Some(expected_pubkey.to_string()),
    };
    signer.sign(template).await
}

/// Default parse-level ceiling for nested publication emission. Sections
/// deeper than this collapse into their nearest ancestor's a-tag chain
/// (see `tree_emit::build_nested_publication_events`). The value is a
/// compromise: high enough to handle the common 3-level outline (book →
/// chapter → section), low enough that adversarial inputs don't explode
/// the event count.
const DEFAULT_PARSE_LEVEL: u8 = 6;

/// Internal function to build publication events with optional signing.
///
/// Branches on section depth:
/// - Any section with `level > 2` → recursive nested 30040/30041 graph
///   via [`tree_emit::build_nested_publication_events`].
/// - All sections flat (`level <= 2`) → the original single-30040 path,
///   unchanged.
fn build_publication_events_internal(
    compose: &mut ComposeState,
    pubkey: &str,
    secret_hex: Option<&str>,
) -> (Value, Vec<Value>) {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Nested-graph path: any section deeper than level 2 means the
    // publication is hierarchical and needs the recursive emitter.
    let is_nested = compose.sections.iter().any(|s| s.level > 2);
    if is_nested {
        let pub_d_tag = compose.publication_d_tag();
        return tree_emit::build_nested_publication_events(
            compose,
            &pub_d_tag,
            pubkey,
            timestamp,
            DEFAULT_PARSE_LEVEL,
            secret_hex,
        );
    }

    // Flat path (unchanged): build section events first (need their
    // d-tags for references), then the single 30040 index.
    let mut section_events = Vec::new();
    for i in 0..compose.sections.len() {
        let section_d_tag = compose.section_d_tag(i);
        let section_event = build_section_event_internal(&compose.sections[i], &section_d_tag, pubkey, timestamp, secret_hex);
        section_events.push(section_event);
    }

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
        // `title` = display; `T` = indexable title for search/discovery.
        tags.push(json!(["title", &section.title]));
        tags.push(json!([
            "T",
            crate::tree::state::ComposeState::generate_d_tag(&section.title)
        ]));
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
    compose: &mut ComposeState,
    pubkey: &str,
    timestamp: u64,
    secret_hex: Option<&str>,
) -> Value {
    use serde_json::json;

    let pub_d_tag = compose.publication_d_tag();

    // Build publication tags
    let mut tags: Vec<Value> = vec![json!(["d", &pub_d_tag])];

    if !compose.title.is_empty() {
        // `title` = display; `T` = indexable title for search/discovery.
        tags.push(json!(["title", &compose.title]));
        tags.push(json!([
            "T",
            crate::tree::state::ComposeState::generate_d_tag(&compose.title)
        ]));
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
        // `title` = display; `T` = indexable title for search/discovery.
        tags.push(json!(["title", &block.title]));
        tags.push(json!([
            "T",
            crate::tree::state::ComposeState::generate_d_tag(&block.title)
        ]));
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
        // `title` = display; `T` = indexable title for search/discovery.
        tags.push(json!(["title", &state.title]));
        tags.push(json!(["T", ComposeState::generate_d_tag(&state.title)]));
    }

    // Custom tags
    for tag_vec in ComposeState::tags_to_nostr_format(&state.tags) {
        tags.push(serde_json::to_value(tag_vec).unwrap_or(json!([])));
    }

    // Fork lineage (NIP-54): if this draft was seeded from an existing
    // 30040, emit `a` and `e` tags with `fork` marker pointing at the
    // source. This applies whenever any block is forked, the order
    // changed, or new blocks were added — the client decides whether to
    // populate `source_publication_addr` based on its own structural-
    // change check.
    if let Some(src) = &state.source_publication_addr {
        tags.push(json!([
            "a",
            format!("{}:{}:{}", KIND_PUBLICATION_INDEX, src.pubkey, src.d_tag),
            "",
            "fork"
        ]));
        if let Some(eid) = &state.source_publication_event_id {
            tags.push(json!(["e", eid, "", "fork"]));
        }
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
    use crate::tree::state::{ComposeBlockState, SectionCompose, TagEntry};

    fn d_of(ev: &Value) -> String {
        ev["tags"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|t| {
                let a = t.as_array()?;
                if a.first()?.as_str()? == "d" {
                    Some(a.get(1)?.as_str()?.to_string())
                } else {
                    None
                }
            })
            .expect("event missing d tag")
    }

    fn t_of(ev: &Value, key: &str) -> Option<String> {
        ev["tags"].as_array()?.iter().find_map(|t| {
            let a = t.as_array()?;
            if a.first()?.as_str()? == key {
                Some(a.get(1)?.as_str()?.to_string())
            } else {
                None
            }
        })
    }

    /// End-to-end integration: a multi-level ComposeState routed through
    /// the public `build_publication_events` entry-point should branch
    /// into the nested emitter and produce a tree of 30040 + 30041
    /// events with stable nanoid d-tags and consistent T/title tagging.
    #[test]
    fn build_publication_events_nested_path() {
        use crate::tree::state::ComposeState;

        // Outer (lvl 2) → Inner (lvl 3); presence of lvl 3 triggers the
        // nested branch in build_publication_events_internal.
        let mut compose = ComposeState {
            title: "Integration".to_string(),
            sections: vec![
                SectionCompose {
                    title: "Outer".to_string(),
                    content: "outer body".to_string(),
                    level: 2,
                    ..Default::default()
                },
                SectionCompose {
                    title: "Inner".to_string(),
                    content: "inner body".to_string(),
                    level: 3,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let pubkey = "feedface".repeat(8);

        let (root, children) = build_publication_events(&mut compose, &pubkey);

        // Root is a 30040 with empty content and matching T+title tags.
        assert_eq!(root["kind"], 30040);
        assert_eq!(root["content"], "");
        assert_eq!(t_of(&root, "T").as_deref(), Some("integration"));
        assert_eq!(t_of(&root, "title").as_deref(), Some("Integration"));

        // Three children: Outer-30040, Outer-30041, Inner-30041 (pre-order).
        assert_eq!(children.len(), 3);
        assert_eq!(children[0]["kind"], 30040);
        assert_eq!(children[0]["content"], "");
        assert_eq!(children[1]["kind"], 30041);
        assert_eq!(children[2]["kind"], 30041);

        // All d-tags are nanoid-shaped.
        let nanoid_shape = |s: &str| {
            s.len() == 21
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };
        assert!(nanoid_shape(&d_of(&root)));
        for ev in &children {
            assert!(nanoid_shape(&d_of(ev)));
        }

        // The Outer 30040 carries an `a` tag pointing at the Outer 30041
        // (own content), AND an `a` tag pointing at the Inner 30041.
        let outer_index_a_tags: Vec<String> = children[0]["tags"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| {
                let a = t.as_array()?;
                if a.first()?.as_str()? == "a" {
                    Some(a.get(1)?.as_str()?.to_string())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(outer_index_a_tags.len(), 2);
        // First a-tag is own content (30041 → outer's d).
        assert!(outer_index_a_tags[0].starts_with("30041:"));
        assert!(outer_index_a_tags[0].ends_with(&d_of(&children[1])));
        // Second a-tag is the inner leaf.
        assert!(outer_index_a_tags[1].starts_with("30041:"));
        assert!(outer_index_a_tags[1].ends_with(&d_of(&children[2])));
    }

    /// Flat path (no section level > 2) keeps the original
    /// single-30040 + N-30041 shape — the branch in
    /// build_publication_events_internal must not regress this.
    #[test]
    fn build_publication_events_flat_path_unchanged() {
        use crate::tree::state::ComposeState;

        let mut compose = ComposeState {
            title: "Flat".to_string(),
            sections: vec![
                SectionCompose {
                    title: "A".to_string(),
                    content: "a body".to_string(),
                    level: 2,
                    ..Default::default()
                },
                SectionCompose {
                    title: "B".to_string(),
                    content: "b body".to_string(),
                    level: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let pubkey = "feedface".repeat(8);

        let (root, children) = build_publication_events(&mut compose, &pubkey);

        assert_eq!(root["kind"], 30040);
        assert_eq!(children.len(), 2);
        for c in &children {
            assert_eq!(c["kind"], 30041);
        }
    }

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

    // --- from_event tests ---

    #[test]
    fn test_from_event_with_sections() {
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [
                ["d", "my-pub"],
                ["title", "My Publication"],
                ["a", "30041:deadbeef:section-1"],
                ["a", "30041:deadbeef:section-2"]
            ]
        });

        let pub_ = Publication::from_event(&event, true).unwrap();
        assert_eq!(pub_.title, Some("My Publication".to_string()));
        assert_eq!(pub_.sections.len(), 2);
        assert_eq!(pub_.nested.len(), 0);
        assert_eq!(pub_.section_count(), 2);
        assert_eq!(pub_.sections[0].addr.d_tag, "section-1");
        assert_eq!(pub_.sections[1].addr.d_tag, "section-2");
    }

    #[test]
    fn test_from_event_with_nested() {
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [
                ["d", "my-pub"],
                ["a", "30040:alice:chapter-1"],
                ["a", "30040:alice:chapter-2"],
                ["a", "30041:deadbeef:intro"]
            ]
        });

        let pub_ = Publication::from_event(&event, true).unwrap();
        assert_eq!(pub_.sections.len(), 1); // only 30041
        assert_eq!(pub_.nested.len(), 2);   // 30040 sub-pubs
        assert_eq!(pub_.section_count(), 3); // total children
    }

    #[test]
    fn test_from_event_detects_fork_marker_a_tag() {
        // NIP-54: an `a` tag with "fork" in the 4th slot marks lineage.
        // The fork-marker `a` MUST NOT be folded into the section list —
        // it doesn't reference content.
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [
                ["d", "my-pub"],
                ["a", "30040:alice:original-pub", "", "fork"],
                ["a", "30041:deadbeef:s1"]
            ]
        });
        let pub_ = Publication::from_event(&event, true).unwrap();
        assert!(pub_.forked, "fork-marker a tag must set forked=true");
        assert_eq!(pub_.sections.len(), 1, "fork a-tag must not become a section");
        assert_eq!(pub_.nested.len(), 0, "fork a-tag must not become a nested pub");
    }

    #[test]
    fn test_from_event_detects_fork_marker_e_tag() {
        // The `e` variant (event-id form) of the fork marker is the
        // companion to the `a` variant — either alone is sufficient.
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [
                ["d", "my-pub"],
                ["e", "feedfeed", "", "fork"]
            ]
        });
        let pub_ = Publication::from_event(&event, true).unwrap();
        assert!(pub_.forked, "fork-marker e tag must set forked=true");
    }

    #[test]
    fn test_from_event_no_fork_marker() {
        // A plain publication with sections only — forked must be false.
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [
                ["d", "my-pub"],
                ["a", "30041:deadbeef:s1"]
            ]
        });
        let pub_ = Publication::from_event(&event, true).unwrap();
        assert!(!pub_.forked);
    }

    #[test]
    fn test_from_event_with_content_accepted() {
        // Events with content should be accepted (many real events have it)
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "some content that should be ignored",
            "tags": [
                ["d", "my-pub"],
                ["a", "30041:deadbeef:section-1"]
            ]
        });

        let pub_ = Publication::from_event(&event, true).unwrap();
        assert_eq!(pub_.sections.len(), 1);
    }

    #[test]
    fn test_from_event_empty_no_sections() {
        let event = serde_json::json!({
            "id": "abc123",
            "pubkey": "deadbeef",
            "created_at": 1700000000u64,
            "kind": 30040,
            "content": "",
            "tags": [["d", "empty-pub"]]
        });

        let pub_ = Publication::from_event(&event, true).unwrap();
        assert_eq!(pub_.sections.len(), 0);
        assert_eq!(pub_.nested.len(), 0);
        assert_eq!(pub_.section_count(), 0);
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

    #[test]
    fn test_is_zettel_kind() {
        assert!(is_zettel_kind(KIND_PUBLICATION_SECTION));
        assert!(is_zettel_kind(30023));
        assert!(is_zettel_kind(30818));
        assert!(is_zettel_kind(30817));
        assert!(!is_zettel_kind(KIND_PUBLICATION_INDEX)); // index, not a leaf
        assert!(!is_zettel_kind(1));
    }

    #[test]
    fn test_from_event_buckets_zettel_kinds() {
        let pk = test_pubkey();
        let s_30041 = NAddr::new(KIND_PUBLICATION_SECTION, &pk, "sec-41");
        let s_30023 = NAddr::new(30023, &pk, "longform");
        let s_30818 = NAddr::new(30818, &pk, "wiki");
        let nested = NAddr::new(KIND_PUBLICATION_INDEX, &pk, "nested-idx");
        let junk = NAddr::new(1, &pk, "a-note");
        let index = fixture_index(
            &pk,
            "z-root",
            "Zettel Root",
            &[&s_30041, &s_30023, &s_30818, &nested, &junk],
        );
        let publication = Publication::from_event(&index, true).expect("from_event");
        // All three zettel kinds become content-leaf sections.
        let section_kinds: Vec<u64> =
            publication.sections.iter().map(|s| s.addr.kind).collect();
        assert_eq!(publication.sections.len(), 3, "kinds: {section_kinds:?}");
        assert!(section_kinds.contains(&KIND_PUBLICATION_SECTION));
        assert!(section_kinds.contains(&30023));
        assert!(section_kinds.contains(&30818));
        // The 30040 a-tag is a nested index, not a leaf.
        assert_eq!(publication.nested.len(), 1);
        assert_eq!(publication.nested[0].addr.kind, KIND_PUBLICATION_INDEX);
        // A plain note (kind 1) is neither — dropped.
        assert!(!section_kinds.contains(&1));
    }

    // --- load_publication_tree (recursive depth-N loader) ---

    /// Test signing key (shared with the engine's signed-publication tests).
    const TEST_SECRET: &str =
        "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";

    /// Derive the x-only pubkey hex for [`TEST_SECRET`].
    fn test_pubkey() -> String {
        use secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&hex::decode(TEST_SECRET).unwrap()).unwrap();
        let pk = PublicKey::from_secret_key(&secp, &sk);
        hex::encode(&pk.serialize()[1..33])
    }

    /// Build a fixture event with a correct NIP-01 id and a real Schnorr
    /// signature, so nostrdb accepts it on ingest.
    fn fixture_event(pubkey: &str, kind: u64, tags: Value, content: &str) -> Value {
        let created_at = 1_700_000_000u64;
        let id_array = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
        let id = calculate_event_id(&id_array);
        let sig = crate::identity::sign_event_hash(&id, TEST_SECRET).expect("sign fixture");
        serde_json::json!({
            "id": id,
            "pubkey": pubkey,
            "created_at": created_at,
            "kind": kind,
            "tags": tags,
            "content": content,
            "sig": sig,
        })
    }

    fn fixture_index(pubkey: &str, d_tag: &str, title: &str, children: &[&NAddr]) -> Value {
        let mut tags = vec![
            serde_json::json!(["d", d_tag]),
            serde_json::json!(["title", title]),
        ];
        for c in children {
            tags.push(serde_json::json!(["a", c.to_a_tag()]));
        }
        fixture_event(pubkey, KIND_PUBLICATION_INDEX, Value::Array(tags), "")
    }

    fn fixture_section(pubkey: &str, d_tag: &str, title: &str, content: &str) -> Value {
        let tags = vec![
            serde_json::json!(["d", d_tag]),
            serde_json::json!(["title", title]),
        ];
        fixture_event(pubkey, KIND_PUBLICATION_SECTION, Value::Array(tags), content)
    }

    /// Spin up an in-memory engine, ingest the events, and poll until every
    /// ingested addressable event is locally queryable (nostrdb processes
    /// ingests asynchronously). The returned `TempDir` must be kept alive for
    /// as long as the engine.
    async fn engine_with_events(events: &[Value]) -> (Engine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();
        for ev in events {
            engine
                .ingest_event(&serde_json::to_string(ev).unwrap())
                .expect("ingest");
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let mut all_visible = true;
            for ev in events {
                let kind = ev["kind"].as_u64().unwrap();
                let pubkey = ev["pubkey"].as_str().unwrap();
                let d_tag = ev["tags"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find_map(|t| {
                        let t = t.as_array()?;
                        (t.first()?.as_str()? == "d").then(|| t.get(1)?.as_str())?
                    })
                    .expect("event has a d tag");
                let found = engine
                    .get_addressable(kind, pubkey, d_tag, FetchPolicy::LocalOnly)
                    .await
                    .unwrap()
                    .is_some();
                if !found {
                    all_visible = false;
                    break;
                }
            }
            if all_visible {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "ingested events did not become queryable within timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        (engine, dir)
    }

    #[tokio::test]
    async fn test_load_publication_tree_depth() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let root_sec = NAddr::new(KIND_PUBLICATION_SECTION, pk, "root-sec");
        let nested_sec = NAddr::new(KIND_PUBLICATION_SECTION, pk, "nested-sec");
        let nested_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "nested-idx");
        let root_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "root-idx");

        let (engine, _dir) = engine_with_events(&[
            fixture_section(pk, "root-sec", "Root Section", "root body"),
            fixture_section(pk, "nested-sec", "Nested Section", "nested body"),
            fixture_index(pk, "nested-idx", "Nested", &[&nested_sec]),
            fixture_index(pk, "root-idx", "Root", &[&root_sec, &nested_idx]),
        ])
        .await;
        let pe = PublicationEngine::new(&engine);

        // depth 1: root sections + one level of nested indexes resolved.
        let tree = pe
            .load_publication_tree(&root_idx, 1, FetchPolicy::LocalOnly)
            .await
            .expect("load depth 1");
        assert!(tree.is_root);
        assert_eq!(tree.sections.len(), 1);
        assert!(tree.sections[0].event.is_loaded());
        assert_eq!(tree.nested.len(), 1);
        assert!(
            tree.nested[0].index.is_loaded(),
            "nested index resolved at depth 1"
        );
        assert!(!tree.nested[0].is_root);
        assert_eq!(tree.nested[0].sections.len(), 1);
        assert!(tree.nested[0].sections[0].event.is_loaded());

        // depth 0: the nested index stays a Pending stub.
        let tree0 = pe
            .load_publication_tree(&root_idx, 0, FetchPolicy::LocalOnly)
            .await
            .expect("load depth 0");
        assert!(tree0.sections[0].event.is_loaded());
        assert_eq!(tree0.nested.len(), 1);
        assert!(
            !tree0.nested[0].index.is_loaded(),
            "nested index NOT resolved at depth 0"
        );
        assert!(tree0.nested[0].sections.is_empty());
    }

    #[tokio::test]
    async fn test_load_publication_tree_cycle_safe() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let a = NAddr::new(KIND_PUBLICATION_INDEX, pk, "cyc-a");
        let b = NAddr::new(KIND_PUBLICATION_INDEX, pk, "cyc-b");

        // A references B; B references A back — a cycle.
        let (engine, _dir) = engine_with_events(&[
            fixture_index(pk, "cyc-a", "A", &[&b]),
            fixture_index(pk, "cyc-b", "B", &[&a]),
        ])
        .await;
        let pe = PublicationEngine::new(&engine);

        // A deliberately large depth must still terminate.
        let tree = pe
            .load_publication_tree(&a, 100, FetchPolicy::LocalOnly)
            .await
            .expect("cyclic load terminates");

        assert_eq!(tree.addr, a);
        assert_eq!(tree.nested.len(), 1);
        let b_node = &tree.nested[0];
        assert_eq!(b_node.addr, b);
        assert!(b_node.index.is_loaded(), "B resolved under A");

        // B references A, but A is an ancestor — cycle guard leaves it Pending.
        assert_eq!(b_node.nested.len(), 1);
        assert_eq!(b_node.nested[0].addr, a);
        assert!(
            !b_node.nested[0].index.is_loaded(),
            "ancestor A not re-recursed (cycle guard)"
        );
    }

    // --- stream_publication_tree (streaming loader) ---

    /// Drain a streaming load into a vec of events (test helper).
    async fn stream_to_vec(
        engine: &Engine,
        addr: &NAddr,
        max_depth: usize,
    ) -> (usize, Vec<PubLoadEvent>) {
        let pe = PublicationEngine::new(engine);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<PubLoadEvent>(256);
        let loader = pe.stream_publication_tree(addr, max_depth, FetchPolicy::LocalOnly, tx);
        let drain = async {
            let mut events = Vec::new();
            while let Some(ev) = rx.recv().await {
                events.push(ev);
            }
            events
        };
        tokio::join!(loader, drain)
    }

    #[tokio::test]
    async fn test_stream_tree_emits_per_node() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let root_sec = NAddr::new(KIND_PUBLICATION_SECTION, pk, "root-sec");
        let nested_sec = NAddr::new(KIND_PUBLICATION_SECTION, pk, "nested-sec");
        let nested_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "nested-idx");
        let root_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "root-idx");

        let (engine, _dir) = engine_with_events(&[
            fixture_section(pk, "root-sec", "Root Section", "root body"),
            fixture_section(pk, "nested-sec", "Nested Section", "nested body"),
            fixture_index(pk, "nested-idx", "Nested", &[&nested_sec]),
            fixture_index(pk, "root-idx", "Root", &[&root_sec, &nested_idx]),
        ])
        .await;

        let (total, events) = stream_to_vec(&engine, &root_idx, 1).await;

        let indexes = events
            .iter()
            .filter(|e| matches!(e, PubLoadEvent::Index { .. }))
            .count();
        let leaves = events
            .iter()
            .filter(|e| matches!(e, PubLoadEvent::Leaf { .. }))
            .count();
        let errors = events
            .iter()
            .filter(|e| matches!(e, PubLoadEvent::Error { .. }))
            .count();
        assert_eq!(indexes, 2, "one Index per 30040");
        assert_eq!(leaves, 2, "one Leaf per resolved section");
        assert_eq!(errors, 0);

        // Exactly one Done, last, carrying the authoritative in-horizon total.
        match events.last() {
            Some(PubLoadEvent::Done { total: t }) => {
                assert_eq!(*t, total);
                assert_eq!(*t, 4, "2 indexes + 2 leaves");
            }
            other => panic!("expected Done last, got {other:?}"),
        }

        // The root index is emitted first, before any nested events.
        match events.first() {
            Some(PubLoadEvent::Index { is_root, .. }) => assert!(*is_root),
            other => panic!("expected root Index first, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_stream_tree_in_horizon_flag() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let nested_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "nested-idx");
        let root_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "root-idx");

        let (engine, _dir) = engine_with_events(&[
            fixture_index(pk, "nested-idx", "Nested", &[]),
            fixture_index(pk, "root-idx", "Root", &[&nested_idx]),
        ])
        .await;

        // depth 0: the nested index is beyond the horizon.
        let (_total, events) = stream_to_vec(&engine, &root_idx, 0).await;

        let children = events
            .iter()
            .find_map(|e| match e {
                PubLoadEvent::Index {
                    is_root: true,
                    children,
                    ..
                } => Some(children),
                _ => None,
            })
            .expect("root Index present");
        assert_eq!(children.len(), 1);
        assert!(children[0].is_index);
        assert!(
            !children[0].in_horizon,
            "nested index beyond depth 0 is a frontier stub"
        );
        // No Index event was streamed for the beyond-horizon nested index.
        let nested_indexes = events
            .iter()
            .filter(|e| matches!(e, PubLoadEvent::Index { is_root: false, .. }))
            .count();
        assert_eq!(nested_indexes, 0);
    }

    #[tokio::test]
    async fn test_stream_tree_zettel_leaf() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let longform = NAddr::new(30023, pk, "lf");
        let root_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "z-idx");

        let longform_event = fixture_event(
            pk,
            30023,
            serde_json::json!([["d", "lf"], ["title", "Longform"]]),
            "longform body",
        );
        let (engine, _dir) = engine_with_events(&[
            longform_event,
            fixture_index(pk, "z-idx", "Zettel Index", &[&longform]),
        ])
        .await;

        let (_total, events) = stream_to_vec(&engine, &root_idx, 0).await;

        let (addr, content) = events
            .iter()
            .find_map(|e| match e {
                PubLoadEvent::Leaf { addr, content, .. } => Some((addr, content)),
                _ => None,
            })
            .expect("a 30023 leaf was streamed");
        assert_eq!(addr.kind, 30023);
        assert_eq!(content.as_deref(), Some("longform body"));
    }

    #[tokio::test]
    async fn test_stream_tree_cancellation() {
        let pk_owned = test_pubkey();
        let pk = pk_owned.as_str();
        let sec = NAddr::new(KIND_PUBLICATION_SECTION, pk, "sec");
        let root_idx = NAddr::new(KIND_PUBLICATION_INDEX, pk, "root-idx");

        let (engine, _dir) = engine_with_events(&[
            fixture_section(pk, "sec", "Section", "body"),
            fixture_index(pk, "root-idx", "Root", &[&sec]),
        ])
        .await;
        let pe = PublicationEngine::new(&engine);

        // Receiver dropped before the loader starts — every send fails, so the
        // recursion unwinds at once. This test completing proves termination.
        let (tx, rx) = tokio::sync::mpsc::channel::<PubLoadEvent>(256);
        drop(rx);
        let total = pe
            .stream_publication_tree(&root_idx, 5, FetchPolicy::LocalOnly, tx)
            .await;
        assert!(total <= 2, "aborted load emits almost nothing, got {total}");
    }
}
