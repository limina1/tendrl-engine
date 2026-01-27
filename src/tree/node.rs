//! Tree node types for NKBIP-01 publications
//!
//! Provides the core node types that represent publications and sections
//! in a navigable tree structure.

use crate::publication::{NAddr, Publication, Section};
use serde::{Deserialize, Serialize};

/// Unique identifier for tree nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl NodeId {
    /// Generate a new NodeId from an address
    pub fn from_addr(addr: &NAddr) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        addr.to_a_tag().hash(&mut hasher);
        NodeId(hasher.finish())
    }

    /// Generate a root node ID
    pub fn root() -> Self {
        NodeId(0)
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "node:{:016x}", self.0)
    }
}

/// Content format mode for sections
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ContentMode {
    #[default]
    Markdown,
    OrgMode,
    AsciiDoc,
    PlainText,
}

impl ContentMode {
    /// Detect content mode from event tags
    pub fn from_tags(tags: &[Vec<String>]) -> Self {
        for tag in tags {
            if tag.len() >= 2 && tag[0] == "format" {
                return match tag[1].to_lowercase().as_str() {
                    "org" | "org-mode" | "orgmode" => ContentMode::OrgMode,
                    "asciidoc" | "adoc" => ContentMode::AsciiDoc,
                    "plain" | "text" | "txt" => ContentMode::PlainText,
                    _ => ContentMode::Markdown,
                };
            }
        }
        ContentMode::Markdown
    }

    /// File extension for this content mode
    pub fn extension(&self) -> &'static str {
        match self {
            ContentMode::Markdown => "md",
            ContentMode::OrgMode => "org",
            ContentMode::AsciiDoc => "adoc",
            ContentMode::PlainText => "txt",
        }
    }

    /// Display name for this content mode
    pub fn name(&self) -> &'static str {
        match self {
            ContentMode::Markdown => "Markdown",
            ContentMode::OrgMode => "Org Mode",
            ContentMode::AsciiDoc => "AsciiDoc",
            ContentMode::PlainText => "Plain Text",
        }
    }
}

/// Sync status indicating whether an event exists on relays
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SyncStatus {
    /// Event was fetched from a relay (confirmed on network)
    #[default]
    Remote,
    /// Event only exists locally (not yet sent to relays or unconfirmed)
    LocalOnly,
    /// Unsigned local draft (not yet signed or published)
    Draft,
}

impl SyncStatus {
    /// Check if this is a draft (unsigned)
    pub fn is_draft(&self) -> bool {
        matches!(self, SyncStatus::Draft)
    }
}

/// A node in the publication tree
#[derive(Debug, Clone)]
pub enum TreeNode {
    /// A publication (kind 30040) - can contain sections and nested publications
    Publication(PublicationNode),
    /// A section (kind 30041) - leaf content
    Section(SectionNode),
}

impl TreeNode {
    /// Get the node's unique identifier
    pub fn id(&self) -> NodeId {
        match self {
            TreeNode::Publication(p) => p.id,
            TreeNode::Section(s) => s.id,
        }
    }

    /// Get the node's address
    pub fn addr(&self) -> &NAddr {
        match self {
            TreeNode::Publication(p) => &p.addr,
            TreeNode::Section(s) => &s.addr,
        }
    }

    /// Get the node's display title
    ///
    /// For loaded nodes, returns the title. For unloaded nodes, returns
    /// a shortened address format: "kind:abcd...wxyz:d-tag"
    pub fn title(&self) -> String {
        match self {
            TreeNode::Publication(p) => p
                .title
                .clone()
                .unwrap_or_else(|| p.addr.short_format()),
            TreeNode::Section(s) => s.title.clone().unwrap_or_else(|| s.addr.short_format()),
        }
    }

    /// Get the parent node ID
    pub fn parent(&self) -> Option<NodeId> {
        match self {
            TreeNode::Publication(p) => p.parent,
            TreeNode::Section(s) => s.parent,
        }
    }

    /// Check if this node has children
    pub fn has_children(&self) -> bool {
        match self {
            TreeNode::Publication(p) => !p.children.is_empty(),
            TreeNode::Section(_) => false,
        }
    }

    /// Get child node IDs
    pub fn children(&self) -> &[NodeId] {
        match self {
            TreeNode::Publication(p) => &p.children,
            TreeNode::Section(_) => &[],
        }
    }

    /// Check if this is a publication node
    pub fn is_publication(&self) -> bool {
        matches!(self, TreeNode::Publication(_))
    }

    /// Check if this is a section node
    pub fn is_section(&self) -> bool {
        matches!(self, TreeNode::Section(_))
    }

    /// Check if content is loaded
    pub fn is_loaded(&self) -> bool {
        match self {
            TreeNode::Publication(p) => p.loaded,
            TreeNode::Section(s) => s.loaded,
        }
    }

    /// Check if this node is currently loading
    pub fn is_loading(&self) -> bool {
        match self {
            TreeNode::Publication(p) => p.loading,
            TreeNode::Section(s) => s.loading,
        }
    }

    /// Get the sync status (Remote = fetched from relay, LocalOnly = local db only)
    pub fn sync_status(&self) -> SyncStatus {
        match self {
            TreeNode::Publication(p) => p.sync_status,
            TreeNode::Section(s) => s.sync_status,
        }
    }
}

