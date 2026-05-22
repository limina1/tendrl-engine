//! Tree state management
//!
//! Contains the pure state types for the tree view, including cursor position,
//! expanded nodes, selections, and view configuration.

use super::command::{all_commands, CommandInfo};
use super::node::{NodeId, TreeNode};
use super::undo::UndoStack;
use crate::identity::Identity;
use crate::publication::NAddr;
use crate::user_data::UserData;
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
    /// Compose mode: creating a new publication or note
    Compose,
}

impl AppMode {
    /// Get the display name for this mode
    pub fn name(&self) -> &'static str {
        match self {
            AppMode::Feed => "Feed",
            AppMode::Reader => "Reader",
            AppMode::Compose => "Compose",
        }
    }
}

/// The complete state of the tree view
#[derive(Debug, Clone)]
pub struct TreeState {
    /// Current application mode (Feed, Reader, or Compose)
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
    /// Command palette state
    pub command_palette: CommandPaletteState,
    /// Compose mode state (structured)
    pub compose: ComposeState,
    /// Editor compose mode state (single buffer)
    pub editor_compose: EditorComposeState,
    /// Whether to use editor compose (true) or structured compose (false)
    pub use_editor_compose: bool,
    /// Window manager for overlay windows
    pub windows: WindowManager,
    /// Identity/login state
    pub identity: Identity,
    /// Login dialog state (if open)
    pub login_dialog: Option<LoginDialogState>,
    /// User profile data (loaded after login)
    pub user_data: UserData,
    /// User data menu state (if open)
    pub user_data_menu: Option<UserDataMenuState>,
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
            command_palette: CommandPaletteState::new(),
            compose: ComposeState::new(),
            editor_compose: EditorComposeState::new(),
            use_editor_compose: false,
            windows: WindowManager::new(),
            identity: Identity::new(),
            login_dialog: None,
            user_data: UserData::new(),
            user_data_menu: None,
        }
    }

    /// Enter compose mode (structured)
    pub fn enter_compose(&mut self) {
        self.compose.reset();
        self.use_editor_compose = false;
        self.mode = AppMode::Compose;
    }

    /// Enter editor compose mode (single buffer)
    pub fn enter_editor_compose(&mut self) {
        self.editor_compose = EditorComposeState::new();
        self.use_editor_compose = true;
        self.mode = AppMode::Compose;
    }

    /// Toggle between structured and editor compose modes
    pub fn toggle_compose_style(&mut self) {
        self.use_editor_compose = !self.use_editor_compose;
    }

    /// Check if using editor compose mode
    pub fn is_editor_compose(&self) -> bool {
        self.mode == AppMode::Compose && self.use_editor_compose
    }

    /// Exit compose mode and return to feed
    pub fn exit_compose(&mut self) {
        self.mode = AppMode::Feed;
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

    /// Check if in compose mode
    pub fn is_compose_mode(&self) -> bool {
        self.mode == AppMode::Compose
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

/// View mode for the editor compose view
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorViewMode {
    /// Plain text editor (default)
    #[default]
    Plain,
    /// JSON preview showing the events that would be generated
    Json,
    /// Structured view showing the parsed document tree
    Structured,
}

impl EditorViewMode {
    /// Get the display name for this mode
    pub fn name(&self) -> &'static str {
        match self {
            EditorViewMode::Plain => "Plain",
            EditorViewMode::Json => "JSON",
            EditorViewMode::Structured => "Structured",
        }
    }

    /// Cycle to the next view mode
    pub fn next(&self) -> Self {
        match self {
            EditorViewMode::Plain => EditorViewMode::Json,
            EditorViewMode::Json => EditorViewMode::Structured,
            EditorViewMode::Structured => EditorViewMode::Plain,
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
    /// Whether the preview panel is focused (for scrolling)
    pub preview_focused: bool,
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
    /// Filter to show only drafts (unsigned publications)
    pub filter_drafts: bool,
}

impl ViewState {
    pub fn new() -> Self {
        ViewState {
            mode: ViewMode::default(),
            show_preview: true,
            preview_focused: false,
            preview_width: 50,
            tree_scroll: 0,
            preview_scroll: 0,
            current_section: 0,
            content_scroll: 0,
            search_query: None,
            filter: FilterMode::None,
            filter_drafts: false,
        }
    }

    /// Toggle preview visibility and focus
    /// Cycles: hidden -> visible (unfocused) -> visible (focused) -> hidden
    pub fn toggle_preview(&mut self) {
        if !self.show_preview {
            // Hidden -> visible (unfocused)
            self.show_preview = true;
            self.preview_focused = false;
        } else if !self.preview_focused {
            // Visible but unfocused -> focus it
            self.preview_focused = true;
        } else {
            // Visible and focused -> hide
            self.show_preview = false;
            self.preview_focused = false;
        }
    }

    /// Focus the preview panel (when visible)
    pub fn focus_preview(&mut self) {
        if self.show_preview {
            self.preview_focused = true;
        }
    }

    /// Unfocus the preview panel
    pub fn unfocus_preview(&mut self) {
        self.preview_focused = false;
    }

    /// Check if preview is focused
    pub fn is_preview_focused(&self) -> bool {
        self.show_preview && self.preview_focused
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

/// Focus position within compose mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComposeFocus {
    /// Focused on the publication title field
    #[default]
    Title,
    /// Focused on a tag name input (for publication tags)
    TagName,
    /// Focused on a tag value input (for publication tags)
    TagValue,
    /// Focused on a section title (index into sections vec)
    SectionTitle(usize),
    /// Focused on section tags name input
    SectionTagName(usize),
    /// Focused on section tags value input
    SectionTagValue(usize),
    /// Focused on a section content (index into sections vec)
    SectionContent(usize),
}

/// Auto-update mode for publications (NKBIP-01)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AutoUpdateMode {
    /// Automatically update to newer versions
    #[default]
    Yes,
    /// Ask user before updating
    Ask,
    /// Never auto-update
    No,
}

impl AutoUpdateMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AutoUpdateMode::Yes => "yes",
            AutoUpdateMode::Ask => "ask",
            AutoUpdateMode::No => "no",
        }
    }
}

/// A tag entry for compose mode
#[derive(Debug, Clone, Default)]
pub struct TagEntry {
    /// Tag name (e.g., "t" for hashtags, "author", or custom)
    pub name: String,
    /// Tag value (for "tags" name, comma-separated values become multiple "t" tags)
    pub value: String,
}

/// A section being composed (for 30041 events)
#[derive(Debug, Clone)]
pub struct SectionCompose {
    /// Section title (displayed; also emitted as `T` and `title` tags)
    pub title: String,
    /// Section content (the actual text)
    pub content: String,
    /// Section-specific tags
    pub tags: Vec<TagEntry>,
    /// Heading depth — 2 = top-level section (current default), 3+ = nested.
    /// Drives the recursive 30040/30041 emission: any section with deeper
    /// children becomes a 30040 index in addition to its own 30041.
    pub level: u8,
    /// Stable opaque d-tag (nanoid). `None` until first mint; once set it
    /// persists across edits so renaming the title does not break the
    /// addressable identity of the section's 30041 event.
    pub d_tag: Option<String>,
    /// Whether currently in tag creation mode for this section
    pub tag_mode: bool,
    /// Current tag name being typed
    pub current_tag_name: String,
    /// Current tag value being typed
    pub current_tag_value: String,
}

impl Default for SectionCompose {
    fn default() -> Self {
        Self {
            title: String::new(),
            content: String::new(),
            tags: Vec::new(),
            level: 2,
            d_tag: None,
            tag_mode: false,
            current_tag_name: String::new(),
            current_tag_value: String::new(),
        }
    }
}

/// State for compose mode (NKBIP-01 publications)
///
/// Creates kind 30040 (publication index) and kind 30041 (sections).
/// The 30040 event has NO content - only title, tags, and `a` tag references.
/// The 30041 events contain the actual content.
#[derive(Debug, Clone, Default)]
pub struct ComposeState {
    /// Current focus position
    pub focus: ComposeFocus,
    /// Publication title (displayed; also emitted as `T` and `title` tags)
    pub title: String,
    /// Stable opaque d-tag (nanoid). `None` until first mint; once set it
    /// persists across title edits so the addressable identity of the 30040
    /// index event survives renames. Seeded from `source_publication_addr`
    /// when loading an existing publication for edit.
    pub d_tag: Option<String>,
    /// Tags for the publication (30040)
    pub tags: Vec<TagEntry>,
    /// Sections (30041 events)
    pub sections: Vec<SectionCompose>,
    /// Whether currently in tag creation mode
    pub tag_mode: bool,
    /// Current tag name being typed (in tag mode)
    pub current_tag_name: String,
    /// Current tag value being typed (in tag mode)
    pub current_tag_value: String,
    /// Cursor position within current text field
    pub cursor_pos: usize,
    /// Scroll offset for content display
    pub content_scroll: usize,
    /// Whether to show the event preview panel
    pub show_preview: bool,
    /// Auto-update preference for the publication
    pub auto_update: AutoUpdateMode,
}

impl ComposeState {
    /// Create a new empty compose state
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the compose state for a new composition
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Toggle the preview panel visibility
    pub fn toggle_preview(&mut self) {
        self.show_preview = !self.show_preview;
    }

    /// Get the current text field content based on focus
    pub fn current_text(&self) -> &str {
        match self.focus {
            ComposeFocus::Title => &self.title,
            ComposeFocus::TagName => &self.current_tag_name,
            ComposeFocus::TagValue => &self.current_tag_value,
            ComposeFocus::SectionTitle(idx) => {
                self.sections.get(idx).map(|s| s.title.as_str()).unwrap_or("")
            }
            ComposeFocus::SectionTagName(idx) => {
                self.sections.get(idx).map(|s| s.current_tag_name.as_str()).unwrap_or("")
            }
            ComposeFocus::SectionTagValue(idx) => {
                self.sections.get(idx).map(|s| s.current_tag_value.as_str()).unwrap_or("")
            }
            ComposeFocus::SectionContent(idx) => {
                self.sections.get(idx).map(|s| s.content.as_str()).unwrap_or("")
            }
        }
    }

