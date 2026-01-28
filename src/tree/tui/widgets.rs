//! TUI widgets for tree rendering
//!
//! Provides ratatui widgets for rendering the tree view and content preview.

use crate::identity::LoginStatus;
use crate::tree::command::CommandCategory;
use crate::tree::node::{NodeId, TreeNode};
use crate::tree::render::{visible_nodes, RenderOptions, VisibleNode};
use crate::tree::state::{CommandPaletteState, ComposeFocus, ComposeState, LoginDialogState, TreeState, UserDataMenuState};
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
                SyncStatus::LocalCreated => Color::Rgb(255, 165, 0), // Orange
                SyncStatus::Draft => Color::Red,
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
        let visible_roots = self.get_visible_roots();
        let mut items: Vec<ListItem<'a>> = visible_roots
            .iter()
            .enumerate()
            .map(|(idx, &node_id)| self.render_publication_card(node_id, idx))
            .collect();

        // Add loading indicator or end-of-feed message
        if self.state.loading_more && !self.state.view.filter_drafts {
            let loading_text = if let Some(frame) = self.spinner_frame {
                format!("{} Loading more...", frame)
            } else {
                "Loading more...".to_string()
            };
            items.push(ListItem::new(Line::from(Span::styled(
                loading_text,
                Style::default().fg(Color::Yellow).italic(),
            ))));
        } else if self.state.feed_exhausted && !self.state.view.filter_drafts {
            items.push(ListItem::new(Line::from(Span::styled(
                "— End of feed —",
                Style::default().fg(Color::DarkGray).italic(),
            ))));
        } else if self.state.view.filter_drafts && items.is_empty() {
            items.push(ListItem::new(Line::from(Span::styled(
                "— No drafts —",
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

            // Check if this is a draft or local-created
            let is_draft = p.sync_status.is_draft();
            let is_local_created = p.sync_status.is_local_created();

            // Build multi-line card (sync bar rendered separately on right edge)
            let mut lines = Vec::new();

            // Add status banner
            if is_local_created {
                lines.push(Line::from(Span::styled(
                    "  [LOCAL - Not Published]",
                    Style::default().fg(Color::Rgb(255, 165, 0)).bold().italic(),
                )));
            } else if is_draft {
                lines.push(Line::from(Span::styled(
                    "  [DRAFT - Unsigned]",
                    Style::default().fg(Color::Red).bold().italic(),
                )));
            }

            // Title with color based on status
            let title_color = if is_local_created {
                Color::Rgb(255, 165, 0) // Orange
            } else if is_draft {
                Color::Red
            } else {
                Color::Cyan
            };
            lines.push(Line::from(Span::styled(title, Style::default().fg(title_color).bold())));

            lines.push(Line::from(Span::styled(
                format!("  by {} • {}", author, section_info),
                Style::default().fg(Color::DarkGray),
            )));

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
        self.get_visible_roots()
            .iter()
            .map(|&node_id| {
                if let Some(TreeNode::Publication(p)) = self.state.nodes.get(&node_id) {
                    // Base: Title + author line + separator = 3
                    // + summary if present = 4
                    // + draft/local banner if present = +1
                    let mut lines = if p.summary.is_some() { 4 } else { 3 };
                    if p.sync_status.is_draft() || p.sync_status.is_local_created() {
                        lines += 1;
                    }
                    (node_id, lines)
                } else {
                    (node_id, 1)
                }
            })
            .collect()
    }

    /// Get the visible roots (filtered if filter_drafts is enabled)
    fn get_visible_roots(&self) -> Vec<crate::tree::node::NodeId> {
        if self.state.view.filter_drafts {
            // Only show drafts
            self.state
                .roots
                .iter()
                .filter(|&node_id| {
                    if let Some(TreeNode::Publication(p)) = self.state.nodes.get(node_id) {
                        p.sync_status.is_draft()
                    } else {
                        false
                    }
                })
                .copied()
                .collect()
        } else {
            self.state.roots.clone()
        }
    }
}

impl<'a> Widget for FeedWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::node::SyncStatus;

        let card_lines = self.get_card_line_counts();
        let items = self.create_items();

        // Title includes filter indicator
        let title = if self.state.view.filter_drafts {
            " Publications [Drafts Only] "
        } else {
            " Publications "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title);

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
                    SyncStatus::LocalCreated => Color::Rgb(255, 165, 0), // Orange
                    SyncStatus::Draft => Color::Red,
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
                        let title = s
                            .title
                            .clone()
                            .unwrap_or_else(|| s.addr.short_format());

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
                SyncStatus::LocalCreated => Color::Rgb(255, 165, 0), // Orange
                SyncStatus::Draft => Color::Red,
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
            for &child_id in p.children.iter() {
                if let Some(TreeNode::Section(s)) = self.state.nodes.get(&child_id) {
                    let section_title = s
                        .title
                        .clone()
                        .unwrap_or_else(|| s.addr.d_tag.clone());
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
                    let title = s
                        .title
                        .clone()
                        .unwrap_or_else(|| s.addr.d_tag.clone());
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

        // Build the middle section (cursor | mode | view | preview)
        let middle = format!(
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

        // Get identity indicator components
        let (identity_text, identity_style, banner_text, banner_style) =
            get_identity_indicator(&self.state.identity.status);

        // Calculate total right-side length (middle + identity)
        let identity_section_len = if let (Some(ref id_text), Some(ref banner)) = (&identity_text, &banner_text) {
            id_text.len() + 1 + banner.len() + 1 // "[id_text] [banner] "
        } else {
            0
        };
        let right_total_len = middle.len() + identity_section_len;

        // Fill background first
        let bg_style = Style::default().fg(Color::Black).bg(Color::White);
        for x in area.x..area.x + area.width {
            buf[(x, area.y)].set_char(' ').set_style(bg_style);
        }

        // Render left section
        let left_display: String = left.chars().take((area.width as usize).saturating_sub(right_total_len + 2)).collect();
        buf.set_string(area.x, area.y, &left_display, bg_style);

        // Calculate where the right section starts
        let right_start = area.x + area.width - right_total_len as u16;

        // Render middle section
        buf.set_string(right_start, area.y, &middle, bg_style);

        // Render identity section if logged in
        if let (Some(id_text), Some(id_style), Some(banner), Some(b_style)) =
            (identity_text, identity_style, banner_text, banner_style)
        {
            let identity_x = right_start + middle.len() as u16 + 1;
            // Render the abbreviated pubkey
            buf.set_string(identity_x, area.y, &id_text, id_style.bg(Color::White));
            // Render the banner
            let banner_x = identity_x + id_text.len() as u16 + 1;
            buf.set_string(banner_x, area.y, &banner, b_style);
        }
    }
}

/// Get the identity indicator components for the status bar
fn get_identity_indicator(status: &LoginStatus) -> (Option<String>, Option<Style>, Option<String>, Option<Style>) {
    use crate::identity::abbreviate_pubkey_hex;

    match status {
        LoginStatus::None => (None, None, None, None),
        LoginStatus::ReadOnly { pubkey, .. } => (
            Some(abbreviate_pubkey_hex(pubkey)),
            Some(Style::default().fg(Color::Magenta)),
            Some(" Read Only ".to_string()),
            Some(Style::default().fg(Color::White).bg(Color::Magenta)),
        ),
        LoginStatus::EncryptedLocked { pubkey, .. } => {
            let display = pubkey
                .as_ref()
                .map(|pk| abbreviate_pubkey_hex(pk))
                .unwrap_or_else(|| "ncryptsec".to_string());
            (
                Some(display),
                Some(Style::default().fg(Color::Green)),
                Some(" need password ".to_string()),
                Some(Style::default().fg(Color::Black).bg(Color::Yellow)),
            )
        }
        LoginStatus::SignedIn { pubkey, .. } => (
            Some(abbreviate_pubkey_hex(pubkey)),
            Some(Style::default().fg(Color::Green)),
            Some(" signed in ".to_string()),
            Some(Style::default().fg(Color::Black).bg(Color::Green)),
        ),
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
                "Tab:Add Tag  Shift+Tab:Delete Tag  Ctrl+t:Exit Tags  Ctrl+p:Preview  Ctrl+Enter:Publish  Esc:Cancel"
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
            CommandCategory::Window => Style::default().fg(Color::LightMagenta),
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
            .title(" Commands (SPC/M-x) ")
            .style(Style::default().bg(Color::Black))
            .inner(area);

        // Render border
        Block::default()
            .borders(Borders::ALL)
            .title(" Commands (SPC/M-x) ")
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
    /// Optional pubkey to show in preview (from identity)
    pubkey: Option<&'a str>,
}

impl<'a> ComposeWidget<'a> {
    pub fn new(state: &'a ComposeState) -> Self {
        ComposeWidget { state, pubkey: None }
    }

    /// Set the pubkey to use in the event preview
    pub fn with_pubkey(mut self, pubkey: Option<&'a str>) -> Self {
        self.pubkey = pubkey;
        self
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

        // Sections (NKBIP-01: 30040 has no content, only sections)
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

                // Section tags (if any or in tag mode)
                if !section.tags.is_empty() || section.tag_mode {
                    let _max_tag_height = 3u16;
                    // Render section tags inline
                    let is_tag_name_focused = matches!(self.state.focus, ComposeFocus::SectionTagName(i) if i == idx);
                    let is_tag_value_focused = matches!(self.state.focus, ComposeFocus::SectionTagValue(i) if i == idx);

                    // Show existing tags
                    if !section.tags.is_empty() {
                        let tags_str = section.tags.iter()
                            .map(|t| format!("[{}:{}]", t.name, t.value))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let tags_display: String = tags_str.chars().take(inner.width as usize).collect();
                        buf.set_string(inner.x, y, &tags_display, Style::default().fg(Color::Yellow));
                        y += 1;
                    }

                    // Show tag input if in tag mode
                    if section.tag_mode && y < inner.y + inner.height - 2 {
                        self.render_text_field(
                            buf,
                            Rect { x: inner.x, y, width: inner.width / 2, height: 1 },
                            "Tag",
                            &section.current_tag_name,
                            is_tag_name_focused,
                            if is_tag_name_focused { self.state.cursor_pos } else { 0 },
                        );
                        self.render_text_field(
                            buf,
                            Rect { x: inner.x + inner.width / 2, y, width: inner.width / 2, height: 1 },
                            "Value",
                            &section.current_tag_value,
                            is_tag_value_focused,
                            if is_tag_value_focused { self.state.cursor_pos } else { 0 },
                        );
                        y += 1;
                    }

                    // Divider
                    let section_divider: String = "─".repeat(inner.width as usize);
                    buf.set_string(inner.x, y, &section_divider, Style::default().fg(Color::DarkGray));
                    y += 1;
                }

                // Section content
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
            // No sections yet - prompt to add one
            let hint1 = "NKBIP-01 publications require at least one section.";
            let hint2 = "Press Ctrl+s to add a section.";
            buf.set_string(inner.x, y, hint1, Style::default().fg(Color::DarkGray));
            y += 1;
            if y < inner.y + inner.height {
                buf.set_string(inner.x, y, hint2, Style::default().fg(Color::Yellow));
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

        // Get the preview JSON (with pubkey if available)
        let preview_json = self.state.preview_event_json_with_pubkey(self.pubkey);

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

/// Widget for rendering a window overlay
pub struct WindowWidget<'a> {
    window: &'a crate::tree::state::WindowState,
    is_focused: bool,
}

impl<'a> WindowWidget<'a> {
    pub fn new(window: &'a crate::tree::state::WindowState, is_focused: bool) -> Self {
        WindowWidget { window, is_focused }
    }

    /// Calculate the area for a window based on its size hints
    pub fn calculate_area(parent: Rect, width_percent: u16, height_percent: u16) -> Rect {
        let width = (parent.width as u32 * width_percent as u32 / 100) as u16;
        let height = (parent.height as u32 * height_percent as u32 / 100) as u16;
        let width = width.max(20).min(parent.width);
        let height = height.max(5).min(parent.height);
        let x = (parent.width.saturating_sub(width)) / 2;
        let y = (parent.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for WindowWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear area
        Clear.render(area, buf);

        // Border style based on focus
        let border_style = if self.is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = format!(" {} ", self.window.title);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title.as_str())
            .style(Style::default().bg(Color::Black));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 1 || inner.width < 1 {
            return;
        }

        // Render content lines
        let viewport_height = inner.height as usize;
        let visible_lines = self.window.visible_lines(viewport_height);

        for (i, line) in visible_lines.iter().enumerate() {
            if i >= inner.height as usize {
                break;
            }
            let y = inner.y + i as u16;

            // Truncate line to fit width
            let display_line: String = line.chars().take(inner.width as usize).collect();

            // Simple syntax highlighting for JSON-like content
            let style = if display_line.contains("\":") {
                // Key
                Style::default().fg(Color::Cyan)
            } else if display_line.contains("\"") {
                // String value
                Style::default().fg(Color::Green)
            } else if display_line.trim().starts_with('{')
                || display_line.trim().starts_with('}')
                || display_line.trim().starts_with('[')
                || display_line.trim().starts_with(']')
            {
                // Braces
                Style::default().fg(Color::White)
            } else if display_line.trim().parse::<f64>().is_ok() {
                // Number
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::White)
            };

            buf.set_string(inner.x, y, &display_line, style);
        }

        // Scroll indicator
        let total = self.window.total_lines();
        if total > viewport_height {
            let scroll_info = format!(
                " {}-{}/{} ",
                self.window.scroll_offset + 1,
                (self.window.scroll_offset + viewport_height).min(total),
                total
            );
            let info_x = area.x + area.width - scroll_info.len() as u16 - 1;
            buf.set_string(info_x, area.y, &scroll_info, Style::default().fg(Color::DarkGray));
        }

        // Help hint at bottom
        let help = if self.is_focused { " j/k:scroll  gg/G:top/bottom  q:close " } else { " Tab:focus " };
        let help_x = area.x + 1;
        let help_y = area.y + area.height - 1;
        if help.len() < area.width as usize - 2 {
            buf.set_string(help_x, help_y, help, Style::default().fg(Color::DarkGray));
        }
    }
}

/// Widget for the login dialog
pub struct LoginDialogWidget<'a> {
    state: &'a LoginDialogState,
}

impl<'a> LoginDialogWidget<'a> {
    pub fn new(state: &'a LoginDialogState) -> Self {
        LoginDialogWidget { state }
    }

    /// Calculate the area for the dialog (centered popup)
    pub fn calculate_area(parent: Rect) -> Rect {
        let width = parent.width.min(60).max(40);
        let height = 7; // Title bar + input + error + help + borders
        let x = (parent.width.saturating_sub(width)) / 2;
        let y = (parent.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for LoginDialogWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first
        Clear.render(area, buf);

        // Dialog border and title
        let title = format!(" {} ", self.state.title());
        let border_style = Style::default().fg(Color::Cyan);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title.as_str())
            .style(Style::default().bg(Color::Black));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let mut y = inner.y;

        // Input label
        let label = if self.state.awaiting_password {
            "Password:"
        } else {
            "Key:"
        };
        buf.set_string(inner.x, y, label, Style::default().fg(Color::Cyan));
        y += 1;

        // Input field with cursor
        let input_style = Style::default().bg(Color::DarkGray);
        let display_text = self.state.display_text();
        let placeholder = self.state.placeholder();

        // Clear input area background
        for x in inner.x..inner.x + inner.width {
            buf[(x, y)].set_char(' ').set_style(input_style);
        }

        // Show placeholder or input
        if display_text.is_empty() {
            buf.set_string(inner.x, y, placeholder, Style::default().fg(Color::DarkGray).bg(Color::DarkGray).italic());
        } else {
            // Calculate visible portion (scroll if text is longer than width)
            let max_visible = inner.width as usize - 1;
            let cursor_pos = self.state.cursor_pos;
            let visible_start = if cursor_pos > max_visible {
                cursor_pos - max_visible + 1
            } else {
                0
            };
            let visible_text: String = display_text.chars().skip(visible_start).take(max_visible).collect();
            buf.set_string(inner.x, y, &visible_text, input_style);

            // Render cursor
            let cursor_screen_pos = cursor_pos.saturating_sub(visible_start);
            let cursor_x = inner.x + cursor_screen_pos as u16;
            if cursor_x < inner.x + inner.width {
                buf[(cursor_x, y)].set_style(Style::default().bg(Color::White).fg(Color::Black));
            }
        }
        y += 1;

        // Error message (if any)
        if let Some(ref error) = self.state.error {
            let error_display: String = error.chars().take(inner.width as usize).collect();
            buf.set_string(inner.x, y, &error_display, Style::default().fg(Color::Red));
        }
        y += 1;

        // Help text at bottom
        let help = if self.state.awaiting_password {
            "Enter:Submit  Esc:Cancel"
        } else {
            "Enter:Login  Esc:Cancel"
        };
        if y < inner.y + inner.height {
            buf.set_string(inner.x, y, help, Style::default().fg(Color::DarkGray));
        }
    }
}

/// Widget for the user data menu (NIP-51 list selection)
pub struct UserDataMenuWidget<'a> {
    state: &'a UserDataMenuState,
}

impl<'a> UserDataMenuWidget<'a> {
    pub fn new(state: &'a UserDataMenuState) -> Self {
        UserDataMenuWidget { state }
    }

    /// Calculate the area for the menu (centered popup)
    pub fn calculate_area(parent: Rect) -> Rect {
        let width = parent.width.min(45).max(35);
        let height = 12; // Title + 8 items + help + borders
        let x = (parent.width.saturating_sub(width)) / 2;
        let y = (parent.height.saturating_sub(height)) / 2;
        Rect::new(x, y, width, height)
    }
}

impl Widget for UserDataMenuWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear the area first
        Clear.render(area, buf);

        // Dialog border and title
        let border_style = Style::default().fg(Color::Cyan);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" User Data ")
            .style(Style::default().bg(Color::Black));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 3 || inner.width < 10 {
            return;
        }

        let mut y = inner.y;

        // Render each item
        for (i, item) in self.state.items.iter().enumerate() {
            if y >= inner.y + inner.height - 1 {
                break;
            }

            let is_selected = i == self.state.selected;
            let style = if is_selected {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };

            // Clear line
            for x in inner.x..inner.x + inner.width {
                buf[(x, y)].set_char(' ').set_style(style);
            }

            // Selection indicator
            let prefix = if is_selected { "> " } else { "  " };

            // Render item
            let text = format!("{}{}", prefix, item.display_name());
            let display: String = text.chars().take(inner.width as usize).collect();
            buf.set_string(inner.x, y, &display, style);

            y += 1;
        }

        // Help text at bottom
        if y < inner.y + inner.height {
            let help = "j/k:Navigate  Enter:Select  q:Close";
            buf.set_string(inner.x, y, help, Style::default().fg(Color::DarkGray));
        }
    }
}

/// Widget for editor-style compose mode
///
/// Renders a single text buffer with visual indicators for structure:
/// - Headings highlighted with event kind (30040/30041)
/// - Attributes shown with "t" indicator
/// - Code blocks with language label and distinct background
pub struct EditorComposeWidget<'a> {
    state: &'a crate::tree::state::EditorComposeState,
}

