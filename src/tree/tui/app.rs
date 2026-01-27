//! TUI Application
//!
//! Main application loop for the terminal interface with async bridge.

use crate::engine::{Engine, FetchPolicy};
use crate::publication::{NAddr, PublicationEngine};
use crate::tree::command::{AsyncRequest, AsyncResult, CommandResult, ConfigAction};
use crate::tree::engine::{init_from_publications, TreeEngine};
use crate::tree::node::{NodeId, SectionNode, TreeNode};
use crate::tree::state::TreeState;
use crate::tree::tui::input::{KeyContext, KeyMapper};
use crate::tree::state::ViewMode;
use crate::tree::tui::widgets::{
    ContentPreview, ContinuousWidget, FeedWidget, HelpBar, OutlineWidget, PaginatedWidget,
    StatusBar, TreeWidget,
};

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use std::io::{self, stdout};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Message type for async operations
enum AsyncMessage {
    Result(AsyncResult),
}

/// The main TUI application
pub struct TuiApp {
    /// Tree state
    state: TreeState,
    /// Tree engine
    engine: TreeEngine,
    /// Key mapper
    key_mapper: KeyMapper,
    /// Nostr engine for data fetching
    nostr_engine: Arc<Engine>,
    /// Fetch policy
    policy: FetchPolicy,
    /// Custom relay URLs (overrides engine defaults when non-empty)
    custom_relays: Vec<String>,
    /// Status message
    status_message: Option<String>,
    /// Async message channel
    async_rx: mpsc::Receiver<AsyncMessage>,
    async_tx: mpsc::Sender<AsyncMessage>,
}

impl TuiApp {
    /// Create a new TUI application
    pub fn new(nostr_engine: Arc<Engine>, policy: FetchPolicy) -> Self {
        let (async_tx, async_rx) = mpsc::channel(32);

        TuiApp {
            state: TreeState::new(),
            engine: TreeEngine::new(),
            key_mapper: KeyMapper::new(),
            nostr_engine,
            policy,
            custom_relays: Vec::new(),
            status_message: None,
            async_rx,
            async_tx,
        }
    }

    /// Get the effective relays (custom if set, otherwise engine defaults)
    fn effective_relays(&self) -> Option<Vec<String>> {
        if self.custom_relays.is_empty() {
            None
        } else {
            Some(self.custom_relays.clone())
        }
    }

    /// Add a relay URL
    pub fn add_relay(&mut self, url: String) {
        if !self.custom_relays.contains(&url) {
            self.custom_relays.push(url.clone());
            self.status_message = Some(format!("Added relay: {}", url));
        } else {
            self.status_message = Some(format!("Relay already in list: {}", url));
        }
    }

    /// Remove a relay URL
    pub fn remove_relay(&mut self, url: &str) {
        if let Some(pos) = self.custom_relays.iter().position(|r| r == url) {
            self.custom_relays.remove(pos);
            self.status_message = Some(format!("Removed relay: {}", url));
        } else {
            self.status_message = Some(format!("Relay not found: {}", url));
        }
    }

    /// Clear all custom relays
    pub fn clear_relays(&mut self) {
        self.custom_relays.clear();
        self.status_message = Some("Cleared custom relays, using defaults".to_string());
    }

    /// Show current relay configuration
    pub fn show_relays(&mut self) {
        let relays = if self.custom_relays.is_empty() {
            self.nostr_engine.relays().to_vec()
        } else {
            self.custom_relays.clone()
        };
        let relay_list = relays.join(", ");
        let prefix = if self.custom_relays.is_empty() {
            "Default relays"
        } else {
            "Custom relays"
        };
        self.status_message = Some(format!("{}: {}", prefix, relay_list));
    }

    /// Load initial publications
    pub async fn load_initial(&mut self) -> anyhow::Result<()> {
        self.status_message = Some("Loading publications...".to_string());

        let pub_engine = PublicationEngine::new(&self.nostr_engine);
        let publications = pub_engine.list_root_publications(self.policy, 15).await?;

        let pubs: Vec<(NAddr, Option<String>)> = publications
            .iter()
            .map(|p| (p.addr.clone(), p.title.clone()))
            .collect();

        init_from_publications(&mut self.state, pubs);

        // Track oldest timestamp for pagination
        let mut oldest: Option<u64> = None;

        // Mark loaded publications as loaded in state
        for pub_ in &publications {
            let id = NodeId::from_addr(&pub_.addr);

            // Track oldest timestamp
            let created_at = pub_.created_at;
            if created_at > 0 {
                match oldest {
                    Some(old) if created_at < old => oldest = Some(created_at),
                    None => oldest = Some(created_at),
                    _ => {}
                }
            }

            // First, create all child nodes and collect IDs
            let mut child_ids = Vec::new();
            let mut new_nodes = Vec::new();

            for (i, section) in pub_.sections.iter().enumerate() {
                let child_id = NodeId::from_addr(&section.addr);
                child_ids.push(child_id);

                let mut sec_node = SectionNode::stub(section.addr.clone(), id, i);
                sec_node.title = section.title.clone();
                new_nodes.push((child_id, TreeNode::Section(sec_node)));
            }

            // Insert all child nodes
            for (child_id, node) in new_nodes {
                self.state.nodes.insert(child_id, node);
            }

            // Now update the parent publication node
            if let Some(TreeNode::Publication(ref mut node)) = self.state.nodes.get_mut(&id) {
                node.loaded = true;
                node.title = pub_.title.clone();
                node.summary = pub_.summary.clone();
                node.author = pub_.author_pubkey.clone();
                node.author_name = pub_.author_name.clone();
                node.version = pub_.version.clone();
                node.created_at = pub_.created_at;
                node.children = child_ids;
            }
        }

        // Store oldest timestamp for pagination
        self.state.oldest_timestamp = oldest;

        self.status_message = Some(format!("Loaded {} publications", publications.len()));
        Ok(())
    }

