//! TUI widgets for tree rendering
//!
//! Provides ratatui widgets for rendering the tree view and content preview.

use crate::tree::command::CommandCategory;
use crate::tree::node::{NodeId, TreeNode};
use crate::tree::render::{visible_nodes, RenderOptions, VisibleNode};
use crate::tree::state::{CommandPaletteState, ComposeFocus, ComposeState, TreeState};
use ratatui::prelude::*;
use ratatui::layout::{Layout, Direction, Constraint};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

/// Widget for rendering the tree view
pub struct TreeWidget<'a> {
    state: &'a TreeState,
    options: RenderOptions,
    spinner_frame: Option<char>,
}

impl<'a> TreeWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        TreeWidget {
            state,
            options: RenderOptions::tui(),
            spinner_frame: None,
        }
    }

    pub fn with_options(mut self, options: RenderOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_spinner(mut self, frame: char) -> Self {
        self.spinner_frame = Some(frame);
        self
    }

    /// Get the list state for the widget
    pub fn list_state(&self) -> ListState {
        let mut list_state = ListState::default();
        let nodes = visible_nodes(self.state);
        if let Some(idx) = nodes.iter().position(|n| n.id == self.state.cursor) {
            list_state.select(Some(idx));
        }
        list_state
    }

    /// Create list items from visible nodes
    fn create_items(&self) -> Vec<ListItem<'a>> {
        let nodes = visible_nodes(self.state);
        nodes
            .into_iter()
            .map(|node| self.render_node(&node))
            .collect()
    }

    fn render_node(&self, node: &VisibleNode) -> ListItem<'a> {
        // Build indent
        let indent = "  ".repeat(node.depth);

        // Choose indicator
        let indicator = if node.has_children {
            if node.is_expanded {
                &self.options.expanded_indicator
            } else {
                &self.options.collapsed_indicator
            }
        } else {
            &self.options.leaf_indicator
        };

        // Build status indicators
        let mut suffix = String::new();
        if node.is_loading {
            if let Some(frame) = self.spinner_frame {
                suffix.push(' ');
                suffix.push(frame);
            } else {
                suffix.push_str(" ...");
            }
        }
        if node.alternate_count > 0 {
            suffix.push_str(&format!(" [{}v]", node.alternate_count));
        }
        if node.error.is_some() {
            suffix.push_str(" !");
        }

        // Format the line
        let line = format!("{}{} {}{}", indent, indicator, node.title, suffix);

        // Apply styling based on state
        let style = if node.is_cursor && node.is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .bold()
        } else if node.is_cursor {
            Style::default().fg(Color::Black).bg(Color::White)
        } else if node.is_selected {
            Style::default().fg(Color::Yellow).bold()
        } else if node.is_loading {
            Style::default().fg(Color::DarkGray).italic()
        } else if node.error.is_some() {
            Style::default().fg(Color::Red)
        } else if !node.is_loaded {
            Style::default().fg(Color::DarkGray)
        } else if node.is_publication {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        ListItem::new(Line::from(line)).style(style)
    }
}

impl<'a> Widget for TreeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::node::SyncStatus;

        let nodes = visible_nodes(self.state);
        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Tree ");

        let inner = block.inner(area);

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);

        // Draw sync status bars on the right edge
        let bar_x = area.x + area.width - 2; // Inside the border
        let scroll = self.state.view.tree_scroll;

        for (i, node) in nodes.iter().skip(scroll).take(inner.height as usize).enumerate() {
            let y = inner.y + i as u16;
            let bar_color = match node.sync_status {
                SyncStatus::Remote => Color::Cyan,
                SyncStatus::LocalOnly => Color::Yellow,
            };
            buf[(bar_x, y)].set_char('▌').set_fg(bar_color);
        }
    }
}

/// Widget for rendering the feed view (list of publications as cards)
pub struct FeedWidget<'a> {
    state: &'a TreeState,
    spinner_frame: Option<char>,
}

