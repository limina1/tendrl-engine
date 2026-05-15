//! Tree rendering abstraction
//!
//! Provides a trait for rendering the tree state to different backends
//! (HTML, plain text, etc.) and utilities for computing visible nodes.

use super::node::{NodeId, SyncStatus, TreeNode};
use super::state::TreeState;

/// A node prepared for rendering with display metadata
#[derive(Debug, Clone)]
pub struct VisibleNode {
    /// The node ID
    pub id: NodeId,
    /// Nesting depth (0 = root level)
    pub depth: usize,
    /// Display title
    pub title: String,
    /// Whether this node has children
    pub has_children: bool,
    /// Whether this node is expanded
    pub is_expanded: bool,
    /// Whether this node is the cursor
    pub is_cursor: bool,
    /// Whether this node is selected
    pub is_selected: bool,
    /// Whether this node is loading
    pub is_loading: bool,
    /// Whether this node is loaded
    pub is_loaded: bool,
    /// Whether this is a publication (vs section)
    pub is_publication: bool,
    /// Error message if load failed
    pub error: Option<String>,
    /// Number of alternate versions (for sections)
    pub alternate_count: usize,
    /// Sync status (Remote = from relay, LocalOnly = local db only)
    pub sync_status: SyncStatus,
}

impl VisibleNode {
    /// Create from a tree node with state context
    pub fn from_node(node: &TreeNode, state: &TreeState, depth: usize) -> Self {
        let id = node.id();
        VisibleNode {
            id,
            depth,
            title: node.title().to_string(),
            has_children: node.has_children(),
            is_expanded: state.is_expanded(id),
            is_cursor: state.cursor == id,
            is_selected: state.is_selected(id),
            is_loading: node.is_loading(),
            is_loaded: node.is_loaded(),
            is_publication: node.is_publication(),
            error: match node {
                TreeNode::Publication(p) => p.error.clone(),
                TreeNode::Section(s) => s.error.clone(),
            },
            alternate_count: match node {
                TreeNode::Section(s) => s.alternate_count,
                _ => 0,
            },
            sync_status: node.sync_status(),
        }
    }
}

/// Options for rendering
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// Maximum depth to render (None = unlimited)
    pub max_depth: Option<usize>,
    /// Show loading indicators
    pub show_loading: bool,
    /// Show error indicators
    pub show_errors: bool,
    /// Indent string per depth level
    pub indent: String,
    /// Collapse indicator
    pub collapsed_indicator: String,
    /// Expand indicator
    pub expanded_indicator: String,
    /// Leaf indicator
    pub leaf_indicator: String,
}

impl RenderOptions {
    pub fn new() -> Self {
        RenderOptions {
            max_depth: None,
            show_loading: true,
            show_errors: true,
            indent: "  ".to_string(),
            collapsed_indicator: "▸".to_string(),
            expanded_indicator: "▾".to_string(),
            leaf_indicator: "•".to_string(),
        }
    }

    /// Create options for plain text rendering
    pub fn plain() -> Self {
        RenderOptions {
            max_depth: None,
            show_loading: true,
            show_errors: true,
            indent: "  ".to_string(),
            collapsed_indicator: "+".to_string(),
            expanded_indicator: "-".to_string(),
            leaf_indicator: "*".to_string(),
        }
    }
}

/// Trait for rendering tree state
pub trait TreeRenderer {
    /// The output type for rendering
    type Output;

    /// Render the tree state
    fn render(&self, state: &TreeState, options: &RenderOptions) -> Self::Output;
}

/// Get all visible nodes in display order
///
/// In Feed mode, returns all roots with their expanded children.
/// In Reader mode, only returns the selected publication and its children.
pub fn visible_nodes(state: &TreeState) -> Vec<VisibleNode> {
    let mut result = Vec::new();

    // In Reader mode, only show the selected publication's tree
    if state.is_reader_mode() {
        if let Some(pub_id) = state.selected_publication {
            collect_visible(state, pub_id, 0, &mut result);
            return result;
        }
    }

    // In Feed mode (or fallback), show all roots
    for &root_id in &state.roots {
        collect_visible(state, root_id, 0, &mut result);
    }
    result
}

