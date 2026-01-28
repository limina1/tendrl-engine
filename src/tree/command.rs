//! Tree commands and async request types
//!
//! Defines the commands that can be executed on the tree and the async
//! requests that the engine returns when IO is needed.

use super::node::NodeId;
use super::state::SectionCompose;
use crate::publication::NAddr;
use crate::user_data::UserData;
use serde::{Deserialize, Serialize};

/// Information about a command for the command palette
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// The command variant
    pub command: TreeCommand,
    /// Human-readable display name
    pub name: &'static str,
    /// Description of what the command does
    pub description: &'static str,
    /// Category for grouping
    pub category: CommandCategory,
    /// Keybinding hint (if any)
    pub keybinding: Option<&'static str>,
}

/// Command categories for grouping in the palette
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCategory {
    Navigation,
    Selection,
    Manipulation,
    Versioning,
    View,
    UndoRedo,
    Mode,
    Application,
    Configuration,
    Compose,
    Window,
}

impl CommandCategory {
    pub fn name(&self) -> &'static str {
        match self {
            CommandCategory::Navigation => "Navigation",
            CommandCategory::Selection => "Selection",
            CommandCategory::Manipulation => "Manipulation",
            CommandCategory::Versioning => "Versioning",
            CommandCategory::View => "View",
            CommandCategory::UndoRedo => "Undo/Redo",
            CommandCategory::Mode => "Mode",
            CommandCategory::Application => "Application",
            CommandCategory::Configuration => "Configuration",
            CommandCategory::Compose => "Compose",
            CommandCategory::Window => "Window",
        }
    }
}