impl<'a> FeedWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        FeedWidget {
            state,
            spinner_frame: None,
        }
    }

    pub fn with_spinner(mut self, frame: char) -> Self {
        self.spinner_frame = Some(frame);
        self
    }

    /// Get the list state for the widget
    pub fn list_state(&self) -> ListState {
        let mut list_state = ListState::default();
        list_state.select(Some(self.state.feed_cursor));
        list_state
    }

    /// Create list items from root publications
    fn create_items(&self) -> Vec<ListItem<'a>> {
        let mut items: Vec<ListItem<'a>> = self.state
            .roots
            .iter()
            .enumerate()
            .map(|(idx, &node_id)| self.render_publication_card(node_id, idx))
            .collect();

        // Add loading indicator or end-of-feed message
        if self.state.loading_more {
            let loading_text = if let Some(frame) = self.spinner_frame {
                format!("{} Loading more...", frame)
            } else {
                "Loading more...".to_string()
            };
            items.push(ListItem::new(Line::from(Span::styled(
                loading_text,
                Style::default().fg(Color::Yellow).italic(),
            ))));
        } else if self.state.feed_exhausted {
            items.push(ListItem::new(Line::from(Span::styled(
                "— End of feed —",
                Style::default().fg(Color::DarkGray).italic(),
            ))));
        }

        items
    }

    fn render_publication_card(&self, node_id: crate::tree::node::NodeId, idx: usize) -> ListItem<'a> {
        let is_selected = idx == self.state.feed_cursor;

        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&node_id) {
            // Build the card content
            let title = p.title.clone().unwrap_or_else(|| "[Untitled]".to_string());
            let author = p.author_name.clone().unwrap_or_else(|| {
                if p.author.len() >= 8 {
                    format!("{}...", &p.author[..8])
                } else {
                    p.author.clone()
                }
            });

            // Build summary line (truncated)
            let summary = p
                .summary
                .as_ref()
                .map(|s| {
                    let s = s.replace('\n', " ");
                    if s.len() > 60 {
                        format!("{}...", &s[..57])
                    } else {
                        s
                    }
                })
                .unwrap_or_default();

            // Section count info
            let section_info = if p.loaded {
                format!("{} sections", p.children.len())
            } else if p.loading {
                if let Some(frame) = self.spinner_frame {
                    format!("{} Loading...", frame)
                } else {
                    "Loading...".to_string()
                }
            } else {
                "Not loaded".to_string()
            };

            // Build multi-line card (sync bar rendered separately on right edge)
            let mut lines = vec![
                Line::from(Span::styled(title, Style::default().fg(Color::Cyan).bold())),
                Line::from(Span::styled(
                    format!("  by {} • {}", author, section_info),
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            if !summary.is_empty() {
                lines.push(Line::from(Span::styled(format!("  {}", summary), Style::default().fg(Color::White))));
            }

            // Add separator line
            lines.push(Line::from(""));

            let style = if is_selected {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };

            ListItem::new(lines).style(style)
        } else {
            ListItem::new(Line::from("[Unknown]"))
        }
    }

    /// Get the number of lines each publication card uses
    fn get_card_line_counts(&self) -> Vec<(crate::tree::node::NodeId, usize)> {
        self.state
            .roots
            .iter()
            .map(|&node_id| {
                if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&node_id) {
                    // Title + author line + separator = 3, + summary if present = 4
                    let lines = if p.summary.is_some() { 4 } else { 3 };
                    (node_id, lines)
                } else {
                    (node_id, 1)
                }
            })
            .collect()
    }
}

impl<'a> Widget for FeedWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::node::SyncStatus;

        let card_lines = self.get_card_line_counts();
        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Publications ");

        let inner = block.inner(area);

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);

        // Draw sync status bars on the right edge
        let bar_x = area.x + area.width - 2; // Inside the border

        // Calculate which lines belong to which publications
        // We need to track the scroll offset from the list state
        let mut y = inner.y;
        for (node_id, line_count) in &card_lines {
            if y >= inner.y + inner.height {
                break;
            }

            // Get sync status for this publication
            let bar_color = if let Some(TreeNode::Publication(p)) = self.state.nodes.get(node_id) {
                match p.sync_status {
                    SyncStatus::Remote => Color::Cyan,
                    SyncStatus::LocalOnly => Color::Yellow,
                }
            } else {
                Color::DarkGray
            };

            // Draw bar for each line of this card
            for line_offset in 0..*line_count {
                let bar_y = y + line_offset as u16;
                if bar_y < inner.y + inner.height {
                    buf[(bar_x, bar_y)].set_char('▌').set_fg(bar_color);
                }
            }

            y += *line_count as u16;
        }
    }
}

/// Widget for Outline view - shows sections as cards with content preview
pub struct OutlineWidget<'a> {
    state: &'a TreeState,
}

