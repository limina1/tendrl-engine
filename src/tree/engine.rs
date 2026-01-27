//! TreeEngine - Core logic for tree navigation and manipulation
//!
//! The TreeEngine executes commands synchronously on TreeState. When IO is
//! needed (loading publications/sections), it returns an AsyncRequest that
//! the caller must handle. Results are then applied via `apply_async_result()`.

use super::command::{AsyncRequest, AsyncResult, CommandResult, ConfigAction, TreeCommand};
use super::node::{NodeId, PublicationNode, SectionNode, TreeNode};
use super::render::visible_nodes;
use super::state::{AppMode, ClipboardContent, TreeState};
use crate::publication::NAddr;

/// The TreeEngine processes commands on TreeState
#[derive(Debug, Clone, Default)]
pub struct TreeEngine {
    // Configuration could go here in the future
}

impl TreeEngine {
    /// Create a new TreeEngine
    pub fn new() -> Self {
        TreeEngine {}
    }

    /// Execute a command on the tree state
    pub fn execute(&self, state: &mut TreeState, command: TreeCommand) -> CommandResult {
        // Route navigation commands based on current mode
        if state.is_feed_mode() {
            match &command {
                TreeCommand::MoveUp => return self.feed_move_up(state),
                TreeCommand::MoveDown => return self.feed_move_down(state),
                TreeCommand::MoveToFirst => {
                    if state.feed_cursor != 0 {
                        state.feed_cursor = 0;
                        return CommandResult::StateChanged;
                    }
                    return CommandResult::NoOp;
                }
                TreeCommand::MoveToLast => {
                    let last = state.roots.len().saturating_sub(1);
                    if state.feed_cursor != last {
                        state.feed_cursor = last;
                        return CommandResult::StateChanged;
                    }
                    return CommandResult::NoOp;
                }
                TreeCommand::Enter => return self.feed_enter(state),
                // Other commands fall through to normal handling
                _ => {}
            }
        }

        match command {
            // Navigation (Reader mode)
            TreeCommand::MoveUp => self.move_up(state),
            TreeCommand::MoveDown => self.move_down(state),
            TreeCommand::MoveToFirst => self.move_to_first(state),
            TreeCommand::MoveToLast => self.move_to_last(state),
            TreeCommand::MoveToParent => self.move_to_parent(state),
            TreeCommand::Enter => self.enter(state),
            TreeCommand::Collapse => self.collapse(state),
            TreeCommand::ToggleExpand => self.toggle_expand(state),

            // Selection
            TreeCommand::ToggleSelect => self.toggle_select(state),
            TreeCommand::SelectAll => self.select_all(state),
            TreeCommand::ClearSelection => self.clear_selection(state),

            // View
            TreeCommand::TogglePreview => {
                state.view.toggle_preview();
                CommandResult::StateChanged
            }
            TreeCommand::ScrollPreviewUp => {
                if state.view.preview_scroll > 0 {
                    state.view.preview_scroll -= 1;
                    CommandResult::StateChanged
                } else {
                    CommandResult::NoOp
                }
            }
            TreeCommand::ScrollPreviewDown => {
                state.view.preview_scroll += 1;
                CommandResult::StateChanged
            }
            TreeCommand::ScrollContentUp => {
                if state.view.content_scroll > 0 {
                    state.view.content_scroll = state.view.content_scroll.saturating_sub(3);
                    CommandResult::StateChanged
                } else {
                    CommandResult::NoOp
                }
            }
            TreeCommand::ScrollContentDown => {
                state.view.content_scroll += 3;
                CommandResult::StateChanged
            }
            TreeCommand::NextPage => self.next_page(state),
            TreeCommand::PrevPage => self.prev_page(state),
            TreeCommand::Refresh => CommandResult::NeedsAsync(AsyncRequest::RefreshAll),

            // Undo/Redo (stub for Phase 4)
            TreeCommand::Undo => {
                if state.undo_stack.can_undo() {
                    // TODO: Implement actual undo
                    CommandResult::Error("Undo not yet implemented".to_string())
                } else {
                    CommandResult::NoOp
                }
            }
            TreeCommand::Redo => {
                if state.undo_stack.can_redo() {
                    // TODO: Implement actual redo
                    CommandResult::Error("Redo not yet implemented".to_string())
                } else {
                    CommandResult::NoOp
                }
            }

            // Manipulation (stub for Phase 4)
            TreeCommand::MoveSectionUp => self.move_section(state, -1),
            TreeCommand::MoveSectionDown => self.move_section(state, 1),
            TreeCommand::Delete => self.delete(state),
            TreeCommand::Yank => self.yank(state),
            TreeCommand::PasteAfter => self.paste(state, false),
            TreeCommand::PasteBefore => self.paste(state, true),

            // Versioning (stub for Phase 5)
            TreeCommand::Fork => CommandResult::Error("Fork not yet implemented".to_string()),
            TreeCommand::ShowAlternates => self.show_alternates(state),
            TreeCommand::SlotInVersion { version_index } => {
                CommandResult::Error(format!("SlotInVersion({}) not yet implemented", version_index))
            }

            // Mode switching
            TreeCommand::Back => self.back(state),
            TreeCommand::CycleViewMode => self.cycle_view_mode(state),
            TreeCommand::SetViewMode { mode } => self.set_view_mode(state, mode),

            // Application
            TreeCommand::Quit => CommandResult::Exit,

            // Configuration (handled by app, not engine)
            TreeCommand::AddRelay { url } => {
                CommandResult::ConfigChange(ConfigAction::AddRelay(url))
            }
            TreeCommand::RemoveRelay { url } => {
                CommandResult::ConfigChange(ConfigAction::RemoveRelay(url))
            }
            TreeCommand::ClearRelays => CommandResult::ConfigChange(ConfigAction::ClearRelays),
            TreeCommand::ShowRelays => CommandResult::ConfigChange(ConfigAction::ShowRelays),
        }
    }