/// Get all available commands with their metadata
pub fn all_commands() -> Vec<CommandInfo> {
    vec![
        // Navigation
        CommandInfo {
            command: TreeCommand::MoveUp,
            name: "Move Up",
            description: "Move cursor up one visible node",
            category: CommandCategory::Navigation,
            keybinding: Some("k / ↑"),
        },
        CommandInfo {
            command: TreeCommand::MoveDown,
            name: "Move Down",
            description: "Move cursor down one visible node",
            category: CommandCategory::Navigation,
            keybinding: Some("j / ↓"),
        },
        CommandInfo {
            command: TreeCommand::MoveToFirst,
            name: "Move to First",
            description: "Move cursor to first visible node",
            category: CommandCategory::Navigation,
            keybinding: Some("gg"),
        },
        CommandInfo {
            command: TreeCommand::MoveToLast,
            name: "Move to Last",
            description: "Move cursor to last visible node",
            category: CommandCategory::Navigation,
            keybinding: Some("G"),
        },
        CommandInfo {
            command: TreeCommand::MoveToParent,
            name: "Move to Parent",
            description: "Move cursor to parent node",
            category: CommandCategory::Navigation,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::Enter,
            name: "Enter / Expand",
            description: "Enter/expand current node or load content",
            category: CommandCategory::Navigation,
            keybinding: Some("Enter / l"),
        },
        CommandInfo {
            command: TreeCommand::Collapse,
            name: "Collapse",
            description: "Collapse current node",
            category: CommandCategory::Navigation,
            keybinding: Some("h"),
        },
        CommandInfo {
            command: TreeCommand::ToggleExpand,
            name: "Toggle Expand",
            description: "Toggle expand/collapse of current node",
            category: CommandCategory::Navigation,
            keybinding: Some("Space"),
        },
        // Selection
        CommandInfo {
            command: TreeCommand::ToggleSelect,
            name: "Toggle Select",
            description: "Toggle selection of current node",
            category: CommandCategory::Selection,
            keybinding: Some("V"),
        },
        CommandInfo {
            command: TreeCommand::SelectAll,
            name: "Select All",
            description: "Select all visible nodes",
            category: CommandCategory::Selection,
            keybinding: Some("s"),
        },
        CommandInfo {
            command: TreeCommand::ClearSelection,
            name: "Clear Selection",
            description: "Clear all selections",
            category: CommandCategory::Selection,
            keybinding: None,
        },
        // Manipulation
        CommandInfo {
            command: TreeCommand::MoveSectionUp,
            name: "Move Section Up",
            description: "Move section up within its parent",
            category: CommandCategory::Manipulation,
            keybinding: Some("K"),
        },
        CommandInfo {
            command: TreeCommand::MoveSectionDown,
            name: "Move Section Down",
            description: "Move section down within its parent",
            category: CommandCategory::Manipulation,
            keybinding: Some("J"),
        },
        CommandInfo {
            command: TreeCommand::Delete,
            name: "Delete",
            description: "Delete current node or selection",
            category: CommandCategory::Manipulation,
            keybinding: Some("d"),
        },
        CommandInfo {
            command: TreeCommand::Yank,
            name: "Yank (Copy)",
            description: "Copy current node to clipboard",
            category: CommandCategory::Manipulation,
            keybinding: Some("y"),
        },
        CommandInfo {
            command: TreeCommand::PasteAfter,
            name: "Paste After",
            description: "Paste after current node",
            category: CommandCategory::Manipulation,
            keybinding: Some("p"),
        },
        CommandInfo {
            command: TreeCommand::PasteBefore,
            name: "Paste Before",
            description: "Paste before current node",
            category: CommandCategory::Manipulation,
            keybinding: Some("P"),
        },
        // Versioning
        CommandInfo {
            command: TreeCommand::Fork,
            name: "Fork Section",
            description: "Create new version of current section",
            category: CommandCategory::Versioning,
            keybinding: Some("f"),
        },
        CommandInfo {
            command: TreeCommand::ShowAlternates,
            name: "Show Alternates",
            description: "Show alternate versions of current section",
            category: CommandCategory::Versioning,
            keybinding: Some("a"),
        },
        // View
        CommandInfo {
            command: TreeCommand::TogglePreview,
            name: "Toggle Preview",
            description: "Toggle content preview panel",
            category: CommandCategory::View,
            keybinding: Some("Tab"),
        },
        CommandInfo {
            command: TreeCommand::ScrollPreviewUp,
            name: "Scroll Preview Up",
            description: "Scroll preview panel up",
            category: CommandCategory::View,
            keybinding: Some("PageUp"),
        },
        CommandInfo {
            command: TreeCommand::ScrollPreviewDown,
            name: "Scroll Preview Down",
            description: "Scroll preview panel down",
            category: CommandCategory::View,
            keybinding: Some("PageDown"),
        },
        CommandInfo {
            command: TreeCommand::ScrollContentUp,
            name: "Scroll Content Up",
            description: "Scroll content up (Continuous mode)",
            category: CommandCategory::View,
            keybinding: Some("k"),
        },
        CommandInfo {
            command: TreeCommand::ScrollContentDown,
            name: "Scroll Content Down",
            description: "Scroll content down (Continuous mode)",
            category: CommandCategory::View,
            keybinding: Some("j"),
        },
        CommandInfo {
            command: TreeCommand::NextPage,
            name: "Next Page/Section",
            description: "Go to next page or section (Paginated mode)",
            category: CommandCategory::View,
            keybinding: Some("J"),
        },
        CommandInfo {
            command: TreeCommand::PrevPage,
            name: "Previous Page/Section",
            description: "Go to previous page or section (Paginated mode)",
            category: CommandCategory::View,
            keybinding: Some("K"),
        },
        CommandInfo {
            command: TreeCommand::Refresh,
            name: "Refresh",
            description: "Refresh current view / reload data",
            category: CommandCategory::View,
            keybinding: Some("R"),
        },
        // Undo/Redo
        CommandInfo {
            command: TreeCommand::Undo,
            name: "Undo",
            description: "Undo last operation",
            category: CommandCategory::UndoRedo,
            keybinding: Some("u"),
        },
        CommandInfo {
            command: TreeCommand::Redo,
            name: "Redo",
            description: "Redo last undone operation",
            category: CommandCategory::UndoRedo,
            keybinding: Some("Ctrl+r"),
        },
        // Mode
        CommandInfo {
            command: TreeCommand::Back,
            name: "Back",
            description: "Exit reader mode, return to feed",
            category: CommandCategory::Mode,
            keybinding: Some("Esc"),
        },
        CommandInfo {
            command: TreeCommand::CycleViewMode,
            name: "Cycle View Mode",
            description: "Cycle through view modes (Tree → Outline → Continuous → Paginated)",
            category: CommandCategory::Mode,
            keybinding: Some("v"),
        },
        CommandInfo {
            command: TreeCommand::SetViewMode { mode: crate::tree::state::ViewMode::Tree },
            name: "Tree View",
            description: "Switch to hierarchical tree view",
            category: CommandCategory::Mode,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::SetViewMode { mode: crate::tree::state::ViewMode::Outline },
            name: "Outline View",
            description: "Switch to outline card view",
            category: CommandCategory::Mode,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::SetViewMode { mode: crate::tree::state::ViewMode::Continuous },
            name: "Continuous View",
            description: "Switch to scrollable continuous view",
            category: CommandCategory::Mode,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::SetViewMode { mode: crate::tree::state::ViewMode::Paginated },
            name: "Paginated View",
            description: "Switch to paginated view (one section at a time)",
            category: CommandCategory::Mode,
            keybinding: None,
        },
        // Application
        CommandInfo {
            command: TreeCommand::Quit,
            name: "Quit",
            description: "Quit the application",
            category: CommandCategory::Application,
            keybinding: Some("q / Ctrl+c"),
        },
        CommandInfo {
            command: TreeCommand::LoadBufferEvents,
            name: "Load Visible Events",
            description: "Load all unloaded visible events in buffer",
            category: CommandCategory::Application,
            keybinding: Some("L"),
        },
        CommandInfo {
            command: TreeCommand::RefreshBuffer,
            name: "Refresh Buffer",
            description: "Refresh all visible events to newest versions",
            category: CommandCategory::Application,
            keybinding: None,
        },
        // Configuration
        CommandInfo {
            command: TreeCommand::ShowRelays,
            name: "Show Relays",
            description: "Show relay configuration",
            category: CommandCategory::Configuration,
            keybinding: Some(":"),
        },
        CommandInfo {
            command: TreeCommand::ClearRelays,
            name: "Clear Relays",
            description: "Clear custom relays (use defaults)",
            category: CommandCategory::Configuration,
            keybinding: None,
        },
        // Command Palette
        CommandInfo {
            command: TreeCommand::ShowCommandPalette,
            name: "Command Palette",
            description: "Show the command palette (M-x)",
            category: CommandCategory::Application,
            keybinding: Some("M-x / ?"),
        },
        // Compose
        CommandInfo {
            command: TreeCommand::EnterCompose,
            name: "Compose",
            description: "Create a new publication or note",
            category: CommandCategory::Compose,
            keybinding: Some("c"),
        },
        // Window management
        CommandInfo {
            command: TreeCommand::CloseWindow,
            name: "Close Window",
            description: "Close the currently focused window",
            category: CommandCategory::Window,
            keybinding: Some("q / Esc"),
        },
        CommandInfo {
            command: TreeCommand::CloseAllWindows,
            name: "Close All Windows",
            description: "Close all open windows",
            category: CommandCategory::Window,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::FocusNextWindow,
            name: "Focus Next Window",
            description: "Focus the next window in the stack",
            category: CommandCategory::Window,
            keybinding: Some("Tab"),
        },
        CommandInfo {
            command: TreeCommand::FocusPrevWindow,
            name: "Focus Previous Window",
            description: "Focus the previous window in the stack",
            category: CommandCategory::Window,
            keybinding: Some("Shift+Tab"),
        },
        CommandInfo {
            command: TreeCommand::WindowScrollUp,
            name: "Scroll Window Up",
            description: "Scroll the focused window content up",
            category: CommandCategory::Window,
            keybinding: Some("k / ↑"),
        },
        CommandInfo {
            command: TreeCommand::WindowScrollDown,
            name: "Scroll Window Down",
            description: "Scroll the focused window content down",
            category: CommandCategory::Window,
            keybinding: Some("j / ↓"),
        },
        CommandInfo {
            command: TreeCommand::WindowScrollToTop,
            name: "Scroll Window to Top",
            description: "Scroll the focused window to the top",
            category: CommandCategory::Window,
            keybinding: Some("gg"),
        },
        CommandInfo {
            command: TreeCommand::WindowScrollToBottom,
            name: "Scroll Window to Bottom",
            description: "Scroll the focused window to the bottom",
            category: CommandCategory::Window,
            keybinding: Some("G"),
        },
        CommandInfo {
            command: TreeCommand::ShowEventJson,
            name: "Show Event JSON",
            description: "Show the raw JSON for the current selection in a window",
            category: CommandCategory::View,
            keybinding: None,
        },
        // Draft management
        CommandInfo {
            command: TreeCommand::SaveDraft,
            name: "Save Draft",
            description: "Save current composition as an unsigned draft",
            category: CommandCategory::Compose,
            keybinding: Some("Ctrl+d"),
        },
        // Editor view modes
        CommandInfo {
            command: TreeCommand::CycleEditorViewMode,
            name: "Cycle Editor View",
            description: "Cycle editor view mode (Plain → JSON → Structured)",
            category: CommandCategory::Compose,
            keybinding: Some("v / Ctrl+v"),
        },
        CommandInfo {
            command: TreeCommand::SetEditorViewMode { mode: crate::tree::state::EditorViewMode::Plain },
            name: "Plain Editor View",
            description: "Switch to plain text editor view",
            category: CommandCategory::Compose,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::SetEditorViewMode { mode: crate::tree::state::EditorViewMode::Json },
            name: "JSON Preview",
            description: "Switch to JSON event preview view",
            category: CommandCategory::Compose,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::SetEditorViewMode { mode: crate::tree::state::EditorViewMode::Structured },
            name: "Structured View",
            description: "Switch to structured document tree view",
            category: CommandCategory::Compose,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::FilterDrafts,
            name: "Filter Drafts",
            description: "Toggle filter to show only unsigned drafts",
            category: CommandCategory::View,
            keybinding: Some("Ctrl+u"),
        },
        // Identity
        CommandInfo {
            command: TreeCommand::OpenLoginDialog,
            name: "Login",
            description: "Open the login dialog to sign in with npub/nsec/ncryptsec",
            category: CommandCategory::Application,
            keybinding: Some("i"),
        },
        CommandInfo {
            command: TreeCommand::Logout,
            name: "Logout",
            description: "Sign out from the current identity",
            category: CommandCategory::Application,
            keybinding: None,
        },
        CommandInfo {
            command: TreeCommand::ShowUserData,
            name: "User Data Menu",
            description: "Open menu to select and view NIP-51 lists",
            category: CommandCategory::Application,
            keybinding: Some("U"),
        },
    ]
}