impl<'a> OutlineWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        OutlineWidget { state }
    }

    /// Get the publication's children (sections)
    fn get_children(&self) -> Vec<NodeId> {
        let pub_id = self.state.selected_publication.unwrap_or(self.state.cursor);
        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&pub_id) {
            p.children.clone()
        } else {
            Vec::new()
        }
    }

    /// Get the list state for cursor position
    fn list_state(&self) -> ListState {
        let mut list_state = ListState::default();
        let children = self.get_children();
        if let Some(idx) = children.iter().position(|&id| id == self.state.cursor) {
            list_state.select(Some(idx));
        }
        list_state
    }

    /// Create list items from sections
    fn create_items(&self) -> Vec<ListItem<'a>> {
        let pub_id = self.state.selected_publication.unwrap_or(self.state.cursor);

        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&pub_id) {
            p.children
                .iter()
                .enumerate()
                .map(|(idx, &child_id)| {
                    let is_cursor = child_id == self.state.cursor;

                    if let Some(TreeNode::Section(s)) = self.state.nodes.get(&child_id) {
                        let title = s.title.clone().unwrap_or_else(|| format!("Section {}", idx + 1));

                        // Build preview (truncated, single line)
                        let preview = s.content.as_ref().map(|c| {
                            let clean: String = c.replace('\n', " ").chars().take(80).collect();
                            if c.len() > 80 {
                                format!("{}...", clean)
                            } else {
                                clean
                            }
                        });

                        // Build multi-line card
                        let mut lines = vec![
                            Line::from(Span::styled(
                                format!("{}. {}", idx + 1, title),
                                Style::default().fg(Color::Cyan).bold(),
                            )),
                        ];

                        if let Some(preview_text) = preview {
                            lines.push(Line::from(Span::styled(
                                format!("   {}", preview_text),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }

                        // Add blank line as separator
                        lines.push(Line::from(""));

                        let style = if is_cursor {
                            Style::default().bg(Color::DarkGray)
                        } else {
                            Style::default()
                        };

                        ListItem::new(lines).style(style)
                    } else {
                        ListItem::new(Line::from("[Unknown section]"))
                    }
                })
                .collect()
        } else {
            vec![ListItem::new(Line::from("No sections loaded. Press Enter to load."))]
        }
    }
}

impl<'a> OutlineWidget<'a> {
    /// Get the number of lines each section card uses along with sync status
    fn get_section_info(&self) -> Vec<(NodeId, usize, crate::tree::node::SyncStatus)> {
        use crate::tree::node::SyncStatus;

        let pub_id = self.state.selected_publication.unwrap_or(self.state.cursor);

        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&pub_id) {
            p.children
                .iter()
                .map(|&child_id| {
                    if let Some(TreeNode::Section(s)) = self.state.nodes.get(&child_id) {
                        // Title + separator = 2, + preview if content exists = 3
                        let lines = if s.content.is_some() { 3 } else { 2 };
                        (child_id, lines, s.sync_status)
                    } else {
                        (child_id, 1, SyncStatus::default())
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    }
}

impl<'a> Widget for OutlineWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::node::SyncStatus;

        let pub_title = self.state.selected_publication
            .and_then(|id| self.state.nodes.get(&id))
            .map(|n| n.title().to_string())
            .unwrap_or_else(|| "Publication".to_string());

        let section_info = self.get_section_info();
        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} - Outline ", pub_title));

        let inner = block.inner(area);

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);

        // Draw sync status bars on the right edge
        let bar_x = area.x + area.width - 2;

        let mut y = inner.y;
        for (_node_id, line_count, sync_status) in &section_info {
            if y >= inner.y + inner.height {
                break;
            }

            let bar_color = match sync_status {
                SyncStatus::Remote => Color::Cyan,
                SyncStatus::LocalOnly => Color::Yellow,
            };

            // Draw bar for each line of this section card
            for line_offset in 0..*line_count {
                let bar_y = y + line_offset as u16;
                if bar_y < inner.y + inner.height {
                    buf[(bar_x, bar_y)].set_char('▌').set_fg(bar_color);
                }
            }

            y += *line_count as u16;
        }
    }
}

/// Widget for Continuous view - scrollable content of all sections
pub struct ContinuousWidget<'a> {
    state: &'a TreeState,
}

impl<'a> ContinuousWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        ContinuousWidget { state }
    }

    /// Build the full content from all sections
    fn build_content(&self) -> String {
        let pub_id = self.state.selected_publication.unwrap_or(self.state.cursor);

        let mut content = String::new();

        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&pub_id) {
            // Add publication title
            if let Some(ref title) = p.title {
                content.push_str(&format!("# {}\n\n", title));
            }

            // Add summary if available
            if let Some(ref summary) = p.summary {
                content.push_str(summary);
                content.push_str("\n\n---\n\n");
            }

            // Add each section
            for (i, &child_id) in p.children.iter().enumerate() {
                if let Some(TreeNode::Section(s)) = self.state.nodes.get(&child_id) {
                    let section_title = s.title.clone().unwrap_or_else(|| format!("Section {}", i + 1));
                    content.push_str(&format!("## {}\n\n", section_title));

                    if let Some(ref section_content) = s.content {
                        content.push_str(section_content);
                    } else {
                        content.push_str("[Content not loaded]");
                    }
                    content.push_str("\n\n");
                }
            }
        }

        content
    }
}

impl<'a> Widget for ContinuousWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let content = self.build_content();
        let pub_title = self.state.selected_publication
            .and_then(|id| self.state.nodes.get(&id))
            .map(|n| n.title().to_string())
            .unwrap_or_else(|| "Publication".to_string());

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} - Continuous ", pub_title));

        let paragraph = Paragraph::new(content)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.state.view.content_scroll as u16, 0));

        paragraph.render(area, buf);

        // Render scrollbar
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state = ScrollbarState::new(100).position(self.state.view.content_scroll);
        scrollbar.render(area, buf, &mut scrollbar_state);
    }
}

/// Widget for Paginated view - one section at a time
pub struct PaginatedWidget<'a> {
    state: &'a TreeState,
}

