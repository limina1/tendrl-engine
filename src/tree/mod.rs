//! Tree navigation and manipulation for NKBIP-01 publications
//!
//! This module provides an interface-agnostic TreeEngine for navigating and
//! manipulating publications (kind 30040) and sections (kind 30041) with
//! vim-style keybindings.
//!
//! # Architecture
//!
//! The tree module is designed with clear separation of concerns:
//!
//! - **Pure state** (`state.rs`): TreeState holds all tree data with no IO
//! - **Commands** (`command.rs`): TreeCommand enum defines all operations
//! - **Engine** (`engine.rs`): TreeEngine executes commands synchronously
//! - **Async bridge**: When IO is needed, engine returns AsyncRequest
//!
//! # Example
//!
//! ```rust,no_run
//! use nostr_engine::tree::{TreeEngine, TreeState, TreeCommand, CommandResult};
//!
//! let mut state = TreeState::new();
//! let engine = TreeEngine::new();
//!
//! // Execute a navigation command
//! match engine.execute(&mut state, TreeCommand::MoveDown) {
//!     CommandResult::Ok => println!("Moved down"),
//!     CommandResult::NeedsAsync(req) => {
//!         // Handle async loading in your UI layer
//!         println!("Need to load: {}", req.description());
//!     }
//!     CommandResult::Error(e) => eprintln!("Error: {}", e),
//!     _ => {}
//! }
//! ```

pub mod command;
pub mod content;
pub mod engine;
pub mod node;
pub mod render;
pub mod state;
pub mod undo;

#[cfg(feature = "tui")]
pub mod tui;

// Re-export main types
pub use command::{
    all_commands, AsyncRequest, AsyncResult, CommandCategory, CommandInfo, CommandResult,
    ConfigAction, TreeCommand,
};
pub use content::ContentDetector;
pub use engine::TreeEngine;
pub use node::{ContentMode, NodeId, PublicationNode, SectionNode, TreeNode};
pub use render::{RenderOptions, TreeRenderer, VisibleNode};
pub use state::{
    AppMode, ClipboardContent, CommandPaletteState, FilterMode, TreeState, ViewMode, ViewState,
};
pub use undo::{Operation, UndoStack};