/// Commands that can be executed on the tree
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeCommand {
    // Navigation
    /// Move cursor up one visible node
    MoveUp,
    /// Move cursor down one visible node
    MoveDown,
    /// Move cursor to first visible node
    MoveToFirst,
    /// Move cursor to last visible node
    MoveToLast,
    /// Move cursor to parent node
    MoveToParent,
    /// Enter/expand current node (if expandable) or load content
    Enter,
    /// Collapse current node
    Collapse,
    /// Toggle expand/collapse of current node
    ToggleExpand,

    // Selection
    /// Toggle selection of current node
    ToggleSelect,
    /// Select all visible nodes
    SelectAll,
    /// Clear all selections
    ClearSelection,

    // Manipulation (Phase 4)
    /// Move section up within its parent
    MoveSectionUp,
    /// Move section down within its parent
    MoveSectionDown,
    /// Delete current node (or selection)
    Delete,
    /// Yank (copy) current node to clipboard
    Yank,
    /// Paste after current node
    PasteAfter,
    /// Paste before current node
    PasteBefore,

    // Versioning (Phase 5)
    /// Fork current section (create new version)
    Fork,
    /// Show alternate versions of current section
    ShowAlternates,
    /// Slot in a specific version by index
    SlotInVersion { version_index: usize },

    // View
    /// Toggle content preview panel
    TogglePreview,
    /// Scroll preview up
    ScrollPreviewUp,
    /// Scroll preview down
    ScrollPreviewDown,
    /// Scroll content up (for Continuous mode)
    ScrollContentUp,
    /// Scroll content down (for Continuous mode)
    ScrollContentDown,
    /// Go to next page/section (for Paginated mode)
    NextPage,
    /// Go to previous page/section (for Paginated mode)
    PrevPage,
    /// Refresh current view / reload data
    Refresh,

    // Undo/Redo (Phase 4)
    /// Undo last operation
    Undo,
    /// Redo last undone operation
    Redo,

    // Mode switching
    /// Go back (exit reader mode, return to feed)
    Back,
    /// Cycle through view modes (Tree -> Outline -> Continuous -> Paginated)
    CycleViewMode,
    /// Set specific view mode
    SetViewMode { mode: crate::tree::state::ViewMode },

    // Application
    /// Quit the application
    Quit,

    // Configuration
    /// Add a relay to the fetch list
    AddRelay { url: String },
    /// Remove a relay from the fetch list
    RemoveRelay { url: String },
    /// Clear all custom relays (use defaults)
    ClearRelays,
    /// Show relay configuration
    ShowRelays,

    // Batch loading
    /// Load all unloaded visible events in buffer
    LoadBufferEvents,
    /// Refresh all visible events to newest versions
    RefreshBuffer,

    // UI
    /// Show command palette (M-x style)
    ShowCommandPalette,

    // Compose mode
    /// Enter compose mode (structured)
    EnterCompose,
    /// Enter editor compose mode (single buffer)
    EnterEditorCompose,
    /// Exit compose mode (back to feed)
    ExitCompose,
    /// Insert a character at cursor
    InsertChar { c: char },
    /// Delete character before cursor (backspace)
    DeleteChar,
    /// Delete character at cursor (delete key)
    DeleteCharForward,
    /// Move cursor left
    CursorLeft,
    /// Move cursor right
    CursorRight,
    /// Move cursor to start of field
    CursorHome,
    /// Move cursor to end of field
    CursorEnd,
    /// Move to next field (Tab)
    NextField,
    /// Move to previous field (Shift+Tab, also deletes tags in tag mode)
    PrevField,
    /// Enter tag creation mode (Ctrl+t)
    CreateTags,
    /// Exit tag creation mode (Ctrl+e)
    EndTags,
    /// Add a new section (Ctrl+s)
    AddSection,
    /// Remove the last section
    RemoveSection,
    /// Insert newline in content field
    InsertNewline,
    /// Publish the composed content (Ctrl+Enter)
    Publish,
    /// Toggle between structured and editor compose modes
    ToggleComposeStyle,

    // Editor compose mode
    /// Toggle insert/normal mode in editor compose
    EditorToggleMode,
    /// Insert character in editor compose
    EditorInsertChar { c: char },
    /// Insert newline in editor compose
    EditorInsertNewline,
    /// Backspace in editor compose
    EditorBackspace,
    /// Delete at cursor in editor compose
    EditorDelete,
    /// Delete entire line in editor compose
    EditorDeleteLine,
    /// Cursor left in editor compose
    EditorCursorLeft,
    /// Cursor right in editor compose
    EditorCursorRight,
    /// Cursor up in editor compose
    EditorCursorUp,
    /// Cursor down in editor compose
    EditorCursorDown,
    /// Cursor to start of line in editor compose
    EditorCursorHome,
    /// Cursor to end of line in editor compose
    EditorCursorEnd,
    /// Cursor to end of document in editor compose
    EditorCursorToEnd,
    /// Enter insert mode after cursor (a)
    EditorInsertAfter,
    /// Insert new line below and enter insert mode (o)
    EditorInsertLineBelow,
    /// Insert new line above and enter insert mode (O)
    EditorInsertLineAbove,
    /// Cycle editor view mode (Plain → JSON → Structured)
    CycleEditorViewMode,
    /// Set editor view mode to a specific mode
    SetEditorViewMode { mode: crate::tree::state::EditorViewMode },

    // Window management
    /// Open a window with JSON content (e.g., raw event data)
    ShowJson { title: String, content: String },
    /// Close the currently focused window
    CloseWindow,
    /// Close all open windows
    CloseAllWindows,
    /// Focus the next window
    FocusNextWindow,
    /// Focus the previous window
    FocusPrevWindow,
    /// Scroll window content up
    WindowScrollUp,
    /// Scroll window content down
    WindowScrollDown,
    /// Scroll window to top
    WindowScrollToTop,
    /// Scroll window to bottom
    WindowScrollToBottom,
    /// Show event JSON for current selection (opens in a window)
    ShowEventJson,

    // Draft management
    /// Save current compose as draft (Ctrl+d)
    SaveDraft,
    /// Load a draft for editing
    LoadDraft { draft_id: String },
    /// Delete a draft
    DeleteDraft { draft_id: String },
    /// Toggle draft-only filter in feed
    FilterDrafts,

    // Identity management
    /// Open the login dialog
    OpenLoginDialog,
    /// Close the login dialog
    CloseLoginDialog,
    /// Submit a key for login (from dialog)
    SubmitLogin { key: String },
    /// Submit a password for ncryptsec unlock
    SubmitPassword { password: String },
    /// Logout from the current identity
    Logout,
    /// Show user data menu (NIP-51 lists selection)
    ShowUserData,
    /// Close the user data menu
    CloseUserDataMenu,
    /// Navigate up in user data menu
    UserDataMenuUp,
    /// Navigate down in user data menu
    UserDataMenuDown,
    /// Select current item in user data menu
    UserDataMenuSelect,
}