    /// Get a mutable reference to the current text field
    pub fn current_text_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            ComposeFocus::Title => Some(&mut self.title),
            ComposeFocus::TagName => Some(&mut self.current_tag_name),
            ComposeFocus::TagValue => Some(&mut self.current_tag_value),
            ComposeFocus::SectionTitle(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.title)
            }
            ComposeFocus::SectionTagName(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.current_tag_name)
            }
            ComposeFocus::SectionTagValue(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.current_tag_value)
            }
            ComposeFocus::SectionContent(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.content)
            }
        }
    }

    /// Check if currently in any tag editing mode (publication or section tags)
    pub fn is_in_tag_mode(&self) -> bool {
        matches!(
            self.focus,
            ComposeFocus::TagName
                | ComposeFocus::TagValue
                | ComposeFocus::SectionTagName(_)
                | ComposeFocus::SectionTagValue(_)
        ) || self.tag_mode
    }

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor_pos;
        if let Some(text) = self.current_text_mut() {
            let pos = pos.min(text.len());
            text.insert(pos, c);
            self.cursor_pos = pos + 1;
        }
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let pos = self.cursor_pos;
            if let Some(text) = self.current_text_mut() {
                let pos = pos.min(text.len());
                if pos > 0 {
                    text.remove(pos - 1);
                    self.cursor_pos = pos - 1;
                }
            }
        }
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char_forward(&mut self) {
        let pos = self.cursor_pos;
        if let Some(text) = self.current_text_mut() {
            let pos = pos.min(text.len());
            if pos < text.len() {
                text.remove(pos);
            }
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        let len = self.current_text().len();
        if self.cursor_pos < len {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to beginning
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.current_text().len();
    }

    /// Move to next field
    /// Flow: Title -> [Tags if tag_mode] -> Section Title -> [Section Tags if tag_mode] -> Section Content -> next section...
    pub fn next_field(&mut self) {
        match self.focus {
            ComposeFocus::Title => {
                if self.tag_mode {
                    self.focus = ComposeFocus::TagName;
                } else if !self.sections.is_empty() {
                    self.focus = ComposeFocus::SectionTitle(0);
                } else {
                    // No sections yet, stay on title
                    self.focus = ComposeFocus::Title;
                }
            }
            ComposeFocus::TagName => {
                self.focus = ComposeFocus::TagValue;
            }
            ComposeFocus::TagValue => {
                // Add the current tag if both name and value are non-empty
                if !self.current_tag_name.is_empty() && !self.current_tag_value.is_empty() {
                    self.tags.push(TagEntry {
                        name: self.current_tag_name.clone(),
                        value: self.current_tag_value.clone(),
                    });
                    self.current_tag_name.clear();
                    self.current_tag_value.clear();
                }
                self.focus = ComposeFocus::TagName;
            }
            ComposeFocus::SectionTitle(idx) => {
                // Check if this section has tag mode enabled
                if self.sections.get(idx).map(|s| s.tag_mode).unwrap_or(false) {
                    self.focus = ComposeFocus::SectionTagName(idx);
                } else {
                    self.focus = ComposeFocus::SectionContent(idx);
                }
            }
            ComposeFocus::SectionTagName(idx) => {
                self.focus = ComposeFocus::SectionTagValue(idx);
            }
            ComposeFocus::SectionTagValue(idx) => {
                // Add the tag to the section
                if let Some(section) = self.sections.get_mut(idx) {
                    if !section.current_tag_name.is_empty() && !section.current_tag_value.is_empty() {
                        section.tags.push(TagEntry {
                            name: section.current_tag_name.clone(),
                            value: section.current_tag_value.clone(),
                        });
                        section.current_tag_name.clear();
                        section.current_tag_value.clear();
                    }
                }
                self.focus = ComposeFocus::SectionTagName(idx);
            }
            ComposeFocus::SectionContent(idx) => {
                if idx + 1 < self.sections.len() {
                    self.focus = ComposeFocus::SectionTitle(idx + 1);
                } else {
                    self.focus = ComposeFocus::Title;
                }
            }
        }
        self.cursor_pos = self.current_text().len();
    }

    /// Move to previous field
    pub fn prev_field(&mut self) {
        match self.focus {
            ComposeFocus::Title => {
                if !self.sections.is_empty() {
                    let last_idx = self.sections.len() - 1;
                    self.focus = ComposeFocus::SectionContent(last_idx);
                }
                // No sections, stay on title
            }
            ComposeFocus::TagName => {
                // In tag mode, Shift+Tab deletes last tag
                if !self.tags.is_empty() {
                    self.tags.pop();
                } else {
                    self.focus = ComposeFocus::Title;
                    self.tag_mode = false;
                }
            }
            ComposeFocus::TagValue => {
                self.focus = ComposeFocus::TagName;
            }
            ComposeFocus::SectionTitle(idx) => {
                if idx > 0 {
                    self.focus = ComposeFocus::SectionContent(idx - 1);
                } else if self.tag_mode {
                    self.focus = ComposeFocus::TagName;
                } else {
                    self.focus = ComposeFocus::Title;
                }
            }
            ComposeFocus::SectionTagName(idx) => {
                // Delete last section tag or exit section tag mode
                if let Some(section) = self.sections.get_mut(idx) {
                    if !section.tags.is_empty() {
                        section.tags.pop();
                    } else {
                        section.tag_mode = false;
                        self.focus = ComposeFocus::SectionTitle(idx);
                    }
                }
            }
            ComposeFocus::SectionTagValue(idx) => {
                self.focus = ComposeFocus::SectionTagName(idx);
            }
            ComposeFocus::SectionContent(idx) => {
                // Check if section has tag mode
                if self.sections.get(idx).map(|s| s.tag_mode).unwrap_or(false) {
                    self.focus = ComposeFocus::SectionTagName(idx);
                } else {
                    self.focus = ComposeFocus::SectionTitle(idx);
                }
            }
        }
        self.cursor_pos = self.current_text().len();
    }

    /// Enter tag creation mode (for publication or current section)
    pub fn enter_tag_mode(&mut self) {
        match self.focus {
            ComposeFocus::SectionTitle(idx) | ComposeFocus::SectionContent(idx) => {
                // Enter section tag mode
                if let Some(section) = self.sections.get_mut(idx) {
                    section.tag_mode = true;
                    self.focus = ComposeFocus::SectionTagName(idx);
                }
            }
            _ => {
                // Enter publication tag mode
                self.tag_mode = true;
                self.focus = ComposeFocus::TagName;
            }
        }
        self.cursor_pos = 0;
    }

    /// Exit tag creation mode
    pub fn exit_tag_mode(&mut self) {
        match self.focus {
            ComposeFocus::SectionTagName(idx) | ComposeFocus::SectionTagValue(idx) => {
                // Exit section tag mode
                if let Some(section) = self.sections.get_mut(idx) {
                    if !section.current_tag_name.is_empty() && !section.current_tag_value.is_empty() {
                        section.tags.push(TagEntry {
                            name: section.current_tag_name.clone(),
                            value: section.current_tag_value.clone(),
                        });
                    }
                    section.current_tag_name.clear();
                    section.current_tag_value.clear();
                    section.tag_mode = false;
                }
                self.focus = ComposeFocus::SectionContent(idx);
            }
            _ => {
                // Exit publication tag mode
                if !self.current_tag_name.is_empty() && !self.current_tag_value.is_empty() {
                    self.tags.push(TagEntry {
                        name: self.current_tag_name.clone(),
                        value: self.current_tag_value.clone(),
                    });
                }
                self.current_tag_name.clear();
                self.current_tag_value.clear();
                self.tag_mode = false;
                // Go to first section, or create one if none exist
                if self.sections.is_empty() {
                    self.sections.push(SectionCompose::default());
                }
                self.focus = ComposeFocus::SectionTitle(0);
            }
        }
        self.cursor_pos = self.current_text().len();
    }

    /// Add a new section
    pub fn add_section(&mut self) {
        let new_idx = self.sections.len();
        self.sections.push(SectionCompose::default());
        self.focus = ComposeFocus::SectionTitle(new_idx);
        self.cursor_pos = 0;
    }

    /// Remove the last section
    pub fn remove_section(&mut self) {
        if !self.sections.is_empty() {
            self.sections.pop();
            // Adjust focus if needed
            match self.focus {
                ComposeFocus::SectionTitle(idx)
                | ComposeFocus::SectionTagName(idx)
                | ComposeFocus::SectionTagValue(idx)
                | ComposeFocus::SectionContent(idx) => {
                    if idx >= self.sections.len() {
                        if self.sections.is_empty() {
                            self.focus = ComposeFocus::Title;
                        } else {
                            self.focus = ComposeFocus::SectionContent(self.sections.len() - 1);
                        }
                    }
                }
                _ => {}
            }
            self.cursor_pos = self.current_text().len();
        }
    }

    /// Insert a newline in content fields
    pub fn insert_newline(&mut self) {
        match self.focus {
            ComposeFocus::SectionContent(_) => {
                self.insert_char('\n');
            }
            _ => {
                // In other fields, newline moves to next field
                self.next_field();
            }
        }
    }

    /// Generate a d-tag from a title by normalizing it
    /// - Lowercase
    /// - Replace spaces with hyphens
    /// - Remove non-alphanumeric characters (except hyphens)
    /// - Collapse multiple hyphens
    pub fn generate_d_tag(title: &str) -> String {
        let normalized: String = title
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c
                } else if c.is_whitespace() || c == '-' || c == '_' {
                    '-'
                } else {
                    '-'
                }
            })
            .collect();

        // Collapse multiple hyphens and trim
        let mut result = String::new();
        let mut last_was_hyphen = false;
        for c in normalized.chars() {
            if c == '-' {
                if !last_was_hyphen && !result.is_empty() {
                    result.push(c);
                    last_was_hyphen = true;
                }
            } else {
                result.push(c);
                last_was_hyphen = false;
            }
        }
        // Trim trailing hyphen
        result.trim_end_matches('-').to_string()
    }

    /// Get the publication d-tag, minting one on first call.
    ///
    /// The d-tag is an opaque nanoid (21-char URL-safe) so the publication's
    /// addressable identity is decoupled from the title and survives renames.
    /// Once minted, the value is stored on `self.d_tag` and returned on every
    /// subsequent call. Seed `self.d_tag` from a loaded event when entering
    /// edit mode so re-publishing updates that event rather than forking.
    pub fn publication_d_tag(&mut self) -> String {
        self.d_tag
            .get_or_insert_with(crate::publication::mint_d_tag)
            .clone()
    }

    /// Read-only variant: returns the publication d-tag if one has been
    /// minted, otherwise `None`. Use this for preview / display code paths
    /// that should not have the side-effect of minting.
    pub fn publication_d_tag_or_pending(&self) -> Option<&str> {
        self.d_tag.as_deref()
    }

    /// Get a section's d-tag, minting one on first call.
    ///
    /// Each section carries its own stable nanoid so child events can outlive
    /// title or position changes. Out-of-range indices return a deterministic
    /// `"pending-s{idx}"` placeholder rather than panicking — this only
    /// surfaces in preview paths before the section list is finalised.
    pub fn section_d_tag(&mut self, section_idx: usize) -> String {
        match self.sections.get_mut(section_idx) {
            Some(section) => section
                .d_tag
                .get_or_insert_with(crate::publication::mint_d_tag)
                .clone(),
            None => format!("pending-s{}", section_idx),
        }
    }

    /// Read-only variant for previews — returns the section d-tag only if
    /// one has been minted.
    pub fn section_d_tag_or_pending(&self, section_idx: usize) -> Option<&str> {
        self.sections.get(section_idx).and_then(|s| s.d_tag.as_deref())
    }

    /// Convert tags to the format expected by Nostr events
    ///
    /// Syntax:
    /// - `[tags] [val1, val2]` → `["t", "val1"]`, `["t", "val2"]` (expands to multiple t tags)
    /// - `[name] [val1, val2, val3]` → `["name", "val1", "val2", "val3"]` (multi-value single tag)
    /// - `[name] [value]` → `["name", "value"]` (simple tag)
    pub fn tags_to_nostr_format(tags: &[TagEntry]) -> Vec<Vec<String>> {
        let mut result = Vec::new();
        for tag in tags {
            if tag.name == "tags" {
                // Special case: "tags" expands comma-separated values into individual "t" tags
                for t in tag.value.split(',') {
                    let t = t.trim();
                    if !t.is_empty() {
                        result.push(vec!["t".to_string(), t.to_string()]);
                    }
                }
            } else if tag.value.contains(',') {
                // Multi-value tag: [name] [val1, val2, val3] → ["name", "val1", "val2", "val3"]
                let mut tag_vec = vec![tag.name.clone()];
                for v in tag.value.split(',') {
                    let v = v.trim();
                    if !v.is_empty() {
                        tag_vec.push(v.to_string());
                    }
                }
                result.push(tag_vec);
            } else {
                // Simple tag: [name] [value] → ["name", "value"]
                result.push(vec![tag.name.clone(), tag.value.clone()]);
            }
        }
        result
    }

    /// Check if there's any content to publish
    /// For NKBIP-01, we need at least a title and one section
    pub fn has_content(&self) -> bool {
        !self.title.is_empty() && !self.sections.is_empty()
    }

    /// Check if ready to publish (has title and at least one section with content)
    pub fn is_ready_to_publish(&self) -> bool {
        !self.title.is_empty()
            && !self.sections.is_empty()
            && self.sections.iter().all(|s| !s.title.is_empty() && !s.content.is_empty())
    }

    /// Generate a preview of the current event based on focus
    /// - When focused on publication fields (Title, Tags) → shows 30040 event
    /// - When focused on section fields → shows that section's 30041 event
    ///
    /// Takes `&mut self` so the preview can show real d-tags — they are
    /// lazily minted on first call to `publication_d_tag`/`section_d_tag`
    /// and persist for the lifetime of the compose state.
    pub fn preview_event_json(&mut self) -> String {
        self.preview_event_json_with_pubkey(None)
    }

    /// Generate a preview of the current event with an optional pubkey
    pub fn preview_event_json_with_pubkey(&mut self, pubkey: Option<&str>) -> String {
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Determine which event to show based on focus
        let section_idx = match self.focus {
            ComposeFocus::SectionTitle(idx)
            | ComposeFocus::SectionTagName(idx)
            | ComposeFocus::SectionTagValue(idx)
            | ComposeFocus::SectionContent(idx) => Some(idx),
            _ => None,
        };

        if let Some(idx) = section_idx {
            // Show the section's 30041 event
            self.preview_section_json(idx, now, pubkey)
        } else {
            // Show the publication's 30040 event
            self.preview_publication_json(now, pubkey)
        }
    }

    /// Generate JSON for the 30040 publication event
    fn preview_publication_json(&mut self, now: u64, pubkey: Option<&str>) -> String {
        use serde_json::{json, Value};

        let pub_d_tag = self.publication_d_tag();
        let pubkey_display = pubkey.unwrap_or("<not signed in>");

        // Build section `a` tag references
        let mut section_a_tags: Vec<Value> = Vec::new();
        for i in 0..self.sections.len() {
            let section_d_tag = self.section_d_tag(i);
            let a_tag_pubkey = pubkey.unwrap_or("<pubkey>");
            section_a_tags.push(json!(["a", format!("30041:{}:{}", a_tag_pubkey, section_d_tag), ""]));
        }

        // Build publication tags
        let mut pub_tags: Vec<Value> = vec![
            json!(["d", &pub_d_tag]),
        ];

        if !self.title.is_empty() {
            // `title` = raw display; `T` = normalized slug for `#T` queries.
            pub_tags.push(json!(["title", &self.title]));
            pub_tags.push(json!(["T", Self::generate_d_tag(&self.title)]));
        }

        // Add custom tags
        for tag_vec in Self::tags_to_nostr_format(&self.tags) {
            pub_tags.push(json!(tag_vec));
        }

        // Add section references
        pub_tags.extend(section_a_tags);

        // Add auto-update tag
        pub_tags.push(json!(["auto-update", self.auto_update.as_str()]));

        let pub_event = json!({
            "id": "<unsigned>",
            "pubkey": pubkey_display,
            "created_at": now,
            "kind": 30040,
            "tags": pub_tags,
            "content": "",  // MUST be empty for 30040
            "sig": "<unsigned>"
        });

        serde_json::to_string_pretty(&pub_event).unwrap_or_else(|_| "{}".to_string())
    }

    /// Generate JSON for a section's 30041 event
    fn preview_section_json(&mut self, idx: usize, now: u64, pubkey: Option<&str>) -> String {
        use serde_json::{json, Value};

        // Mint the d-tag first so the immutable borrow below doesn't
        // conflict with the &mut needed for minting.
        let section_d_tag = self.section_d_tag(idx);
        let section = match self.sections.get(idx) {
            Some(s) => s,
            None => return "{}".to_string(),
        };
        let pubkey_display = pubkey.unwrap_or("<not signed in>");

        // Build section tags
        let mut section_tags: Vec<Value> = vec![
            json!(["d", &section_d_tag]),
        ];

        if !section.title.is_empty() {
            // `title` = raw display; `T` = normalized slug for `#T` queries.
            section_tags.push(json!(["title", &section.title]));
            section_tags.push(json!(["T", Self::generate_d_tag(&section.title)]));
        }

        // Add section-specific tags
        for tag_vec in Self::tags_to_nostr_format(&section.tags) {
            section_tags.push(json!(tag_vec));
        }

        let section_event = json!({
            "id": "<unsigned>",
            "pubkey": pubkey_display,
            "created_at": now,
            "kind": 30041,
            "tags": section_tags,
            "content": &section.content,
            "sig": "<unsigned>"
        });

        serde_json::to_string_pretty(&section_event).unwrap_or_else(|_| "{}".to_string())
    }
}

