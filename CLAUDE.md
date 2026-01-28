# nostr-engine

A Rust library and TUI for working with NKBIP-01 publications (kind 30040/30041) on Nostr.

## External Dependencies & Reference Code

### Dependencies (from external projects)

| Library | Source | Purpose |
|---------|--------|---------|
| `nostrdb` | [damus-io/nostrdb-rs](https://github.com/damus-io/nostrdb-rs) | Local Nostr event database (LMDB-backed) |

### Reference Repositories (in project directory)

The following repositories are included locally for reference patterns. **Do not modify
these directly** - they exist to show idiomatic usage of nostrdb and Nostr patterns.

| Directory | Purpose |
|-----------|---------|
| `notedeck/` | Reference for nostrdb usage patterns, tag parsing, account management |
| `gc-alexandria`| Reference for NKBIP-01 reading and writing events|
| `notedeck-compose/` | Older version, less relevant |
| `nips/` | NIP specifications for protocol reference |


### When to Consult gc-alexandria/
If you are implementing a collection related events. 

It is the reference implementation for:
- fetching publications
- displaying publications
- composing documents through a text editor.

### When to Consult notedeck/

**Before modifying nostrdb interaction code**, check how notedeck handles it:

1. **Tag parsing** - notedeck works directly with `Note` objects, not JSON:
   - Use `.variant().id()` for binary pubkeys/event IDs → `Option<&[u8; 32]>`
   - Use `.variant().str()` for strings (relay URLs, hashtags, d-tags)
   - See: `notedeck/crates/notedeck/src/filter.rs`, `notedeck/crates/notedeck/src/account/mute.rs`

2. **NIP-51 lists** (mute, bookmarks, relays):
   - See: `notedeck/crates/notedeck/src/account/mute.rs`
   - See: `notedeck/crates/notedeck/src/account/relay.rs`

3. **Follow list / contacts (kind 3)**:
   - See: `notedeck/crates/notedeck/src/filter.rs:428-448`

4. **Event building**:
   - See: `notedeck/crates/notedeck_columns/src/profile.rs:165-167` for tag variant handling

### Code Ownership

| Path | Origin | Notes |
|------|--------|-------|
| `src/` | **tendrl-engine (original)** | Our code |
| `notedeck/` | notedeck (reference only) | Do not modify |
| `notedeck-compose/` | notedeck (reference only) | Do not modify |
| `nips/` | nostr-protocol/nips (reference) | NIP specifications |

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
- `i` - Login (open identity dialog)
- `U` - User data menu (NIP-51 list selection)
- `c` - Compose new publication
- `:` - Show relay configuration
- `q` - Quit

## Running

```bash
# Run TUI
cargo run --features tui --bin nostr-tree

# With custom relays
cargo run --features tui --bin nostr-tree -- --relay wss://relay.example.com

# Purge local database (events will be re-fetched from relays)
cargo run --features tui --bin nostr-tree -- --purge-db -y

# Purge and start with empty feed (no relay fetching)
cargo run --features tui --bin nostr-tree -- --purge-db -y --policy local_only

# Purge database AND clear identity from OS keyring
cargo run --features tui --bin nostr-tree -- --purge-db --purge-identity -y
```

## Relay Configuration

### Default Relays
When no custom relays are specified via `--relay`, the default is:
- ws://localhost:3334

To change default relays, edit `src/relay.rs:DEFAULT_RELAYS`.

### Custom Relays
Override the default relay list with `--relay` (can be specified multiple times):
```bash
# Single relay
cargo run --features tui --bin nostr-tree -- --relay wss://relay.example.com

# Multiple relays
cargo run --features tui --bin nostr-tree -- \
    --relay wss://relay1.example.com \
    --relay wss://relay2.example.com

# Local relay only (for offline/testing)
cargo run --features tui --bin nostr-tree -- --relay ws://localhost:3334
```

Note: When using `--relay`, the local relay auto-detection is skipped (only
explicitly specified relays are used).

## Testing

```bash
cargo test --features tui
```

### Identity Test Credentials

For testing the login system with ncryptsec (NIP-49):

**Fast test (log_n=8):**
```
ncryptsec: ncryptsec1qgyqzqsrqszsvpcgpy9qkrqdpc83qygjzv2p29shrqv35xcur50p7gppyg3jgffxyu5z8wu7vvv70gz4s7p5qnaualstkwcj8tr5zxa67u6vrdfcum674j8lkajw8aul9qw05hstpjx9cprargvjj0tz
password: nostr-engine-test
secret:   e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6
```

**Production strength (log_n=16, nak default):**
```
ncryptsec: ncryptsec1qggqzqsrqszsvpcgpy9qkrqdpc83qygjzv2p29shrqv35xcur50p7gppyg3jgffxyu5fu084qapxen8l6nyvkeexzyjf5wsh23tjyrmqg2r5aajj9gn578l6gu30usjgxjrjyl4n43rklcw0pvhqxme9
password: nostr-engine-test
secret:   e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6
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