impl TreeCommand {
    /// Check if this command requires async IO
    pub fn may_need_async(&self) -> bool {
        matches!(
            self,
            TreeCommand::Enter
                | TreeCommand::Refresh
                | TreeCommand::Fork
                | TreeCommand::ShowAlternates
                | TreeCommand::SlotInVersion { .. }
                | TreeCommand::LoadBufferEvents
                | TreeCommand::RefreshBuffer
                | TreeCommand::Publish
                | TreeCommand::SaveDraft
        )
    }

    /// Check if this command modifies state
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            TreeCommand::MoveSectionUp
                | TreeCommand::MoveSectionDown
                | TreeCommand::Delete
                | TreeCommand::PasteAfter
                | TreeCommand::PasteBefore
                | TreeCommand::Fork
                | TreeCommand::SlotInVersion { .. }
                | TreeCommand::InsertChar { .. }
                | TreeCommand::DeleteChar
                | TreeCommand::DeleteCharForward
                | TreeCommand::InsertNewline
                | TreeCommand::AddSection
                | TreeCommand::RemoveSection
        )
    }
}

/// Result of executing a command
#[derive(Debug, Clone)]
pub enum CommandResult {
    /// Command executed successfully
    Ok,
    /// Command needs async operation to complete
    NeedsAsync(AsyncRequest),
    /// Command failed with error
    Error(String),
    /// Application should exit
    Exit,
    /// State changed, UI should refresh
    StateChanged,
    /// No-op, command had no effect
    NoOp,
    /// Configuration change requested (handled by app, not engine)
    ConfigChange(ConfigAction),
    /// Mode changed (Feed <-> Reader)
    ModeChanged(crate::tree::state::AppMode),
}