    /// Apply the result of an async operation to the state
    pub fn apply_async_result(&self, state: &mut TreeState, result: AsyncResult) -> CommandResult {
        match result {
            AsyncResult::PublicationLoaded {
                node_id,
                title,
                children,
            } => {
                // First, check if node exists
                if !state.nodes.contains_key(&node_id) {
                    return CommandResult::Error("Node not found for publication load result".to_string());
                }

                // Create child nodes first (collect into vec to insert later)
                let mut child_ids = Vec::new();
                let mut new_nodes = Vec::new();

                for (i, addr) in children.iter().enumerate() {
                    let child_id = NodeId::from_addr(addr);
                    child_ids.push(child_id);

                    let child_node = if addr.kind == 30040 {
                        TreeNode::Publication(PublicationNode::stub(addr.clone(), Some(node_id)))
                    } else {
                        TreeNode::Section(SectionNode::stub(addr.clone(), node_id, i))
                    };
                    new_nodes.push((child_id, child_node));
                }

                // Insert all child nodes
                for (id, node) in new_nodes {
                    state.nodes.insert(id, node);
                }

                // Now update the parent node
                if let Some(TreeNode::Publication(ref mut pub_node)) = state.nodes.get_mut(&node_id) {
                    pub_node.title = title;
                    pub_node.loaded = true;
                    pub_node.loading = false;
                    pub_node.error = None;
                    pub_node.children = child_ids;
                }

                CommandResult::StateChanged
            }

            AsyncResult::SectionLoaded {
                node_id,
                title,
                content,
            } => {
                if let Some(TreeNode::Section(ref mut sec_node)) = state.nodes.get_mut(&node_id) {
                    sec_node.title = title;
                    sec_node.content = content;
                    sec_node.loaded = true;
                    sec_node.loading = false;
                    sec_node.error = None;

                    CommandResult::StateChanged
                } else {
                    CommandResult::Error("Node not found for section load result".to_string())
                }
            }

            AsyncResult::ChildrenLoaded { parent_id, children } => {
                // First, check if parent exists
                if !state.nodes.contains_key(&parent_id) {
                    return CommandResult::Error("Parent node not found for children load".to_string());
                }

                // Create child nodes first
                let mut child_ids = Vec::new();
                let mut new_nodes = Vec::new();

                for (i, child) in children.iter().enumerate() {
                    let child_id = NodeId::from_addr(&child.addr);
                    child_ids.push(child_id);

                    let child_node = if child.is_publication {
                        let mut pn = PublicationNode::stub(child.addr.clone(), Some(parent_id));
                        pn.title = child.title.clone();
                        TreeNode::Publication(pn)
                    } else {
                        let mut sn = SectionNode::stub(child.addr.clone(), parent_id, i);
                        sn.title = child.title.clone();
                        TreeNode::Section(sn)
                    };
                    new_nodes.push((child_id, child_node));
                }

                // Insert all child nodes
                for (id, node) in new_nodes {
                    state.nodes.insert(id, node);
                }

                // Update parent
                if let Some(TreeNode::Publication(ref mut pub_node)) = state.nodes.get_mut(&parent_id) {
                    pub_node.children = child_ids;
                }

                CommandResult::StateChanged
            }

            AsyncResult::AlternatesFound { node_id, versions } => {
                if let Some(TreeNode::Section(ref mut sec_node)) = state.nodes.get_mut(&node_id) {
                    sec_node.alternate_count = versions.len();
                    // Store versions somewhere accessible for the UI
                    // For now, just update the count
                    CommandResult::StateChanged
                } else {
                    CommandResult::Error("Node not found for alternates result".to_string())
                }
            }

            AsyncResult::MorePublicationsLoaded { publications } => {
                state.loading_more = false;

                if publications.is_empty() {
                    state.feed_exhausted = true;
                    return CommandResult::StateChanged;
                }

                // Add new publications to the tree
                for pub_data in &publications {
                    let id = NodeId::from_addr(&pub_data.addr);

                    // Skip if already exists
                    if state.nodes.contains_key(&id) {
                        continue;
                    }

                    let mut pub_node = PublicationNode::stub(pub_data.addr.clone(), None);
                    pub_node.title = pub_data.title.clone();
                    pub_node.summary = pub_data.summary.clone();
                    pub_node.author = pub_data.author.clone();
                    pub_node.author_name = pub_data.author_name.clone();
                    pub_node.created_at = pub_data.created_at;
                    pub_node.loaded = !pub_data.sections.is_empty();

                    // Add section stubs
                    let mut child_ids = Vec::new();
                    for (i, sec_addr) in pub_data.sections.iter().enumerate() {
                        let child_id = NodeId::from_addr(sec_addr);
                        child_ids.push(child_id);

                        let sec_node = SectionNode::stub(sec_addr.clone(), id, i);
                        state.nodes.insert(child_id, TreeNode::Section(sec_node));
                    }
                    pub_node.children = child_ids;

                    state.nodes.insert(id, TreeNode::Publication(pub_node));
                    state.roots.push(id);

                    // Update oldest timestamp
                    if let Some(oldest) = state.oldest_timestamp {
                        if pub_data.created_at < oldest {
                            state.oldest_timestamp = Some(pub_data.created_at);
                        }
                    } else {
                        state.oldest_timestamp = Some(pub_data.created_at);
                    }
                }

                CommandResult::StateChanged
            }

            AsyncResult::Error { request, error } => {
                // Try to mark the relevant node as errored
                if let Some(node_id) = request.target_node() {
                    if let Some(node) = state.nodes.get_mut(&node_id) {
                        match node {
                            TreeNode::Publication(p) => {
                                p.loading = false;
                                p.error = Some(error.clone());
                            }
                            TreeNode::Section(s) => {
                                s.loading = false;
                                s.error = Some(error.clone());
                            }
                        }
                    }
                }
                CommandResult::Error(error)
            }
        }
    }

