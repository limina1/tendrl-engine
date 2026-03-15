//! TUI Application
//!
//! Main application loop for the terminal interface with async bridge.

use crate::drafts::{DraftStore, LocalPublicationTracker};
use crate::engine::{Engine, FetchPolicy};
use crate::identity::{parse_key, decrypt_ncryptsec, IdentityKeyring, KeyType, LoginStatus};
use crate::publication::{NAddr, PublicationEngine};
use crate::tree::command::{AsyncRequest, AsyncResult, CommandResult, ConfigAction, LoadedDraft};
use crate::tree::engine::{init_from_publications, TreeEngine};
use crate::tree::node::{NodeId, SectionNode, SyncStatus, TreeNode};
use crate::tree::state::{LoginDialogState, TreeState, ViewMode};
use crate::tree::tui::input::{KeyContext, KeyMapper};
use crate::tree::tui::spinner::Spinner;
use crate::tree::tui::widgets::{
    CommandPaletteWidget, ComposeWidget, ContentPreview, ContinuousWidget, EditorComposeWidget,
    FeedWidget, HelpBar, JsonPreview, LoginDialogWidget, OutlineWidget, PaginatedWidget, StatusBar,
    TreeWidget, UserDataMenuWidget, WindowWidget,
};

use crate::tree::command::TreeCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
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
    /// Animated spinner for loading indicators
    spinner: Spinner,
    /// Number of pending async requests (for showing spinner)
    pending_count: usize,
    /// Draft storage for unsigned publications
    draft_store: Option<DraftStore>,
    /// Identity keyring for secure storage
    keyring: IdentityKeyring,
    /// Session secret cache (fallback when keyring fails)
    /// Maps pubkey -> secret_hex
    session_secrets: std::collections::HashMap<String, String>,
    /// Tracker for locally-created publications (not yet published to relays)
    local_tracker: Option<LocalPublicationTracker>,
}

