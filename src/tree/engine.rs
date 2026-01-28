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
        // Handle compose mode commands
        if state.is_compose_mode() {
            return self.execute_compose(state, command);
        }

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
                TreeCommand::EnterCompose => return self.enter_compose(state),
                TreeCommand::EnterEditorCompose => return self.enter_editor_compose(state),
                TreeCommand::FilterDrafts => {
                    state.view.filter_drafts = !state.view.filter_drafts;
                    state.feed_cursor = 0; // Reset cursor when toggling filter
                    return CommandResult::StateChanged;
                }
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

            // Batch loading
            TreeCommand::LoadBufferEvents => self.load_buffer_events(state),
            TreeCommand::RefreshBuffer => self.refresh_buffer(state),

            // UI (handled by app, not engine)
            TreeCommand::ShowCommandPalette => {
                // This is handled directly in app.rs, but we need to have a match arm
                CommandResult::NoOp
            }

            // Compose mode commands - only valid in compose mode, handled by execute_compose
            TreeCommand::EnterCompose | TreeCommand::EnterEditorCompose => {
                // In feed mode, these are handled above; in other modes it's a no-op
                CommandResult::NoOp
            }
            TreeCommand::ExitCompose
            | TreeCommand::InsertChar { .. }
            | TreeCommand::DeleteChar
            | TreeCommand::DeleteCharForward
            | TreeCommand::CursorLeft
            | TreeCommand::CursorRight
            | TreeCommand::CursorHome
            | TreeCommand::CursorEnd
            | TreeCommand::NextField
            | TreeCommand::PrevField
            | TreeCommand::CreateTags
            | TreeCommand::EndTags
            | TreeCommand::AddSection
            | TreeCommand::RemoveSection
            | TreeCommand::InsertNewline
            | TreeCommand::Publish
            | TreeCommand::ToggleComposeStyle
            | TreeCommand::EditorToggleMode
            | TreeCommand::EditorInsertChar { .. }
            | TreeCommand::EditorInsertNewline
            | TreeCommand::EditorBackspace
            | TreeCommand::EditorDelete
            | TreeCommand::EditorDeleteLine
            | TreeCommand::EditorCursorLeft
            | TreeCommand::EditorCursorRight
            | TreeCommand::EditorCursorUp
            | TreeCommand::EditorCursorDown
            | TreeCommand::EditorCursorHome
            | TreeCommand::EditorCursorEnd
            | TreeCommand::EditorCursorToEnd
            | TreeCommand::EditorInsertAfter
            | TreeCommand::EditorInsertLineBelow
            | TreeCommand::EditorInsertLineAbove
            | TreeCommand::CycleEditorViewMode
            | TreeCommand::SetEditorViewMode { .. } => {
                // These are only valid in compose mode
                CommandResult::NoOp
            }

            // Window management (handled by app.rs, not engine)
            TreeCommand::ShowJson { .. }
            | TreeCommand::CloseWindow
            | TreeCommand::CloseAllWindows
            | TreeCommand::FocusNextWindow
            | TreeCommand::FocusPrevWindow
            | TreeCommand::WindowScrollUp
            | TreeCommand::WindowScrollDown
            | TreeCommand::WindowScrollToTop
            | TreeCommand::WindowScrollToBottom
            | TreeCommand::ShowEventJson
            | TreeCommand::ShowUserData
            | TreeCommand::CloseUserDataMenu
            | TreeCommand::UserDataMenuUp
            | TreeCommand::UserDataMenuDown
            | TreeCommand::UserDataMenuSelect => {
                // Window and menu commands are handled directly in app.rs
                CommandResult::NoOp
            }

            // Draft management (SaveDraft handled in execute_compose, others handled by app)
            TreeCommand::SaveDraft => {
                // Only valid in compose mode, handled by execute_compose
                CommandResult::NoOp
            }
            TreeCommand::LoadDraft { .. } | TreeCommand::DeleteDraft { .. } => {
                // Handled by app.rs
                CommandResult::NoOp
            }
            TreeCommand::FilterDrafts => {
                // Only valid in feed mode, handled above
                CommandResult::NoOp
            }

            // Identity management (handled by app.rs, not engine)
            TreeCommand::OpenLoginDialog
            | TreeCommand::CloseLoginDialog
            | TreeCommand::SubmitLogin { .. }
            | TreeCommand::SubmitPassword { .. }
            | TreeCommand::Logout => {
                // Login/logout commands are handled directly in app.rs
                CommandResult::NoOp
            }
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

            AsyncResult::DraftSaved { draft_id: _ } => {
                // Draft saved successfully - exit compose mode
                state.exit_compose();
                CommandResult::ModeChanged(super::state::AppMode::Feed)
            }

            AsyncResult::DraftsLoaded { drafts } => {
                use crate::publication::NAddr;
                use crate::tree::node::SyncStatus;

                // Add drafts to the tree as publications with Draft sync status
                // Note: LoadedDraft only has metadata, not full section content
                // Full draft loading with sections is done in load_drafts_sync
                for draft in drafts {
                    // Create a pseudo-NAddr for the draft
                    let addr = NAddr::new(30040, &"0".repeat(64), &draft.draft_id);
                    let id = NodeId::from_addr(&addr);

                    // Skip if already exists
                    if state.nodes.contains_key(&id) {
                        continue;
                    }

                    let mut pub_node = PublicationNode::stub(addr, None);
                    pub_node.title = Some(draft.title.clone());
                    pub_node.summary = Some(format!("{} sections", draft.section_count));
                    pub_node.author = "Draft".to_string();
                    pub_node.author_name = Some("Unsigned Draft".to_string());
                    pub_node.created_at = draft.modified_at;
                    // Mark as not loaded since we don't have section details in LoadedDraft
                    pub_node.loaded = false;
                    pub_node.sync_status = SyncStatus::Draft;
                    pub_node.draft_id = Some(draft.draft_id.clone());

                    state.nodes.insert(id, TreeNode::Publication(pub_node));
                    // Insert drafts at the beginning of roots (they're local and should be prominent)
                    state.roots.insert(0, id);
                }

                CommandResult::StateChanged
            }

            AsyncResult::UserDataLoaded { user_data } => {
                // Store the user data in the state
                state.user_data = user_data;
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

    // --- Buffer loading commands ---

    fn load_buffer_events(&self, state: &mut TreeState) -> CommandResult {
        // Get visible nodes and filter to unloaded ones
        let visible = visible_nodes(state);
        let mut requests = Vec::new();

        for node in visible {
            if node.is_loaded || node.is_loading {
                continue;
            }

            // Get the node to build the request
            if let Some(tree_node) = state.nodes.get(&node.id) {
                match tree_node {
                    TreeNode::Publication(p) => {
                        requests.push(AsyncRequest::LoadPublication {
                            addr: p.addr.clone(),
                            parent: p.parent,
                        });
                    }
                    TreeNode::Section(s) => {
                        requests.push(AsyncRequest::LoadSection {
                            addr: s.addr.clone(),
                            parent: s.parent.unwrap_or(NodeId::root()),
                        });
                    }
                }
            }
        }

        if requests.is_empty() {
            return CommandResult::NoOp;
        }

        // Mark all nodes as loading
        for req in &requests {
            match req {
                AsyncRequest::LoadPublication { addr, .. } => {
                    let id = NodeId::from_addr(addr);
                    if let Some(TreeNode::Publication(ref mut p)) = state.nodes.get_mut(&id) {
                        p.loading = true;
                    }
                }
                AsyncRequest::LoadSection { addr, .. } => {
                    let id = NodeId::from_addr(addr);
                    if let Some(TreeNode::Section(ref mut s)) = state.nodes.get_mut(&id) {
                        s.loading = true;
                    }
                }
                _ => {}
            }
        }

        CommandResult::NeedsAsync(AsyncRequest::LoadBatch { requests })
    }

    fn refresh_buffer(&self, state: &mut TreeState) -> CommandResult {
        // Get visible nodes that are loaded (to refresh them)
        let visible = visible_nodes(state);
        let mut requests = Vec::new();

        for node in visible {
            if !node.is_loaded {
                continue;
            }

            // Get the node to build the request
            if let Some(tree_node) = state.nodes.get(&node.id) {
                match tree_node {
                    TreeNode::Publication(p) => {
                        requests.push(AsyncRequest::LoadPublication {
                            addr: p.addr.clone(),
                            parent: p.parent,
                        });
                    }
                    TreeNode::Section(s) => {
                        requests.push(AsyncRequest::LoadSection {
                            addr: s.addr.clone(),
                            parent: s.parent.unwrap_or(NodeId::root()),
                        });
                    }
                }
            }
        }

        if requests.is_empty() {
            return CommandResult::NoOp;
        }

        // Mark all nodes as loading
        for req in &requests {
            match req {
                AsyncRequest::LoadPublication { addr, .. } => {
                    let id = NodeId::from_addr(addr);
                    if let Some(TreeNode::Publication(ref mut p)) = state.nodes.get_mut(&id) {
                        p.loading = true;
                    }
                }
                AsyncRequest::LoadSection { addr, .. } => {
                    let id = NodeId::from_addr(addr);
                    if let Some(TreeNode::Section(ref mut s)) = state.nodes.get_mut(&id) {
                        s.loading = true;
                    }
                }
                _ => {}
            }
        }

        CommandResult::NeedsAsync(AsyncRequest::LoadBatch { requests })
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

    // --- Compose mode commands ---

    fn enter_compose(&self, state: &mut TreeState) -> CommandResult {
        state.enter_compose();
        CommandResult::ModeChanged(AppMode::Compose)
    }

    fn enter_editor_compose(&self, state: &mut TreeState) -> CommandResult {
        state.enter_editor_compose();
        CommandResult::ModeChanged(AppMode::Compose)
    }

    fn execute_compose(&self, state: &mut TreeState, command: TreeCommand) -> CommandResult {
        match command {
            TreeCommand::ExitCompose | TreeCommand::Back => {
                state.exit_compose();
                CommandResult::ModeChanged(AppMode::Feed)
            }
            TreeCommand::InsertChar { c } => {
                state.compose.insert_char(c);
                CommandResult::StateChanged
            }
            TreeCommand::DeleteChar => {
                state.compose.delete_char();
                CommandResult::StateChanged
            }
            TreeCommand::DeleteCharForward => {
                state.compose.delete_char_forward();
                CommandResult::StateChanged
            }
            TreeCommand::CursorLeft => {
                state.compose.cursor_left();
                CommandResult::StateChanged
            }
            TreeCommand::CursorRight => {
                state.compose.cursor_right();
                CommandResult::StateChanged
            }
            TreeCommand::CursorHome => {
                state.compose.cursor_home();
                CommandResult::StateChanged
            }
            TreeCommand::CursorEnd => {
                state.compose.cursor_end();
                CommandResult::StateChanged
            }
            TreeCommand::NextField => {
                state.compose.next_field();
                CommandResult::StateChanged
            }
            TreeCommand::PrevField => {
                state.compose.prev_field();
                CommandResult::StateChanged
            }
            TreeCommand::CreateTags => {
                // Toggle tag mode - if in tag mode, exit; otherwise enter
                if state.compose.is_in_tag_mode() {
                    state.compose.exit_tag_mode();
                } else {
                    state.compose.enter_tag_mode();
                }
                CommandResult::StateChanged
            }
            TreeCommand::EndTags => {
                state.compose.exit_tag_mode();
                CommandResult::StateChanged
            }
            TreeCommand::AddSection => {
                state.compose.add_section();
                CommandResult::StateChanged
            }
            TreeCommand::RemoveSection => {
                state.compose.remove_section();
                CommandResult::StateChanged
            }
            TreeCommand::InsertNewline => {
                state.compose.insert_newline();
                CommandResult::StateChanged
            }
            TreeCommand::TogglePreview => {
                state.compose.toggle_preview();
                CommandResult::StateChanged
            }
            TreeCommand::Publish => {
                use crate::tree::state::ComposeState;

                if !state.compose.is_ready_to_publish() {
                    return CommandResult::Error(
                        "Need title and at least one section with title and content".to_string()
                    );
                }

                let tags = ComposeState::tags_to_nostr_format(&state.compose.tags);

                // NKBIP-01: Always create 30040 + 30041 events
                CommandResult::NeedsAsync(AsyncRequest::PublishPublication {
                    title: state.compose.title.clone(),
                    tags,
                    sections: state.compose.sections.clone(),
                })
            }
            TreeCommand::SaveDraft => {
                if !state.compose.has_content() {
                    return CommandResult::Error("No content to save as draft".to_string());
                }
                CommandResult::NeedsAsync(AsyncRequest::SaveDraft {
                    compose: state.compose.clone(),
                })
            }
            TreeCommand::ToggleComposeStyle => {
                state.toggle_compose_style();
                CommandResult::StateChanged
            }
            // Editor compose commands
            TreeCommand::EditorToggleMode => {
                state.editor_compose.insert_mode = !state.editor_compose.insert_mode;
                CommandResult::StateChanged
            }
            TreeCommand::EditorInsertChar { c } => {
                state.editor_compose.insert_char(c);
                CommandResult::StateChanged
            }
            TreeCommand::EditorInsertNewline => {
                state.editor_compose.insert_char('\n');
                CommandResult::StateChanged
            }
            TreeCommand::EditorBackspace => {
                state.editor_compose.delete_char_before();
                CommandResult::StateChanged
            }
            TreeCommand::EditorDelete => {
                state.editor_compose.delete_char_at();
                CommandResult::StateChanged
            }
            TreeCommand::EditorDeleteLine => {
                // Delete current line (simplified: just delete to end of line)
                state.editor_compose.cursor_home();
                while state.editor_compose.get_line(state.editor_compose.cursor_line)
                    .map(|l| !l.is_empty())
                    .unwrap_or(false)
                {
                    state.editor_compose.delete_char_at();
                }
                // Also delete the newline if not at end
                state.editor_compose.delete_char_at();
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorLeft => {
                state.editor_compose.cursor_left();
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorRight => {
                state.editor_compose.cursor_right();
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorUp => {
                use crate::tree::state::EditorViewMode;
                match state.editor_compose.view_mode {
                    EditorViewMode::Plain => state.editor_compose.cursor_up(),
                    // In JSON/Structured views, move view_cursor up (scroll follows)
                    EditorViewMode::Json | EditorViewMode::Structured => {
                        if state.editor_compose.view_cursor > 0 {
                            state.editor_compose.view_cursor -= 1;
                            // Scroll up if cursor goes above visible area
                            if state.editor_compose.view_cursor < state.editor_compose.view_scroll
                            {
                                state.editor_compose.view_scroll =
                                    state.editor_compose.view_cursor;
                            }
                        }
                    }
                }
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorDown => {
                use crate::tree::state::EditorViewMode;
                match state.editor_compose.view_mode {
                    EditorViewMode::Plain => state.editor_compose.cursor_down(),
                    // In JSON/Structured views, move view_cursor down
                    EditorViewMode::Json | EditorViewMode::Structured => {
                        state.editor_compose.view_cursor += 1;
                        state.editor_compose.clamp_view_cursor();
                    }
                }
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorHome => {
                state.editor_compose.cursor_home();
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorEnd => {
                state.editor_compose.cursor_end();
                CommandResult::StateChanged
            }
            TreeCommand::EditorCursorToEnd => {
                // Move to end of document
                let line_count = state.editor_compose.line_count();
                if line_count > 0 {
                    state.editor_compose.cursor_line = line_count - 1;
                    state.editor_compose.cursor_end();
                }
                CommandResult::StateChanged
            }
            TreeCommand::EditorInsertAfter => {
                state.editor_compose.cursor_right();
                state.editor_compose.insert_mode = true;
                CommandResult::StateChanged
            }
            TreeCommand::EditorInsertLineBelow => {
                state.editor_compose.cursor_end();
                state.editor_compose.insert_char('\n');
                state.editor_compose.insert_mode = true;
                CommandResult::StateChanged
            }
            TreeCommand::EditorInsertLineAbove => {
                state.editor_compose.cursor_home();
                state.editor_compose.insert_char('\n');
                state.editor_compose.cursor_up();
                state.editor_compose.insert_mode = true;
                CommandResult::StateChanged
            }
            TreeCommand::CycleEditorViewMode => {
                state.editor_compose.cycle_view_mode();
                CommandResult::StateChanged
            }
            TreeCommand::SetEditorViewMode { mode } => {
                state.editor_compose.set_view_mode(mode);
                CommandResult::StateChanged
            }
            // gg - go to top (works in all view modes)
            TreeCommand::MoveToFirst => {
                use crate::tree::state::EditorViewMode;
                match state.editor_compose.view_mode {
                    EditorViewMode::Plain => {
                        state.editor_compose.cursor_line = 0;
                        state.editor_compose.cursor_col = 0;
                        state.editor_compose.cursor = 0;
                    }
                    EditorViewMode::Json | EditorViewMode::Structured => {
                        state.editor_compose.view_cursor = 0;
                        state.editor_compose.view_scroll = 0;
                    }
                }
                CommandResult::StateChanged
            }
            // G - go to bottom
            TreeCommand::MoveToLast => {
                use crate::tree::state::EditorViewMode;
                match state.editor_compose.view_mode {
                    EditorViewMode::Plain => {
                        let line_count = state.editor_compose.line_count();
                        if line_count > 0 {
                            state.editor_compose.cursor_line = line_count - 1;
                            state.editor_compose.cursor_end();
                        }
                    }
                    // For JSON/Structured views, go to last line
                    EditorViewMode::Json | EditorViewMode::Structured => {
                        state.editor_compose.view_cursor = usize::MAX / 2;
                        state.editor_compose.clamp_view_cursor();
                    }
                }
                CommandResult::StateChanged
            }
            TreeCommand::Quit => CommandResult::Exit,
            // All other commands are no-ops in compose mode
            _ => CommandResult::NoOp,
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