impl<'a> PaginatedWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        PaginatedWidget { state }
    }

    /// Get the current section based on current_section index
    fn get_current_section(&self) -> Option<(usize, usize, String, String)> {
        let pub_id = self.state.selected_publication.unwrap_or(self.state.cursor);

        if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&pub_id) {
            let total = p.children.len();
            let idx = self.state.view.current_section.min(total.saturating_sub(1));

            if let Some(&child_id) = p.children.get(idx) {
                if let Some(TreeNode::Section(s)) = self.state.nodes.get(&child_id) {
                    let title = s.title.clone().unwrap_or_else(|| format!("Section {}", idx + 1));
                    let content = s.content.clone().unwrap_or_else(|| "[Content not loaded - press Enter to load]".to_string());
                    return Some((idx + 1, total, title, content));
                }
            }
        }
        None
    }
}

impl<'a> Widget for PaginatedWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let pub_title = self.state.selected_publication
            .and_then(|id| self.state.nodes.get(&id))
            .map(|n| n.title().to_string())
            .unwrap_or_else(|| "Publication".to_string());

        if let Some((current, total, section_title, content)) = self.get_current_section() {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} - {} ({}/{}) ", pub_title, section_title, current, total));

            let paragraph = Paragraph::new(content)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((self.state.view.preview_scroll as u16, 0));

            paragraph.render(area, buf);
        } else {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} - Paginated ", pub_title));

            let msg = Paragraph::new("No sections available. Press Enter to load the publication.")
                .block(block);
            msg.render(area, buf);
        }
    }
}

/// Widget for rendering content preview
pub struct ContentPreview<'a> {
    state: &'a TreeState,
}

impl<'a> ContentPreview<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        ContentPreview { state }
    }

    fn get_content(&self) -> (String, String) {
        // Get the appropriate node based on current mode
        let node = if self.state.is_feed_mode() {
            // In feed mode, get the publication at feed_cursor
            self.state
                .roots
                .get(self.state.feed_cursor)
                .and_then(|id| self.state.nodes.get(id))
        } else {
            // In reader mode, get the node at tree cursor
            self.state.cursor_node()
        };

        if let Some(node) = node {
            let title = node.title().to_string();
            let content = match node {
                TreeNode::Section(s) => s
                    .content
                    .clone()
                    .unwrap_or_else(|| "[Content not loaded]".to_string()),
                TreeNode::Publication(p) => {
                    let mut info = String::new();
                    if let Some(ref summary) = p.summary {
                        info.push_str(summary);
                        info.push_str("\n\n");
                    }
                    info.push_str(&format!("Author: {}\n", &p.author[..8.min(p.author.len())]));
                    if let Some(ref name) = p.author_name {
                        info.push_str(&format!("        ({})\n", name));
                    }
                    if let Some(ref version) = p.version {
                        info.push_str(&format!("Version: {}\n", version));
                    }
                    info.push_str(&format!("Sections: {}\n", p.children.len()));
                    if !p.loaded {
                        info.push_str("\n[Press Enter to load]");
                    }
                    info
                }
            };
            (title, content)
        } else {
            ("No selection".to_string(), String::new())
        }
    }
}

impl<'a> Widget for ContentPreview<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (title, content) = self.get_content();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", title));

        let paragraph = Paragraph::new(content)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((self.state.view.preview_scroll as u16, 0));

        paragraph.render(area, buf);
    }
}

/// Widget for the status bar
pub struct StatusBar<'a> {
    state: &'a TreeState,
    message: Option<&'a str>,
    spinner_frame: Option<char>,
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        StatusBar {
            state,
            message: None,
            spinner_frame: None,
        }
    }

    pub fn with_message(mut self, message: &'a str) -> Self {
        self.message = Some(message);
        self
    }

    pub fn with_spinner(mut self, frame: char) -> Self {
        self.spinner_frame = Some(frame);
        self
    }
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let (cursor_info, left) = if self.state.is_feed_mode() {
            // Feed mode status
            let cursor_idx = self.state.feed_cursor + 1;
            let total = self.state.roots.len();
            let cursor_info = format!("{}/{}", cursor_idx, total);

            let left = if let Some(msg) = self.message {
                if let Some(frame) = self.spinner_frame {
                    format!("{} {}", frame, msg)
                } else {
                    msg.to_string()
                }
            } else if let Some(&node_id) = self.state.roots.get(self.state.feed_cursor) {
                if let Some(node) = self.state.nodes.get(&node_id) {
                    node.addr().to_a_tag()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            (cursor_info, left)
        } else {
            // Reader mode status
            let nodes = visible_nodes(self.state);
            let cursor_idx = nodes
                .iter()
                .position(|n| n.id == self.state.cursor)
                .map(|i| i + 1)
                .unwrap_or(0);
            let total = nodes.len();
            let cursor_info = format!("{}/{}", cursor_idx, total);

            let left = if let Some(msg) = self.message {
                if let Some(frame) = self.spinner_frame {
                    format!("{} {}", frame, msg)
                } else {
                    msg.to_string()
                }
            } else if let Some(node) = self.state.cursor_node() {
                node.addr().to_a_tag()
            } else {
                String::new()
            };

            (cursor_info, left)
        };

        let mode_name = self.state.mode.name();
        let view_mode = self.state.view.mode.name();

        let right = format!(
            "{} | {} | {} | {}",
            cursor_info,
            mode_name,
            view_mode,
            if self.state.view.show_preview {
                "Preview ON"
            } else {
                "Preview OFF"
            }
        );

        // Calculate padding
        let available = area.width as usize;
        let left_len = left.chars().count();
        let right_len = right.chars().count();

        let line = if left_len + right_len + 2 <= available {
            let padding = available - left_len - right_len;
            format!("{}{:>width$}", left, right, width = padding + right_len)
        } else {
            // Truncate left side if needed
            let max_left = available.saturating_sub(right_len + 3);
            if max_left > 0 {
                format!("{}...{}", &left[..max_left.min(left_len)], right)
            } else {
                right
            }
        };

        let style = Style::default().fg(Color::Black).bg(Color::White);
        buf.set_string(area.x, area.y, &line, style);

        // Fill remaining width
        for x in area.x + line.len() as u16..area.x + area.width {
            buf[(x, area.y)].set_style(style);
        }
    }
}