impl<'a> EditorComposeWidget<'a> {
    pub fn new(state: &'a crate::tree::state::EditorComposeState) -> Self {
        EditorComposeWidget { state }
    }

    /// Get style for a line type
    fn line_style(line_type: &crate::tree::parser::LineType) -> Style {
        use crate::tree::parser::LineType;
        match line_type {
            LineType::Heading { event_kind: 30040, .. } => {
                Style::default().fg(Color::Cyan).bold()
            }
            LineType::Heading { event_kind: 30041, .. } => {
                Style::default().fg(Color::Green).bold()
            }
            LineType::Heading { .. } => Style::default().bold(),
            LineType::Attribute { .. } => Style::default().fg(Color::DarkGray),
            LineType::CodeStart { .. } => {
                Style::default().fg(Color::Yellow).bg(Color::Rgb(30, 30, 40))
            }
            LineType::CodeBody => Style::default().bg(Color::Rgb(30, 30, 40)),
            LineType::CodeEnd => {
                Style::default().fg(Color::Yellow).bg(Color::Rgb(30, 30, 40))
            }
            LineType::Prose => Style::default(),
            LineType::Empty => Style::default(),
        }
    }

    /// Get indicator text for right margin
    fn indicator_style() -> Style {
        Style::default().fg(Color::DarkGray)
    }
}

