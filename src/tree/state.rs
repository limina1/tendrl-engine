//! Tree state management
//!
//! Contains the pure state types for the tree view, including cursor position,
//! expanded nodes, selections, and view configuration.

use super::node::{NodeId, TreeNode};
use super::undo::UndoStack;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Application mode - determines the overall UI layout
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AppMode {
    /// Feed mode: browsing list of publications (like notedeck's timeline)
    #[default]
    Feed,
    /// Reader mode: viewing a specific publication with tree navigation
    Reader,
}

impl AppMode {
    /// Get the display name for this mode
    pub fn name(&self) -> &'static str {
        match self {
            AppMode::Feed => "Feed",
            AppMode::Reader => "Reader",
        }
    }
}

/// The complete state of the tree view
#[derive(Debug, Clone)]
pub struct TreeState {
    /// Current application mode (Feed or Reader)
    pub mode: AppMode,
    /// All nodes indexed by ID
    pub nodes: HashMap<NodeId, TreeNode>,
    /// Root node IDs (publications not nested in others)
    pub roots: Vec<NodeId>,
    /// Currently focused node (cursor position in current view)
    pub cursor: NodeId,
    /// Feed cursor - separate from tree cursor, tracks position in feed list
    pub feed_cursor: usize,
    /// Currently selected publication for Reader mode (if any)
    pub selected_publication: Option<NodeId>,
    /// Expanded nodes (publications with visible children)
    pub expanded: HashSet<NodeId>,
    /// Multi-selected nodes
    pub selected: HashSet<NodeId>,
    /// Clipboard content for copy/paste
    pub clipboard: Option<ClipboardContent>,
    /// Undo/redo stack
    pub undo_stack: UndoStack,
    /// View configuration
    pub view: ViewState,
    /// Counter for generating temporary node IDs
    next_temp_id: u64,
    /// Oldest publication timestamp (for pagination)
    pub oldest_timestamp: Option<u64>,
    /// Whether we're currently loading more publications
    pub loading_more: bool,
    /// Whether we've reached the end (no more publications to load)
    pub feed_exhausted: bool,
}

impl TreeState {
    /// Create an empty tree state
    pub fn new() -> Self {
        TreeState {
            mode: AppMode::default(),
            nodes: HashMap::new(),
            roots: Vec::new(),
            cursor: NodeId::root(),
            feed_cursor: 0,
            selected_publication: None,
            expanded: HashSet::new(),
            selected: HashSet::new(),
            clipboard: None,
            undo_stack: UndoStack::new(),
            view: ViewState::default(),
            next_temp_id: 0x8000_0000_0000_0000, // Start high to avoid collisions
            oldest_timestamp: None,
            loading_more: false,
            feed_exhausted: false,
        }
    }

    /// Enter reader mode for a specific publication
    pub fn enter_reader(&mut self, pub_id: NodeId) {
        self.mode = AppMode::Reader;
        self.selected_publication = Some(pub_id);
        self.cursor = pub_id;
        // Reset view state for new publication
        self.view.current_section = 0;
        self.view.content_scroll = 0;
        self.view.preview_scroll = 0;
    }

    /// Return to feed mode
    pub fn exit_reader(&mut self) {
        self.mode = AppMode::Feed;
        // Keep selected_publication for potential re-entry
    }

    /// Check if in feed mode
    pub fn is_feed_mode(&self) -> bool {
        self.mode == AppMode::Feed
    }

    /// Check if in reader mode
    pub fn is_reader_mode(&self) -> bool {
        self.mode == AppMode::Reader
    }

    /// Get the current publication being viewed (if in reader mode)
    pub fn current_publication(&self) -> Option<NodeId> {
        if self.mode == AppMode::Reader {
            self.selected_publication
        } else {
            None
        }
    }

    /// Generate a temporary node ID (for nodes not yet loaded)
    pub fn next_temp_id(&mut self) -> NodeId {
        let id = NodeId(self.next_temp_id);
        self.next_temp_id += 1;
        id
    }