    /// Run the TUI event loop
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        let result = self.event_loop(&mut terminal).await;

        // Restore terminal
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        result
    }

    async fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
        loop {
            // Draw UI
            terminal.draw(|frame| self.draw(frame))?;

            // Check for async results
            while let Ok(msg) = self.async_rx.try_recv() {
                let AsyncMessage::Result(result) = msg;
                let cmd_result = self.engine.apply_async_result(&mut self.state, result);
                if let CommandResult::Error(e) = cmd_result {
                    self.status_message = Some(format!("Error: {}", e));
                } else {
                    self.status_message = None;
                }
            }

            // Poll for events with timeout
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    // Only handle key press events (not release)
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    let ctx = KeyContext {
                        app_mode: self.state.mode,
                        view_mode: self.state.view.mode,
                    };
                    if let Some(command) = self.key_mapper.map_with_context(key, Some(&ctx)) {
                        match self.engine.execute(&mut self.state, command) {
                            CommandResult::Exit => break,
                            CommandResult::NeedsAsync(request) => {
                                self.status_message = Some(request.description());
                                self.spawn_async_request(request);
                            }
                            CommandResult::Error(e) => {
                                self.status_message = Some(format!("Error: {}", e));
                            }
                            CommandResult::StateChanged => {
                                self.status_message = None;
                            }
                            CommandResult::ConfigChange(action) => {
                                self.handle_config_action(action);
                            }
                            CommandResult::ModeChanged(mode) => {
                                self.status_message = Some(format!("Switched to {} mode", mode.name()));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_config_action(&mut self, action: ConfigAction) {
        match action {
            ConfigAction::AddRelay(url) => self.add_relay(url),
            ConfigAction::RemoveRelay(url) => self.remove_relay(&url),
            ConfigAction::ClearRelays => self.clear_relays(),
            ConfigAction::ShowRelays => self.show_relays(),
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();

        // Calculate main content area (excluding status and help bars)
        let content_height = area.height.saturating_sub(2);
        let content_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: content_height,
        };

        // Render based on current mode
        if self.state.is_feed_mode() {
            self.draw_feed_mode(frame, content_area);
        } else {
            self.draw_reader_mode(frame, content_area);
        }

        // Render status bar
        let status_area = Rect {
            x: area.x,
            y: area.height.saturating_sub(2),
            width: area.width,
            height: 1,
        };

        if let Some(ref msg) = self.status_message {
            frame.render_widget(StatusBar::new(&self.state).with_message(msg), status_area);
        } else {
            frame.render_widget(StatusBar::new(&self.state), status_area);
        }

        // Render help bar
        let help_area = Rect {
            x: area.x,
            y: area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        frame.render_widget(HelpBar::new(&self.state), help_area);
    }

    fn draw_feed_mode(&self, frame: &mut Frame, area: Rect) {
        // Feed mode: list on left, preview on right (if enabled)
        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            // Render feed list
            frame.render_widget(FeedWidget::new(&self.state), chunks[0]);

            // Render preview of selected publication
            frame.render_widget(ContentPreview::new(&self.state), chunks[1]);
        } else {
            // Full width feed list
            frame.render_widget(FeedWidget::new(&self.state), area);
        }
    }

    fn draw_reader_mode(&self, frame: &mut Frame, area: Rect) {
        match self.state.view.mode {
            ViewMode::Tree => self.draw_tree_view(frame, area),
            ViewMode::Outline => self.draw_outline_view(frame, area),
            ViewMode::Continuous => self.draw_continuous_view(frame, area),
            ViewMode::Paginated => self.draw_paginated_view(frame, area),
        }
    }

    fn draw_tree_view(&self, frame: &mut Frame, area: Rect) {
        // Tree mode: tree on left, preview on right (if enabled)
        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            frame.render_widget(TreeWidget::new(&self.state), chunks[0]);
            frame.render_widget(ContentPreview::new(&self.state), chunks[1]);
        } else {
            frame.render_widget(TreeWidget::new(&self.state), area);
        }
    }

    fn draw_outline_view(&self, frame: &mut Frame, area: Rect) {
        // Outline mode: sections as cards, full width
        // Preview can show detailed content of selected section
        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);

            frame.render_widget(OutlineWidget::new(&self.state), chunks[0]);
            frame.render_widget(ContentPreview::new(&self.state), chunks[1]);
        } else {
            frame.render_widget(OutlineWidget::new(&self.state), area);
        }
    }

    fn draw_continuous_view(&self, frame: &mut Frame, area: Rect) {
        // Continuous mode: full scrollable content
        frame.render_widget(ContinuousWidget::new(&self.state), area);
    }

    fn draw_paginated_view(&self, frame: &mut Frame, area: Rect) {
        // Paginated mode: one section at a time, full width
        frame.render_widget(PaginatedWidget::new(&self.state), area);
    }

    fn spawn_async_request(&self, request: AsyncRequest) {
        let tx = self.async_tx.clone();
        let engine = self.nostr_engine.clone();
        let policy = self.policy;
        let custom_relays = self.effective_relays();

        tokio::spawn(async move {
            let result = execute_async_request(&engine, request.clone(), policy, custom_relays.as_deref()).await;
            match result {
                Ok(res) => {
                    let _ = tx.send(AsyncMessage::Result(res)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AsyncMessage::Result(AsyncResult::Error {
                            request,
                            error: e.to_string(),
                        }))
                        .await;
                }
            }
        });
    }
}