/// Widget for help bar at bottom
pub struct HelpBar<'a> {
    state: &'a TreeState,
}

impl<'a> HelpBar<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        HelpBar { state }
    }
}

impl<'a> Widget for HelpBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::state::ViewMode;

        let help = if self.state.command_palette.visible {
            "↑/↓:Select  Enter:Execute  Esc:Close  Type to filter"
        } else if self.state.is_compose_mode() {
            if self.state.compose.tag_mode {
                "Tab:Add Tag  Shift+Tab:Delete Tag  Ctrl+e:Exit Tags  Ctrl+p:Preview  Ctrl+Enter:Publish  Esc:Cancel"
            } else {
                "Tab:Next Field  Ctrl+t:Tags  Ctrl+s:Section  Ctrl+p:Preview  Ctrl+Enter:Publish  Esc:Cancel"
            }
        } else if self.state.is_feed_mode() {
            "j/k:Nav  Enter:Open  c:Compose  Tab:Preview  v:ViewMode  ?:Commands  q:Quit"
        } else {
            // View mode specific help
            match self.state.view.mode {
                ViewMode::Tree => {
                    "j/k:Nav  h/l:Collapse/Expand  Enter:Load  Esc:Back  ?:Commands  q:Quit"
                }
                ViewMode::Outline => {
                    "j/k:Nav  Enter:Select  Esc:Back  ?:Commands  q:Quit"
                }
                ViewMode::Continuous => {
                    "j/k:Scroll  Esc:Back  ?:Commands  q:Quit"
                }
                ViewMode::Paginated => {
                    "j/k:Scroll  J/K:Next/Prev Section  Esc:Back  ?:Commands  q:Quit"
                }
            }
        };
        let style = Style::default().fg(Color::DarkGray);
        buf.set_string(area.x, area.y, help, style);
    }
}

/// Widget for the command palette (M-x style menu)
pub struct CommandPaletteWidget<'a> {
    state: &'a CommandPaletteState,
}

impl<'a> CommandPaletteWidget<'a> {
    pub fn new(state: &'a CommandPaletteState) -> Self {
        CommandPaletteWidget { state }
    }

    /// Calculate the area for the palette (centered popup)
    pub fn calculate_area(parent: Rect) -> Rect {
        let width = parent.width.min(70).max(40);
        let height = parent.height.min(20).max(10);
        let x = (parent.width.saturating_sub(width)) / 2;
        let y = (parent.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }

    fn get_category_style(category: CommandCategory) -> Style {
        match category {
            CommandCategory::Navigation => Style::default().fg(Color::Blue),
            CommandCategory::Selection => Style::default().fg(Color::Yellow),
            CommandCategory::Manipulation => Style::default().fg(Color::Red),
            CommandCategory::Versioning => Style::default().fg(Color::Magenta),
            CommandCategory::View => Style::default().fg(Color::Cyan),
            CommandCategory::UndoRedo => Style::default().fg(Color::Green),
            CommandCategory::Mode => Style::default().fg(Color::LightBlue),
            CommandCategory::Application => Style::default().fg(Color::White),
            CommandCategory::Configuration => Style::default().fg(Color::Gray),
            CommandCategory::Compose => Style::default().fg(Color::LightGreen),
        }
    }

    /// Render a single command line at the given position
    fn render_command_line(&self, buf: &mut Buffer, cmd: &crate::tree::command::CommandInfo, y: u16, x_start: u16, max_width: u16, is_selected: bool) {
        let mut x = x_start;
        let x_end = x_start + max_width;

        // Background for selected row
        if is_selected {
            for bx in x_start..x_end {
                buf[(bx, y)].set_bg(Color::DarkGray);
            }
        }

        // Category tag
        let cat_text = format!("[{}]", cmd.category.name());
        let cat_style = Self::get_category_style(cmd.category);
        for c in cat_text.chars() {
            if x >= x_end {
                break;
            }
            buf[(x, y)].set_char(c).set_style(cat_style);
            x += 1;
        }

        // Space
        if x < x_end {
            x += 1;
        }

        // Command name
        let name_style = if is_selected {
            Style::default().fg(Color::White).bold()
        } else {
            Style::default().fg(Color::White).bold()
        };
        for c in cmd.name.chars() {
            if x >= x_end {
                break;
            }
            buf[(x, y)].set_char(c).set_style(name_style);
            x += 1;
        }

        // Keybinding (if any)
        if let Some(key) = cmd.keybinding {
            let key_text = format!("  ({})", key);
            let key_style = Style::default().fg(Color::DarkGray);
            for c in key_text.chars() {
                if x >= x_end {
                    break;
                }
                buf[(x, y)].set_char(c).set_style(key_style);
                x += 1;
            }
        }
    }
}

impl<'a> Widget for CommandPaletteWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first (for popup effect)
        Clear.render(area, buf);