    /// Add a node to the tree
    pub fn add_node(&mut self, node: TreeNode) {
        let id = node.id();
        self.nodes.insert(id, node);
    }

    /// Get a node by ID
    pub fn get_node(&self, id: NodeId) -> Option<&TreeNode> {
        self.nodes.get(&id)
    }

    /// Get a mutable node by ID
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut TreeNode> {
        self.nodes.get_mut(&id)
    }

    /// Get the current cursor node
    pub fn cursor_node(&self) -> Option<&TreeNode> {
        self.nodes.get(&self.cursor)
    }

    /// Check if a node is expanded
    pub fn is_expanded(&self, id: NodeId) -> bool {
        self.expanded.contains(&id)
    }

    /// Toggle expansion of a node
    pub fn toggle_expanded(&mut self, id: NodeId) {
        if self.expanded.contains(&id) {
            self.expanded.remove(&id);
        } else {
            self.expanded.insert(id);
        }
    }

    /// Expand a node
    pub fn expand(&mut self, id: NodeId) {
        self.expanded.insert(id);
    }

    /// Collapse a node
    pub fn collapse(&mut self, id: NodeId) {
        self.expanded.remove(&id);
    }

    /// Check if a node is selected
    pub fn is_selected(&self, id: NodeId) -> bool {
        self.selected.contains(&id)
    }

    /// Toggle selection of a node
    pub fn toggle_selected(&mut self, id: NodeId) {
        if self.selected.contains(&id) {
            self.selected.remove(&id);
        } else {
            self.selected.insert(id);
        }
    }

    /// Clear all selections
    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Get the parent of a node
    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        self.nodes.get(&id).and_then(|n| n.parent())
    }

    /// Get siblings of a node (including the node itself)
    pub fn siblings_of(&self, id: NodeId) -> Vec<NodeId> {
        if let Some(parent_id) = self.parent_of(id) {
            if let Some(parent) = self.nodes.get(&parent_id) {
                return parent.children().to_vec();
            }
        }
        // If no parent, check if this is a root
        if self.roots.contains(&id) {
            return self.roots.clone();
        }
        vec![id]
    }

    /// Get the position of a node among its siblings
    pub fn position_in_siblings(&self, id: NodeId) -> Option<usize> {
        let siblings = self.siblings_of(id);
        siblings.iter().position(|&s| s == id)
    }

    /// Get the depth of a node in the tree
    pub fn depth_of(&self, id: NodeId) -> usize {
        let mut depth = 0;
        let mut current = id;
        while let Some(parent) = self.parent_of(current) {
            depth += 1;
            current = parent;
        }
        depth
    }

    /// Get all visible nodes in display order (respecting expanded state)
    pub fn visible_nodes(&self) -> Vec<(NodeId, usize)> {
        let mut result = Vec::new();
        for &root_id in &self.roots {
            self.collect_visible(root_id, 0, &mut result);
        }
        result
    }

    fn collect_visible(&self, id: NodeId, depth: usize, result: &mut Vec<(NodeId, usize)>) {
        result.push((id, depth));

        if let Some(node) = self.nodes.get(&id) {
            if node.has_children() && self.is_expanded(id) {
                for &child_id in node.children() {
                    self.collect_visible(child_id, depth + 1, result);
                }
            }
        }
    }

    /// Find the index of a node in the visible list
    pub fn visible_index(&self, id: NodeId) -> Option<usize> {
        self.visible_nodes().iter().position(|(nid, _)| *nid == id)
    }

    /// Get visible node at a given index
    pub fn visible_at(&self, index: usize) -> Option<NodeId> {
        self.visible_nodes().get(index).map(|(id, _)| *id)
    }

    /// Total count of visible nodes
    pub fn visible_count(&self) -> usize {
        self.visible_nodes().len()
    }
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

/// Reader/view mode for displaying publications
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewMode {
    /// Tree view with expand/collapse (current implementation)
    #[default]
    Tree,
    /// Outline view - shows current node + children as cards (notedeck-style)
    Outline,
    /// Continuous scroll through all sections
    Continuous,
    /// Paginated view - one section at a time
    Paginated,
}