// --- Block-based Compose Model ---

/// The kind of a compose block
#[derive(Debug, Clone)]
pub enum BlockKind {
    /// User-authored editable content
    Editable { content: String, cursor: usize },
    /// Read-only reference to someone else's section
    Imported {
        source_addr: NAddr,
        content: String,
        author: String,
        fork_requested: bool,
    },
    /// Editable copy of someone else's section, with lineage
    Forked {
        original_addr: NAddr,
        content: String,
        cursor: usize,
        original_author: String,
    },
}

/// A single block in a block-based composition
#[derive(Debug, Clone)]
pub struct ComposeBlock {
    pub block_id: usize,
    pub kind: BlockKind,
    pub title: String,
    pub tags: Vec<TagEntry>,
    pub collapsed: bool,
}

impl ComposeBlock {
    /// Returns the content from any block variant
    pub fn content(&self) -> &str {
        match &self.kind {
            BlockKind::Editable { content, .. } => content,
            BlockKind::Imported { content, .. } => content,
            BlockKind::Forked { content, .. } => content,
        }
    }

    /// Returns true for Editable and Forked blocks
    pub fn is_editable(&self) -> bool {
        matches!(self.kind, BlockKind::Editable { .. } | BlockKind::Forked { .. })
    }

    /// Returns cursor position for editable block kinds
    pub fn cursor(&self) -> Option<usize> {
        match &self.kind {
            BlockKind::Editable { cursor, .. } => Some(*cursor),
            BlockKind::Forked { cursor, .. } => Some(*cursor),
            BlockKind::Imported { .. } => None,
        }
    }
}