        // Calculate inner areas
        let inner = Block::default()
            .borders(Borders::ALL)
            .title(" Commands (M-x) ")
            .style(Style::default().bg(Color::Black))
            .inner(area);

        // Render border
        Block::default()
            .borders(Borders::ALL)
            .title(" Commands (M-x) ")
            .style(Style::default().bg(Color::Black))
            .render(area, buf);

        if inner.height < 3 {
            return;
        }

        // Search input area (1 line at top)
        let search_text = format!("> {}_", self.state.query);
        let search_style = Style::default().fg(Color::Yellow);
        buf.set_string(inner.x, inner.y, &search_text, search_style);

        // Divider
        let divider: String = "─".repeat(inner.width as usize);
        buf.set_string(inner.x, inner.y + 1, &divider, Style::default().fg(Color::DarkGray));

        // Command list area
        let list_height = inner.height.saturating_sub(2);
        if list_height == 0 {
            return;
        }

        let item_count = self.state.filtered_commands.len();
        let visible_items = list_height as usize;

        // Calculate scroll offset to keep selected item visible
        let scroll_offset = if self.state.selected >= visible_items {
            self.state.selected - visible_items + 1
        } else {
            0
        };

        // Render items with scroll offset
        for (i, cmd) in self.state.filtered_commands.iter()
            .skip(scroll_offset)
            .take(visible_items)
            .enumerate()
        {
            let y = inner.y + 2 + i as u16;
            if y < inner.y + inner.height {
                let is_selected = scroll_offset + i == self.state.selected;
                self.render_command_line(buf, cmd, y, inner.x, inner.width, is_selected);
            }
        }

        // Show count at bottom if there are more items
        if item_count > visible_items {
            let count_text = format!(" {}/{} ", self.state.selected + 1, item_count);
            let count_x = area.x + area.width - count_text.len() as u16 - 1;
            buf.set_string(count_x, area.y + area.height - 1, &count_text, Style::default().fg(Color::DarkGray));
        }
    }
}

/// Widget for rendering the compose view
pub struct ComposeWidget<'a> {
    state: &'a ComposeState,
}

impl<'a> ComposeWidget<'a> {
    pub fn new(state: &'a ComposeState) -> Self {
        ComposeWidget { state }
    }

