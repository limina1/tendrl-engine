//! TUI widgets for tree rendering
//!
//! Provides ratatui widgets for rendering the tree view and content preview.

use crate::tree::node::{NodeId, TreeNode};
use crate::tree::render::{visible_nodes, RenderOptions, VisibleNode};
use crate::tree::state::TreeState;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};

/// Widget for rendering the tree view
pub struct TreeWidget<'a> {
    state: &'a TreeState,
    options: RenderOptions,
}

impl<'a> TreeWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        TreeWidget {
            state,
            options: RenderOptions::tui(),
        }
    }

    pub fn with_options(mut self, options: RenderOptions) -> Self {
        self.options = options;
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
            suffix.push_str(" ...");
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
        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Tree ");

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);
    }
}

/// Widget for rendering the feed view (list of publications as cards)
pub struct FeedWidget<'a> {
    state: &'a TreeState,
}

impl<'a> FeedWidget<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        FeedWidget { state }
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
            items.push(ListItem::new(Line::from(Span::styled(
                "Loading more...",
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
                "Loading...".to_string()
            } else {
                "Not loaded".to_string()
            };

            // Build multi-line card
            let mut lines = vec![
                Line::from(Span::styled(
                    title,
                    Style::default().fg(Color::Cyan).bold(),
                )),
                Line::from(Span::styled(
                    format!("by {} • {}", author, section_info),
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            if !summary.is_empty() {
                lines.push(Line::from(Span::styled(summary, Style::default().fg(Color::White))));
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
}

impl<'a> Widget for FeedWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Publications ");

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);
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

impl<'a> Widget for OutlineWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let pub_title = self.state.selected_publication
            .and_then(|id| self.state.nodes.get(&id))
            .map(|n| n.title().to_string())
            .unwrap_or_else(|| "Publication".to_string());

        let items = self.create_items();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} - Outline ", pub_title));

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray));

        let mut list_state = self.list_state();
        StatefulWidget::render(list, area, buf, &mut list_state);
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
}

impl<'a> StatusBar<'a> {
    pub fn new(state: &'a TreeState) -> Self {
        StatusBar {
            state,
            message: None,
        }
    }

    pub fn with_message(mut self, message: &'a str) -> Self {
        self.message = Some(message);
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
                msg.to_string()
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
                msg.to_string()
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

        let help = if self.state.is_feed_mode() {
            "j/k:Nav  Enter:Open  Tab:Preview  v:ViewMode  ::Relays  q:Quit"
        } else {
            // View mode specific help
            match self.state.view.mode {
                ViewMode::Tree => {
                    "j/k:Nav  h/l:Collapse/Expand  Enter:Load  Esc:Back  Tab:Preview  v:ViewMode  q:Quit"
                }
                ViewMode::Outline => {
                    "j/k:Nav  Enter:Select  Esc:Back  Tab:Preview  v:ViewMode  q:Quit"
                }
                ViewMode::Continuous => {
                    "j/k:Scroll  Esc:Back  v:ViewMode  q:Quit"
                }
                ViewMode::Paginated => {
                    "j/k:Scroll  J/K:Next/Prev Section  Esc:Back  v:ViewMode  q:Quit"
                }
            }
        };
        let style = Style::default().fg(Color::DarkGray);
        buf.set_string(area.x, area.y, help, style);
    }
}