/// Configuration actions that the application handles
#[derive(Debug, Clone)]
pub enum ConfigAction {
    /// Add a relay URL
    AddRelay(String),
    /// Remove a relay URL
    RemoveRelay(String),
    /// Clear all custom relays
    ClearRelays,
    /// Show current relay configuration
    ShowRelays,
}

impl CommandResult {
    /// Check if the result indicates success
    pub fn is_ok(&self) -> bool {
        matches!(self, CommandResult::Ok | CommandResult::StateChanged)
    }

    /// Check if async work is needed
    pub fn needs_async(&self) -> bool {
        matches!(self, CommandResult::NeedsAsync(_))
    }
}

/// Async operation requests from the engine
#[derive(Debug, Clone)]
pub enum AsyncRequest {
    /// Load a publication by address
    LoadPublication {
        addr: NAddr,
        parent: Option<NodeId>,
    },
    /// Load a section by address
    LoadSection {
        addr: NAddr,
        parent: NodeId,
    },
    /// Load all children of a publication
    LoadChildren {
        parent: NodeId,
    },
    /// Find alternate versions of a section
    FindAlternates {
        addr: NAddr,
        node_id: NodeId,
    },
    /// Refresh the entire tree
    RefreshAll,
    /// Search for publications matching a query
    Search {
        query: String,
    },
    /// Load more publications (pagination) - fetch events before the given timestamp
    LoadMorePublications {
        before_timestamp: u64,
        limit: usize,
    },
    /// Batch load multiple items at once
    LoadBatch {
        requests: Vec<AsyncRequest>,
    },
    /// Publish a simple note
    PublishNote {
        content: String,
        tags: Vec<Vec<String>>,
    },
    /// Publish a multi-section publication (30040 + 30041)
    PublishPublication {
        title: String,
        tags: Vec<Vec<String>>,
        sections: Vec<SectionCompose>,
    },
    /// Save compose state as a draft
    SaveDraft {
        compose: crate::tree::state::ComposeState,
    },
    /// Load all drafts from storage
    LoadDrafts,
    /// Load user profile data (metadata, follows, mutes, relays, etc.)
    LoadUserData {
        /// The user's public key (hex)
        pubkey: String,
    },
}