impl ViewMode {
    /// Get the display name for this mode
    pub fn name(&self) -> &'static str {
        match self {
            ViewMode::Tree => "Tree",
            ViewMode::Outline => "Outline",
            ViewMode::Continuous => "Continuous",
            ViewMode::Paginated => "Paginated",
        }
    }

    /// Cycle to the next view mode
    pub fn next(&self) -> Self {
        match self {
            ViewMode::Tree => ViewMode::Outline,
            ViewMode::Outline => ViewMode::Continuous,
            ViewMode::Continuous => ViewMode::Paginated,
            ViewMode::Paginated => ViewMode::Tree,
        }
    }
}

/// View configuration state
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewState {
    /// Current view mode
    pub mode: ViewMode,
    /// Show content preview panel (for Tree mode)
    pub show_preview: bool,
    /// Preview panel width (percentage, 0-100)
    pub preview_width: u16,
    /// Scroll offset in the tree view
    pub tree_scroll: usize,
    /// Scroll offset in the preview
    pub preview_scroll: usize,
    /// Current section index for paginated mode
    pub current_section: usize,
    /// Content scroll offset for continuous mode
    pub content_scroll: usize,
    /// Search query if active
    pub search_query: Option<String>,
    /// Filter mode
    pub filter: FilterMode,
}

impl ViewState {
    pub fn new() -> Self {
        ViewState {
            mode: ViewMode::default(),
            show_preview: true,
            preview_width: 50,
            tree_scroll: 0,
            preview_scroll: 0,
            current_section: 0,
            content_scroll: 0,
            search_query: None,
            filter: FilterMode::None,
        }
    }

    /// Toggle preview visibility
    pub fn toggle_preview(&mut self) {
        self.show_preview = !self.show_preview;
    }
}

/// Filter mode for the tree view
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    /// Show all nodes
    #[default]
    None,
    /// Show only loaded nodes
    Loaded,
    /// Show only nodes matching search
    Search,
    /// Show only selected nodes
    Selected,
}

/// Clipboard content for copy/paste operations
#[derive(Debug, Clone)]
pub enum ClipboardContent {
    /// A single node reference
    Single(NodeId),
    /// Multiple node references
    Multiple(Vec<NodeId>),
    /// Copied section content (can be pasted into other publications)
    SectionData {
        title: Option<String>,
        content: String,
        addr: crate::publication::NAddr,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_state_visibility() {
        let mut state = TreeState::new();

        // Add a simple tree structure
        let root_id = NodeId(1);
        let child1_id = NodeId(2);
        let child2_id = NodeId(3);

        // Create nodes (simplified for testing)
        use super::super::node::{PublicationNode, SectionNode};
        use crate::publication::NAddr;

        let mut pub_node = PublicationNode::stub(
            NAddr::new(30040, "pub", "root"),
            None,
        );
        pub_node.id = root_id;
        pub_node.children = vec![child1_id, child2_id];

        let mut sec1 = SectionNode::stub(
            NAddr::new(30041, "pub", "sec1"),
            root_id,
            0,
        );
        sec1.id = child1_id;

        let mut sec2 = SectionNode::stub(
            NAddr::new(30041, "pub", "sec2"),
            root_id,
            1,
        );
        sec2.id = child2_id;

        state.add_node(TreeNode::Publication(pub_node));
        state.add_node(TreeNode::Section(sec1));
        state.add_node(TreeNode::Section(sec2));
        state.roots.push(root_id);
        state.cursor = root_id;

        // Initially collapsed, only root visible
        assert_eq!(state.visible_count(), 1);

        // Expand root
        state.expand(root_id);
        assert_eq!(state.visible_count(), 3);

        // Check order
        let visible = state.visible_nodes();
        assert_eq!(visible[0], (root_id, 0));
        assert_eq!(visible[1], (child1_id, 1));
        assert_eq!(visible[2], (child2_id, 1));
    }
}