    /// Render a text field with cursor
    fn render_text_field(
        &self,
        buf: &mut Buffer,
        area: Rect,
        label: &str,
        value: &str,
        is_focused: bool,
        cursor_pos: usize,
    ) {
        let label_width = label.len() as u16 + 1;

        // Render label
        let label_style = Style::default().fg(Color::Cyan);
        buf.set_string(area.x, area.y, label, label_style);
        buf.set_string(area.x + label.len() as u16, area.y, ":", label_style);

        // Render input area
        let input_x = area.x + label_width + 1;
        let input_width = area.width.saturating_sub(label_width + 2);

        // Calculate visible portion of text
        let visible_start = if cursor_pos > input_width as usize {
            cursor_pos - input_width as usize + 1
        } else {
            0
        };

        let visible_text: String = value.chars().skip(visible_start).take(input_width as usize).collect();

        let input_style = if is_focused {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        // Clear input area
        for x in input_x..input_x + input_width {
            buf[(x, area.y)].set_char(' ').set_style(input_style);
        }

        // Render text
        buf.set_string(input_x, area.y, &visible_text, input_style);

        // Render cursor if focused
        if is_focused {
            let cursor_screen_pos = cursor_pos.saturating_sub(visible_start);
            let cursor_x = input_x + cursor_screen_pos as u16;
            if cursor_x < input_x + input_width {
                buf[(cursor_x, area.y)].set_style(
                    Style::default().bg(Color::White).fg(Color::Black)
                );
            }
        }
    }

    /// Render a multiline content area
    fn render_content_area(
        &self,
        buf: &mut Buffer,
        area: Rect,
        label: &str,
        value: &str,
        is_focused: bool,
        cursor_pos: usize,
    ) {
        // Render label
        let label_style = Style::default().fg(Color::Cyan);
        buf.set_string(area.x, area.y, label, label_style);

        // Content area starts on next line
        let content_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        let border_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Draw border
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);

        let inner = block.inner(content_area);
        block.render(content_area, buf);

        // Render content lines
        let lines: Vec<&str> = value.split('\n').collect();

        // Calculate cursor line and column
        let mut char_count = 0;
        let mut cursor_line = 0;
        let mut cursor_col = 0;
        for (i, line) in lines.iter().enumerate() {
            let line_end = char_count + line.len() + 1; // +1 for newline
            if cursor_pos <= line_end || i == lines.len() - 1 {
                cursor_line = i;
                cursor_col = cursor_pos.saturating_sub(char_count);
                break;
            }
            char_count = line_end;
        }

        // Render visible lines
        let scroll = self.state.content_scroll;
        for (i, line) in lines.iter().skip(scroll).take(inner.height as usize).enumerate() {
            let y = inner.y + i as u16;
            let display_line: String = line.chars().take(inner.width as usize).collect();
            buf.set_string(inner.x, y, &display_line, Style::default());
        }

        // Render cursor if focused
        if is_focused {
            let cursor_screen_line = cursor_line.saturating_sub(scroll);
            if cursor_screen_line < inner.height as usize {
                let cursor_y = inner.y + cursor_screen_line as u16;
                let cursor_x = inner.x + (cursor_col.min(inner.width as usize)) as u16;
                if cursor_x < inner.x + inner.width {
                    buf[(cursor_x, cursor_y)].set_style(
                        Style::default().bg(Color::White).fg(Color::Black)
                    );
                }
            }
        }
    }

    /// Render tags display (returns number of lines used)
    fn render_tags(&self, buf: &mut Buffer, area: Rect) -> u16 {
        let mut y = area.y;
        let max_y = area.y + area.height;

        // Render existing tags, one per line
        for tag in &self.state.tags {
            if y >= max_y {
                break;
            }
            let tag_text = format!("[{}] [{}]", tag.name, tag.value);
            let display: String = tag_text.chars().take(area.width as usize).collect();
            buf.set_string(area.x, y, &display, Style::default().fg(Color::Green));
            y += 1;
        }

        // Show tag input if in tag mode
        if self.state.tag_mode && y < max_y {
            let is_name_focused = matches!(self.state.focus, ComposeFocus::TagName);
            let is_value_focused = matches!(self.state.focus, ComposeFocus::TagValue);

            let mut x = area.x;

            // Tag name input
            let name_style = if is_name_focused {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            buf.set_string(x, y, "[", Style::default());
            x += 1;

            let name_display = if self.state.current_tag_name.is_empty() && !is_name_focused {
                "name"
            } else {
                &self.state.current_tag_name
            };
            buf.set_string(x, y, name_display, name_style);

            if is_name_focused {
                let cursor_x = x + self.state.cursor_pos as u16;
                if cursor_x < area.x + area.width {
                    buf[(cursor_x, y)].set_style(
                        Style::default().bg(Color::White).fg(Color::Black)
                    );
                }
            }

            x += name_display.len().max(4) as u16;
            buf.set_string(x, y, "]", Style::default());
            x += 2;

            // Tag value input
            let value_style = if is_value_focused {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            buf.set_string(x, y, "[", Style::default());
            x += 1;

            let value_display = if self.state.current_tag_value.is_empty() && !is_value_focused {
                "value"
            } else {
                &self.state.current_tag_value
            };
            buf.set_string(x, y, value_display, value_style);

            if is_value_focused {
                let cursor_x = x + self.state.cursor_pos as u16;
                if cursor_x < area.x + area.width {
                    buf[(cursor_x, y)].set_style(
                        Style::default().bg(Color::White).fg(Color::Black)
                    );
                }
            }

            x += value_display.len().max(5) as u16;
            buf.set_string(x, y, "]", Style::default());
            y += 1;
        }

        y - area.y
    }
}

impl<'a> Widget for ComposeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.state.show_preview {
            // Split into left (input) and right (preview) panels
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            // Left panel: Input
            self.render_input_panel(buf, chunks[0]);

            // Right panel: Event JSON preview
            self.render_preview_panel(buf, chunks[1]);
        } else {
            // Full width input panel
            self.render_input_panel(buf, area);
        }
    }
}

