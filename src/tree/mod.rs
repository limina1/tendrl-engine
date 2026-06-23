//! Pure document-structure core for NKBIP-01 publications.
//!
//! What remains here is the frontend-agnostic, IO-free core that any frontend
//! (web today; emacs/nvim planned) can share as the single source of truth for
//! turning text into structured events:
//!
//! - [`node`]: the `TreeNode` data structure (publication/section nodes) and
//!   `ContentMode`.
//! - [`parser`]: line-by-line classification of compose input — which lines
//!   become kind 30040 indexes vs. kind 30041 sections.
//! - [`content`]: `ContentDetector` content-format detection.
//!
//! The former ratatui TUI lived here too (a `TreeState`/`TreeEngine`/
//! `TreeCommand` navigation machine plus window/palette/clipboard view-state).
//! It had no production consumer after the TUI was removed and was deleted in
//! the Phase 3 boundary cleanup. View/interaction state belongs
//! in the frontend; the compose payload types moved to
//! [`crate::publication::compose`].

pub mod content;
pub mod node;
pub mod parser;

// Re-export the pure core types.
pub use content::ContentDetector;
pub use node::{ContentMode, NodeId, PublicationNode, SectionNode, TreeNode};
pub use parser::{LineType, ParsedDocument, ParsedLine, Section as ParsedSection};
