# nostr-engine

A Rust library and TUI for working with NKBIP-01 publications (kind 30040/30041) on Nostr.

## Architecture

### Core Engine (`src/engine.rs`)
- Manages nostrdb (local database) and relay connections
- Implements `FetchPolicy`: `LocalOnly`, `LocalFirst`, `FetchAlways`
- `LocalFirst` checks DB first, fetches from relays only if not enough results

### Publication Engine (`src/publication.rs`)
- `Publication` and `Section` types for NKBIP-01
- `PublicationEngine` for loading, listing, and paginating publications
- Defensive validation: rejects 30040 events with non-empty content

### Tree Module (`src/tree/`)
The tree module provides an interface-agnostic engine for navigating publications:

- **`state.rs`** - Pure state types
  - `AppMode`: Feed (list) vs Reader (single publication)
  - `ViewMode`: Tree, Outline, Continuous, Paginated
  - `TreeState`: nodes, cursor, feed_cursor, pagination state

- **`command.rs`** - Commands and async requests
  - `TreeCommand`: navigation, selection, mode switching
  - `AsyncRequest`: LoadPublication, LoadSection, LoadMorePublications
  - `AsyncResult`: results from async operations

- **`engine.rs`** - Synchronous command execution
  - Routes commands based on AppMode
  - Returns `AsyncRequest` when IO needed
  - Handles feed pagination (loads 15 more when near bottom)

- **`node.rs`** - Node types (PublicationNode, SectionNode)
- **`render.rs`** - Text rendering for tree view
- **`content.rs`** - Content type detection (Markdown, Org, AsciiDoc)

### TUI (`src/tree/tui/`)
Feature-gated with `--features tui`:

- **`app.rs`** - Main TUI application, async bridge
- **`input.rs`** - Vim-style keybindings with context awareness
- **`widgets.rs`** - ratatui widgets:
  - `FeedWidget`: publication list with infinite scroll
  - `TreeWidget`: hierarchical tree view
  - `OutlineWidget`: sections as cards
  - `ContinuousWidget`: scrollable full content
  - `PaginatedWidget`: one section at a time

## Key Features

### Feed View
- Card-based list of publications
- Infinite scroll (auto-loads 15 more when near bottom)
- Shows title, author, summary, section count
- "Loading more..." / "End of feed" indicators

### Reader View Modes
| Mode | Description | Navigation |
|------|-------------|------------|
| Tree | Hierarchical with expand/collapse | j/k nav, h/l collapse/expand |
| Outline | Sections as cards | j/k nav, preview panel |
| Continuous | Scrollable full content | j/k scroll |
| Paginated | One section at a time | j/k scroll, J/K next/prev section |

### Keybindings
- `j/k` - Navigate (Feed/Tree/Outline) or Scroll (Continuous/Paginated)
- `J/K` - Next/Prev section (Paginated) or Move section (Tree)
- `Enter` - Open publication / Expand node / Load content
- `Esc` - Back to feed
- `v` - Cycle view mode
- `Tab` - Toggle preview panel
- `:` - Show relay configuration
- `q` - Quit

## Running

```bash
# Run TUI
cargo run --features tui --bin nostr-tree

# With custom relays
cargo run --features tui --bin nostr-tree -- --relay wss://relay.example.com

# Purge local database
cargo run --features tui --bin nostr-tree -- --purge-db -y
```

## Default Relays
- wss://theforest.nostr1.com
- wss://thecitadel.nostr1.com
- wss://relay.damus.io

## Testing

```bash
cargo test --features tui
```

## Design Principles

1. **Local-first**: Always check nostrdb before hitting relays
2. **Synchronous engine**: TreeEngine is sync, returns AsyncRequest for IO
3. **Interface-agnostic**: Tree module works with any UI (TUI, GUI, etc.)
4. **Defensive**: Validate events, reject malformed data
5. **Vim-style**: Familiar keybindings for navigation

## Known Limitations

### nostrdb Deletion
nostrdb does not currently support deleting individual events. While it has
infrastructure for deletion (flags like `NDB_NOTE_FLAG_DELETED`), these are
not actively used. Kind 5 deletion events are stored but don't mark referenced
events as deleted. See `docs/nostrdb-deletion-architecture.org` for details.

For delete functionality in nostr-engine:
- Session-only: Remove from TreeState (events remain in nostrdb)
- NIP-09: Publish deletion requests for user-owned events (network signal only)

## File Structure

```
src/
├── lib.rs              # Library root
├── main.rs             # CLI entry (non-TUI)
├── engine.rs           # Core nostr engine
├── publication.rs      # NKBIP-01 types
├── relay.rs            # Relay connections, default relays
├── query.rs            # nostrdb queries
├── bin/
│   └── nostr-tree.rs   # TUI binary
└── tree/
    ├── mod.rs          # Module exports
    ├── state.rs        # TreeState, AppMode, ViewMode
    ├── command.rs      # Commands, AsyncRequest/Result
    ├── engine.rs       # TreeEngine
    ├── node.rs         # Node types
    ├── render.rs       # Text rendering
    ├── content.rs      # Content detection
    ├── undo.rs         # Undo stack (Phase 4)
    └── tui/
        ├── mod.rs
        ├── app.rs      # TuiApp
        ├── input.rs    # KeyMapper
        └── widgets.rs  # All widgets
```
