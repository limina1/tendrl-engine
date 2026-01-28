//! Key mapping for TUI input
//!
//! Maps keyboard input to TreeCommands with vim-style keybindings.

use crate::tree::command::TreeCommand;
use crate::tree::state::{AppMode, ComposeFocus, ViewMode};
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
    pub compose_focus: Option<ComposeFocus>,
    /// Whether a window overlay is currently focused
    pub window_focused: bool,
    /// Whether the user data menu is open
    pub user_data_menu_open: bool,
    /// Whether using editor compose (true) vs structured compose (false)
    pub editor_compose: bool,
    /// Whether in insert mode (for editor compose)
    pub editor_insert_mode: bool,
    /// Whether the preview panel is focused
    pub preview_focused: bool,
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
        // Handle window-focused mode - captures navigation input
        if let Some(c) = ctx {
            if c.window_focused {
                return self.map_window(key);
            }
        }

        // Handle user data menu - captures navigation input
        if let Some(c) = ctx {
            if c.user_data_menu_open {
                return self.map_user_data_menu(key);
            }
        }

        // Handle preview-focused mode - captures navigation for scrolling preview
        if let Some(c) = ctx {
            if c.preview_focused {
                return self.map_preview(key);
            }
        }

        // Handle compose mode separately - it captures most input
        if let Some(c) = ctx {
            if c.app_mode == AppMode::Compose {
                if c.editor_compose {
                    return self.map_editor_compose(key, c.editor_insert_mode);
                } else {
                    return self.map_compose(key);
                }
            }
        }

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
            KeyCode::Char(' ') => Some(TreeCommand::ShowCommandPalette),
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
            // Draft filter toggle (Ctrl+u in feed mode)
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::FilterDrafts)
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

            // Command palette (M-x style)
            KeyCode::Char('?') => Some(TreeCommand::ShowCommandPalette),
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::ALT) => {
                Some(TreeCommand::ShowCommandPalette)
            }

            // Quit
            KeyCode::Char('q') => Some(TreeCommand::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Quit)
            }

            // Compose (from feed mode)
            KeyCode::Char('c') => Some(TreeCommand::EnterCompose),
            // Editor compose (C = shift+c)
            KeyCode::Char('C') => Some(TreeCommand::EnterEditorCompose),

            // Login (from feed mode)
            KeyCode::Char('i') => Some(TreeCommand::OpenLoginDialog),

            // User data (show NIP-51 lists)
            KeyCode::Char('U') => Some(TreeCommand::ShowUserData),

            _ => None,
        }
    }

    /// Map keys in compose mode
    fn map_compose(&mut self, key: KeyEvent) -> Option<TreeCommand> {
        match key.code {
            // Exit compose
            KeyCode::Esc => Some(TreeCommand::ExitCompose),

            // Navigation
            KeyCode::Left => Some(TreeCommand::CursorLeft),
            KeyCode::Right => Some(TreeCommand::CursorRight),
            KeyCode::Home => Some(TreeCommand::CursorHome),
            KeyCode::End => Some(TreeCommand::CursorEnd),

            // Field navigation
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    Some(TreeCommand::PrevField)
                } else {
                    Some(TreeCommand::NextField)
                }
            }
            KeyCode::BackTab => Some(TreeCommand::PrevField),

            // Text editing
            KeyCode::Backspace => Some(TreeCommand::DeleteChar),
            KeyCode::Delete => Some(TreeCommand::DeleteCharForward),

            // Enter - either newline or publish
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    Some(TreeCommand::Publish)
                } else {
                    Some(TreeCommand::InsertNewline)
                }
            }

            // Control commands
            KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::CreateTags)
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::EndTags)
            }
            KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::AddSection)
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::TogglePreview)
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::SaveDraft)
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Quit)
            }

            // Command palette - available in all modes
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::ALT) => {
                Some(TreeCommand::ShowCommandPalette)
            }

            // Character input
            KeyCode::Char(c) => Some(TreeCommand::InsertChar { c }),

            _ => None,
        }
    }

    /// Map keys for editor compose mode
    fn map_editor_compose(&mut self, key: KeyEvent, insert_mode: bool) -> Option<TreeCommand> {
        if insert_mode {
            // Insert mode - most keys insert characters
            match key.code {
                // Exit insert mode
                KeyCode::Esc => Some(TreeCommand::EditorToggleMode),

                // Navigation (arrow keys work in insert mode)
                KeyCode::Left => Some(TreeCommand::EditorCursorLeft),
                KeyCode::Right => Some(TreeCommand::EditorCursorRight),
                KeyCode::Up => Some(TreeCommand::EditorCursorUp),
                KeyCode::Down => Some(TreeCommand::EditorCursorDown),
                KeyCode::Home => Some(TreeCommand::EditorCursorHome),
                KeyCode::End => Some(TreeCommand::EditorCursorEnd),

                // Text editing
                KeyCode::Backspace => Some(TreeCommand::EditorBackspace),
                KeyCode::Delete => Some(TreeCommand::EditorDelete),
                KeyCode::Enter => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        Some(TreeCommand::Publish)
                    } else {
                        Some(TreeCommand::EditorInsertNewline)
                    }
                }

                // Control commands
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::SaveDraft)
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::ExitCompose)
                }
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    // Toggle to structured compose
                    Some(TreeCommand::ToggleComposeStyle)
                }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::CycleEditorViewMode)
                }

                // Character input
                KeyCode::Char(c) => Some(TreeCommand::EditorInsertChar { c }),

                _ => None,
            }
        } else {
            // Normal mode - vim-like navigation

            // Handle g prefix for gg
            if self.g_prefix {
                self.g_prefix = false;
                if key.code == KeyCode::Char('g') {
                    return Some(TreeCommand::MoveToFirst); // Reuse for go-to-top
                }
                return None;
            }

            match key.code {
                // Enter insert mode
                KeyCode::Char('i') => Some(TreeCommand::EditorToggleMode),
                KeyCode::Char('a') => Some(TreeCommand::EditorInsertAfter),
                KeyCode::Char('o') => Some(TreeCommand::EditorInsertLineBelow),
                KeyCode::Char('O') => Some(TreeCommand::EditorInsertLineAbove),

                // Exit compose
                KeyCode::Esc | KeyCode::Char('q') => Some(TreeCommand::ExitCompose),

                // Navigation
                KeyCode::Char('h') | KeyCode::Left => Some(TreeCommand::EditorCursorLeft),
                KeyCode::Char('l') | KeyCode::Right => Some(TreeCommand::EditorCursorRight),
                KeyCode::Char('j') | KeyCode::Down => Some(TreeCommand::EditorCursorDown),
                KeyCode::Char('k') | KeyCode::Up => Some(TreeCommand::EditorCursorUp),
                KeyCode::Char('0') | KeyCode::Home => Some(TreeCommand::EditorCursorHome),
                KeyCode::Char('$') | KeyCode::End => Some(TreeCommand::EditorCursorEnd),
                KeyCode::Char('g') => {
                    self.g_prefix = true;
                    None
                }
                KeyCode::Char('G') => Some(TreeCommand::MoveToLast),

                // Delete
                KeyCode::Char('x') => Some(TreeCommand::EditorDelete),
                KeyCode::Char('d') => Some(TreeCommand::EditorDeleteLine),

                // Control commands
                KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::ToggleComposeStyle)
                }
                KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::CycleEditorViewMode)
                }

                // View mode cycling (v in normal mode)
                KeyCode::Char('v') => Some(TreeCommand::CycleEditorViewMode),

                // Publish (P in normal mode or Ctrl+Enter)
                KeyCode::Char('P') => Some(TreeCommand::Publish),
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(TreeCommand::Publish)
                }

                _ => None,
            }
        }
    }

    /// Map keys when a window is focused
    fn map_window(&mut self, key: KeyEvent) -> Option<TreeCommand> {
        // Handle g prefix for gg
        if self.g_prefix {
            self.g_prefix = false;
            if key.code == KeyCode::Char('g') {
                return Some(TreeCommand::WindowScrollToTop);
            }
            return None;
        }

        match key.code {
            // Scrolling
            KeyCode::Char('j') | KeyCode::Down => Some(TreeCommand::WindowScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Some(TreeCommand::WindowScrollUp),
            KeyCode::Char('g') => {
                self.g_prefix = true;
                None
            }
            KeyCode::Char('G') => Some(TreeCommand::WindowScrollToBottom),

            // Page scrolling
            KeyCode::PageDown | KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::WindowScrollDown) // Could be a larger scroll
            }
            KeyCode::PageUp | KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::WindowScrollUp)
            }

            // Window focus cycling
            KeyCode::Tab => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    Some(TreeCommand::FocusPrevWindow)
                } else {
                    Some(TreeCommand::FocusNextWindow)
                }
            }
            KeyCode::BackTab => Some(TreeCommand::FocusPrevWindow),

            // Close window
            KeyCode::Esc | KeyCode::Char('q') => Some(TreeCommand::CloseWindow),

            // Command palette still accessible
            KeyCode::Char(' ') => Some(TreeCommand::ShowCommandPalette),
            KeyCode::Char('?') => Some(TreeCommand::ShowCommandPalette),
            KeyCode::Char('x') if key.modifiers.contains(KeyModifiers::ALT) => {
                Some(TreeCommand::ShowCommandPalette)
            }

            // Quit
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Quit)
            }

            _ => None,
        }
    }

    /// Map keys when the preview panel is focused
    fn map_preview(&mut self, key: KeyEvent) -> Option<TreeCommand> {
        // Handle g prefix for gg
        if self.g_prefix {
            self.g_prefix = false;
            if key.code == KeyCode::Char('g') {
                return Some(TreeCommand::ScrollPreviewToTop);
            }
            return None;
        }

        match key.code {
            // Scrolling
            KeyCode::Char('j') | KeyCode::Down => Some(TreeCommand::ScrollPreviewDown),
            KeyCode::Char('k') | KeyCode::Up => Some(TreeCommand::ScrollPreviewUp),
            KeyCode::Char('g') => {
                self.g_prefix = true;
                None
            }
            KeyCode::Char('G') => Some(TreeCommand::ScrollPreviewToBottom),

            // Page scrolling
            KeyCode::PageDown | KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::ScrollPreviewDown) // Could be larger scroll
            }
            KeyCode::PageUp | KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::ScrollPreviewUp)
            }

            // Exit preview focus (back to main content)
            KeyCode::Tab => Some(TreeCommand::TogglePreview),
            KeyCode::Esc | KeyCode::Char('h') => Some(TreeCommand::UnfocusPreview),

            // Quit
            KeyCode::Char('q') => Some(TreeCommand::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(TreeCommand::Quit)
            }

            _ => None,
        }
    }

    /// Map keys when the user data menu is open
    fn map_user_data_menu(&mut self, key: KeyEvent) -> Option<TreeCommand> {
        match key.code {
            // Navigation
            KeyCode::Char('j') | KeyCode::Down => Some(TreeCommand::UserDataMenuDown),
            KeyCode::Char('k') | KeyCode::Up => Some(TreeCommand::UserDataMenuUp),

            // Selection
            KeyCode::Enter | KeyCode::Char('l') => Some(TreeCommand::UserDataMenuSelect),

            // Close menu
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') => {
                Some(TreeCommand::CloseUserDataMenu)
            }

            // Quit app
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
        ("SPC/M-x/?", "Command palette"),
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
        ("Ctrl+u", "Filter drafts"),
        ("Ctrl+d", "Save draft (compose)"),
        ("V/s", "Select/Select all"),
        (":", "Show relays"),
        ("i", "Login"),
        ("U", "User data menu (NIP-51)"),
        ("c", "Compose"),
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