/// State for block-based composition (supports mixed editable/imported/forked blocks)
pub struct ComposeBlockState {
    pub blocks: Vec<ComposeBlock>,
    pub block_cursor: usize,
    next_block_id: usize,
    pub title: String,
    pub tags: Vec<TagEntry>,
    pub auto_update: AutoUpdateMode,
    /// When this draft was seeded from an existing 30040, this is its NAddr.
    /// Used to emit a `fork`-marker `a` tag on the new publication index so
    /// the lineage is queryable per NIP-54.
    pub source_publication_addr: Option<NAddr>,
    /// Event id of the specific 30040 version we forked from. Used to emit
    /// the matching `fork`-marker `e` tag.
    pub source_publication_event_id: Option<String>,
}

impl ComposeBlockState {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            block_cursor: 0,
            next_block_id: 0,
            title: String::new(),
            tags: Vec::new(),
            auto_update: AutoUpdateMode::default(),
            source_publication_addr: None,
            source_publication_event_id: None,
        }
    }

    /// Append an empty editable block
    pub fn add_editable(&mut self) {
        let id = self.next_block_id;
        self.next_block_id += 1;
        self.blocks.push(ComposeBlock {
            block_id: id,
            kind: BlockKind::Editable {
                content: String::new(),
                cursor: 0,
            },
            title: String::new(),
            tags: Vec::new(),
            collapsed: false,
        });
        self.block_cursor = self.blocks.len() - 1;
    }

    /// Append an imported (read-only reference) block
    pub fn add_imported(&mut self, addr: NAddr, content: String, author: String, title: String) {
        let id = self.next_block_id;
        self.next_block_id += 1;
        self.blocks.push(ComposeBlock {
            block_id: id,
            kind: BlockKind::Imported {
                source_addr: addr,
                content,
                author,
                fork_requested: false,
            },
            title,
            tags: Vec::new(),
            collapsed: false,
        });
        self.block_cursor = self.blocks.len() - 1;
    }

    /// Toggle a block between Imported and Forked states.
    ///
    /// Imported → Forked: copies content, resets cursor.
    /// Forked → Imported: only if content is unchanged from original.
    pub fn toggle_fork(&mut self, idx: usize) {
        let block = match self.blocks.get_mut(idx) {
            Some(b) => b,
            None => return,
        };
        match block.kind.clone() {
            BlockKind::Imported {
                source_addr,
                content,
                author,
                ..
            } => {
                block.kind = BlockKind::Forked {
                    original_addr: source_addr,
                    content: content.clone(),
                    cursor: 0,
                    original_author: author,
                };
            }
            BlockKind::Forked {
                original_addr,
                content,
                original_author,
                ..
            } => {
                // Only revert if content is unchanged (would need original content)
                // For now, always allow back-toggle
                block.kind = BlockKind::Imported {
                    source_addr: original_addr,
                    content,
                    author: original_author,
                    fork_requested: false,
                };
            }
            BlockKind::Editable { .. } => {} // no-op for editable blocks
        }
    }

    /// Move a block up in the list
    pub fn move_block_up(&mut self, idx: usize) {
        if idx > 0 && idx < self.blocks.len() {
            self.blocks.swap(idx, idx - 1);
            if self.block_cursor == idx {
                self.block_cursor = idx - 1;
            }
        }
    }

    /// Move a block down in the list
    pub fn move_block_down(&mut self, idx: usize) {
        if idx + 1 < self.blocks.len() {
            self.blocks.swap(idx, idx + 1);
            if self.block_cursor == idx {
                self.block_cursor = idx + 1;
            }
        }
    }

    /// Remove a block, clamping the cursor
    pub fn remove_block(&mut self, idx: usize) {
        if idx < self.blocks.len() {
            self.blocks.remove(idx);
            if self.blocks.is_empty() {
                self.block_cursor = 0;
            } else if self.block_cursor >= self.blocks.len() {
                self.block_cursor = self.blocks.len() - 1;
            }
        }
    }

    /// Check if ready to publish (has title + at least one block)
    pub fn is_ready_to_publish(&self) -> bool {
        !self.title.is_empty() && !self.blocks.is_empty()
    }

    /// Generate the d-tag for the publication
    pub fn publication_d_tag(&self) -> String {
        if self.title.is_empty() {
            "untitled".to_string()
        } else {
            ComposeState::generate_d_tag(&self.title)
        }
    }

    /// Generate the d-tag for a block at the given index.
    ///
    /// Pattern: `pub-d-tag-section-d-tag`
    /// For forked blocks, uses the original addr's d-tag with a "fork-" prefix.
    pub fn block_d_tag(&self, idx: usize) -> String {
        let pub_d_tag = self.publication_d_tag();
        match self.blocks.get(idx) {
            Some(block) => {
                let section_part = if block.title.is_empty() {
                    format!("section-{}", idx)
                } else {
                    ComposeState::generate_d_tag(&block.title)
                };
                match &block.kind {
                    BlockKind::Forked { original_addr, .. } => {
                        format!("{}-fork-{}", pub_d_tag, original_addr.d_tag)
                    }
                    _ => format!("{}-{}", pub_d_tag, section_part),
                }
            }
            None => format!("{}-section-{}", pub_d_tag, idx),
        }
    }
}

/// State for editor-style compose mode (single text buffer)
///
/// Unlike structured ComposeState, this provides a free-form text editor
/// where structure (headings, code blocks, attributes) is detected automatically
/// using the parser module.
#[derive(Debug, Clone)]
pub struct EditorComposeState {
    /// The raw text content
    pub content: String,
    /// Content format mode
    pub mode: super::node::ContentMode,
    /// View mode (Plain, JSON, Structured)
    pub view_mode: EditorViewMode,
    /// Cursor position (byte offset)
    pub cursor: usize,
    /// Cursor line (0-indexed)
    pub cursor_line: usize,
    /// Cursor column (0-indexed)
    pub cursor_col: usize,
    /// Scroll offset (first visible line) for Plain view
    pub scroll: usize,
    /// Whether in insert mode (vs normal mode)
    pub insert_mode: bool,
    /// Scroll offset for read-only views (JSON/Structured)
    pub view_scroll: usize,
    /// Cursor position within read-only views (JSON/Structured) - 0-indexed line
    pub view_cursor: usize,
    /// Cached parsed document (updated on content change)
    parsed: Option<super::parser::ParsedDocument>,
}

impl Default for EditorComposeState {
    fn default() -> Self {
        Self {
            content: String::new(),
            mode: super::node::ContentMode::Markdown,
            view_mode: EditorViewMode::default(),
            cursor: 0,
            cursor_line: 0,
            cursor_col: 0,
            scroll: 0,
            insert_mode: true,
            view_scroll: 0,
            view_cursor: 0,
            parsed: None,
        }
    }
}

impl EditorComposeState {
    /// Create a new editor compose state
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with specific content mode
    pub fn with_mode(mode: super::node::ContentMode) -> Self {
        Self {
            mode,
            ..Self::default()
        }
    }

    /// Set content and reparse
    pub fn set_content(&mut self, content: String) {
        self.content = content;
        self.invalidate_parse();
        self.update_cursor_position();
    }

    /// Get the parsed document (parses on first access after change)
    pub fn parsed(&mut self) -> &super::parser::ParsedDocument {
        if self.parsed.is_none() {
            self.parsed = Some(super::parser::ParsedDocument::parse(&self.content, self.mode));
        }
        self.parsed.as_ref().unwrap()
    }

    /// Invalidate the cached parse (call after content changes)
    fn invalidate_parse(&mut self) {
        self.parsed = None;
    }

    /// Update cursor line/col from byte offset
    fn update_cursor_position(&mut self) {
        let before_cursor = &self.content[..self.cursor.min(self.content.len())];
        self.cursor_line = before_cursor.lines().count().saturating_sub(1);
        self.cursor_col = before_cursor.lines().last().map(|l| l.len()).unwrap_or(0);
    }

    /// Get total line count
    pub fn line_count(&self) -> usize {
        self.content.lines().count().max(1)
    }

    /// Get a specific line by number (0-indexed)
    pub fn get_line(&self, line_num: usize) -> Option<&str> {
        self.content.lines().nth(line_num)
    }

    /// Insert a character at cursor
    pub fn insert_char(&mut self, c: char) {
        if self.cursor <= self.content.len() {
            self.content.insert(self.cursor, c);
            self.cursor += c.len_utf8();
            self.invalidate_parse();
            self.update_cursor_position();
        }
    }

