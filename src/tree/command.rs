//! Tree commands and async request types
//!
//! Defines the commands that can be executed on the tree and the async
//! requests that the engine returns when IO is needed.

use super::node::NodeId;
use crate::publication::NAddr;
use serde::{Deserialize, Serialize};

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
            | AsyncRequest::LoadMorePublications { .. } => None,
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
    /// Operation failed
    Error {
        request: AsyncRequest,
        error: String,
    },
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
