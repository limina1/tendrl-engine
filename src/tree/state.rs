//! Tree state management
//!
//! Contains the pure state types for the tree view, including cursor position,
//! expanded nodes, selections, and view configuration.

use super::command::{all_commands, CommandInfo};
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
    /// Compose mode state
    pub compose: ComposeState,
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
        }
    }

    /// Enter compose mode
    pub fn enter_compose(&mut self) {
        self.compose.reset();
        self.mode = AppMode::Compose;
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

/// Focus position within compose mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ComposeFocus {
    /// Focused on the title field
    #[default]
    Title,
    /// Focused on a tag name input
    TagName,
    /// Focused on a tag value input
    TagValue,
    /// Focused on the main content field
    Content,
    /// Focused on a section title (index into sections vec)
    SectionTitle(usize),
    /// Focused on a section content (index into sections vec)
    SectionContent(usize),
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
#[derive(Debug, Clone, Default)]
pub struct SectionCompose {
    /// Section title
    pub title: String,
    /// Section content
    pub content: String,
    /// Section-specific tags
    pub tags: Vec<TagEntry>,
}

/// State for compose mode
#[derive(Debug, Clone, Default)]
pub struct ComposeState {
    /// Current focus position
    pub focus: ComposeFocus,
    /// Publication/note title
    pub title: String,
    /// Main content (for simple notes without sections)
    pub content: String,
    /// Tags for the publication
    pub tags: Vec<TagEntry>,
    /// Sections (for multi-section publications)
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
            ComposeFocus::Content => &self.content,
            ComposeFocus::SectionTitle(idx) => {
                self.sections.get(idx).map(|s| s.title.as_str()).unwrap_or("")
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
            ComposeFocus::Content => Some(&mut self.content),
            ComposeFocus::SectionTitle(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.title)
            }
            ComposeFocus::SectionContent(idx) => {
                self.sections.get_mut(idx).map(|s| &mut s.content)
            }
        }
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
    pub fn next_field(&mut self) {
        match self.focus {
            ComposeFocus::Title => {
                if self.tag_mode {
                    self.focus = ComposeFocus::TagName;
                } else if !self.sections.is_empty() {
                    self.focus = ComposeFocus::SectionTitle(0);
                } else {
                    self.focus = ComposeFocus::Content;
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
            ComposeFocus::Content => {
                // Stay on content or cycle to title
                self.focus = ComposeFocus::Title;
            }
            ComposeFocus::SectionTitle(idx) => {
                self.focus = ComposeFocus::SectionContent(idx);
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
                } else {
                    self.focus = ComposeFocus::Content;
                }
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
            ComposeFocus::Content => {
                if self.tag_mode {
                    self.focus = ComposeFocus::TagName;
                } else {
                    self.focus = ComposeFocus::Title;
                }
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
            ComposeFocus::SectionContent(idx) => {
                self.focus = ComposeFocus::SectionTitle(idx);
            }
        }
        self.cursor_pos = self.current_text().len();
    }

    /// Enter tag creation mode
    pub fn enter_tag_mode(&mut self) {
        self.tag_mode = true;
        self.focus = ComposeFocus::TagName;
        self.cursor_pos = 0;
    }

    /// Exit tag creation mode
    pub fn exit_tag_mode(&mut self) {
        // Save any pending tag
        if !self.current_tag_name.is_empty() && !self.current_tag_value.is_empty() {
            self.tags.push(TagEntry {
                name: self.current_tag_name.clone(),
                value: self.current_tag_value.clone(),
            });
        }
        self.current_tag_name.clear();
        self.current_tag_value.clear();
        self.tag_mode = false;
        self.focus = ComposeFocus::Content;
        self.cursor_pos = self.content.len();
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
                ComposeFocus::SectionTitle(idx) | ComposeFocus::SectionContent(idx) => {
                    if idx >= self.sections.len() {
                        if self.sections.is_empty() {
                            self.focus = ComposeFocus::Content;
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
            ComposeFocus::Content | ComposeFocus::SectionContent(_) => {
                self.insert_char('\n');
            }
            _ => {
                // In other fields, newline moves to next field
                self.next_field();
            }
        }
    }

    /// Convert tags to the format expected by Nostr events
    ///
    /// Syntax:
    /// - `[tags] [val1, val2]` → `["t", "val1"]`, `["t", "val2"]` (expands to multiple t tags)
    /// - `[name] [val1, val2, val3]` → `["name", "val1", "val2", "val3"]` (multi-value single tag)
    /// - `[name] [value]` → `["name", "value"]` (simple tag)
    pub fn tags_to_nostr_format(&self) -> Vec<Vec<String>> {
        let mut result = Vec::new();
        for tag in &self.tags {
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
    pub fn has_content(&self) -> bool {
        !self.title.is_empty() || !self.content.is_empty() || !self.sections.is_empty()
    }

    /// Generate a preview of the event(s) that would be published
    /// Returns JSON representation with placeholder id, pubkey, sig
    pub fn preview_event_json(&self) -> String {
        use serde_json::{json, Value};
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if self.sections.is_empty() {
            // Simple note (kind 1) or single content
            let mut tags: Vec<Value> = self.tags_to_nostr_format()
                .into_iter()
                .map(|t| json!(t))
                .collect();

            // Add title tag if present
            if !self.title.is_empty() {
                tags.insert(0, json!(["title", &self.title]));
            }

            let event = json!({
                "id": "<unsigned>",
                "pubkey": "<not signed in>",
                "created_at": now,
                "kind": 1,
                "tags": tags,
                "content": &self.content,
                "sig": "<unsigned>"
            });

            serde_json::to_string_pretty(&event).unwrap_or_else(|_| "{}".to_string())
        } else {
            // Multi-section publication (kind 30040 + 30041)
            let mut events: Vec<Value> = Vec::new();

            // Generate section events first (30041)
            let mut section_refs: Vec<Value> = Vec::new();
            for (i, section) in self.sections.iter().enumerate() {
                let d_tag = format!("section-{}", i);
                let mut section_tags: Vec<Value> = vec![
                    json!(["d", &d_tag]),
                ];
                if !section.title.is_empty() {
                    section_tags.push(json!(["title", &section.title]));
                }

                section_refs.push(json!(["a", format!("30041:<pubkey>:{}", d_tag)]));

                let section_event = json!({
                    "id": "<unsigned>",
                    "pubkey": "<not signed in>",
                    "created_at": now,
                    "kind": 30041,
                    "tags": section_tags,
                    "content": &section.content,
                    "sig": "<unsigned>"
                });
                events.push(section_event);
            }

            // Generate publication event (30040)
            let mut pub_tags: Vec<Value> = vec![
                json!(["d", "publication"]),
            ];
            if !self.title.is_empty() {
                pub_tags.push(json!(["title", &self.title]));
            }

            // Add custom tags
            for tag in &self.tags {
                if tag.name == "tags" {
                    for t in tag.value.split(',') {
                        let t = t.trim();
                        if !t.is_empty() {
                            pub_tags.push(json!(["t", t]));
                        }
                    }
                } else {
                    pub_tags.push(json!([&tag.name, &tag.value]));
                }
            }

            // Add section references
            pub_tags.extend(section_refs);

            let pub_event = json!({
                "id": "<unsigned>",
                "pubkey": "<not signed in>",
                "created_at": now,
                "kind": 30040,
                "tags": pub_tags,
                "content": "",
                "sig": "<unsigned>"
            });

            // Publication event first, then sections
            let mut all_events = vec![pub_event];
            all_events.extend(events);

            serde_json::to_string_pretty(&all_events).unwrap_or_else(|_| "[]".to_string())
        }
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

        // Tab to content
        compose.next_field();
        assert!(matches!(compose.focus, ComposeFocus::Content));

        // Back to title
        compose.prev_field();
        assert!(matches!(compose.focus, ComposeFocus::Title));
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

        // Exit tag mode
        compose.exit_tag_mode();
        assert!(!compose.tag_mode);
        assert!(matches!(compose.focus, ComposeFocus::Content));
    }

    #[test]
    fn test_compose_state_tags_to_nostr_format() {
        let mut compose = ComposeState::new();

        // Add a "tags" entry (comma-separated) - expands to multiple t tags
        compose.tags.push(TagEntry {
            name: "tags".to_string(),
            value: "rust, nostr, programming".to_string(),
        });

        // Add a simple single-value tag
        compose.tags.push(TagEntry {
            name: "author".to_string(),
            value: "testuser".to_string(),
        });

        // Add a multi-value tag (comma-separated becomes single tag with multiple values)
        compose.tags.push(TagEntry {
            name: "relay".to_string(),
            value: "wss://relay1.com, wss://relay2.com".to_string(),
        });

        let nostr_tags = compose.tags_to_nostr_format();

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

        // Empty state has no content
        assert!(!compose.has_content());

        // Title makes it have content
        compose.title = "Test".to_string();
        assert!(compose.has_content());

        compose.title.clear();
        assert!(!compose.has_content());

        // Content makes it have content
        compose.content = "Some content".to_string();
        assert!(compose.has_content());

        compose.content.clear();
        assert!(!compose.has_content());

        // Sections make it have content
        compose.sections.push(SectionCompose::default());
        assert!(compose.has_content());
    }

    #[test]
    fn test_compose_state_preview_json_simple() {
        let mut compose = ComposeState::new();
        compose.title = "Test Note".to_string();
        compose.content = "Hello world".to_string();

        let json = compose.preview_event_json();

        // Should be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Should be valid JSON");

        // Check structure
        assert_eq!(parsed["kind"], 1);
        assert_eq!(parsed["content"], "Hello world");
        assert!(parsed["tags"].as_array().unwrap().iter().any(|t| {
            t.as_array().map(|a| a[0] == "title" && a[1] == "Test Note").unwrap_or(false)
        }));
    }

    #[test]
    fn test_compose_state_preview_json_publication() {
        let mut compose = ComposeState::new();
        compose.title = "My Publication".to_string();
        compose.sections.push(SectionCompose {
            title: "Chapter 1".to_string(),
            content: "First chapter content".to_string(),
            tags: vec![],
        });

        let json = compose.preview_event_json();

        // Should be valid JSON array
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("Should be valid JSON");
        let events = parsed.as_array().expect("Should be array of events");

        // Should have 2 events: 1 publication (30040) + 1 section (30041)
        assert_eq!(events.len(), 2);

        // First should be publication
        assert_eq!(events[0]["kind"], 30040);

        // Second should be section
        assert_eq!(events[1]["kind"], 30041);
        assert_eq!(events[1]["content"], "First chapter content");
    }
}
