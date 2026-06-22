//! Compose payload types for NKBIP-01 publication authoring.
//!
//! These are the pure, frontend-agnostic data structures that turn an
//! in-progress composition into signed Nostr events (kind 30040 indexes +
//! kind 30041 sections). They carry no IO and no rendering concerns — d-tag
//! minting, tag normalization, and `a`-tag coordinate emission all live here
//! so every frontend (web, future emacs/nvim) shares one source of truth.
//!
//! Extracted from the former `tree::state` ratatui TUI state machine as part
//! of the frontend/backend boundary cleanup (docs/eval/09).

use crate::publication::NAddr;

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
    /// Output kind. `None`/`Some(30040)` = NKBIP-01 publication (30040 index +
    /// 30041 sections). `Some(k)` for any other addressable kind (NIP-23 blog
    /// 30023, NIP-54 wiki 30818, or custom) marks an *atomic* draft — one event
    /// whose body is the whole compose, not a section graph. Persisted so a
    /// resumed draft reopens in the right mode (cf. NIP-37's `k` tag).
    pub kind: Option<u32>,
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
    /// Stable opaque d-tag (nanoid) for the 30041 this block emits. `None`
    /// until first mint; once set it persists so re-publishing replaces the
    /// same addressable event. Imported blocks never use it — the index
    /// references the original's own coordinate.
    pub d_tag: Option<String>,
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
    /// Stable opaque d-tag (nanoid) for the publication's 30040 — same
    /// identity model as `ComposeState::d_tag`: `None` until first mint,
    /// then persistent so title edits never break the addressable identity.
    pub d_tag: Option<String>,
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
            d_tag: None,
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
            d_tag: None,
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
            d_tag: None,
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

    /// Get the publication d-tag, minting an opaque nanoid on first call —
    /// the same identity model as `ComposeState::publication_d_tag`. Seed
    /// `self.d_tag` when re-publishing an existing 30040 so it replaces
    /// rather than forks.
    pub fn publication_d_tag(&mut self) -> String {
        self.d_tag
            .get_or_insert_with(crate::publication::mint_d_tag)
            .clone()
    }

    /// Get a block's d-tag, minting an opaque nanoid on first call. Only
    /// Editable and Forked blocks emit a 30041 that uses it — a fork copy
    /// gets a fresh identity of its own; lineage lives in the `fork`-marker
    /// `a` tag, not the d-tag. Out-of-range indices return a deterministic
    /// placeholder rather than panicking.
    pub fn block_d_tag(&mut self, idx: usize) -> String {
        match self.blocks.get_mut(idx) {
            Some(block) => block
                .d_tag
                .get_or_insert_with(crate::publication::mint_d_tag)
                .clone(),
            None => format!("pending-b{}", idx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Block d-tags are opaque nanoids (21-char URL-safe), independent of
    /// titles, and stable across repeated calls — same rules as the
    /// ComposeState path.
    #[test]
    fn test_compose_block_d_tag_is_stable_nanoid() {
        let is_nanoid = |s: &str| {
            s.len() == 21
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        };

        let mut state = ComposeBlockState::new();
        state.title = "My Article".into();
        state.add_editable();
        state.blocks[0].title = "Introduction".into();

        let pub_d = state.publication_d_tag();
        let block_d = state.block_d_tag(0);
        assert!(is_nanoid(&pub_d), "pub d-tag should be a nanoid: {pub_d:?}");
        assert!(is_nanoid(&block_d), "block d-tag should be a nanoid: {block_d:?}");
        assert_ne!(pub_d, block_d);

        // Stable: a second call returns the stored value, even after a rename.
        state.title = "Renamed".into();
        state.blocks[0].title = "Renamed Section".into();
        assert_eq!(state.publication_d_tag(), pub_d);
        assert_eq!(state.block_d_tag(0), block_d);
    }

    /// A forked block mints its own fresh nanoid — fork lineage is carried
    /// by the `fork`-marker `a` tag, never encoded into the d-tag.
    #[test]
    fn test_compose_block_d_tag_forked_is_fresh_nanoid() {
        let mut state = ComposeBlockState::new();
        state.title = "My Article".into();
        let addr = NAddr::new(30041, "author123", "original-section");
        state.add_imported(addr, "text".into(), "alice".into(), "Title".into());
        state.toggle_fork(0); // → Forked

        let d = state.block_d_tag(0);
        assert_eq!(d.len(), 21);
        assert!(!d.contains("original-section"));
        assert!(!d.contains("fork"));
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