impl AsyncRequest {
    /// Get a descriptive message for this request
    pub fn description(&self) -> String {
        match self {
            AsyncRequest::LoadPublication { addr, .. } => {
                format!("Loading publication {}", addr.d_tag)
            }
            AsyncRequest::LoadSection { addr, .. } => {
                format!("Loading section {}", addr.d_tag)
            }
            AsyncRequest::LoadChildren { .. } => "Loading children".to_string(),
            AsyncRequest::FindAlternates { addr, .. } => {
                format!("Finding alternates for {}", addr.d_tag)
            }
            AsyncRequest::RefreshAll => "Refreshing...".to_string(),
            AsyncRequest::Search { query } => format!("Searching: {}", query),
            AsyncRequest::LoadMorePublications { limit, .. } => {
                format!("Loading {} more publications...", limit)
            }
            AsyncRequest::LoadBatch { requests } => {
                format!("Loading {} items...", requests.len())
            }
            AsyncRequest::PublishNote { .. } => "Publishing note...".to_string(),
            AsyncRequest::PublishPublication { title, .. } => {
                format!("Publishing {}...", title)
            }
            AsyncRequest::SaveDraft { .. } => "Saving draft...".to_string(),
            AsyncRequest::LoadDrafts => "Loading drafts...".to_string(),
            AsyncRequest::LoadUserData { .. } => "Loading user data...".to_string(),
        }
    }