/// Execute an async request and return the result
async fn execute_async_request(
    engine: &Engine,
    request: AsyncRequest,
    policy: FetchPolicy,
    _override_relays: Option<&[String]>,
) -> anyhow::Result<AsyncResult> {
    // TODO: Pass override_relays to PublicationEngine methods once supported
    let pub_engine = PublicationEngine::new(engine);

    match request {
        AsyncRequest::LoadPublication { addr, parent: _ } => {
            let publication = pub_engine.load_publication(&addr, policy).await?;
            let node_id = NodeId::from_addr(&addr);

            let children: Vec<NAddr> = publication
                .sections
                .iter()
                .map(|s| s.addr.clone())
                .collect();

            Ok(AsyncResult::PublicationLoaded {
                node_id,
                title: publication.title,
                children,
            })
        }

        AsyncRequest::LoadSection { addr, parent: _ } => {
            let event = engine
                .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, policy)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Section not found"))?;

            let node_id = NodeId::from_addr(&addr);
            let title = event
                .get("tags")
                .and_then(|v| v.as_array())
                .and_then(|tags| {
                    tags.iter().find_map(|tag| {
                        let arr = tag.as_array()?;
                        if arr.first()?.as_str()? == "title" {
                            arr.get(1)?.as_str().map(String::from)
                        } else {
                            None
                        }
                    })
                });

            let content = event.get("content").and_then(|v| v.as_str()).map(String::from);

            Ok(AsyncResult::SectionLoaded {
                node_id,
                title,
                content,
            })
        }

        AsyncRequest::LoadChildren { parent } => {
            // This would need the parent's address to work properly
            // For now, return empty
            Ok(AsyncResult::ChildrenLoaded {
                parent_id: parent,
                children: Vec::new(),
            })
        }

        AsyncRequest::FindAlternates { addr, node_id } => {
            let versions = pub_engine.find_section_versions(&addr, policy).await?;

            let alternates = versions
                .into_iter()
                .map(|v| crate::tree::command::AlternateVersion {
                    author: v.author,
                    created_at: v.created_at,
                    version_label: v.version,
                })
                .collect();

            Ok(AsyncResult::AlternatesFound {
                node_id,
                versions: alternates,
            })
        }

        AsyncRequest::RefreshAll => {
            // Refresh would re-fetch all loaded data
            // For now, just return success
            Ok(AsyncResult::ChildrenLoaded {
                parent_id: NodeId::root(),
                children: Vec::new(),
            })
        }

        AsyncRequest::Search { query: _ } => {
            // Search not yet implemented
            Ok(AsyncResult::ChildrenLoaded {
                parent_id: NodeId::root(),
                children: Vec::new(),
            })
        }

        AsyncRequest::LoadMorePublications { before_timestamp, limit } => {
            let publications = pub_engine
                .list_publications_before(before_timestamp, policy, limit)
                .await?;

            let loaded: Vec<crate::tree::command::LoadedPublication> = publications
                .into_iter()
                .map(|p| crate::tree::command::LoadedPublication {
                    addr: p.addr,
                    title: p.title,
                    summary: p.summary,
                    author: p.author_pubkey,
                    author_name: p.author_name,
                    created_at: p.created_at,
                    sections: p.sections.iter().map(|s| s.addr.clone()).collect(),
                })
                .collect();

            Ok(AsyncResult::MorePublicationsLoaded { publications: loaded })
        }
    }
}