fn collect_visible(state: &TreeState, id: NodeId, depth: usize, result: &mut Vec<VisibleNode>) {
    if let Some(node) = state.get_node(id) {
        result.push(VisibleNode::from_node(node, state, depth));

        if node.has_children() && state.is_expanded(id) {
            for &child_id in node.children() {
                collect_visible(state, child_id, depth + 1, result);
            }
        }
    }
}

/// Get the index of the cursor in the visible list
pub fn cursor_visible_index(state: &TreeState) -> Option<usize> {
    visible_nodes(state)
        .iter()
        .position(|n| n.is_cursor)
}

/// Get visible nodes in a range (for virtual scrolling)
pub fn visible_nodes_range(state: &TreeState, start: usize, count: usize) -> Vec<VisibleNode> {
    visible_nodes(state)
        .into_iter()
        .skip(start)
        .take(count)
        .collect()
}

/// Simple text renderer for debugging
pub struct TextRenderer;

impl TreeRenderer for TextRenderer {
    type Output = String;

    fn render(&self, state: &TreeState, options: &RenderOptions) -> String {
        let nodes = visible_nodes(state);
        let mut output = String::new();

        for node in nodes {
            // Check max depth
            if let Some(max) = options.max_depth {
                if node.depth > max {
                    continue;
                }
            }

            // Build indent
            let indent = options.indent.repeat(node.depth);

            // Choose indicator
            let indicator = if node.has_children {
                if node.is_expanded {
                    &options.expanded_indicator
                } else {
                    &options.collapsed_indicator
                }
            } else {
                &options.leaf_indicator
            };

            // Build status suffix
            let mut suffix = String::new();
            if options.show_loading && node.is_loading {
                suffix.push_str(" [loading...]");
            }
            if options.show_errors {
                if let Some(ref err) = node.error {
                    suffix.push_str(&format!(" [error: {}]", err));
                }
            }
            if node.is_cursor {
                suffix.push_str(" <");
            }
            if node.is_selected {
                suffix.push_str(" *");
            }

            output.push_str(&format!(
                "{}{} {}{}\n",
                indent, indicator, node.title, suffix
            ));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::NAddr;
    use crate::tree::node::{PublicationNode, SectionNode};

    fn make_test_state() -> TreeState {
        let mut state = TreeState::new();

        let root_id = NodeId(1);
        let child1_id = NodeId(2);
        let child2_id = NodeId(3);

        let mut pub_node = PublicationNode::stub(NAddr::new(30040, "pub", "root"), None);
        pub_node.id = root_id;
        pub_node.title = Some("My Publication".to_string());
        pub_node.children = vec![child1_id, child2_id];
        pub_node.loaded = true;

        let mut sec1 = SectionNode::stub(NAddr::new(30041, "pub", "sec1"), root_id, 0);
        sec1.id = child1_id;
        sec1.title = Some("Chapter 1".to_string());
        sec1.loaded = true;

        let mut sec2 = SectionNode::stub(NAddr::new(30041, "pub", "sec2"), root_id, 1);
        sec2.id = child2_id;
        sec2.title = Some("Chapter 2".to_string());
        sec2.loaded = false;

        state.add_node(TreeNode::Publication(pub_node));
        state.add_node(TreeNode::Section(sec1));
        state.add_node(TreeNode::Section(sec2));
        state.roots.push(root_id);
        state.cursor = root_id;

        state
    }

    #[test]
    fn test_visible_nodes_collapsed() {
        let state = make_test_state();
        let nodes = visible_nodes(&state);

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].title, "My Publication");
        assert!(nodes[0].is_cursor);
    }

    #[test]
    fn test_visible_nodes_expanded() {
        let mut state = make_test_state();
        state.expand(NodeId(1));

        let nodes = visible_nodes(&state);
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].title, "My Publication");
        assert_eq!(nodes[0].depth, 0);
        assert_eq!(nodes[1].title, "Chapter 1");
        assert_eq!(nodes[1].depth, 1);
        assert_eq!(nodes[2].title, "Chapter 2");
        assert_eq!(nodes[2].depth, 1);
    }

    #[test]
    fn test_text_renderer() {
        let mut state = make_test_state();
        state.expand(NodeId(1));

        let renderer = TextRenderer;
        let output = renderer.render(&state, &RenderOptions::plain());

        assert!(output.contains("My Publication"));
        assert!(output.contains("Chapter 1"));
        assert!(output.contains("Chapter 2"));
    }
}