impl<'a> Widget for EditorComposeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        use crate::tree::state::EditorViewMode;

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(
                " Editor [{}] [{}] ",
                self.state.mode.name(),
                self.state.view_mode.name()
            ));

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 2 || inner.width < 10 {
            return;
        }

        // Dispatch to the appropriate view renderer
        match self.state.view_mode {
            EditorViewMode::Plain => self.render_plain_view(inner, buf),
            EditorViewMode::Json => self.render_json_view(inner, buf),
            EditorViewMode::Structured => self.render_structured_view(inner, buf),
        }
    }
}

impl<'a> EditorComposeWidget<'a> {
    /// Render the plain text editor view
    fn render_plain_view(&self, inner: Rect, buf: &mut Buffer) {
        use crate::tree::parser::LineType;

        // Parse the document
        let parsed = crate::tree::parser::ParsedDocument::parse(&self.state.content, self.state.mode);

        // Reserve space for indicators on the right
        let indicator_width = 6;
        let text_width = inner.width.saturating_sub(indicator_width + 1);

        // Calculate effective scroll to keep cursor visible
        let visible_lines = inner.height as usize;
        let scroll = {
            let mut s = self.state.scroll;
            if self.state.cursor_line < s {
                s = self.state.cursor_line;
            } else if self.state.cursor_line >= s + visible_lines {
                s = self.state.cursor_line - visible_lines + 1;
            }
            s
        };

        // Render visible lines
        for (i, line_idx) in (scroll..scroll + visible_lines).enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }

            // Get parsed line info
            let (line_content, line_type) = if let Some(parsed_line) = parsed.lines.get(line_idx) {
                (parsed_line.content.as_str(), &parsed_line.line_type)
            } else {
                // Past end of document - show empty line
                ("", &LineType::Empty)
            };

            // Determine if this is the cursor line
            let is_cursor_line = line_idx == self.state.cursor_line;

            // Get base style for this line type
            let mut style = Self::line_style(line_type);

            // Highlight cursor line
            if is_cursor_line && !self.state.insert_mode {
                style = style.bg(Color::Rgb(40, 40, 50));
            }

            // Clear the line with style
            for x in inner.x..inner.x + text_width {
                buf[(x, y)].set_char(' ').set_style(style);
            }

            // Render line content (truncated to fit)
            let display: String = line_content.chars().take(text_width as usize).collect();
            buf.set_string(inner.x, y, &display, style);

            // Render cursor if in insert mode and on this line
            if is_cursor_line && self.state.insert_mode {
                let cursor_x = inner.x + self.state.cursor_col.min(text_width as usize) as u16;
                if cursor_x < inner.x + text_width {
                    buf[(cursor_x, y)]
                        .set_style(Style::default().bg(Color::White).fg(Color::Black));
                }
            }

            // Render indicator on the right
            let indicator = line_type.indicator();
            if !indicator.is_empty() {
                let indicator_x = inner.x + text_width + 1;
                let display_indicator: String = indicator.chars().take(indicator_width as usize).collect();
                buf.set_string(indicator_x, y, &display_indicator, Self::indicator_style());
            }
        }

        // Status line at bottom (if room)
        if inner.height > 1 {
            let status_y = inner.y + inner.height - 1;
            let mode_str = if self.state.insert_mode { "INSERT" } else { "NORMAL" };
            let status = format!(
                " {} | L{}/{} C{} ",
                mode_str,
                self.state.cursor_line + 1,
                self.state.line_count(),
                self.state.cursor_col + 1
            );
            buf.set_string(
                inner.x,
                status_y,
                &status,
                Style::default().fg(Color::DarkGray),
            );

            // Show current code block language if in one
            let blocks = parsed.code_blocks();
            for (start, end, lang) in blocks {
                if self.state.cursor_line >= start && self.state.cursor_line <= end && !lang.is_empty() {
                    let lang_status = format!("[{}]", lang);
                    let lang_x = inner.x + inner.width - lang_status.len() as u16 - 1;
                    buf.set_string(lang_x, status_y, &lang_status, Style::default().fg(Color::Yellow));
                    break;
                }
            }
        }
    }

    /// Render the JSON preview view showing events that would be generated
    fn render_json_view(&self, inner: Rect, buf: &mut Buffer) {
        // Parse document to get sections
        let parsed =
            crate::tree::parser::ParsedDocument::parse(&self.state.content, self.state.mode);
        let sections = parsed.sections();

        // Build a JSON representation with styled segments
        // Each line is a vector of (text, style) pairs for syntax highlighting
        let mut lines: Vec<Vec<(String, Style)>> = Vec::new();

        // Style definitions for JSON syntax highlighting
        let style_comment = Style::default().fg(Color::DarkGray).italic();
        let style_brace = Style::default().fg(Color::White);
        let style_key = Style::default().fg(Color::Cyan);
        let style_string = Style::default().fg(Color::Green);
        let style_number = Style::default().fg(Color::Yellow);
        let style_bracket = Style::default().fg(Color::White);

        // Helper to create a styled line from segments
        fn line(segments: Vec<(&str, Style)>) -> Vec<(String, Style)> {
            segments
                .into_iter()
                .map(|(s, st)| (s.to_string(), st))
                .collect()
        }

        // Header comment
        lines.push(line(vec![("// Events that would be generated:", style_comment)]));
        lines.push(vec![]);

        // Index event (30040) header
        lines.push(line(vec![
            ("// ", style_comment),
            ("[I]", Style::default().fg(Color::Magenta).bold()),
            (" Index Event (kind 30040)", style_comment),
        ]));
        lines.push(line(vec![("{", style_brace)]));
        lines.push(line(vec![
            ("  ", style_brace),
            ("\"kind\"", style_key),
            (": ", style_brace),
            ("30040", style_number),
            (",", style_brace),
        ]));
        lines.push(line(vec![
            ("  ", style_brace),
            ("\"content\"", style_key),
            (": ", style_brace),
            ("\"\"", style_string),
            (",", style_brace),
        ]));

        // d-tag for the publication
        lines.push(line(vec![
            ("  ", style_brace),
            ("\"tags\"", style_key),
            (": [", style_bracket),
        ]));
        lines.push(line(vec![
            ("    [", style_bracket),
            ("\"d\"", style_string),
            (", ", style_bracket),
            ("\"<publication-id>\"", style_string),
            ("],", style_bracket),
        ]));

        // a-tags referencing sections
        for (i, section) in sections.iter().enumerate() {
            let title = if section.title.is_empty() {
                "Untitled"
            } else {
                &section.title
            };
            let comma = if i < sections.len() - 1 { "," } else { "" };
            let d_tag = format!("section-{}", i);

            lines.push(line(vec![
                ("    [", style_bracket),
                ("\"a\"", style_string),
                (", ", style_bracket),
            ]));
            // Split the a-tag value for syntax highlighting
            lines.push(vec![
                ("      \"30041:".to_string(), style_string),
                ("<pubkey>".to_string(), Style::default().fg(Color::Gray)),
                (format!(":{}\"{}", d_tag, comma), style_string),
                (format!("  // {}", title), style_comment),
            ]);
            // Close the array if not last, or leave it for the next iteration
            if i == sections.len() - 1 {
                lines.push(line(vec![("    ]", style_bracket)]));
            } else {
                lines.push(line(vec![("    ],", style_bracket)]));
            }
        }

        if sections.is_empty() {
            // If no sections, close the tags array
            lines.push(line(vec![("  ]", style_bracket)]));
        } else {
            lines.push(line(vec![("  ]", style_bracket)]));
        }
        lines.push(line(vec![("}", style_brace)]));
        lines.push(vec![]);

        // Content events (30041)
        for (i, section) in sections.iter().enumerate() {
            let title = if section.title.is_empty() {
                "Untitled"
            } else {
                &section.title
            };
            let line_count = section.end_line.saturating_sub(section.start_line) + 1;
            let d_tag = format!("section-{}", i);

            // Section header comment
            lines.push(line(vec![
                ("// ", style_comment),
                ("[C]", Style::default().fg(Color::Blue).bold()),
                (&format!(" Section {} (kind 30041): {}", i + 1, title), style_comment),
            ]));
            lines.push(line(vec![("{", style_brace)]));
            lines.push(line(vec![
                ("  ", style_brace),
                ("\"kind\"", style_key),
                (": ", style_brace),
                ("30041", style_number),
                (",", style_brace),
            ]));
            lines.push(line(vec![
                ("  ", style_brace),
                ("\"content\"", style_key),
                (": ", style_brace),
            ]));
            lines.push(vec![(
                format!("    \"<{} lines of content>\"", line_count),
                style_string,
            ), (",".to_string(), style_brace)]);
            lines.push(line(vec![
                ("  ", style_brace),
                ("\"tags\"", style_key),
                (": [", style_bracket),
            ]));
            lines.push(line(vec![
                ("    [", style_bracket),
                ("\"d\"", style_string),
                (", ", style_bracket),
                (&format!("\"{}\"", d_tag), style_string),
                ("],", style_bracket),
            ]));
            lines.push(vec![
                ("    [".to_string(), style_bracket),
                ("\"title\"".to_string(), style_string),
                (", ".to_string(), style_bracket),
                (format!("\"{}\"", title), style_string),
                ("]".to_string(), style_bracket),
            ]);
            lines.push(line(vec![("  ]", style_bracket)]));
            lines.push(line(vec![("}", style_brace)]));
            lines.push(vec![]);
        }

        // Render the lines with scroll support using view_scroll and view_cursor
        let visible_lines = inner.height.saturating_sub(1) as usize; // Reserve 1 line for status
        let total_lines = lines.len();

        // Clamp view_cursor to valid range
        let cursor = self.state.view_cursor.min(total_lines.saturating_sub(1));

        // Adjust scroll to keep cursor visible
        let scroll = if total_lines <= visible_lines {
            0
        } else if cursor < self.state.view_scroll {
            cursor
        } else if cursor >= self.state.view_scroll + visible_lines {
            cursor.saturating_sub(visible_lines) + 1
        } else {
            self.state
                .view_scroll
                .min(total_lines.saturating_sub(visible_lines))
        };

        for (i, styled_line) in lines.iter().skip(scroll).take(visible_lines).enumerate() {
            let y = inner.y + i as u16;
            let line_index = scroll + i;
            let is_current = line_index == cursor;

            // Clear the line first if it's the current line (for background)
            if is_current {
                let bg_style = Style::default().bg(Color::DarkGray);
                buf.set_string(inner.x, y, " ".repeat(inner.width as usize), bg_style);
            }

            // Render each styled segment
            let mut x_offset = 0u16;
            for (text, style) in styled_line {
                let display_text: String = text.chars().take((inner.width as usize).saturating_sub(x_offset as usize)).collect();
                let final_style = if is_current {
                    style.bg(Color::DarkGray)
                } else {
                    *style
                };
                buf.set_string(inner.x + x_offset, y, &display_text, final_style);
                x_offset += display_text.len() as u16;
                if x_offset >= inner.width {
                    break;
                }
            }
        }

        // Status line
        if inner.height > 1 {
            let status_y = inner.y + inner.height - 1;
            let status = format!(
                " {} sections | [I]=Index [C]=Content | L{}/{} | j/k gg/G nav | v switch ",
                sections.len(),
                cursor + 1,
                total_lines
            );
            buf.set_string(
                inner.x,
                status_y,
                &status,
                Style::default().fg(Color::DarkGray),
            );
        }
    }

    /// Render the structured document tree view
    fn render_structured_view(&self, inner: Rect, buf: &mut Buffer) {
        // Parse document to get sections
        let parsed =
            crate::tree::parser::ParsedDocument::parse(&self.state.content, self.state.mode);
        let sections = parsed.sections();

        // Use styled segments like JSON view for better highlighting
        let mut lines: Vec<Vec<(String, Style)>> = Vec::new();

        // Style definitions
        let style_header = Style::default().fg(Color::White).bold();
        let style_dim = Style::default().fg(Color::DarkGray);
        let style_tree = Style::default().fg(Color::Gray);
        let style_title = Style::default().fg(Color::Green).bold();
        let style_index = Style::default().fg(Color::Magenta).bold();
        let style_content = Style::default().fg(Color::Blue).bold();
        let style_value = Style::default().fg(Color::Cyan);

        // Document structure header with statistics
        lines.push(vec![("Document Structure".to_string(), style_header)]);
        lines.push(vec![]);

        // Statistics summary
        let index_count = 1; // Always one 30040 index event
        let content_count = sections.len();
        lines.push(vec![
            ("  Events: ".to_string(), style_dim),
            (format!("{}", index_count + content_count), style_value),
            (" total".to_string(), style_dim),
        ]);
        lines.push(vec![
            ("    ".to_string(), Style::default()),
            ("[I]".to_string(), style_index),
            (format!(" Index (30040): {}", index_count), style_dim),
        ]);
        lines.push(vec![
            ("    ".to_string(), Style::default()),
            ("[C]".to_string(), style_content),
            (format!(" Content (30041): {}", content_count), style_dim),
        ]);
        lines.push(vec![]);

        // Index event representation
        lines.push(vec![
            ("[I]".to_string(), style_index),
            (" Publication Index".to_string(), style_header),
        ]);
        lines.push(vec![
            ("    ".to_string(), Style::default()),
            ("d-tag: ".to_string(), style_dim),
            ("<publication-id>".to_string(), style_value),
        ]);
        lines.push(vec![
            ("    ".to_string(), Style::default()),
            (format!("references: {} sections", sections.len()), style_dim),
        ]);
        lines.push(vec![]);

        // Render each section as a tree node
        for (i, section) in sections.iter().enumerate() {
            let title = if section.title.is_empty() {
                "Untitled"
            } else {
                &section.title
            };
            let is_last = i == sections.len() - 1;
            let tree_char = if is_last { "└" } else { "├" };
            let detail_prefix = if is_last { "    " } else { "│   " };
            let line_count = section.end_line.saturating_sub(section.start_line) + 1;
            let d_tag = format!("section-{}", i);

            // Section header with event kind indicator
            lines.push(vec![
                (format!("{}── ", tree_char), style_tree),
                ("[C]".to_string(), style_content),
                (format!(" {}", title), style_title),
            ]);

            // Section details
            lines.push(vec![
                (format!("{}  ", detail_prefix), style_tree),
                ("d-tag: ".to_string(), style_dim),
                (d_tag, style_value),
            ]);
            lines.push(vec![
                (format!("{}  ", detail_prefix), style_tree),
                ("lines: ".to_string(), style_dim),
                (
                    format!("{}-{}", section.start_line + 1, section.end_line + 1),
                    style_value,
                ),
                (format!(" ({} lines)", line_count), style_dim),
            ]);
        }

        // Render the lines with scroll support using view_scroll and view_cursor
        let visible_lines = inner.height.saturating_sub(1) as usize;
        let total_lines = lines.len();

        // Clamp view_cursor to valid range
        let cursor = self.state.view_cursor.min(total_lines.saturating_sub(1));

        // Adjust scroll to keep cursor visible
        let scroll = if total_lines <= visible_lines {
            0
        } else if cursor < self.state.view_scroll {
            cursor
        } else if cursor >= self.state.view_scroll + visible_lines {
            cursor.saturating_sub(visible_lines) + 1
        } else {
            self.state
                .view_scroll
                .min(total_lines.saturating_sub(visible_lines))
        };

        for (i, styled_line) in lines.iter().skip(scroll).take(visible_lines).enumerate() {
            let y = inner.y + i as u16;
            let line_index = scroll + i;
            let is_current = line_index == cursor;

            // Clear the line first if it's the current line (for background)
            if is_current {
                let bg_style = Style::default().bg(Color::DarkGray);
                buf.set_string(inner.x, y, " ".repeat(inner.width as usize), bg_style);
            }

            // Render each styled segment
            let mut x_offset = 0u16;
            for (text, style) in styled_line {
                let display_text: String = text
                    .chars()
                    .take((inner.width as usize).saturating_sub(x_offset as usize))
                    .collect();
                let final_style = if is_current {
                    style.bg(Color::DarkGray)
                } else {
                    *style
                };
                buf.set_string(inner.x + x_offset, y, &display_text, final_style);
                x_offset += display_text.len() as u16;
                if x_offset >= inner.width {
                    break;
                }
            }
        }

        // Status line
        if inner.height > 1 {
            let status_y = inner.y + inner.height - 1;
            let status = format!(
                " {} events | [I]=30040 [C]=30041 | L{}/{} | j/k gg/G nav | v switch ",
                1 + sections.len(),
                cursor + 1,
                total_lines
            );
            buf.set_string(
                inner.x,
                status_y,
                &status,
                Style::default().fg(Color::DarkGray),
            );
        }
    }
}