/// A publication node in the tree
#[derive(Debug, Clone)]
pub struct PublicationNode {
    /// Unique node identifier
    pub id: NodeId,
    /// The publication's address
    pub addr: NAddr,
    /// Parent node (None for root publications)
    pub parent: Option<NodeId>,
    /// Child nodes (sections and nested publications)
    pub children: Vec<NodeId>,
    /// Display title
    pub title: Option<String>,
    /// Summary/description
    pub summary: Option<String>,
    /// Author pubkey
    pub author: String,
    /// Author display name (if resolved)
    pub author_name: Option<String>,
    /// Version tag
    pub version: Option<String>,
    /// Creation timestamp
    pub created_at: u64,
    /// Whether the publication index is loaded
    pub loaded: bool,
    /// Whether currently loading
    pub loading: bool,
    /// Load error if any
    pub error: Option<String>,
    /// Sync status - whether event was fetched from relay or is local-only
    pub sync_status: SyncStatus,
    /// Draft ID if this is an unsigned draft
    pub draft_id: Option<String>,
}

impl PublicationNode {
    /// Check if this publication has children
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

impl PublicationNode {
    /// Create from a Publication
    pub fn from_publication(pub_: &Publication, parent: Option<NodeId>) -> Self {
        let id = NodeId::from_addr(&pub_.addr);
        PublicationNode {
            id,
            addr: pub_.addr.clone(),
            parent,
            children: Vec::new(), // Will be populated separately
            title: pub_.title.clone(),
            summary: pub_.summary.clone(),
            author: pub_.author_pubkey.clone(),
            author_name: pub_.author_name.clone(),
            version: pub_.version.clone(),
            created_at: pub_.created_at,
            loaded: pub_.index.is_loaded(),
            loading: false,
            error: None,
            sync_status: SyncStatus::Remote, // Fetched from relay
            draft_id: None,
        }
    }

    /// Create a stub node from an address (not yet loaded)
    pub fn stub(addr: NAddr, parent: Option<NodeId>) -> Self {
        let id = NodeId::from_addr(&addr);
        PublicationNode {
            id,
            addr,
            parent,
            children: Vec::new(),
            title: None,
            summary: None,
            author: String::new(),
            author_name: None,
            version: None,
            created_at: 0,
            loaded: false,
            loading: false,
            error: None,
            sync_status: SyncStatus::default(),
            draft_id: None,
        }
    }
}

/// A section node in the tree
#[derive(Debug, Clone)]
pub struct SectionNode {
    /// Unique node identifier
    pub id: NodeId,
    /// The section's address
    pub addr: NAddr,
    /// Parent publication node
    pub parent: Option<NodeId>,
    /// Display title
    pub title: Option<String>,
    /// Section content
    pub content: Option<String>,
    /// Content format mode
    pub content_mode: ContentMode,
    /// Position within parent
    pub position: usize,
    /// Whether the section is loaded
    pub loaded: bool,
    /// Whether currently loading
    pub loading: bool,
    /// Load error if any
    pub error: Option<String>,
    /// Number of alternate versions available
    pub alternate_count: usize,
    /// Sync status - whether event was fetched from relay or is local-only
    pub sync_status: SyncStatus,
    /// Draft ID if this is an unsigned draft section
    pub draft_id: Option<String>,
}

impl SectionNode {
    /// Create from a Section
    pub fn from_section(section: &Section, parent: NodeId) -> Self {
        let id = NodeId::from_addr(&section.addr);
        SectionNode {
            id,
            addr: section.addr.clone(),
            parent: Some(parent),
            title: section.title.clone(),
            content: section.content.clone(),
            content_mode: ContentMode::default(), // Will be detected from tags
            position: section.position,
            loaded: section.event.is_loaded(),
            loading: false,
            error: None,
            alternate_count: section.alternates.len(),
            sync_status: SyncStatus::Remote, // Fetched from relay
            draft_id: None,
        }
    }

    /// Create a stub node from an address (not yet loaded)
    pub fn stub(addr: NAddr, parent: NodeId, position: usize) -> Self {
        let id = NodeId::from_addr(&addr);
        SectionNode {
            id,
            addr,
            parent: Some(parent),
            title: None,
            content: None,
            content_mode: ContentMode::default(),
            position,
            loaded: false,
            loading: false,
            error: None,
            alternate_count: 0,
            sync_status: SyncStatus::default(),
            draft_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id_from_addr() {
        let addr1 = NAddr::new(30040, "pubkey1", "doc1");
        let addr2 = NAddr::new(30040, "pubkey1", "doc1");
        let addr3 = NAddr::new(30040, "pubkey1", "doc2");

        // Same address should produce same ID
        assert_eq!(NodeId::from_addr(&addr1), NodeId::from_addr(&addr2));
        // Different address should produce different ID
        assert_ne!(NodeId::from_addr(&addr1), NodeId::from_addr(&addr3));
    }

    #[test]
    fn test_content_mode_from_tags() {
        let org_tags = vec![vec!["format".to_string(), "org".to_string()]];
        assert_eq!(ContentMode::from_tags(&org_tags), ContentMode::OrgMode);

        let md_tags = vec![vec!["format".to_string(), "markdown".to_string()]];
        assert_eq!(ContentMode::from_tags(&md_tags), ContentMode::Markdown);

        let empty_tags: Vec<Vec<String>> = vec![];
        assert_eq!(ContentMode::from_tags(&empty_tags), ContentMode::Markdown);
    }
}