    /// Insert a string at cursor
    pub fn insert_str(&mut self, s: &str) {
        if self.cursor <= self.content.len() {
            self.content.insert_str(self.cursor, s);
            self.cursor += s.len();
            self.invalidate_parse();
            self.update_cursor_position();
        }
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char_before(&mut self) {
        if self.cursor > 0 {
            // Find the start of the previous character
            let prev_char_start = self.content[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.content.remove(prev_char_start);
            self.cursor = prev_char_start;
            self.invalidate_parse();
            self.update_cursor_position();
        }
    }

    /// Delete character at cursor (delete)
    pub fn delete_char_at(&mut self) {
        if self.cursor < self.content.len() {
            self.content.remove(self.cursor);
            self.invalidate_parse();
            // Cursor position unchanged
        }
    }

    /// Move cursor up one line
    pub fn cursor_up(&mut self) {
        if self.cursor_line > 0 {
            let target_line = self.cursor_line - 1;
            self.move_to_line(target_line);
        }
    }

    /// Move cursor down one line
    pub fn cursor_down(&mut self) {
        if self.cursor_line < self.line_count().saturating_sub(1) {
            let target_line = self.cursor_line + 1;
            self.move_to_line(target_line);
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor > 0 {
            // Move to start of previous character
            self.cursor = self.content[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.update_cursor_position();
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.cursor < self.content.len() {
            // Move past current character
            let c = self.content[self.cursor..].chars().next().unwrap();
            self.cursor += c.len_utf8();
            self.update_cursor_position();
        }
    }

    /// Move cursor to start of line
    pub fn cursor_home(&mut self) {
        self.move_to_line(self.cursor_line);
        self.cursor_col = 0;
    }

    /// Move cursor to end of line
    pub fn cursor_end(&mut self) {
        if let Some(line) = self.get_line(self.cursor_line) {
            let line_len = line.len();
            self.cursor_col = line_len;
            // Recalculate byte offset
            let mut offset = 0;
            for (i, l) in self.content.lines().enumerate() {
                if i == self.cursor_line {
                    self.cursor = offset + line_len;
                    break;
                }
                offset += l.len() + 1; // +1 for newline
            }
        }
    }

    /// Move to a specific line, preserving column if possible
    fn move_to_line(&mut self, target_line: usize) {
        let target_col = self.cursor_col;
        let mut offset = 0;

        for (i, line) in self.content.lines().enumerate() {
            if i == target_line {
                let actual_col = target_col.min(line.len());
                self.cursor = offset + actual_col;
                self.cursor_line = target_line;
                self.cursor_col = actual_col;
                return;
            }
            offset += line.len() + 1; // +1 for newline
        }
    }

    /// Scroll view to ensure cursor is visible
    pub fn ensure_cursor_visible(&mut self, visible_lines: usize) {
        if self.cursor_line < self.scroll {
            self.scroll = self.cursor_line;
        } else if self.cursor_line >= self.scroll + visible_lines {
            self.scroll = self.cursor_line - visible_lines + 1;
        }
    }

    /// Get the line type at cursor position
    pub fn current_line_type(&mut self) -> super::parser::LineType {
        let cursor_line = self.cursor_line;
        let parsed = self.parsed();
        parsed
            .lines
            .get(cursor_line)
            .map(|l| l.line_type.clone())
            .unwrap_or(super::parser::LineType::Empty)
    }

    /// Get sections extracted from the document
    pub fn sections(&mut self) -> Vec<super::parser::Section> {
        // Need to clone to avoid borrow issues
        let parsed = super::parser::ParsedDocument::parse(&self.content, self.mode);
        parsed.sections()
    }

    /// Get code blocks from the document
    pub fn code_blocks(&mut self) -> Vec<(usize, usize, String)> {
        let parsed = super::parser::ParsedDocument::parse(&self.content, self.mode);
        parsed.code_blocks()
    }

    /// Check if cursor is inside a code block
    pub fn cursor_in_code_block(&mut self) -> Option<(usize, usize, String)> {
        let blocks = self.code_blocks();
        for (start, end, lang) in blocks {
            if self.cursor_line >= start && self.cursor_line <= end {
                return Some((start, end, lang));
            }
        }
        None
    }

    /// Get line count for JSON view (based on parsed sections)
    pub fn json_view_line_count(&mut self) -> usize {
        let sections = self.parsed().sections();
        // Lines in JSON view:
        // Fixed: 2 (header+blank) + 1 (index comment) + 1 (open) + 3 (kind,content,tags) + 1 (d-tag) = 8
        // Per section in index: 3 lines (open bracket, value, close bracket)
        // Fixed end: 2 (close tags, close brace) + 1 (blank) = 3
        // Per content section: 11 lines (comment, open, kind, content key, value, tags, d-tag, title, close, close, blank)
        let section_count = sections.len();
        11 + section_count * 14
    }

    /// Get line count for Structured view (based on parsed sections)
    pub fn structured_view_line_count(&mut self) -> usize {
        let sections = self.parsed().sections();
        // Lines in Structured view:
        // Fixed: header + blank + 3 stats lines + blank + 3 index lines + blank = 10
        // Per section: 3 lines (header, d-tag, lines)
        10 + sections.len() * 3
    }

    /// Clamp view_cursor to valid range based on current view mode
    pub fn clamp_view_cursor(&mut self) {
        let max_lines = match self.view_mode {
            EditorViewMode::Plain => return, // Plain mode doesn't use view_cursor
            EditorViewMode::Json => self.json_view_line_count(),
            EditorViewMode::Structured => self.structured_view_line_count(),
        };
        self.view_cursor = self.view_cursor.min(max_lines.saturating_sub(1));
    }

    /// Cycle to the next view mode
    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.next();
        // Reset scroll position for the new view
        self.view_scroll = 0;
        self.view_cursor = 0;
        // JSON/Structured views are read-only, so exit insert mode
        if self.view_mode != EditorViewMode::Plain {
            self.insert_mode = false;
        }
    }

    /// Set the view mode
    pub fn set_view_mode(&mut self, mode: EditorViewMode) {
        self.view_mode = mode;
        // Reset scroll position for the new view
        self.view_scroll = 0;
        self.view_cursor = 0;
        // JSON/Structured views are read-only, so exit insert mode
        if self.view_mode != EditorViewMode::Plain {
            self.insert_mode = false;
        }
    }

    /// Convert editor compose state to structured ComposeState for publishing
    ///
    /// Parses the document to extract sections and their content.
    pub fn to_compose_state(&self) -> ComposeState {
        let parsed = super::parser::ParsedDocument::parse(&self.content, self.mode);
        let sections = parsed.sections();
        let lines: Vec<&str> = self.content.lines().collect();

        let mut compose = ComposeState::new();

        // If no sections found, treat the entire content as a single section
        if sections.is_empty() {
            // Use first line as title if it looks like a heading, otherwise use default
            let (title, content) = if let Some(first_line) = lines.first() {
                let trimmed = first_line.trim();
                if trimmed.starts_with('#') || trimmed.starts_with('*') {
                    // Strip markdown/org heading markers
                    let title = trimmed.trim_start_matches('#').trim_start_matches('*').trim();
                    let content = lines.iter().skip(1).copied().collect::<Vec<_>>().join("\n");
                    (title.to_string(), content)
                } else {
                    ("Untitled".to_string(), self.content.clone())
                }
            } else {
                ("Untitled".to_string(), self.content.clone())
            };

            compose.title = title.clone();
            compose.sections.push(SectionCompose {
                title,
                content,
                ..Default::default()
            });
        } else {
            // Use first section's title as publication title
            compose.title = if sections[0].title.is_empty() {
                "Untitled".to_string()
            } else {
                sections[0].title.clone()
            };

            // Convert each parsed section to SectionCompose
            for section in &sections {
                // Extract content lines for this section (skip the heading line)
                let content_start = section.start_line + 1;
                let content_end = section.end_line;

                let content = if content_start <= content_end && content_end < lines.len() {
                    lines[content_start..=content_end].join("\n")
                } else if content_start < lines.len() {
                    lines[content_start..].join("\n")
                } else {
                    String::new()
                };

                compose.sections.push(SectionCompose {
                    title: section.title.clone(),
                    content: content.trim().to_string(),
                    ..Default::default()
                });
            }
        }

        compose
    }

    /// Check if there's publishable content
    pub fn has_content(&self) -> bool {
        !self.content.trim().is_empty()
    }

    /// Check if ready to publish (has non-empty content)
    pub fn is_ready_to_publish(&self) -> bool {
        !self.content.trim().is_empty()
    }
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

/// State for the command palette (M-x style menu)
#[derive(Debug, Clone)]
pub struct CommandPaletteState {
    /// Whether the command palette is visible
    pub visible: bool,
    /// Search/filter text
    pub query: String,
    /// Selected index in the filtered list
    pub selected: usize,
    /// Cached filtered commands
    pub filtered_commands: Vec<CommandInfo>,
}

impl CommandPaletteState {
    pub fn new() -> Self {
        CommandPaletteState {
            visible: false,
            query: String::new(),
            selected: 0,
            filtered_commands: all_commands(),
        }
    }

    /// Open the command palette
    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.selected = 0;
        self.filtered_commands = all_commands();
    }

    /// Close the command palette
    pub fn close(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected = 0;
    }

    /// Update the search query and filter commands
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.filter_commands();
        // Reset selection if it's out of bounds
        if self.selected >= self.filtered_commands.len() {
            self.selected = 0;
        }
    }

    /// Add a character to the query
    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.filter_commands();
        if self.selected >= self.filtered_commands.len() {
            self.selected = 0;
        }
    }

    /// Remove last character from query
    pub fn pop_char(&mut self) {
        self.query.pop();
        self.filter_commands();
        if self.selected >= self.filtered_commands.len() {
            self.selected = 0;
        }
    }

    /// Filter commands based on query (fuzzy match on name and description)
    fn filter_commands(&mut self) {
        let all = all_commands();
        if self.query.is_empty() {
            self.filtered_commands = all;
            return;
        }

        let query_lower = self.query.to_lowercase();
        self.filtered_commands = all
            .into_iter()
            .filter(|cmd| {
                let name_match = cmd.name.to_lowercase().contains(&query_lower);
                let desc_match = cmd.description.to_lowercase().contains(&query_lower);
                let cat_match = cmd.category.name().to_lowercase().contains(&query_lower);
                name_match || desc_match || cat_match
            })
            .collect();
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if !self.filtered_commands.is_empty() {
            if self.selected == 0 {
                self.selected = self.filtered_commands.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if !self.filtered_commands.is_empty() {
            self.selected = (self.selected + 1) % self.filtered_commands.len();
        }
    }

    /// Get the currently selected command
    pub fn selected_command(&self) -> Option<&CommandInfo> {
        self.filtered_commands.get(self.selected)
    }
}

impl Default for CommandPaletteState {
    fn default() -> Self {
        Self::new()
    }
}

/// A unique identifier for a window
pub type WindowId = String;

/// State for a single window overlay
#[derive(Debug, Clone)]
pub struct WindowState {
    /// Unique identifier for this window
    pub id: WindowId,
    /// Window title displayed in the border
    pub title: String,
    /// Content to display (text lines)
    pub content: Vec<String>,
    /// Current scroll offset (line number at top of view)
    pub scroll_offset: usize,
    /// Whether the window is read-only (no editing, just viewing)
    pub readonly: bool,
    /// Window width as percentage of screen (0-100)
    pub width_percent: u16,
    /// Window height as percentage of screen (0-100)
    pub height_percent: u16,
}

impl WindowState {
    /// Create a new window with the given content
    pub fn new(id: impl Into<String>, title: impl Into<String>, content: String) -> Self {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        WindowState {
            id: id.into(),
            title: title.into(),
            content: lines,
            scroll_offset: 0,
            readonly: true,
            width_percent: 80,
            height_percent: 80,
        }
    }

    /// Create a window for displaying JSON
    pub fn json(id: impl Into<String>, title: impl Into<String>, json: &str) -> Self {
        Self::new(id, title, json.to_string())
    }

    /// Total number of lines in the content
    pub fn total_lines(&self) -> usize {
        self.content.len()
    }

    /// Scroll down by n lines
    pub fn scroll_down(&mut self, n: usize, viewport_height: usize) {
        let max_scroll = self.total_lines().saturating_sub(viewport_height);
        self.scroll_offset = (self.scroll_offset + n).min(max_scroll);
    }

    /// Scroll up by n lines
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll to the top
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// Scroll to the bottom
    pub fn scroll_to_bottom(&mut self, viewport_height: usize) {
        self.scroll_offset = self.total_lines().saturating_sub(viewport_height);
    }

    /// Get visible lines for the current scroll position
    pub fn visible_lines(&self, viewport_height: usize) -> &[String] {
        let start = self.scroll_offset;
        let end = (start + viewport_height).min(self.content.len());
        &self.content[start..end]
    }
}

/// Manages multiple windows as overlays
#[derive(Debug, Clone, Default)]
pub struct WindowManager {
    /// Stack of open windows (last is on top)
    pub windows: Vec<WindowState>,
    /// Index of the currently focused window (if any)
    pub focused: Option<usize>,
}

impl WindowManager {
    pub fn new() -> Self {
        WindowManager {
            windows: Vec::new(),
            focused: None,
        }
    }

    /// Open a new window and focus it
    pub fn open(&mut self, window: WindowState) {
        self.windows.push(window);
        self.focused = Some(self.windows.len() - 1);
    }

    /// Close the focused window
    pub fn close_focused(&mut self) {
        if let Some(idx) = self.focused {
            if idx < self.windows.len() {
                self.windows.remove(idx);
                // Update focus
                if self.windows.is_empty() {
                    self.focused = None;
                } else {
                    self.focused = Some(idx.min(self.windows.len() - 1));
                }
            }
        }
    }

    /// Close a window by ID
    pub fn close_by_id(&mut self, id: &str) {
        if let Some(pos) = self.windows.iter().position(|w| w.id == id) {
            self.windows.remove(pos);
            // Update focus
            if self.windows.is_empty() {
                self.focused = None;
            } else if let Some(focused_idx) = self.focused {
                if focused_idx >= self.windows.len() {
                    self.focused = Some(self.windows.len() - 1);
                } else if focused_idx > pos {
                    self.focused = Some(focused_idx - 1);
                }
            }
        }
    }

    /// Close all windows
    pub fn close_all(&mut self) {
        self.windows.clear();
        self.focused = None;
    }

    /// Get the focused window
    pub fn focused_window(&self) -> Option<&WindowState> {
        self.focused.and_then(|idx| self.windows.get(idx))
    }

    /// Get the focused window mutably
    pub fn focused_window_mut(&mut self) -> Option<&mut WindowState> {
        self.focused.and_then(|idx| self.windows.get_mut(idx))
    }

    /// Check if any window is open
    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Check if a window is focused
    pub fn is_focused(&self) -> bool {
        self.focused.is_some() && !self.windows.is_empty()
    }

    /// Focus the next window (cycle through)
    pub fn focus_next(&mut self) {
        if !self.windows.is_empty() {
            self.focused = Some(match self.focused {
                Some(idx) => (idx + 1) % self.windows.len(),
                None => 0,
            });
        }
    }

    /// Focus the previous window (cycle through)
    pub fn focus_prev(&mut self) {
        if !self.windows.is_empty() {
            self.focused = Some(match self.focused {
                Some(0) => self.windows.len() - 1,
                Some(idx) => idx - 1,
                None => self.windows.len() - 1,
            });
        }
    }

    /// Scroll the focused window down
    pub fn scroll_down(&mut self, n: usize, viewport_height: usize) {
        if let Some(window) = self.focused_window_mut() {
            window.scroll_down(n, viewport_height);
        }
    }

    /// Scroll the focused window up
    pub fn scroll_up(&mut self, n: usize) {
        if let Some(window) = self.focused_window_mut() {
            window.scroll_up(n);
        }
    }

    /// Scroll focused window to top
    pub fn scroll_to_top(&mut self) {
        if let Some(window) = self.focused_window_mut() {
            window.scroll_to_top();
        }
    }

    /// Scroll focused window to bottom
    pub fn scroll_to_bottom(&mut self, viewport_height: usize) {
        if let Some(window) = self.focused_window_mut() {
            window.scroll_to_bottom(viewport_height);
        }
    }
}

/// A user data list type (NIP-51)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserDataListType {
    /// Profile metadata (kind 0, NIP-01)
    Profile,
    /// Follow list (kind 3, NIP-02)
    FollowList,
    /// Follow list raw JSON (for debugging)
    FollowListJson,
    /// Mute list (kind 10000, NIP-51)
    MuteList,
    /// Read/write relays (kind 10002, NIP-65)
    RelayList,
    /// Bookmarks (kind 10003, NIP-51)
    Bookmarks,
    /// Blocked relays (kind 10006, NIP-51)
    BlockedRelays,
    /// Search relays (kind 10007, NIP-51)
    SearchRelays,
    /// Relay sets (kind 30002, NIP-51)
    RelaySets,
}

impl UserDataListType {
    /// Get all list types in display order
    pub fn all() -> Vec<Self> {
        vec![
            Self::Profile,
            Self::FollowList,
            Self::FollowListJson,
            Self::MuteList,
            Self::RelayList,
            Self::Bookmarks,
            Self::BlockedRelays,
            Self::SearchRelays,
            Self::RelaySets,
        ]
    }

    /// Get the display name with kind number
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Profile => "Profile [kind:0]",
            Self::FollowList => "Follow list [kind:3]",
            Self::FollowListJson => "  -> Raw JSON (debug)",
            Self::MuteList => "Mute list [kind:10000]",
            Self::RelayList => "Read/write relays [kind:10002]",
            Self::Bookmarks => "Bookmarks [kind:10003]",
            Self::BlockedRelays => "Blocked relays [kind:10006]",
            Self::SearchRelays => "Search relays [kind:10007]",
            Self::RelaySets => "Relay sets [kind:30002]",
        }
    }

    /// Get the window title for this list type
    pub fn window_title(&self) -> &'static str {
        match self {
            Self::Profile => "Profile (NIP-01)",
            Self::FollowList => "Follow list (NIP-02)",
            Self::FollowListJson => "Follow list - Raw JSON",
            Self::MuteList => "Mute list (NIP-51)",
            Self::RelayList => "Read/write relays (NIP-65)",
            Self::Bookmarks => "Bookmarks (NIP-51)",
            Self::BlockedRelays => "Blocked relays (NIP-51)",
            Self::SearchRelays => "Search relays (NIP-51)",
            Self::RelaySets => "Relay sets (NIP-51)",
        }
    }
}

