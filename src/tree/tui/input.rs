//! Key mapping for TUI input
//!
//! Maps keyboard input to TreeCommands with vim-style keybindings.

use crate::tree::command::TreeCommand;
use crate::tree::state::{AppMode, ViewMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Maps key events to tree commands
pub struct KeyMapper {
    /// Whether we're in "g" prefix mode (for gg)
    g_prefix: bool,
}

/// Context for key mapping decisions
pub struct KeyContext {
    pub app_mode: AppMode,
    pub view_mode: ViewMode,
}

impl KeyMapper {
    pub fn new() -> Self {
        KeyMapper { g_prefix: false }
    }

    /// Map a key event to a command (without context - uses default behavior)
    pub fn map(&mut self, key: KeyEvent) -> Option<TreeCommand> {
        self.map_with_context(key, None)
    }

    /// Map a key event to a command with context for view-mode-aware behavior
    pub fn map_with_context(&mut self, key: KeyEvent, ctx: Option<&KeyContext>) -> Option<TreeCommand> {
        // Handle g prefix for gg
        if self.g_prefix {
            self.g_prefix = false;
            if key.code == KeyCode::Char('g') {
                return Some(TreeCommand::MoveToFirst);
            }
            // g followed by anything else is ignored
            return None;
        }

        // Check if we're in a scrollable view mode (Continuous or Paginated in Reader mode)
        let is_scrollable_view = ctx.map_or(false, |c| {
            c.app_mode == AppMode::Reader
                && matches!(c.view_mode, ViewMode::Continuous | ViewMode::Paginated)
        });

        match key.code {
            // Navigation - context-aware for scrollable views
            KeyCode::Char('j') | KeyCode::Down => {
                if is_scrollable_view {
                    Some(TreeCommand::ScrollContentDown)
                } else {
                    Some(TreeCommand::MoveDown)
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if is_scrollable_view {
                    Some(TreeCommand::ScrollContentUp)
                } else {
                    Some(TreeCommand::MoveUp)
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                // h or left: collapse or move to parent
                Some(TreeCommand::Collapse)
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                // l, right, or enter: expand/enter
                Some(TreeCommand::Enter)
            }
            KeyCode::Char(' ') => Some(TreeCommand::ToggleExpand),
            KeyCode::Char('g') => {
                // Start g prefix
                self.g_prefix = true;
                None
            }
            KeyCode::Char('G') => Some(TreeCommand::MoveToLast),

            // Section manipulation / Page navigation
            KeyCode::Char('J') => {
                // In Paginated mode, J goes to next section
                if ctx.map_or(false, |c| c.view_mode == ViewMode::Paginated) {
                    Some(TreeCommand::NextPage)
                } else {
                    Some(TreeCommand::MoveSectionDown)
                }
            }
            KeyCode::Char('K') => {
                // In Paginated mode, K goes to previous section
                if ctx.map_or(false, |c| c.view_mode == ViewMode::Paginated) {
                    Some(TreeCommand::PrevPage)
                } else {
                    Some(TreeCommand::MoveSectionUp)
                }
            }

            // Clipboard
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::NONE) => {
                // dd - but we need double tap detection, for now just single d
                // TODO: Implement proper dd with timing
                Some(TreeCommand::Delete)
            }
            KeyCode::Char('y') => Some(TreeCommand::Yank),
            KeyCode::Char('p') => Some(TreeCommand::PasteAfter),
            KeyCode::Char('P') => Some(TreeCommand::PasteBefore),

            // Versioning
            KeyCode::Char('f') => Some(TreeCommand::Fork),
            KeyCode::Char('a') => Some(TreeCommand::ShowAlternates),

            // View
            KeyCode::Tab => Some(TreeCommand::TogglePreview),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Redo)
            }
            KeyCode::Char('u') => Some(TreeCommand::Undo),

            // Scrolling (larger jumps)
            KeyCode::PageUp => Some(TreeCommand::ScrollPreviewUp),
            KeyCode::PageDown => Some(TreeCommand::ScrollPreviewDown),

            // View mode cycling
            KeyCode::Char('v') => Some(TreeCommand::CycleViewMode),

            // Selection
            KeyCode::Char('V') => Some(TreeCommand::ToggleSelect),
            KeyCode::Char('s') => Some(TreeCommand::SelectAll),

            // Back (exit reader mode) / Clear selection
            KeyCode::Esc => Some(TreeCommand::Back),
            KeyCode::Backspace => Some(TreeCommand::Back),

            // Refresh and batch loading
            KeyCode::Char('R') => Some(TreeCommand::Refresh),
            KeyCode::Char('L') => Some(TreeCommand::LoadBufferEvents),

            // Relays
            KeyCode::Char(':') => Some(TreeCommand::ShowRelays),

            // Quit
            KeyCode::Char('q') => Some(TreeCommand::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Quit)
            }

            _ => None,
        }
    }

    /// Reset any prefix state
    pub fn reset(&mut self) {
        self.g_prefix = false;
    }
}

impl Default for KeyMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Get help text for keybindings
pub fn keybinding_help() -> Vec<(&'static str, &'static str)> {
    vec![
        ("j/k", "Move/Scroll (context-aware)"),
        ("h/l", "Collapse/Expand (Tree mode)"),
        ("Enter", "Enter/Open/Load"),
        ("Esc/Backspace", "Back to feed"),
        ("Space", "Toggle expand"),
        ("gg/G", "First/Last"),
        ("v", "Cycle view mode"),
        ("J/K", "Next/Prev section (Paginated) or Move section (Tree)"),
        ("dd", "Delete"),
        ("yy", "Yank"),
        ("p/P", "Paste after/before"),
        ("f", "Fork section"),
        ("a", "Show alternates"),
        ("Tab", "Toggle preview"),
        ("u/Ctrl+r", "Undo/Redo"),
        ("V/s", "Select/Select all"),
        (":", "Show relays"),
        ("R", "Refresh"),
        ("L", "Load visible"),
        ("q", "Quit"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn key_with_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn test_basic_navigation() {
        let mut mapper = KeyMapper::new();

        assert_eq!(mapper.map(key(KeyCode::Char('j'))), Some(TreeCommand::MoveDown));
        assert_eq!(mapper.map(key(KeyCode::Char('k'))), Some(TreeCommand::MoveUp));
        assert_eq!(mapper.map(key(KeyCode::Down)), Some(TreeCommand::MoveDown));
        assert_eq!(mapper.map(key(KeyCode::Up)), Some(TreeCommand::MoveUp));
    }

    #[test]
    fn test_gg_sequence() {
        let mut mapper = KeyMapper::new();

        // First g sets prefix
        assert_eq!(mapper.map(key(KeyCode::Char('g'))), None);
        // Second g triggers MoveToFirst
        assert_eq!(
            mapper.map(key(KeyCode::Char('g'))),
            Some(TreeCommand::MoveToFirst)
        );

        // Single G goes to last
        assert_eq!(
            mapper.map(key(KeyCode::Char('G'))),
            Some(TreeCommand::MoveToLast)
        );
    }

    #[test]
    fn test_ctrl_modifiers() {
        let mut mapper = KeyMapper::new();

        assert_eq!(
            mapper.map(key_with_mod(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(TreeCommand::Redo)
        );
        assert_eq!(
            mapper.map(key_with_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(TreeCommand::Quit)
        );
    }
}