    /// Get the target node ID if applicable
    pub fn target_node(&self) -> Option<NodeId> {
        match self {
            AsyncRequest::LoadPublication { parent, .. } => *parent,
            AsyncRequest::LoadSection { parent, .. } => Some(*parent),
            AsyncRequest::LoadChildren { parent } => Some(*parent),
            AsyncRequest::FindAlternates { node_id, .. } => Some(*node_id),
            AsyncRequest::RefreshAll
            | AsyncRequest::Search { .. }
            | AsyncRequest::LoadMorePublications { .. }
            | AsyncRequest::LoadBatch { .. }
            | AsyncRequest::PublishNote { .. }
            | AsyncRequest::PublishPublication { .. }
            | AsyncRequest::SaveDraft { .. }
            | AsyncRequest::LoadDrafts
            | AsyncRequest::LoadUserData { .. } => None,
        }
    }
}

/// Result from completing an async operation
#[derive(Debug, Clone)]
pub enum AsyncResult {
    /// Publication loaded successfully
    PublicationLoaded {
        node_id: NodeId,
        title: Option<String>,
        children: Vec<NAddr>,
    },
    /// Section loaded successfully
    SectionLoaded {
        node_id: NodeId,
        title: Option<String>,
        content: Option<String>,
    },
    /// Children loaded for a publication
    ChildrenLoaded {
        parent_id: NodeId,
        children: Vec<LoadedChild>,
    },
    /// Alternates found for a section
    AlternatesFound {
        node_id: NodeId,
        versions: Vec<AlternateVersion>,
    },
    /// More publications loaded (for feed pagination)
    MorePublicationsLoaded {
        publications: Vec<LoadedPublication>,
    },
    /// Draft saved successfully
    DraftSaved {
        draft_id: String,
    },
    /// Drafts loaded from storage
    DraftsLoaded {
        drafts: Vec<LoadedDraft>,
    },
    /// User data loaded successfully
    UserDataLoaded {
        /// The loaded user data
        user_data: UserData,
    },
    /// Publication created locally (stored in nostrdb, not yet synced to relays)
    PublicationCreated {
        /// The address of the created publication
        addr: NAddr,
        /// Publication title
        title: Option<String>,
        /// Section addresses with titles and content
        sections: Vec<CreatedSection>,
    },
    /// Operation failed
    Error {
        request: AsyncRequest,
        error: String,
    },
}