    // --- Navigation commands ---

    fn move_up(&self, state: &mut TreeState) -> CommandResult {
        let visible = visible_nodes(state);
        if let Some(idx) = visible.iter().position(|n| n.id == state.cursor) {
            if idx > 0 {
                state.cursor = visible[idx - 1].id;
                self.adjust_scroll(state);
                return CommandResult::StateChanged;
            }
        }
        CommandResult::NoOp
    }

    fn move_down(&self, state: &mut TreeState) -> CommandResult {
        let visible = visible_nodes(state);
        if let Some(idx) = visible.iter().position(|n| n.id == state.cursor) {
            if idx + 1 < visible.len() {
                state.cursor = visible[idx + 1].id;
                self.adjust_scroll(state);
                return CommandResult::StateChanged;
            }
        }
        CommandResult::NoOp
    }

    fn move_to_first(&self, state: &mut TreeState) -> CommandResult {
        let visible = visible_nodes(state);
        if let Some(first) = visible.first() {
            if state.cursor != first.id {
                state.cursor = first.id;
                state.view.tree_scroll = 0;
                return CommandResult::StateChanged;
            }
        }
        CommandResult::NoOp
    }

    fn move_to_last(&self, state: &mut TreeState) -> CommandResult {
        let visible = visible_nodes(state);
        if let Some(last) = visible.last() {
            if state.cursor != last.id {
                state.cursor = last.id;
                self.adjust_scroll(state);
                return CommandResult::StateChanged;
            }
        }
        CommandResult::NoOp
    }