/// State for the user data menu (selection of NIP-51 lists)
#[derive(Debug, Clone)]
pub struct UserDataMenuState {
    /// Currently selected index
    pub selected: usize,
    /// Available list types
    pub items: Vec<UserDataListType>,
}

impl UserDataMenuState {
    /// Create a new user data menu
    pub fn new() -> Self {
        UserDataMenuState {
            selected: 0,
            items: UserDataListType::all(),
        }
    }

    /// Move selection up
    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Move selection down
    pub fn select_next(&mut self) {
        if self.selected < self.items.len().saturating_sub(1) {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }

    /// Get the currently selected list type
    pub fn selected_item(&self) -> Option<UserDataListType> {
        self.items.get(self.selected).copied()
    }
}

impl Default for UserDataMenuState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for the login dialog
#[derive(Debug, Clone, Default)]
pub struct LoginDialogState {
    /// Current input text (key or password)
    pub input: String,
    /// Cursor position within the input
    pub cursor_pos: usize,
    /// Error message to display (if any)
    pub error: Option<String>,
    /// Whether we're awaiting a password (vs initial key input)
    pub awaiting_password: bool,
    /// The ncryptsec that needs to be unlocked (if awaiting_password)
    pub pending_ncryptsec: Option<String>,
    /// Whether to mask the input (for password entry)
    pub mask_input: bool,
}

impl LoginDialogState {
    /// Create a new login dialog state
    pub fn new() -> Self {
        LoginDialogState {
            input: String::new(),
            cursor_pos: 0,
            error: None,
            awaiting_password: false,
            pending_ncryptsec: None,
            mask_input: false,
        }
    }