impl<'a> ComposeWidget<'a> {
    fn render_input_panel(&self, buf: &mut Buffer, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Compose ");

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 4 {
            return;
        }

        let mut y = inner.y;
        let has_sections = !self.state.sections.is_empty();

        // Title field
        let title_label = if has_sections { "Publication Title" } else { "Title" };
        let is_title_focused = matches!(self.state.focus, ComposeFocus::Title);
        self.render_text_field(
            buf,
            Rect { x: inner.x, y, width: inner.width, height: 1 },
            title_label,
            &self.state.title,
            is_title_focused,
            if is_title_focused { self.state.cursor_pos } else { 0 },
        );
        y += 1;

        // Divider
        let divider: String = "─".repeat(inner.width as usize);
        buf.set_string(inner.x, y, &divider, Style::default().fg(Color::DarkGray));
        y += 1;

        // Tags section (if any tags or in tag mode)
        if !self.state.tags.is_empty() || self.state.tag_mode {
            // Calculate available height for tags (leave room for content)
            let max_tag_height = (inner.y + inner.height).saturating_sub(y).saturating_sub(4).min(10);
            let lines_used = self.render_tags(buf, Rect { x: inner.x, y, width: inner.width, height: max_tag_height });
            y += lines_used;

            // Divider after tags
            buf.set_string(inner.x, y, &divider, Style::default().fg(Color::DarkGray));
            y += 1;
        }

        // Sections or Content
        if has_sections {
            // Render sections
            for (idx, section) in self.state.sections.iter().enumerate() {
                if y >= inner.y + inner.height - 1 {
                    break;
                }

                // Section header
                let header = format!("─ Section {} ", idx + 1);
                let header_fill: String = "─".repeat((inner.width as usize).saturating_sub(header.len()));
                buf.set_string(inner.x, y, &header, Style::default().fg(Color::Cyan));
                buf.set_string(inner.x + header.len() as u16, y, &header_fill, Style::default().fg(Color::DarkGray));
                y += 1;

                // Section title
                let is_sec_title_focused = matches!(self.state.focus, ComposeFocus::SectionTitle(i) if i == idx);
                self.render_text_field(
                    buf,
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                    "Title",
                    &section.title,
                    is_sec_title_focused,
                    if is_sec_title_focused { self.state.cursor_pos } else { 0 },
                );
                y += 1;

                // Section content (abbreviated preview)
                let is_sec_content_focused = matches!(self.state.focus, ComposeFocus::SectionContent(i) if i == idx);
                let content_height = (inner.y + inner.height).saturating_sub(y).saturating_sub(1).min(4);
                if content_height > 0 {
                    self.render_content_area(
                        buf,
                        Rect { x: inner.x, y, width: inner.width, height: content_height },
                        "Content",
                        &section.content,
                        is_sec_content_focused,
                        if is_sec_content_focused { self.state.cursor_pos } else { 0 },
                    );
                    y += content_height;
                }
            }

            // Add section hint
            if y < inner.y + inner.height {
                let hint = "[Ctrl+s: Add Section]";
                buf.set_string(inner.x, y, hint, Style::default().fg(Color::DarkGray).italic());
            }
        } else {
            // Main content area
            let is_content_focused = matches!(self.state.focus, ComposeFocus::Content);
            let content_height = (inner.y + inner.height).saturating_sub(y);
            if content_height > 1 {
                self.render_content_area(
                    buf,
                    Rect { x: inner.x, y, width: inner.width, height: content_height },
                    "Content",
                    &self.state.content,
                    is_content_focused,
                    if is_content_focused { self.state.cursor_pos } else { 0 },
                );
            }
        }
    }

    fn render_preview_panel(&self, buf: &mut Buffer, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Event Preview ")
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 2 {
            return;
        }

        // Get the preview JSON
        let preview_json = self.state.preview_event_json();

        // Render the JSON with syntax highlighting
        let lines: Vec<&str> = preview_json.lines().collect();
        let scroll = self.state.content_scroll;

        for (i, line) in lines.iter().skip(scroll).take(inner.height as usize).enumerate() {
            let y = inner.y + i as u16;
            let display_line: String = line.chars().take(inner.width as usize).collect();

            // Simple syntax highlighting for JSON
            let style = if display_line.trim().starts_with('"') && display_line.contains(':') {
                // Key
                Style::default().fg(Color::Cyan)
            } else if display_line.contains("\"<") {
                // Placeholder values
                Style::default().fg(Color::Yellow).italic()
            } else if display_line.trim().starts_with('"') {
                // String value
                Style::default().fg(Color::Green)
            } else if display_line.trim().chars().next().map(|c| c.is_numeric()).unwrap_or(false) {
                // Number
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::White)
            };

            buf.set_string(inner.x, y, &display_line, style);
        }

        // Show scroll indicator if there's more content
        if lines.len() > inner.height as usize {
            let indicator = format!(" {}/{} ", scroll + 1, lines.len().saturating_sub(inner.height as usize) + 1);
            let indicator_x = area.x + area.width - indicator.len() as u16 - 1;
            buf.set_string(indicator_x, area.y, &indicator, Style::default().fg(Color::DarkGray));
        }
    }
}