    fn move_to_parent(&self, state: &mut TreeState) -> CommandResult {
        if let Some(parent_id) = state.parent_of(state.cursor) {
            state.cursor = parent_id;
            self.adjust_scroll(state);
            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    fn enter(&self, state: &mut TreeState) -> CommandResult {
        if let Some(node) = state.get_node(state.cursor) {
            match node {
                TreeNode::Publication(p) => {
                    if !p.loaded && !p.loading {
                        // Need to load the publication
                        let addr = p.addr.clone();
                        let parent = p.parent;

                        // Mark as loading
                        if let Some(TreeNode::Publication(ref mut pn)) =
                            state.nodes.get_mut(&state.cursor)
                        {
                            pn.loading = true;
                        }

                        return CommandResult::NeedsAsync(AsyncRequest::LoadPublication {
                            addr,
                            parent,
                        });
                    }

                    // Publication is loaded, toggle expansion
                    if p.has_children() {
                        let cursor = state.cursor;
                        state.toggle_expanded(cursor);
                        return CommandResult::StateChanged;
                    }
                }
                TreeNode::Section(s) => {
                    if !s.loaded && !s.loading {
                        // Need to load the section
                        let addr = s.addr.clone();
                        let parent = s.parent.unwrap_or(NodeId::root());

                        // Mark as loading
                        if let Some(TreeNode::Section(ref mut sn)) =
                            state.nodes.get_mut(&state.cursor)
                        {
                            sn.loading = true;
                        }

                        return CommandResult::NeedsAsync(AsyncRequest::LoadSection { addr, parent });
                    }
                    // Section is loaded, nothing to expand
                }
            }
        }
        CommandResult::NoOp
    }

    fn collapse(&self, state: &mut TreeState) -> CommandResult {
        let cursor = state.cursor;
        if state.is_expanded(cursor) {
            state.collapse(cursor);
            CommandResult::StateChanged
        } else if let Some(parent_id) = state.parent_of(cursor) {
            // Move to parent and collapse it
            state.cursor = parent_id;
            state.collapse(parent_id);
            self.adjust_scroll(state);
            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    fn toggle_expand(&self, state: &mut TreeState) -> CommandResult {
        if let Some(node) = state.get_node(state.cursor) {
            if node.has_children() {
                let cursor = state.cursor;
                state.toggle_expanded(cursor);
                return CommandResult::StateChanged;
            } else if !node.is_loaded() {
                // Try to load unloaded node
                return self.enter(state);
            }
        }
        CommandResult::NoOp
    }

    // --- Selection commands ---

    fn toggle_select(&self, state: &mut TreeState) -> CommandResult {
        let cursor = state.cursor;
        state.toggle_selected(cursor);
        CommandResult::StateChanged
    }

    fn select_all(&self, state: &mut TreeState) -> CommandResult {
        let visible = visible_nodes(state);
        for node in visible {
            state.selected.insert(node.id);
        }
        CommandResult::StateChanged
    }

    fn clear_selection(&self, state: &mut TreeState) -> CommandResult {
        if state.selected.is_empty() {
            CommandResult::NoOp
        } else {
            state.clear_selection();
            CommandResult::StateChanged
        }
    }

    // --- Manipulation commands (stubs for Phase 4) ---

    fn move_section(&self, state: &mut TreeState, _direction: i32) -> CommandResult {
        // Only sections can be moved
        if let Some(TreeNode::Section(_)) = state.get_node(state.cursor) {
            // TODO: Implement actual movement
            CommandResult::Error("Move section not yet implemented".to_string())
        } else {
            CommandResult::Error("Can only move sections".to_string())
        }
    }

    fn delete(&self, _state: &mut TreeState) -> CommandResult {
        // TODO: Implement delete
        CommandResult::Error("Delete not yet implemented".to_string())
    }

    fn yank(&self, state: &mut TreeState) -> CommandResult {
        let cursor = state.cursor;
        if state.selected.is_empty() {
            state.clipboard = Some(ClipboardContent::Single(cursor));
        } else {
            state.clipboard = Some(ClipboardContent::Multiple(
                state.selected.iter().copied().collect(),
            ));
        }
        CommandResult::StateChanged
    }

    fn paste(&self, state: &mut TreeState, _before: bool) -> CommandResult {
        if state.clipboard.is_none() {
            return CommandResult::Error("Nothing in clipboard".to_string());
        }
        // TODO: Implement actual paste
        CommandResult::Error("Paste not yet implemented".to_string())
    }

    // --- Versioning commands (stubs for Phase 5) ---

    fn show_alternates(&self, state: &mut TreeState) -> CommandResult {
        if let Some(TreeNode::Section(s)) = state.get_node(state.cursor) {
            let addr = s.addr.clone();
            let node_id = s.id;
            CommandResult::NeedsAsync(AsyncRequest::FindAlternates { addr, node_id })
        } else {
            CommandResult::Error("Can only show alternates for sections".to_string())
        }
    }

    // --- Mode switching commands ---

    fn back(&self, state: &mut TreeState) -> CommandResult {
        if state.is_reader_mode() {
            state.exit_reader();
            CommandResult::ModeChanged(AppMode::Feed)
        } else {
            CommandResult::NoOp
        }
    }

    fn cycle_view_mode(&self, state: &mut TreeState) -> CommandResult {
        state.view.mode = state.view.mode.next();
        CommandResult::StateChanged
    }

    fn set_view_mode(
        &self,
        state: &mut TreeState,
        mode: super::state::ViewMode,
    ) -> CommandResult {
        if state.view.mode != mode {
            state.view.mode = mode;
            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    fn next_page(&self, state: &mut TreeState) -> CommandResult {
        // Get the total number of sections in the current publication
        let pub_id = state.selected_publication.unwrap_or(state.cursor);
        if let Some(TreeNode::Publication(p)) = state.nodes.get(&pub_id) {
            let total = p.children.len();
            if total == 0 {
                return CommandResult::NoOp;
            }
            if state.view.current_section + 1 < total {
                state.view.current_section += 1;
                state.view.preview_scroll = 0; // Reset scroll for new section
                return CommandResult::StateChanged;
            }
        }
        CommandResult::NoOp
    }

    fn prev_page(&self, state: &mut TreeState) -> CommandResult {
        if state.view.current_section > 0 {
            state.view.current_section -= 1;
            state.view.preview_scroll = 0; // Reset scroll for new section
            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    // --- Feed mode navigation ---

    fn feed_move_up(&self, state: &mut TreeState) -> CommandResult {
        if state.feed_cursor > 0 {
            state.feed_cursor -= 1;
            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    fn feed_move_down(&self, state: &mut TreeState) -> CommandResult {
        if state.feed_cursor + 1 < state.roots.len() {
            state.feed_cursor += 1;

            // Check if we're near the bottom and should load more
            let near_bottom = state.feed_cursor + 3 >= state.roots.len();
            if near_bottom && !state.loading_more && !state.feed_exhausted {
                if let Some(oldest) = state.oldest_timestamp {
                    state.loading_more = true;
                    return CommandResult::NeedsAsync(AsyncRequest::LoadMorePublications {
                        before_timestamp: oldest,
                        limit: 15,
                    });
                }
            }

            CommandResult::StateChanged
        } else {
            CommandResult::NoOp
        }
    }

    fn feed_enter(&self, state: &mut TreeState) -> CommandResult {
        if let Some(&pub_id) = state.roots.get(state.feed_cursor) {
            // Check if we need to load the publication first
            if let Some(TreeNode::Publication(p)) = state.get_node(pub_id) {
                if !p.loaded && !p.loading {
                    // Mark as loading and request async load
                    let addr = p.addr.clone();
                    let parent = p.parent;

                    if let Some(TreeNode::Publication(ref mut pn)) = state.nodes.get_mut(&pub_id) {
                        pn.loading = true;
                    }

                    return CommandResult::NeedsAsync(AsyncRequest::LoadPublication { addr, parent });
                }
            }

            // Enter reader mode for this publication
            state.enter_reader(pub_id);
            // Auto-expand the publication in reader mode
            state.expand(pub_id);
            return CommandResult::ModeChanged(AppMode::Reader);
        }
        CommandResult::NoOp
    }

    // --- Helper methods ---

    fn adjust_scroll(&self, state: &mut TreeState) {
        // Ensure cursor is visible in the viewport
        // This is a simple implementation; the TUI can do more sophisticated scrolling
        let visible = visible_nodes(state);
        if let Some(cursor_idx) = visible.iter().position(|n| n.id == state.cursor) {
            // Keep cursor in view with some margin
            const MARGIN: usize = 2;
            if cursor_idx < state.view.tree_scroll + MARGIN {
                state.view.tree_scroll = cursor_idx.saturating_sub(MARGIN);
            }
            // Note: we can't adjust for bottom of viewport without knowing viewport height
        }
    }
}

/// Initialize tree state from a list of root publications
pub fn init_from_publications(
    state: &mut TreeState,
    publications: Vec<(NAddr, Option<String>)>,
) {
    for (addr, title) in publications {
        let id = NodeId::from_addr(&addr);
        let mut node = PublicationNode::stub(addr, None);
        node.title = title;
        state.add_node(TreeNode::Publication(node));
        state.roots.push(id);
    }

    // Set cursor to first root if available
    if let Some(&first) = state.roots.first() {
        state.cursor = first;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        sec1.content = Some("Chapter 1 content".to_string());

        let mut sec2 = SectionNode::stub(NAddr::new(30041, "pub", "sec2"), root_id, 1);
        sec2.id = child2_id;
        sec2.title = Some("Chapter 2".to_string());
        sec2.loaded = true;
        sec2.content = Some("Chapter 2 content".to_string());

        state.add_node(TreeNode::Publication(pub_node));
        state.add_node(TreeNode::Section(sec1));
        state.add_node(TreeNode::Section(sec2));
        state.roots.push(root_id);
        state.cursor = root_id;

        state
    }

    #[test]
    fn test_navigation_collapsed() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Enter reader mode for tree navigation testing
        state.enter_reader(NodeId(1));

        // Can't move down when collapsed (only one visible node)
        assert!(matches!(
            engine.execute(&mut state, TreeCommand::MoveDown),
            CommandResult::NoOp
        ));

        // Can't move up from first
        assert!(matches!(
            engine.execute(&mut state, TreeCommand::MoveUp),
            CommandResult::NoOp
        ));
    }

    #[test]
    fn test_navigation_expanded() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Enter reader mode for tree navigation testing
        state.enter_reader(NodeId(1));

        // Expand the root
        state.expand(NodeId(1));

        // Move down to first child
        let result = engine.execute(&mut state, TreeCommand::MoveDown);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.cursor, NodeId(2));

        // Move down to second child
        let result = engine.execute(&mut state, TreeCommand::MoveDown);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.cursor, NodeId(3));

        // Can't move down further
        let result = engine.execute(&mut state, TreeCommand::MoveDown);
        assert!(matches!(result, CommandResult::NoOp));

        // Move up
        let result = engine.execute(&mut state, TreeCommand::MoveUp);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.cursor, NodeId(2));
    }

    #[test]
    fn test_toggle_expand() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Enter reader mode for tree navigation testing
        state.enter_reader(NodeId(1));

        assert!(!state.is_expanded(NodeId(1)));

        // Toggle expand
        engine.execute(&mut state, TreeCommand::ToggleExpand);
        assert!(state.is_expanded(NodeId(1)));

        // Toggle collapse
        engine.execute(&mut state, TreeCommand::ToggleExpand);
        assert!(!state.is_expanded(NodeId(1)));
    }

    #[test]
    fn test_move_to_parent() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Enter reader mode for tree navigation testing
        state.enter_reader(NodeId(1));

        state.expand(NodeId(1));
        state.cursor = NodeId(2); // Move to child

        let result = engine.execute(&mut state, TreeCommand::MoveToParent);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.cursor, NodeId(1));
    }

    #[test]
    fn test_yank() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Enter reader mode for tree navigation testing
        state.enter_reader(NodeId(1));

        assert!(state.clipboard.is_none());

        engine.execute(&mut state, TreeCommand::Yank);

        assert!(matches!(
            state.clipboard,
            Some(ClipboardContent::Single(NodeId(1)))
        ));
    }

    #[test]
    fn test_feed_mode_navigation() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Default mode is Feed
        assert!(state.is_feed_mode());
        assert_eq!(state.feed_cursor, 0);

        // Add another root publication for navigation testing
        let root2_id = NodeId(100);
        let mut pub2 = PublicationNode::stub(NAddr::new(30040, "pub2", "root2"), None);
        pub2.id = root2_id;
        pub2.title = Some("Second Publication".to_string());
        pub2.loaded = true;
        state.add_node(TreeNode::Publication(pub2));
        state.roots.push(root2_id);

        // Move down in feed
        let result = engine.execute(&mut state, TreeCommand::MoveDown);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.feed_cursor, 1);

        // Move up in feed
        let result = engine.execute(&mut state, TreeCommand::MoveUp);
        assert!(matches!(result, CommandResult::StateChanged));
        assert_eq!(state.feed_cursor, 0);
    }

    #[test]
    fn test_enter_reader_mode() {
        let mut state = make_test_state();
        let engine = TreeEngine::new();

        // Start in feed mode
        assert!(state.is_feed_mode());

        // Enter reader mode
        let result = engine.execute(&mut state, TreeCommand::Enter);
        assert!(matches!(result, CommandResult::ModeChanged(AppMode::Reader)));
        assert!(state.is_reader_mode());
        assert_eq!(state.selected_publication, Some(NodeId(1)));

        // Exit reader mode with Back
        let result = engine.execute(&mut state, TreeCommand::Back);
        assert!(matches!(result, CommandResult::ModeChanged(AppMode::Feed)));
        assert!(state.is_feed_mode());
    }
}