    /// Create a password prompt dialog
    pub fn password_prompt(ncryptsec: String) -> Self {
        LoginDialogState {
            input: String::new(),
            cursor_pos: 0,
            error: None,
            awaiting_password: true,
            pending_ncryptsec: Some(ncryptsec),
            mask_input: true,
        }
    }

    /// Insert a character at the cursor position
    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor_pos.min(self.input.len());
        self.input.insert(pos, c);
        self.cursor_pos = pos + 1;
        self.error = None; // Clear error on input
    }

    /// Delete character before cursor (backspace)
    pub fn delete_char(&mut self) {
        if self.cursor_pos > 0 {
            let pos = self.cursor_pos.min(self.input.len());
            if pos > 0 {
                self.input.remove(pos - 1);
                self.cursor_pos = pos - 1;
            }
        }
        self.error = None;
    }

    /// Delete character at cursor (delete key)
    pub fn delete_char_forward(&mut self) {
        let pos = self.cursor_pos.min(self.input.len());
        if pos < self.input.len() {
            self.input.remove(pos);
        }
        self.error = None;
    }

    /// Move cursor left
    pub fn cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            self.cursor_pos -= 1;
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self) {
        if self.cursor_pos < self.input.len() {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to beginning
    pub fn cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end
    pub fn cursor_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    /// Set an error message
    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    /// Get the display text (masked if password)
    pub fn display_text(&self) -> String {
        if self.mask_input {
            "*".repeat(self.input.len())
        } else {
            self.input.clone()
        }
    }

    /// Get the title for the dialog
    pub fn title(&self) -> &'static str {
        if self.awaiting_password {
            "Enter Password"
        } else {
            "Login"
        }
    }

    /// Get the placeholder text
    pub fn placeholder(&self) -> &'static str {
        if self.awaiting_password {
            "Password for ncryptsec..."
        } else {
            "npub1... / nsec1... / ncryptsec1..."
        }
    }
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

    #[test]
    fn test_compose_state_text_editing() {
        let mut compose = ComposeState::new();

        // Test character insertion
        compose.insert_char('H');
        compose.insert_char('e');
        compose.insert_char('l');
        compose.insert_char('l');
        compose.insert_char('o');
        assert_eq!(compose.title, "Hello");
        assert_eq!(compose.cursor_pos, 5);

        // Test cursor movement
        compose.cursor_left();
        assert_eq!(compose.cursor_pos, 4);
        compose.cursor_home();
        assert_eq!(compose.cursor_pos, 0);
        compose.cursor_end();
        assert_eq!(compose.cursor_pos, 5);

        // Test delete
        compose.delete_char();
        assert_eq!(compose.title, "Hell");
        assert_eq!(compose.cursor_pos, 4);
    }

    #[test]
    fn test_compose_state_field_navigation() {
        let mut compose = ComposeState::new();
        compose.title = "Test Title".to_string();

        // Start on title
        assert!(matches!(compose.focus, ComposeFocus::Title));

        // Tab without sections stays on title
        compose.next_field();
        assert!(matches!(compose.focus, ComposeFocus::Title));

        // Add a section
        compose.add_section();
        assert!(matches!(compose.focus, ComposeFocus::SectionTitle(0)));

        // Navigate to content
        compose.next_field();
        assert!(matches!(compose.focus, ComposeFocus::SectionContent(0)));

        // Tab cycles back to title
        compose.next_field();
        assert!(matches!(compose.focus, ComposeFocus::Title));

        // Back goes to last section content
        compose.prev_field();
        assert!(matches!(compose.focus, ComposeFocus::SectionContent(0)));
    }

    #[test]
    fn test_compose_state_tag_mode() {
        let mut compose = ComposeState::new();

        // Enter tag mode
        compose.enter_tag_mode();
        assert!(compose.tag_mode);
        assert!(matches!(compose.focus, ComposeFocus::TagName));

        // Type tag name
        compose.insert_char('t');
        compose.insert_char('a');
        compose.insert_char('g');
        compose.insert_char('s');
        assert_eq!(compose.current_tag_name, "tags");

        // Move to value
        compose.next_field();
        assert!(matches!(compose.focus, ComposeFocus::TagValue));

        // Type tag value
        compose.insert_char('r');
        compose.insert_char('u');
        compose.insert_char('s');
        compose.insert_char('t');
        assert_eq!(compose.current_tag_value, "rust");

        // Tab adds the tag and creates new tag entry
        compose.next_field();
        assert_eq!(compose.tags.len(), 1);
        assert_eq!(compose.tags[0].name, "tags");
        assert_eq!(compose.tags[0].value, "rust");

        // Exit tag mode (creates a section if none exist, then goes to it)
        compose.exit_tag_mode();
        assert!(!compose.tag_mode);
        assert_eq!(compose.sections.len(), 1); // Section was auto-created
        assert!(matches!(compose.focus, ComposeFocus::SectionTitle(0)));
    }

    #[test]
    fn test_compose_state_tags_to_nostr_format() {
        let tags = vec![
            // Add a "tags" entry (comma-separated) - expands to multiple t tags
            TagEntry {
                name: "tags".to_string(),
                value: "rust, nostr, programming".to_string(),
            },
            // Add a simple single-value tag
            TagEntry {
                name: "author".to_string(),
                value: "testuser".to_string(),
            },
            // Add a multi-value tag (comma-separated becomes single tag with multiple values)
            TagEntry {
                name: "relay".to_string(),
                value: "wss://relay1.com, wss://relay2.com".to_string(),
            },
        ];

        let nostr_tags = ComposeState::tags_to_nostr_format(&tags);

        // "tags" should be expanded into multiple "t" tags
        assert_eq!(nostr_tags.len(), 5);
        assert_eq!(nostr_tags[0], vec!["t", "rust"]);
        assert_eq!(nostr_tags[1], vec!["t", "nostr"]);
        assert_eq!(nostr_tags[2], vec!["t", "programming"]);
        // Simple tag
        assert_eq!(nostr_tags[3], vec!["author", "testuser"]);
        // Multi-value tag becomes ["relay", "wss://relay1.com", "wss://relay2.com"]
        assert_eq!(nostr_tags[4], vec!["relay", "wss://relay1.com", "wss://relay2.com"]);
    }

    #[test]
    fn test_compose_state_sections() {
        let mut compose = ComposeState::new();

        // Add sections
        compose.add_section();
        assert_eq!(compose.sections.len(), 1);
        assert!(matches!(compose.focus, ComposeFocus::SectionTitle(0)));

        compose.add_section();
        assert_eq!(compose.sections.len(), 2);
        assert!(matches!(compose.focus, ComposeFocus::SectionTitle(1)));

        // Remove section
        compose.remove_section();
        assert_eq!(compose.sections.len(), 1);
    }

    #[test]
    fn test_compose_state_has_content() {
        let mut compose = ComposeState::new();

        // Empty state has no content (needs title AND sections for NKBIP-01)
        assert!(!compose.has_content());

        // Title alone is not enough
        compose.title = "Test".to_string();
        assert!(!compose.has_content());

        // Sections alone is not enough
        compose.title.clear();
        compose.sections.push(SectionCompose::default());
        assert!(!compose.has_content());

        // Both title and sections is valid content
        compose.title = "Test".to_string();
        assert!(compose.has_content());
    }

    #[test]
    fn test_compose_state_preview_json_publication() {
        // Preview shows current event based on focus
        let mut compose = ComposeState::new();
        compose.title = "Test Note".to_string();
        compose.sections.push(SectionCompose {
            title: "Section 1".to_string(),
            content: "Hello world".to_string(),
            ..Default::default()
        });

        // Default focus is Title, so should show 30040 publication event
        let json = compose.preview_event_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Should be valid JSON");

        // Should be a single 30040 event (not an array)
        assert_eq!(parsed["kind"], 30040);
        assert_eq!(parsed["content"], ""); // MUST be empty
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "title" && a[1] == "Test Note").unwrap_or(false)
        }));
        // Should also carry an indexable `T` tag with the normalized slug
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "T" && a[1] == "test-note").unwrap_or(false)
        }));
        // Should have auto-update tag
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "auto-update").unwrap_or(false)
        }));
        // Should have `a` tag referencing the section
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "a").unwrap_or(false)
        }));
    }

    #[test]
    fn test_compose_state_preview_json_section() {
        let mut compose = ComposeState::new();
        compose.title = "My Publication".to_string();
        compose.sections.push(SectionCompose {
            title: "Chapter 1".to_string(),
            content: "First chapter content".to_string(),
            ..Default::default()
        });

        // Focus on section content
        compose.focus = ComposeFocus::SectionContent(0);
        let json = compose.preview_event_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Should be valid JSON");

        // Should be a single 30041 section event
        assert_eq!(parsed["kind"], 30041);
        assert_eq!(parsed["content"], "First chapter content");

        // Should have an opaque nanoid d-tag (21 chars, URL-safe alphabet)
        let d_value = parsed["tags"]
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
            .expect("section event should carry a d tag");
        assert_eq!(d_value.len(), 21, "nanoid d-tag should be 21 chars, got {:?}", d_value);
        assert!(
            d_value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "d-tag should be URL-safe nanoid alphabet, got {:?}",
            d_value,
        );

        // Should have title tag
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "title" && a[1] == "Chapter 1").unwrap_or(false)
        }));
        // And an indexable `T` tag with the normalized slug
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "T" && a[1] == "chapter-1").unwrap_or(false)
        }));
    }

    #[test]
    fn test_d_tag_generation() {
        // Normal title
        assert_eq!(ComposeState::generate_d_tag("Hello World"), "hello-world");

        // Special characters
        assert_eq!(ComposeState::generate_d_tag("Aesop's Fables"), "aesop-s-fables");

        // Multiple spaces and special chars
        assert_eq!(ComposeState::generate_d_tag("The   Quick--Brown Fox!"), "the-quick-brown-fox");

        // Unicode (accented characters are kept since is_alphanumeric() returns true)
        assert_eq!(ComposeState::generate_d_tag("Café au Lait"), "café-au-lait");
    }

    #[test]
    fn test_editor_compose_state_basic() {
        use super::super::node::ContentMode;

        let mut editor = EditorComposeState::new();
        assert_eq!(editor.content, "");
        assert_eq!(editor.cursor, 0);
        assert_eq!(editor.mode, ContentMode::Markdown);

        // Insert characters
        editor.insert_char('H');
        editor.insert_char('i');
        assert_eq!(editor.content, "Hi");
        assert_eq!(editor.cursor, 2);

        // Insert string
        editor.insert_str(" there");
        assert_eq!(editor.content, "Hi there");
        assert_eq!(editor.cursor, 8);
    }

    #[test]
    fn test_editor_compose_state_cursor_movement() {
        let mut editor = EditorComposeState::new();
        editor.set_content("Line 1\nLine 2\nLine 3".to_string());

        // Should be at start
        assert_eq!(editor.cursor, 0);
        assert_eq!(editor.cursor_line, 0);
        assert_eq!(editor.cursor_col, 0);

        // Move right
        editor.cursor_right();
        assert_eq!(editor.cursor_col, 1);

        // Move to end of line
        editor.cursor_end();
        assert_eq!(editor.cursor_col, 6); // "Line 1" has 6 chars

        // Move down
        editor.cursor_down();
        assert_eq!(editor.cursor_line, 1);

        // Move to start of line
        editor.cursor_home();
        assert_eq!(editor.cursor_col, 0);

        // Move up
        editor.cursor_up();
        assert_eq!(editor.cursor_line, 0);
    }

    #[test]
    fn test_editor_compose_state_delete() {
        let mut editor = EditorComposeState::new();
        editor.set_content("Hello".to_string());
        editor.cursor = 5; // At end
        editor.update_cursor_position();

        // Backspace
        editor.delete_char_before();
        assert_eq!(editor.content, "Hell");
        assert_eq!(editor.cursor, 4);

        // Move to start and delete forward
        editor.cursor = 0;
        editor.delete_char_at();
        assert_eq!(editor.content, "ell");
    }

    #[test]
    fn test_editor_compose_state_parsing() {
        use super::super::parser::LineType;

        let mut editor = EditorComposeState::new();
        editor.set_content("# Title\n\nSome prose.\n\n```rust\ncode\n```".to_string());

        // Get the parsed document
        let parsed = editor.parsed();
        assert_eq!(parsed.lines.len(), 7);

        // First line should be a heading
        assert!(matches!(
            &parsed.lines[0].line_type,
            LineType::Heading { level: 1, event_kind: 30040, .. }
        ));

        // Code block detection
        assert!(matches!(&parsed.lines[4].line_type, LineType::CodeStart { language } if language == "rust"));
        assert!(matches!(&parsed.lines[5].line_type, LineType::CodeBody));
        assert!(matches!(&parsed.lines[6].line_type, LineType::CodeEnd));
    }

    #[test]
    fn test_editor_compose_state_sections() {
        let mut editor = EditorComposeState::new();
        editor.set_content("# My Article\n\n## Section 1\n\nContent 1\n\n## Section 2\n\nContent 2".to_string());

        let sections = editor.sections();
        assert_eq!(sections.len(), 3); // Title + 2 sections

        // First is the title (30040)
        assert_eq!(sections[0].title, "My Article");
        assert_eq!(sections[0].event_kind, 30040);

        // Second is Section 1 (30041)
        assert_eq!(sections[1].title, "Section 1");
        assert_eq!(sections[1].event_kind, 30041);

        // Third is Section 2 (30041)
        assert_eq!(sections[2].title, "Section 2");
        assert_eq!(sections[2].event_kind, 30041);
    }

    #[test]
    fn test_editor_compose_state_code_blocks() {
        let mut editor = EditorComposeState::new();
        editor.set_content("Prose\n\n```python\nprint('hi')\n```\n\nMore prose\n\n```rust\nfn main() {}\n```".to_string());

        let blocks = editor.code_blocks();
        assert_eq!(blocks.len(), 2);

        assert_eq!(blocks[0].2, "python"); // language
        assert_eq!(blocks[1].2, "rust");
    }

    #[test]
    fn test_editor_compose_state_cursor_in_code_block() {
        let mut editor = EditorComposeState::new();
        editor.set_content("Prose\n\n```python\nprint('hi')\n```".to_string());

        // Cursor on prose line
        editor.cursor_line = 0;
        assert!(editor.cursor_in_code_block().is_none());

        // Cursor inside code block
        editor.cursor_line = 3; // "print('hi')" line
        let block = editor.cursor_in_code_block();
        assert!(block.is_some());
        assert_eq!(block.unwrap().2, "python");
    }

    // --- ComposeBlockState tests ---

    #[test]
    fn test_compose_block_state_new() {
        let state = ComposeBlockState::new();
        assert!(state.blocks.is_empty());
        assert_eq!(state.block_cursor, 0);
        assert!(state.title.is_empty());
        assert!(!state.is_ready_to_publish());
    }

    #[test]
    fn test_compose_block_add_editable() {
        let mut state = ComposeBlockState::new();
        state.add_editable();
        assert_eq!(state.blocks.len(), 1);
        assert!(state.blocks[0].is_editable());
        assert_eq!(state.blocks[0].content(), "");
        assert_eq!(state.blocks[0].cursor(), Some(0));
        assert_eq!(state.block_cursor, 0);
    }

    #[test]
    fn test_compose_block_add_imported() {
        let mut state = ComposeBlockState::new();
        let addr = NAddr::new(30041, "author123", "section-1");
        state.add_imported(addr, "imported text".into(), "alice".into(), "Imported Section".into());
        assert_eq!(state.blocks.len(), 1);
        assert!(!state.blocks[0].is_editable());
        assert_eq!(state.blocks[0].content(), "imported text");
        assert_eq!(state.blocks[0].cursor(), None);
        assert_eq!(state.blocks[0].title, "Imported Section");
    }

    #[test]
    fn test_compose_block_toggle_fork_imported_to_forked() {
        let mut state = ComposeBlockState::new();
        let addr = NAddr::new(30041, "author123", "section-1");
        state.add_imported(addr, "original text".into(), "alice".into(), "Title".into());

        state.toggle_fork(0);
        assert!(state.blocks[0].is_editable());
        assert_eq!(state.blocks[0].content(), "original text");
        assert_eq!(state.blocks[0].cursor(), Some(0));
    }

    #[test]
    fn test_compose_block_toggle_fork_back() {
        let mut state = ComposeBlockState::new();
        let addr = NAddr::new(30041, "author123", "section-1");
        state.add_imported(addr, "original text".into(), "alice".into(), "Title".into());

        // Imported → Forked → Imported
        state.toggle_fork(0);
        assert!(state.blocks[0].is_editable());
        state.toggle_fork(0);
        assert!(!state.blocks[0].is_editable());
    }

    #[test]
    fn test_compose_block_move_up_down() {
        let mut state = ComposeBlockState::new();
        state.add_editable();
        state.blocks[0].title = "A".into();
        state.add_editable();
        state.blocks[1].title = "B".into();
        state.add_editable();
        state.blocks[2].title = "C".into();

        // Move B (idx 1) up → becomes idx 0
        state.block_cursor = 1;
        state.move_block_up(1);
        assert_eq!(state.blocks[0].title, "B");
        assert_eq!(state.blocks[1].title, "A");
        assert_eq!(state.block_cursor, 0);

        // Move B (idx 0) down → becomes idx 1
        state.move_block_down(0);
        assert_eq!(state.blocks[0].title, "A");
        assert_eq!(state.blocks[1].title, "B");
        assert_eq!(state.block_cursor, 1);
    }

    #[test]
    fn test_compose_block_remove_adjusts_cursor() {
        let mut state = ComposeBlockState::new();
        state.add_editable();
        state.add_editable();
        state.add_editable();
        state.block_cursor = 2;

        state.remove_block(2);
        assert_eq!(state.blocks.len(), 2);
        assert_eq!(state.block_cursor, 1); // Clamped to last
    }

    #[test]
    fn test_compose_block_d_tag_editable() {
        let mut state = ComposeBlockState::new();
        state.title = "My Article".into();
        state.add_editable();
        state.blocks[0].title = "Introduction".into();

        assert_eq!(state.block_d_tag(0), "my-article-introduction");
    }

    #[test]
    fn test_compose_block_d_tag_forked() {
        let mut state = ComposeBlockState::new();
        state.title = "My Article".into();
        let addr = NAddr::new(30041, "author123", "original-section");
        state.add_imported(addr, "text".into(), "alice".into(), "Title".into());
        state.toggle_fork(0); // → Forked

        assert_eq!(state.block_d_tag(0), "my-article-fork-original-section");
    }

    #[test]
    fn test_compose_block_is_ready_to_publish() {
        let mut state = ComposeBlockState::new();
        assert!(!state.is_ready_to_publish());

        state.title = "Test".into();
        assert!(!state.is_ready_to_publish()); // No blocks

        state.add_editable();
        assert!(state.is_ready_to_publish()); // Title + block
    }
}