impl TuiApp {
    /// Create a new TUI application
    pub fn new(nostr_engine: Arc<Engine>, policy: FetchPolicy) -> Self {
        let (async_tx, async_rx) = mpsc::channel(32);

        // Initialize draft store in the same data directory as the engine
        let draft_store = DraftStore::new(nostr_engine.data_dir()).ok();

        // Initialize local publication tracker
        let local_tracker = LocalPublicationTracker::new(nostr_engine.data_dir()).ok();

        // Initialize keyring for identity storage
        let keyring = IdentityKeyring::new();

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
            spinner: Spinner::new(),
            pending_count: 0,
            draft_store,
            keyring,
            session_secrets: std::collections::HashMap::new(),
            local_tracker,
        }
    }

    /// Set ncryptsec from file (will prompt for password on sign-in)
    pub fn set_ncryptsec(&mut self, ncryptsec: String) {
        if let Err(e) = self.state.identity.login_ncryptsec(&ncryptsec) {
            tracing::warn!("Failed to set ncryptsec: {}", e);
        } else {
            self.status_message = Some("Press 'i' to unlock with password".to_string());
        }
    }

    /// Try to restore identity from keyring on startup
    pub fn restore_identity(&mut self) {
        if let Ok((key_type, key_data)) = self.keyring.get_last_identity() {
            match key_type.as_str() {
                "npub" => {
                    if let Err(e) = self.state.identity.login_npub(&key_data) {
                        tracing::warn!("Failed to restore npub identity: {}", e);
                    } else {
                        self.status_message = Some("Restored read-only session".to_string());
                        // Load user profile data
                        self.load_user_data_for_current_identity();
                    }
                }
                "ncryptsec" => {
                    // For ncryptsec, we restore to locked state (user needs to enter password)
                    // User data will be loaded after password is entered
                    if let Err(e) = self.state.identity.login_ncryptsec(&key_data) {
                        tracing::warn!("Failed to restore ncryptsec identity: {}", e);
                    } else {
                        self.status_message = Some("Session restored - enter password to unlock".to_string());
                    }
                }
                "nsec" => {
                    // For nsec, check if we have the secret stored
                    // We don't store nsec directly in last_identity, only the pubkey
                    // The actual secret should be in the keyring under the pubkey
                    if let Ok(secret) = self.keyring.get_secret(&key_data) {
                        // key_data here is the pubkey, secret is the nsec
                        if let Err(e) = self.state.identity.login_nsec(&secret) {
                            tracing::warn!("Failed to restore nsec identity: {}", e);
                        } else {
                            self.status_message = Some("Session restored".to_string());
                            // Load user profile data
                            self.load_user_data_for_current_identity();
                        }
                    }
                }
                _ => {}
            }
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

    /// Update sync_status for locally-created publications
    ///
    /// Iterates through all publication nodes and marks any that are tracked
    /// as locally-created (not yet published to relays) with SyncStatus::LocalCreated.
    fn update_local_publication_status(&mut self) {
        let tracker = match &self.local_tracker {
            Some(t) => t,
            None => return,
        };

        for node in self.state.nodes.values_mut() {
            if let TreeNode::Publication(ref mut pub_node) = node {
                let a_tag = pub_node.addr.to_a_tag();
                if tracker.is_local(&a_tag) {
                    pub_node.sync_status = SyncStatus::LocalCreated;
                }
            }
        }
    }

    /// Load drafts synchronously and add to state
    fn load_drafts_sync(&mut self) {
        use crate::tree::node::PublicationNode;

        if let Some(ref store) = self.draft_store {
            if let Ok(drafts) = store.list_drafts() {
                for draft in drafts {
                    // Create a pseudo-NAddr for the draft publication
                    let pub_addr = NAddr::new(30040, &"0".repeat(64), &draft.draft_id);
                    let pub_id = NodeId::from_addr(&pub_addr);

                    // Skip if already exists
                    if self.state.nodes.contains_key(&pub_id) {
                        continue;
                    }

                    // Create section nodes from the draft's section events
                    let mut child_ids = Vec::new();
                    for (i, section_event) in draft.section_events.iter().enumerate() {
                        // Extract section info from the event JSON
                        let section_d_tag = section_event
                            .get("tags")
                            .and_then(|t| t.as_array())
                            .and_then(|tags| {
                                tags.iter().find_map(|tag| {
                                    let arr = tag.as_array()?;
                                    if arr.first()?.as_str()? == "d" {
                                        arr.get(1)?.as_str().map(String::from)
                                    } else {
                                        None
                                    }
                                })
                            })
                            .unwrap_or_else(|| format!("{}-section-{}", draft.draft_id, i));

                        let section_title = section_event
                            .get("tags")
                            .and_then(|t| t.as_array())
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

                        let section_content = section_event
                            .get("content")
                            .and_then(|c| c.as_str())
                            .map(String::from);

                        // Create section address and node
                        let section_addr = NAddr::new(30041, &"0".repeat(64), &section_d_tag);
                        let section_id = NodeId::from_addr(&section_addr);
                        child_ids.push(section_id);

                        let mut section_node = SectionNode::stub(section_addr, pub_id, i);
                        section_node.title = section_title;
                        section_node.content = section_content;
                        section_node.loaded = true;
                        section_node.sync_status = SyncStatus::Draft;
                        section_node.draft_id = Some(draft.draft_id.clone());

                        self.state.nodes.insert(section_id, TreeNode::Section(section_node));
                    }

                    // Create publication node with children linked
                    let mut pub_node = PublicationNode::stub(pub_addr, None);
                    pub_node.title = Some(draft.title.clone());
                    pub_node.summary = Some(format!("{} sections", draft.section_events.len()));
                    pub_node.author = "Draft".to_string();
                    pub_node.author_name = Some("Unsigned Draft".to_string());
                    pub_node.created_at = draft.modified_at;
                    pub_node.loaded = true;
                    pub_node.sync_status = SyncStatus::Draft;
                    pub_node.draft_id = Some(draft.draft_id.clone());
                    pub_node.children = child_ids;

                    self.state.nodes.insert(pub_id, TreeNode::Publication(pub_node));
                    // Insert drafts at the beginning of roots
                    self.state.roots.insert(0, pub_id);
                }
            }
        }
    }

    /// Load initial publications
    pub async fn load_initial(&mut self) -> anyhow::Result<()> {
        // Try to restore identity from previous session
        self.restore_identity();

        self.status_message = Some("Loading publications...".to_string());

        // Load drafts first (they appear at top of feed)
        self.load_drafts_sync();

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

                // Check if this publication was locally created (not yet published to relays)
                if let Some(ref tracker) = self.local_tracker {
                    if tracker.is_local(&pub_.addr.to_a_tag()) {
                        node.sync_status = SyncStatus::LocalCreated;
                    }
                }
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
        stdout().execute(EnableMouseCapture)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

        let result = self.event_loop(&mut terminal).await;

        // Restore terminal
        stdout().execute(DisableMouseCapture)?;
        disable_raw_mode()?;
        stdout().execute(LeaveAlternateScreen)?;

        result
    }

    async fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> anyhow::Result<()> {
        loop {
            // Tick spinner for animation
            self.spinner.tick();

            // Draw UI
            terminal.draw(|frame| self.draw(frame))?;

            // Check for async results
            while let Ok(msg) = self.async_rx.try_recv() {
                let AsyncMessage::Result(result) = msg;
                self.pending_count = self.pending_count.saturating_sub(1);

                // Handle special results - show status messages
                if let AsyncResult::PublicationCreated { .. } = result {
                    self.status_message = Some("Saved to local DB. Press 'b' to broadcast to relays.".to_string());
                }

                // Show broadcast progress
                if let AsyncResult::BroadcastProgress {
                    current_relay, total_relays, current_event, total_events, ref relay_name
                } = result {
                    let progress_bar = "█".repeat(current_relay) + &"░".repeat(total_relays - current_relay);
                    self.status_message = Some(format!(
                        "[{}] Relay {}/{}: {} (event {}/{})",
                        progress_bar, current_relay, total_relays, relay_name, current_event, total_events
                    ));
                    // Don't process progress through engine, just update UI
                    continue;
                }

                // Show broadcast completion status
                if let AsyncResult::BroadcastComplete { ref message, successful_relays, .. } = result {
                    if successful_relays > 0 {
                        self.status_message = Some(message.clone());
                    } else {
                        self.status_message = Some(format!("Warning: {}", message));
                    }
                }

                let cmd_result = self.engine.apply_async_result(&mut self.state, result);
                if let CommandResult::Error(e) = cmd_result {
                    self.status_message = Some(format!("Error: {}", e));
                } else if self.pending_count == 0 && self.status_message.is_none() {
                    self.status_message = None;
                }

                // Update sync_status for any locally-created publications
                self.update_local_publication_status();
            }

            // Poll for events with timeout
            if event::poll(Duration::from_millis(100))? {
                let event = event::read()?;
                let term_size = terminal.size()?;

                // Handle mouse scroll for windows
                if let Event::Mouse(mouse) = &event {
                    use crossterm::event::MouseEventKind;
                    let viewport_height = term_size.height.saturating_sub(4) as usize;
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            if self.state.windows.is_focused() {
                                self.state.windows.scroll_up(3);
                            }
                            continue;
                        }
                        MouseEventKind::ScrollDown => {
                            if self.state.windows.is_focused() {
                                self.state.windows.scroll_down(3, viewport_height);
                            }
                            continue;
                        }
                        _ => {}
                    }
                }

                if let Event::Key(key) = event {
                    // Only handle key press events (not release)
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Handle login dialog input first (highest priority modal)
                    if self.state.login_dialog.is_some() {
                        self.handle_login_dialog_input(key);
                        continue;
                    }

                    // Handle command palette input separately
                    if self.state.command_palette.visible {
                        if let Some(result) = self.handle_palette_input(key) {
                            match result {
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
                        continue;
                    }

                    let ctx = KeyContext {
                        app_mode: self.state.mode,
                        view_mode: self.state.view.mode,
                        compose_focus: if self.state.is_compose_mode() {
                            Some(self.state.compose.focus)
                        } else {
                            None
                        },
                        window_focused: self.state.windows.is_focused(),
                        user_data_menu_open: self.state.user_data_menu.is_some(),
                        editor_compose: self.state.use_editor_compose,
                        editor_insert_mode: self.state.editor_compose.insert_mode,
                        preview_focused: self.state.view.is_preview_focused(),
                    };
                    if let Some(command) = self.key_mapper.map_with_context(key, Some(&ctx)) {
                        // Handle ShowCommandPalette specially
                        if matches!(command, TreeCommand::ShowCommandPalette) {
                            self.state.command_palette.open();
                            continue;
                        }

                        // Handle login commands specially
                        if self.handle_login_command(&command) {
                            continue;
                        }

                        // Handle window commands directly
                        if self.handle_window_command(&command, term_size.height as usize) {
                            continue;
                        }

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

    /// Handle window commands directly (returns true if handled)
    fn handle_window_command(&mut self, command: &TreeCommand, viewport_height: usize) -> bool {
        // Calculate approximate viewport height for the window content
        // (account for borders and help line)
        let window_viewport = viewport_height.saturating_sub(4);

        match command {
            TreeCommand::ShowJson { title, content } => {
                use crate::tree::state::WindowState;
                let window = WindowState::json(
                    format!("json-{}", self.state.windows.windows.len()),
                    title,
                    content,
                );
                self.state.windows.open(window);
                true
            }
            TreeCommand::CloseWindow => {
                self.state.windows.close_focused();
                true
            }
            TreeCommand::CloseAllWindows => {
                self.state.windows.close_all();
                true
            }
            TreeCommand::FocusNextWindow => {
                self.state.windows.focus_next();
                true
            }
            TreeCommand::FocusPrevWindow => {
                self.state.windows.focus_prev();
                true
            }
            TreeCommand::WindowScrollUp => {
                self.state.windows.scroll_up(1);
                true
            }
            TreeCommand::WindowScrollDown => {
                self.state.windows.scroll_down(1, window_viewport);
                true
            }
            TreeCommand::WindowScrollToTop => {
                self.state.windows.scroll_to_top();
                true
            }
            TreeCommand::WindowScrollToBottom => {
                self.state.windows.scroll_to_bottom(window_viewport);
                true
            }
            TreeCommand::ShowEventJson => {
                // Get the current node and show its JSON
                let json = self.get_current_node_json();
                if let Some((title, content)) = json {
                    use crate::tree::state::WindowState;
                    let window = WindowState::json("event-json", title, &content);
                    self.state.windows.open(window);
                }
                true
            }
            TreeCommand::ShowUserData => {
                // Open user data menu (NIP-51 list selection)
                if self.state.identity.status.is_logged_in() {
                    use crate::tree::state::UserDataMenuState;
                    self.state.user_data_menu = Some(UserDataMenuState::new());
                } else {
                    self.status_message = Some("Not logged in. Press 'i' to login.".to_string());
                }
                true
            }
            TreeCommand::CloseUserDataMenu => {
                self.state.user_data_menu = None;
                true
            }
            TreeCommand::UserDataMenuUp => {
                if let Some(ref mut menu) = self.state.user_data_menu {
                    menu.select_prev();
                }
                true
            }
            TreeCommand::UserDataMenuDown => {
                if let Some(ref mut menu) = self.state.user_data_menu {
                    menu.select_next();
                }
                true
            }
            TreeCommand::UserDataMenuSelect => {
                if let Some(ref menu) = self.state.user_data_menu {
                    if let Some(list_type) = menu.selected_item() {
                        use crate::tree::state::{UserDataListType, WindowState};
                        let content = match list_type {
                            UserDataListType::Profile => self.state.user_data.format_profile(),
                            UserDataListType::FollowList => self.state.user_data.format_follow_list(),
                            UserDataListType::FollowListJson => self.state.user_data.format_follow_list_json(),
                            UserDataListType::MuteList => self.state.user_data.format_mute_list(),
                            UserDataListType::RelayList => self.state.user_data.format_relay_list(),
                            UserDataListType::Bookmarks => self.state.user_data.format_bookmarks(),
                            UserDataListType::BlockedRelays => self.state.user_data.format_blocked_relays(),
                            UserDataListType::SearchRelays => self.state.user_data.format_search_relays(),
                            UserDataListType::RelaySets => self.state.user_data.format_relay_sets(),
                        };
                        let title = list_type.window_title();
                        // Use JSON window for JSON views
                        let window = if matches!(list_type, UserDataListType::FollowListJson) {
                            WindowState::json("user-data-json", title, &content)
                        } else {
                            WindowState::new("user-data-detail", title, content)
                        };
                        self.state.windows.open(window);
                        // Close the menu after selection
                        self.state.user_data_menu = None;
                    }
                }
                true
            }
            _ => false,
        }
    }

    /// Get JSON representation of the current node
    fn get_current_node_json(&self) -> Option<(String, String)> {
        // In compose mode, show the preview JSON
        if self.state.is_compose_mode() {
            let json = self.state.compose.preview_event_json();
            return Some(("Event Preview".to_string(), json));
        }

        // In feed mode, show the current feed item
        if self.state.is_feed_mode() {
            if let Some(node) = self.state.roots.get(self.state.feed_cursor) {
                if let Some(tree_node) = self.state.nodes.get(node) {
                    let title = tree_node.title();
                    let json = self.node_to_json(tree_node);
                    return Some((title.to_string(), json));
                }
            }
        }

        // In reader mode, show the current cursor node
        if let Some(node) = self.state.nodes.get(&self.state.cursor) {
            let title = node.title();
            let json = self.node_to_json(node);
            return Some((title.to_string(), json));
        }

        None
    }

    /// Convert a tree node to JSON representation
    fn node_to_json(&self, node: &TreeNode) -> String {
        use serde_json::json;

        let value = match node {
            TreeNode::Publication(pub_node) => {
                json!({
                    "type": "publication",
                    "addr": {
                        "kind": pub_node.addr.kind,
                        "pubkey": pub_node.addr.pubkey,
                        "d_tag": pub_node.addr.d_tag,
                    },
                    "title": pub_node.title,
                    "summary": pub_node.summary,
                    "author_name": pub_node.author_name,
                    "loaded": pub_node.loaded,
                    "children_count": pub_node.children.len(),
                })
            }
            TreeNode::Section(sec_node) => {
                json!({
                    "type": "section",
                    "addr": {
                        "kind": sec_node.addr.kind,
                        "pubkey": sec_node.addr.pubkey,
                        "d_tag": sec_node.addr.d_tag,
                    },
                    "title": sec_node.title,
                    "content_preview": sec_node.content.as_ref().map(|c| {
                        if c.len() > 200 { format!("{}...", &c[..200]) } else { c.clone() }
                    }),
                    "loaded": sec_node.loaded,
                    "position": sec_node.position,
                })
            }
        };

        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    }

    /// Handle input when the command palette is open
    fn handle_palette_input(&mut self, key: crossterm::event::KeyEvent) -> Option<CommandResult> {
        match key.code {
            // Close palette
            KeyCode::Esc => {
                self.state.command_palette.close();
                None
            }
            // Execute selected command
            KeyCode::Enter => {
                if let Some(cmd_info) = self.state.command_palette.selected_command() {
                    let command = cmd_info.command.clone();
                    self.state.command_palette.close();

                    // Handle ShowCommandPalette recursively (shouldn't happen but just in case)
                    if matches!(command, TreeCommand::ShowCommandPalette) {
                        self.state.command_palette.open();
                        return None;
                    }

                    // Execute the command
                    Some(self.engine.execute(&mut self.state, command))
                } else {
                    None
                }
            }
            // Navigate up
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.command_palette.select_prev();
                None
            }
            // Navigate down
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.state.command_palette.select_next();
                None
            }
            // Also allow plain up/down
            KeyCode::Up => {
                self.state.command_palette.select_prev();
                None
            }
            KeyCode::Down => {
                self.state.command_palette.select_next();
                None
            }
            // Tab to select next
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.state.command_palette.select_prev();
                } else {
                    self.state.command_palette.select_next();
                }
                None
            }
            // Backspace to delete character
            KeyCode::Backspace => {
                self.state.command_palette.pop_char();
                None
            }
            // Type to filter
            KeyCode::Char(c) => {
                // Don't handle Ctrl+C as typing
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    if c == 'c' {
                        self.state.command_palette.close();
                        return Some(CommandResult::Exit);
                    }
                    return None;
                }
                self.state.command_palette.push_char(c);
                None
            }
            _ => None,
        }
    }

    /// Handle login-related commands (returns true if handled)
    fn handle_login_command(&mut self, command: &TreeCommand) -> bool {
        match command {
            TreeCommand::OpenLoginDialog => {
                // If we already have a locked ncryptsec, go straight to password prompt
                if let LoginStatus::EncryptedLocked { ncryptsec, .. } = &self.state.identity.status {
                    self.state.login_dialog = Some(LoginDialogState::password_prompt(ncryptsec.clone()));
                } else {
                    self.state.login_dialog = Some(LoginDialogState::new());
                }
                true
            }
            TreeCommand::CloseLoginDialog => {
                self.state.login_dialog = None;
                true
            }
            TreeCommand::Logout => {
                self.state.identity.logout();
                let _ = self.keyring.clear_last_identity();
                self.status_message = Some("Logged out".to_string());
                true
            }
            _ => false,
        }
    }

    /// Handle input when the login dialog is open
    fn handle_login_dialog_input(&mut self, key: crossterm::event::KeyEvent) {
        let dialog = match self.state.login_dialog.as_mut() {
            Some(d) => d,
            None => return,
        };

        match key.code {
            // Close dialog
            KeyCode::Esc => {
                self.state.login_dialog = None;
            }
            // Submit
            KeyCode::Enter => {
                if dialog.awaiting_password {
                    // Decrypt ncryptsec with password
                    if let Some(ref ncryptsec) = dialog.pending_ncryptsec.clone() {
                        match decrypt_ncryptsec(ncryptsec, &dialog.input) {
                            Ok((secret_hex, pubkey_hex)) => {
                                // Store secret in session cache (always works)
                                self.session_secrets.insert(pubkey_hex.clone(), secret_hex.clone());

                                // Try to store in keyring (may fail on some systems)
                                if let Err(e) = self.keyring.store_secret(&pubkey_hex, &secret_hex) {
                                    tracing::warn!("Failed to store secret in keyring: {:?} (using session cache)", e);
                                }
                                let _ = self.keyring.store_last_identity("ncryptsec", ncryptsec);

                                // Update identity status
                                self.state.identity.status = LoginStatus::SignedIn {
                                    pubkey: pubkey_hex,
                                    from_ncryptsec: true,
                                };
                                self.state.login_dialog = None;
                                self.status_message = Some("Signed in successfully".to_string());

                                // Load user profile data
                                self.load_user_data_for_current_identity();
                            }
                            Err(e) => {
                                dialog.set_error(format!("Decryption failed: {}", e));
                            }
                        }
                    }
                } else {
                    // Parse and process the key
                    let input = dialog.input.clone();
                    match parse_key(&input) {
                        Ok(KeyType::Npub(npub)) => {
                            if let Err(e) = self.state.identity.login_npub(&npub) {
                                dialog.set_error(format!("Invalid npub: {}", e));
                            } else {
                                // Store for session restoration
                                let _ = self.keyring.store_last_identity("npub", &npub);
                                self.state.login_dialog = None;
                                self.status_message = Some("Logged in (read-only)".to_string());

                                // Load user profile data
                                self.load_user_data_for_current_identity();
                            }
                        }
                        Ok(KeyType::Nsec(nsec)) => {
                            if let Err(e) = self.state.identity.login_nsec(&nsec) {
                                dialog.set_error(format!("Invalid nsec: {}", e));
                            } else {
                                // Store secret in session cache and keyring
                                if let Some(pubkey) = self.state.identity.status.pubkey() {
                                    // Store in session cache (always works)
                                    self.session_secrets.insert(pubkey.to_string(), nsec.clone());

                                    // Try to store in keyring
                                    if let Err(e) = self.keyring.store_secret(pubkey, &nsec) {
                                        tracing::warn!("Failed to store secret in keyring: {:?}", e);
                                    }
                                    let _ = self.keyring.store_last_identity("nsec", pubkey);
                                }
                                self.state.login_dialog = None;
                                self.status_message = Some("Signed in successfully".to_string());

                                // Load user profile data
                                self.load_user_data_for_current_identity();
                            }
                        }
                        Ok(KeyType::Ncryptsec(ncryptsec)) => {
                            // Switch to password entry mode
                            self.state.login_dialog = Some(LoginDialogState::password_prompt(ncryptsec));
                        }
                        Err(e) => {
                            dialog.set_error(format!("Invalid key: {}", e));
                        }
                    }
                }
            }
            // Text editing
            KeyCode::Backspace => {
                dialog.delete_char();
            }
            KeyCode::Delete => {
                dialog.delete_char_forward();
            }
            KeyCode::Left => {
                dialog.cursor_left();
            }
            KeyCode::Right => {
                dialog.cursor_right();
            }
            KeyCode::Home => {
                dialog.cursor_home();
            }
            KeyCode::End => {
                dialog.cursor_end();
            }
            // Character input
            KeyCode::Char(c) => {
                // Don't handle Ctrl+C as typing
                if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' {
                    self.state.login_dialog = None;
                    return;
                }
                dialog.insert_char(c);
            }
            _ => {}
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
        if self.state.is_compose_mode() {
            self.draw_compose_mode(frame, content_area);
        } else if self.state.is_feed_mode() {
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

        let mut status_bar = StatusBar::new(&self.state);
        if self.pending_count > 0 {
            status_bar = status_bar.with_spinner(self.spinner.frame());
        }
        if let Some(ref msg) = self.status_message {
            status_bar = status_bar.with_message(msg);
        }
        frame.render_widget(status_bar, status_area);

        // Render help bar
        let help_area = Rect {
            x: area.x,
            y: area.height.saturating_sub(1),
            width: area.width,
            height: 1,
        };
        frame.render_widget(HelpBar::new(&self.state), help_area);

        // Render windows (rendered before command palette so palette is on top)
        for (i, window) in self.state.windows.windows.iter().enumerate() {
            let is_focused = self.state.windows.focused == Some(i);
            let window_area = WindowWidget::calculate_area(
                area,
                window.width_percent,
                window.height_percent,
            );
            frame.render_widget(WindowWidget::new(window, is_focused), window_area);
        }

        // Render login dialog if open (on top of windows)
        if let Some(ref login_dialog) = self.state.login_dialog {
            let dialog_area = LoginDialogWidget::calculate_area(area);
            frame.render_widget(
                LoginDialogWidget::new(login_dialog),
                dialog_area,
            );
        }

        // Render user data menu if open
        if let Some(ref user_data_menu) = self.state.user_data_menu {
            let menu_area = UserDataMenuWidget::calculate_area(area);
            frame.render_widget(
                UserDataMenuWidget::new(user_data_menu),
                menu_area,
            );
        }

        // Render command palette overlay if visible (on top of everything)
        if self.state.command_palette.visible {
            let palette_area = CommandPaletteWidget::calculate_area(area);
            frame.render_widget(
                CommandPaletteWidget::new(&self.state.command_palette),
                palette_area,
            );
        }
    }

    fn draw_feed_mode(&self, frame: &mut Frame, area: Rect) {
        // Feed mode: list on left, preview on right (if enabled)
        let mut feed_widget = FeedWidget::new(&self.state);
        if self.pending_count > 0 {
            feed_widget = feed_widget.with_spinner(self.spinner.frame());
        }

        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            // Render feed list
            frame.render_widget(feed_widget, chunks[0]);

            // Render preview of selected publication
            frame.render_widget(ContentPreview::new(&self.state), chunks[1]);
        } else {
            // Full width feed list
            frame.render_widget(feed_widget, area);
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
        let mut tree_widget = TreeWidget::new(&self.state);
        if self.pending_count > 0 {
            tree_widget = tree_widget.with_spinner(self.spinner.frame());
        }

        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            frame.render_widget(tree_widget, chunks[0]);
            frame.render_widget(JsonPreview::new(&self.state), chunks[1]);
        } else {
            frame.render_widget(tree_widget, area);
        }
    }

    fn draw_outline_view(&self, frame: &mut Frame, area: Rect) {
        // Outline mode: sections as cards
        // Preview shows JSON of selected section + full publication JSON
        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);

            frame.render_widget(OutlineWidget::new(&self.state), chunks[0]);
            frame.render_widget(JsonPreview::new(&self.state).with_full_json(), chunks[1]);
        } else {
            frame.render_widget(OutlineWidget::new(&self.state), area);
        }
    }

    fn draw_continuous_view(&self, frame: &mut Frame, area: Rect) {
        // Continuous mode: full scrollable content, or full JSON with preview
        if self.state.view.show_preview {
            frame.render_widget(JsonPreview::new(&self.state).with_full_json(), area);
        } else {
            frame.render_widget(ContinuousWidget::new(&self.state), area);
        }
    }

    fn draw_paginated_view(&self, frame: &mut Frame, area: Rect) {
        // Paginated mode: one section at a time, or with JSON preview panel
        if self.state.view.show_preview {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                .split(area);

            frame.render_widget(PaginatedWidget::new(&self.state), chunks[0]);
            frame.render_widget(JsonPreview::new(&self.state), chunks[1]);
        } else {
            frame.render_widget(PaginatedWidget::new(&self.state), area);
        }
    }

    fn draw_compose_mode(&self, frame: &mut Frame, area: Rect) {
        if self.state.use_editor_compose {
            // Editor compose mode - single buffer with structure detection
            frame.render_widget(
                EditorComposeWidget::new(&self.state.editor_compose),
                area
            );
        } else {
            // Structured compose mode - explicit sections
            let pubkey = self.state.identity.status.pubkey();
            frame.render_widget(
                ComposeWidget::new(&self.state.compose).with_pubkey(pubkey),
                area
            );
        }
    }

    fn spawn_async_request(&mut self, request: AsyncRequest) {
        // Handle batch requests by spawning individual requests
        if let AsyncRequest::LoadBatch { requests } = request {
            for req in requests {
                self.spawn_single_async_request(req);
            }
            return;
        }

        self.spawn_single_async_request(request);
    }

    /// Spawn loading of user profile data for the currently logged-in user
    fn load_user_data_for_current_identity(&mut self) {
        if let Some(pubkey) = self.state.identity.status.pubkey() {
            let request = AsyncRequest::LoadUserData {
                pubkey: pubkey.to_string(),
            };
            self.spawn_async_request(request);
        }
    }

    fn spawn_single_async_request(&mut self, request: AsyncRequest) {
        // Handle draft requests synchronously since they need the draft store
        match &request {
            AsyncRequest::SaveDraft { compose } => {
                self.pending_count += 1;
                let result = if let Some(ref store) = self.draft_store {
                    match store.save_draft(compose) {
                        Ok(draft_id) => AsyncResult::DraftSaved { draft_id },
                        Err(e) => AsyncResult::Error {
                            request: request.clone(),
                            error: e.to_string(),
                        },
                    }
                } else {
                    AsyncResult::Error {
                        request: request.clone(),
                        error: "Draft store not initialized".to_string(),
                    }
                };
                // Send result through channel to be processed in event loop
                let tx = self.async_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AsyncMessage::Result(result)).await;
                });
                return;
            }
            AsyncRequest::LoadDrafts => {
                self.pending_count += 1;
                let result = if let Some(ref store) = self.draft_store {
                    match store.list_drafts() {
                        Ok(drafts) => {
                            let loaded: Vec<LoadedDraft> = drafts
                                .into_iter()
                                .map(|d| LoadedDraft {
                                    draft_id: d.draft_id,
                                    title: d.title,
                                    created_at: d.created_at,
                                    modified_at: d.modified_at,
                                    section_count: d.section_events.len(),
                                })
                                .collect();
                            AsyncResult::DraftsLoaded { drafts: loaded }
                        }
                        Err(e) => AsyncResult::Error {
                            request: request.clone(),
                            error: e.to_string(),
                        },
                    }
                } else {
                    AsyncResult::Error {
                        request: request.clone(),
                        error: "Draft store not initialized".to_string(),
                    }
                };
                let tx = self.async_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AsyncMessage::Result(result)).await;
                });
                return;
            }
            AsyncRequest::PublishPublication { title, tags, sections } => {
                self.pending_count += 1;

                // Check if logged in with signing capability
                let pubkey = match &self.state.identity.status {
                    crate::identity::LoginStatus::SignedIn { pubkey, .. } => pubkey.clone(),
                    crate::identity::LoginStatus::EncryptedLocked { .. } => {
                        let tx = self.async_tx.clone();
                        let result = AsyncResult::Error {
                            request: request.clone(),
                            error: "Session is locked - enter password first (press 'i' to unlock)".to_string(),
                        };
                        tokio::spawn(async move {
                            let _ = tx.send(AsyncMessage::Result(result)).await;
                        });
                        return;
                    }
                    crate::identity::LoginStatus::ReadOnly { .. } => {
                        let tx = self.async_tx.clone();
                        let result = AsyncResult::Error {
                            request: request.clone(),
                            error: "Read-only login (npub) cannot create publications - need nsec or ncryptsec".to_string(),
                        };
                        tokio::spawn(async move {
                            let _ = tx.send(AsyncMessage::Result(result)).await;
                        });
                        return;
                    }
                    crate::identity::LoginStatus::None => {
                        let tx = self.async_tx.clone();
                        let result = AsyncResult::Error {
                            request: request.clone(),
                            error: "Must be logged in to create publications (press 'i' to login)".to_string(),
                        };
                        tokio::spawn(async move {
                            let _ = tx.send(AsyncMessage::Result(result)).await;
                        });
                        return;
                    }
                };

                // Get secret key for signing (check session cache first, then keyring)
                let secret_hex = if let Some(secret) = self.session_secrets.get(&pubkey) {
                    // Found in session cache - decode if it's an nsec
                    if secret.starts_with("nsec1") {
                        match crate::identity::decode_nsec(secret) {
                            Ok(hex) => hex,
                            Err(e) => {
                                let tx = self.async_tx.clone();
                                let result = AsyncResult::Error {
                                    request: request.clone(),
                                    error: format!("Invalid nsec in session cache: {}", e),
                                };
                                tokio::spawn(async move {
                                    let _ = tx.send(AsyncMessage::Result(result)).await;
                                });
                                return;
                            }
                        }
                    } else {
                        secret.clone()
                    }
                } else {
                    // Try keyring as fallback
                    match self.keyring.get_secret(&pubkey) {
                        Ok(secret) => {
                            if secret.starts_with("nsec1") {
                                match crate::identity::decode_nsec(&secret) {
                                    Ok(hex) => hex,
                                    Err(e) => {
                                        let tx = self.async_tx.clone();
                                        let result = AsyncResult::Error {
                                            request: request.clone(),
                                            error: format!("Invalid nsec in keyring: {}", e),
                                        };
                                        tokio::spawn(async move {
                                            let _ = tx.send(AsyncMessage::Result(result)).await;
                                        });
                                        return;
                                    }
                                }
                            } else {
                                secret
                            }
                        }
                        Err(_) => {
                            let tx = self.async_tx.clone();
                            let result = AsyncResult::Error {
                                request: request.clone(),
                                error: "Secret not found. Please re-enter your password (press 'i')".to_string(),
                            };
                            tokio::spawn(async move {
                                let _ = tx.send(AsyncMessage::Result(result)).await;
                            });
                            return;
                        }
                    }
                };

                // Build a temporary ComposeState from the request data
                use crate::tree::state::{ComposeState, TagEntry};
                let mut compose = ComposeState::new();
                compose.title = title.clone();
                // Convert tags back to TagEntry format
                for tag_vec in tags.iter() {
                    if tag_vec.len() >= 2 {
                        compose.tags.push(TagEntry {
                            name: tag_vec[0].clone(),
                            value: tag_vec[1..].join(", "),
                        });
                    }
                }
                compose.sections = sections.clone();

                // Build signed events from compose state
                use crate::publication::build_signed_publication_events;
                let (pub_event, section_events) = build_signed_publication_events(&compose, &pubkey, &secret_hex);

                // Log the events being created
                let pub_id = pub_event.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                tracing::info!("Publishing event ID: {}", pub_id);
                tracing::debug!("Publication event: {}", serde_json::to_string(&pub_event).unwrap_or_default());

                // Ingest all events into nostrdb
                let engine = self.nostr_engine.clone();
                let mut ingest_error: Option<String> = None;

                // Ingest section events first
                for (i, section_event) in section_events.iter().enumerate() {
                    let json_str = serde_json::to_string(section_event).unwrap_or_default();
                    let section_id = section_event.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    tracing::info!("Ingesting section {} with ID: {}", i, section_id);
                    if let Err(e) = engine.ingest_event(&json_str) {
                        ingest_error = Some(format!("Failed to ingest section {}: {}", i, e));
                        tracing::error!("Section ingest failed: {}", e);
                        break;
                    }
                }

                // Ingest publication event
                if ingest_error.is_none() {
                    let json_str = serde_json::to_string(&pub_event).unwrap_or_default();
                    tracing::info!("Ingesting publication event");
                    if let Err(e) = engine.ingest_event(&json_str) {
                        ingest_error = Some(format!("Failed to ingest publication: {}", e));
                        tracing::error!("Publication ingest failed: {}", e);
                    } else {
                        tracing::info!("Publication ingested successfully, ID: {}", pub_id);
                    }
                }

                // Wait for async processing
                std::thread::sleep(std::time::Duration::from_millis(200));

                // Verify the event was stored
                let _verified = if ingest_error.is_none() {
                    let verify_filter = serde_json::json!({
                        "ids": [pub_id],
                        "limit": 1
                    });
                    match crate::query::query_local(&engine.ndb(), &[verify_filter]) {
                        Ok(events) if !events.is_empty() => {
                            tracing::info!("VERIFIED: Event {} found in database", pub_id);
                            true
                        }
                        Ok(_) => {
                            tracing::error!("FAILED: Event {} NOT found in database after ingest!", pub_id);
                            ingest_error = Some(format!("Event {} was not stored in database", pub_id));
                            false
                        }
                        Err(e) => {
                            tracing::warn!("Verification query failed: {}", e);
                            false
                        }
                    }
                } else {
                    false
                };

                let result = if let Some(error) = ingest_error {
                    AsyncResult::Error {
                        request: request.clone(),
                        error,
                    }
                } else {
                    // Count total events
                    let all_filter = serde_json::json!({
                        "kinds": [30040],
                        "limit": 100
                    });
                    let total_count = crate::query::query_local(&engine.ndb(), &[all_filter])
                        .map(|e| e.len())
                        .unwrap_or(0);
                    tracing::info!("Total 30040 events in database: {}", total_count);

                    // Build the NAddr for the created publication
                    let pub_d_tag = compose.publication_d_tag();
                    let addr = NAddr::new(30040, &pubkey, &pub_d_tag);

                    // Mark this publication as locally-created (not yet published to relays)
                    if let Some(ref tracker) = self.local_tracker {
                        if let Err(e) = tracker.mark_local(&addr.to_a_tag()) {
                            tracing::warn!("Failed to mark publication as local: {}", e);
                        } else {
                            tracing::info!("Marked publication as local: {}", addr.to_a_tag());
                        }
                    }

                    // Build section data for the result
                    use crate::tree::command::CreatedSection;
                    let created_sections: Vec<CreatedSection> = compose.sections.iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let section_d_tag = compose.section_d_tag(i);
                            CreatedSection {
                                addr: NAddr::new(30041, &pubkey, &section_d_tag),
                                title: if s.title.is_empty() { None } else { Some(s.title.clone()) },
                                content: if s.content.is_empty() { None } else { Some(s.content.clone()) },
                            }
                        })
                        .collect();

                    // Collect signed event JSONs for relay broadcast
                    let mut signed_events = Vec::new();
                    // Add section events first (they're referenced by the publication)
                    for section_event in &section_events {
                        if let Ok(json) = serde_json::to_string(section_event) {
                            signed_events.push(json);
                        }
                    }
                    // Add publication event
                    if let Ok(json) = serde_json::to_string(&pub_event) {
                        signed_events.push(json);
                    }

                    AsyncResult::PublicationCreated {
                        addr,
                        title: Some(title.clone()),
                        sections: created_sections,
                        signed_events,
                    }
                };

                let tx = self.async_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AsyncMessage::Result(result)).await;
                });
                return;
            }
            AsyncRequest::BroadcastToRelays { addr, events, relays } => {
                self.pending_count += 1;

                let addr = addr.clone();
                let events = events.clone();
                let relays = relays.clone();
                let tx = self.async_tx.clone();
                tokio::spawn(async move {
                    use crate::relay::publish_events_to_relays_with_progress;

                    let tx_progress = tx.clone();
                    let (successful, total, _details) = publish_events_to_relays_with_progress(
                        &relays,
                        &events,
                        |progress| {
                            let result = AsyncResult::BroadcastProgress {
                                current_relay: progress.current_relay,
                                total_relays: progress.total_relays,
                                current_event: progress.current_event,
                                total_events: progress.total_events,
                                relay_name: progress.relay_name,
                            };
                            // Use blocking send since we're in a sync closure
                            let _ = tx_progress.try_send(AsyncMessage::Result(result));
                        },
                    ).await;

                    let message = if successful == total {
                        format!("Published to all {} relays", total)
                    } else if successful > 0 {
                        format!("Published to {}/{} relays", successful, total)
                    } else {
                        format!("Failed to publish to any of {} relays", total)
                    };

                    let result = AsyncResult::BroadcastComplete {
                        addr,
                        successful_relays: successful,
                        total_relays: total,
                        message,
                    };
                    let _ = tx.send(AsyncMessage::Result(result)).await;
                });
                return;
            }
            AsyncRequest::BroadcastSelected { addr } => {
                self.pending_count += 1;

                // Query events from nostrdb
                let engine = self.nostr_engine.clone();
                let relays = self.effective_relays()
                    .unwrap_or_else(|| crate::relay::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect());

                if relays.is_empty() {
                    let tx = self.async_tx.clone();
                    let result = AsyncResult::Error {
                        request: request.clone(),
                        error: "No relays configured".to_string(),
                    };
                    tokio::spawn(async move {
                        let _ = tx.send(AsyncMessage::Result(result)).await;
                    });
                    return;
                }

                // Query the publication event
                let pub_filter = serde_json::json!({
                    "kinds": [30040],
                    "authors": [&addr.pubkey],
                    "#d": [&addr.d_tag],
                    "limit": 1
                });

                let pub_events = match crate::query::query_local(&engine.ndb(), &[pub_filter]) {
                    Ok(events) => events,
                    Err(e) => {
                        let tx = self.async_tx.clone();
                        let result = AsyncResult::Error {
                            request: request.clone(),
                            error: format!("Failed to query publication: {}", e),
                        };
                        tokio::spawn(async move {
                            let _ = tx.send(AsyncMessage::Result(result)).await;
                        });
                        return;
                    }
                };

                if pub_events.is_empty() {
                    let tx = self.async_tx.clone();
                    let result = AsyncResult::Error {
                        request: request.clone(),
                        error: "Publication not found in local database".to_string(),
                    };
                    tokio::spawn(async move {
                        let _ = tx.send(AsyncMessage::Result(result)).await;
                    });
                    return;
                }

                let pub_event = &pub_events[0];

                // Extract section d-tags from the publication's "a" tags
                let mut section_d_tags = Vec::new();
                if let Some(tags) = pub_event.get("tags").and_then(|t| t.as_array()) {
                    for tag in tags {
                        if let Some(arr) = tag.as_array() {
                            if arr.len() >= 2 {
                                if let (Some("a"), Some(a_value)) = (arr[0].as_str(), arr[1].as_str()) {
                                    // Parse "30041:pubkey:d-tag"
                                    let parts: Vec<&str> = a_value.split(':').collect();
                                    if parts.len() >= 3 && parts[0] == "30041" {
                                        section_d_tags.push(parts[2].to_string());
                                    }
                                }
                            }
                        }
                    }
                }

                // Query section events
                let mut events = Vec::new();
                for d_tag in &section_d_tags {
                    let sec_filter = serde_json::json!({
                        "kinds": [30041],
                        "authors": [&addr.pubkey],
                        "#d": [d_tag],
                        "limit": 1
                    });
                    if let Ok(sec_events) = crate::query::query_local(&engine.ndb(), &[sec_filter]) {
                        for sec_event in sec_events {
                            if let Ok(json) = serde_json::to_string(&sec_event) {
                                events.push(json);
                            }
                        }
                    }
                }

                // Add publication event last
                if let Ok(json) = serde_json::to_string(pub_event) {
                    events.push(json);
                }

                let event_count = events.len();
                let relay_count = relays.len();
                self.status_message = Some(format!("Broadcasting {} events to {} relays...", event_count, relay_count));

                // Create broadcast request
                let broadcast_req = AsyncRequest::BroadcastToRelays {
                    addr: addr.clone(),
                    events,
                    relays,
                };
                self.spawn_single_async_request(broadcast_req);
                return;
            }
            _ => {}
        }

        self.pending_count += 1;

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

        AsyncRequest::SearchEvents { query } => {
            let response = engine.search(&query, policy, None).await?;
            Ok(AsyncResult::SearchResults {
                results: response.results,
                query,
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

        AsyncRequest::LoadBatch { .. } => {
            // LoadBatch is handled by spawn_async_request, should never reach here
            unreachable!("LoadBatch should be handled by spawn_async_request")
        }

        AsyncRequest::PublishNote { .. } | AsyncRequest::PublishPublication { .. } => {
            // Publishing is deferred for now - return a placeholder result
            // TODO: Implement actual publishing via MCP or signing
            Ok(AsyncResult::Error {
                request,
                error: "Publishing not yet implemented".to_string(),
            })
        }

        AsyncRequest::SaveDraft { .. } | AsyncRequest::LoadDrafts => {
            // Draft operations are handled synchronously in spawn_single_async_request
            unreachable!("Draft operations should be handled by spawn_single_async_request")
        }

        AsyncRequest::LoadUserData { pubkey } => {
            load_user_data(engine, &pubkey, policy).await
        }

        AsyncRequest::BroadcastToRelays { .. } | AsyncRequest::BroadcastSelected { .. } => {
            // Broadcast operations are handled synchronously in spawn_single_async_request
            unreachable!("Broadcast operations should be handled by spawn_single_async_request")
        }
    }
}

/// Load all user profile data for a given pubkey
///
/// This function queries nostrdb directly using the Note API, following notedeck patterns.
/// See: notedeck/crates/notedeck/src/account/mute.rs, relay.rs
async fn load_user_data(
    engine: &Engine,
    pubkey: &str,
    policy: FetchPolicy,
) -> anyhow::Result<AsyncResult> {
    use crate::user_data::{
        BlockedRelays, Bookmarks, FollowList, Metadata, MuteList, RelayList, RelaySet,
        SearchRelays, UserData, USER_DATA_ADDRESSABLE_KINDS, USER_DATA_KINDS,
    };
    use nostrdb::{Filter, Transaction};

    let mut user_data = UserData::new();

    // Parse pubkey to bytes
    let pubkey_bytes: [u8; 32] = hex::decode(pubkey)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid pubkey length"))?;

    // If policy requires relay fetching, do that first to populate nostrdb
    if policy != FetchPolicy::LocalOnly {
        use serde_json::json;
        // Fetch from relays to ensure data is in nostrdb
        let filter = json!({
            "kinds": USER_DATA_KINDS,
            "authors": [pubkey],
            "limit": USER_DATA_KINDS.len()
        });
        let _ = engine.get_events(vec![filter], policy, None).await;

        // Also fetch addressable kinds
        for &kind in USER_DATA_ADDRESSABLE_KINDS {
            let filter = json!({
                "kinds": [kind],
                "authors": [pubkey],
                "limit": 100
            });
            let _ = engine.get_events(vec![filter], policy, None).await;
        }
    }

    // Collect contact pubkeys for later profile fetching (needs to be done before transaction)
    let mut contact_hex_pubkeys: Vec<String> = Vec::new();

    // Now query nostrdb directly using Note API (following notedeck patterns)
    // Scope the transaction to avoid holding it across await points
    {
        let ndb = engine.ndb();
        let txn = Transaction::new(ndb)
            .map_err(|e| anyhow::anyhow!("Failed to create transaction: {}", e))?;

        // Query standard list kinds (0, 3, 10000, 10002, 10003, 10006, 10007)
        let filter = Filter::new()
            .authors([&pubkey_bytes])
            .kinds(USER_DATA_KINDS.iter().copied())
            .limit(USER_DATA_KINDS.len() as u64)
            .build();

        let results = ndb
            .query(&txn, &[filter], USER_DATA_KINDS.len() as i32)
            .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;

        for query_result in results {
            if let Ok(note) = ndb.get_note_by_key(&txn, query_result.note_key) {
                let kind = note.kind() as u64;
                let created_at = note.created_at();

                match kind {
                    0 => {
                        if user_data.metadata.as_ref().map(|m| m.created_at).unwrap_or(0) < created_at {
                            user_data.metadata = Metadata::from_note(&note);
                        }
                    }
                    3 => {
                        if user_data.follows.as_ref().map(|f| f.created_at).unwrap_or(0) < created_at {
                            user_data.follows = Some(FollowList::from_note(&note));
                        }
                    }
                    10000 => {
                        if user_data.mutes.as_ref().map(|m| m.created_at).unwrap_or(0) < created_at {
                            user_data.mutes = Some(MuteList::from_note(&note));
                        }
                    }
                    10002 => {
                        if user_data.relays.as_ref().map(|r| r.created_at).unwrap_or(0) < created_at {
                            user_data.relays = Some(RelayList::from_note(&note));
                        }
                    }
                    10003 => {
                        if user_data.bookmarks.as_ref().map(|b| b.created_at).unwrap_or(0) < created_at {
                            user_data.bookmarks = Some(Bookmarks::from_note(&note));
                        }
                    }
                    10006 => {
                        if user_data.blocked_relays.as_ref().map(|b| b.created_at).unwrap_or(0) < created_at {
                            user_data.blocked_relays = Some(BlockedRelays::from_note(&note));
                        }
                    }
                    10007 => {
                        if user_data.search_relays.as_ref().map(|s| s.created_at).unwrap_or(0) < created_at {
                            user_data.search_relays = Some(SearchRelays::from_note(&note));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Query addressable kinds (30002 relay sets)
        for &kind in USER_DATA_ADDRESSABLE_KINDS {
            let filter = Filter::new()
                .authors([&pubkey_bytes])
                .kinds([kind])
                .limit(100)
                .build();

            let results = ndb
                .query(&txn, &[filter], 100)
                .map_err(|e| anyhow::anyhow!("Query failed: {}", e))?;

            for query_result in results {
                if let Ok(note) = ndb.get_note_by_key(&txn, query_result.note_key) {
                    if kind == 30002 {
                        if let Some(relay_set) = RelaySet::from_note(&note) {
                            let should_update = user_data
                                .relay_sets
                                .get(&relay_set.d_tag)
                                .map(|existing| existing.created_at < relay_set.created_at)
                                .unwrap_or(true);

                            if should_update {
                                user_data.relay_sets.insert(relay_set.d_tag.clone(), relay_set);
                            }
                        }
                    }
                }
            }
        }

        // Collect contact pubkeys for profile fetching (normalize to lowercase)
        if let Some(ref follows) = user_data.follows {
            contact_hex_pubkeys = follows
                .contacts
                .iter()
                .filter(|c| c.pubkey.len() == 64 && c.pubkey.chars().all(|ch| ch.is_ascii_hexdigit()))
                .map(|c| c.pubkey.to_lowercase())
                .collect();
        }
    } // Transaction dropped here

    // Fetch kind 0 profiles for followed contacts
    if !contact_hex_pubkeys.is_empty() {
        use serde_json::json;
        use tracing::{debug, warn};

        debug!("Fetching kind 0 profiles for {} contacts", contact_hex_pubkeys.len());
        let pubkey_refs: Vec<&str> = contact_hex_pubkeys.iter().map(|s| s.as_str()).collect();
        let mut profiles_fetched = 0usize;

        // Batch into chunks of 100 pubkeys
        for (chunk_idx, chunk) in pubkey_refs.chunks(100).enumerate() {
            let filter = json!({
                "kinds": [0],
                "authors": chunk,
                "limit": chunk.len()
            });

            debug!("Fetching profile chunk {} ({} pubkeys)", chunk_idx + 1, chunk.len());

            // get_events returns events from both local db and relays
            match engine.get_events(vec![filter], policy, None).await {
                Ok(response) => {
                    debug!("Got {} events for profile chunk {}", response.events.len(), chunk_idx + 1);
                    // Parse the returned events directly (they're JSON)
                    for event in response.events {
                        if let Some(kind) = event.get("kind").and_then(|v| v.as_u64()) {
                            if kind == 0 {
                                if let (Some(pubkey), Some(content), Some(created_at)) = (
                                    event.get("pubkey").and_then(|v| v.as_str()),
                                    event.get("content").and_then(|v| v.as_str()),
                                    event.get("created_at").and_then(|v| v.as_u64()),
                                ) {
                                    // Only update if newer than existing (use lowercase for comparison)
                                    let pubkey_lower = pubkey.to_lowercase();
                                    let should_update = user_data
                                        .contact_profiles
                                        .get(&pubkey_lower)
                                        .map(|existing| existing.created_at < created_at)
                                        .unwrap_or(true);

                                    if should_update {
                                        if let Some(metadata) = Metadata::from_event_content(content, created_at) {
                                            user_data.contact_profiles.insert(pubkey_lower, metadata);
                                            profiles_fetched += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch profile chunk {}: {}", chunk_idx + 1, e);
                }
            }
        }

        debug!("Loaded {} contact profiles", profiles_fetched);
    }

    Ok(AsyncResult::UserDataLoaded { user_data })
}