/// A loaded draft for display in the feed
#[derive(Debug, Clone)]
pub struct LoadedDraft {
    pub draft_id: String,
    pub title: String,
    pub created_at: u64,
    pub modified_at: u64,
    pub section_count: usize,
}

/// A loaded publication for feed pagination
#[derive(Debug, Clone)]
pub struct LoadedPublication {
    pub addr: NAddr,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub author: String,
    pub author_name: Option<String>,
    pub created_at: u64,
    pub sections: Vec<NAddr>,
}

/// A loaded child node
#[derive(Debug, Clone)]
pub struct LoadedChild {
    pub addr: NAddr,
    pub title: Option<String>,
    pub is_publication: bool,
}

/// A locally created section (for PublicationCreated result)
#[derive(Debug, Clone)]
pub struct CreatedSection {
    pub addr: NAddr,
    pub title: Option<String>,
    pub content: Option<String>,
}

/// An alternate version of a section
#[derive(Debug, Clone)]
pub struct AlternateVersion {
    pub author: String,
    pub created_at: u64,
    pub version_label: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_properties() {
        assert!(TreeCommand::Enter.may_need_async());
        assert!(!TreeCommand::MoveUp.may_need_async());

        assert!(TreeCommand::Delete.is_mutating());
        assert!(!TreeCommand::MoveDown.is_mutating());
    }

    #[test]
    fn test_command_result_checks() {
        assert!(CommandResult::Ok.is_ok());
        assert!(CommandResult::StateChanged.is_ok());
        assert!(!CommandResult::NoOp.is_ok());

        let req = AsyncRequest::RefreshAll;
        assert!(CommandResult::NeedsAsync(req).needs_async());
    }
}
