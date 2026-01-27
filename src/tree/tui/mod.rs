//! TUI implementation for TreeEngine
//!
//! Provides a ratatui-based terminal interface for navigating publications.

pub mod app;
pub mod input;
pub mod widgets;

pub use app::TuiApp;
pub use input::KeyMapper;
