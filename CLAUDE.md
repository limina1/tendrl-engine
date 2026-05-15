# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
# Build
cargo build                         # debug build
cargo build --release               # release build

# Run
cargo run -- -c config.toml         # run engine with config
./start.sh                          # start all services (sidecar + engine)
./start.sh --dev                    # also start web frontend dev server

# Tests
cargo test                          # all tests
cargo test <test_name>              # single test by name
cargo test --lib search             # tests in a specific module
cargo test -- --nocapture           # show println output

# Lint
cargo clippy
cargo fmt --check
```

## Architecture

**tendrl-engine** is a local-first Nostr backend implementing NKBIP-01 (publication indexes and sections). It has three layers:

### Core Engine (`src/engine.rs`)
The `Engine` struct owns a `nostrdb::Ndb` instance, relay config, embedding index, and network activity tracker. All data access goes through Engine. It is wrapped in `Arc<Engine>` and shared across the HTTP server and background tasks.

**FetchPolicy** controls data retrieval: `LocalOnly` (nostrdb only), `LocalFirst` (local then relay backfill), `FetchAlways` (always hit relays). When the engine is in `Offline` NetworkMode, all relay fetches are suppressed regardless of policy.

### Publication System (`src/publication.rs`)
Implements NKBIP-01 structured documents:
- **Kind 30040**: Publication indexes (table of contents referencing sections via `a` tags)
- **Kind 30041**: Publication sections (individual content blocks)
- **NAddr**: `kind:pubkey:d_tag` addressing for replaceable events
- **LoadStatus<T>**: Generic `Pending → Loading → Loaded { data } / Failed { error }` state machine used throughout for lazy-loading

The `PublicationEngine` trait (implemented on `Engine`) provides `load_publication()`, `load_sections()`, `load_section_versions()`, and publishing operations.

### Tree Module (`src/tree/`)
Interface-agnostic tree navigation for publications, with strict separation:
- **`state.rs`**: Pure `TreeState` — all data, no IO. Modes: `Feed`, `Reader`, `Compose`
- **`command.rs`**: `TreeCommand` enum — every possible user action
- **`engine.rs`**: `TreeEngine` — executes commands synchronously on TreeState
- **Async boundary**: When IO is needed, `TreeEngine::execute()` returns `CommandResult::NeedsAsync(AsyncRequest)`. The UI layer handles the async work and feeds results back via `apply_async_result()`. This is the key architectural pattern — tree logic never does IO directly.
- **`node.rs`**: `TreeNode` enum (`Publication` | `Section`), identified by `NodeId`
- **`render.rs`**: Flattens tree into `Vec<VisibleNode>` for display, consumed by the web UI

The tree logic is interface-agnostic — the web UI is its consumer, and the design keeps room for other front-ends (e.g. Emacs).

### HTTP API (`src/api.rs`, `src/main.rs`)
Axum-based REST API. State is `Arc<Engine>`. Routes are mounted under `/api/v1/`. The `EngineError` type in `src/error.rs` implements `IntoResponse` for automatic HTTP error codes.

### Supporting Modules
- **`relay.rs`**: WebSocket relay fetching, NIP-01 REQ/EVENT/EOSE protocol
- **`embedding.rs`**: HNSW vector index (usearch) with dual backends: Python sidecar (HTTP at port 3031) or in-process ONNX (`--features onnx`)
- **`search.rs`**: Structured query parser — supports `kind:N`, `by:npub`, `#tag:val`, `"exact match"`, and `~semantic query` syntax
- **`config.rs`**: TOML config with relay sets (general/fetch/publish), embedding, identity
- **`network.rs`**: Online/Offline mode toggle and `FetchGuard` RAII pattern for tracking active relay fetches
- **`identity.rs`**: Nostr key management — ncryptsec decryption, keyring storage, NIP-49
- **`publication.rs`**: Also contains `PublishPayload` for signing and broadcasting events

## Project Layout

- `src/` — Rust engine (library + the `nostr-engine` binary)
- `web/` — SvelteKit frontend (Svelte 5, pnpm, static adapter, served from `web/build/`)
- `sidecar/` — Python embedding server (sentence-transformers, Flask, uv for venv)
- `config.example.toml` — reference config; copy to `config.toml`
- `scripts/` — utility scripts (MCP server, publishing helpers)
- `knowledgebase/` — local documents for import
- `nips/` — Nostr protocol specs (NIP references and custom NKBIPs)

## Key Patterns

- **Error handling**: `EngineError` (thiserror) with variants for Database, Relay, NotFound, etc. `Result<T>` is aliased to `Result<T, EngineError>`.
- **Relay sets**: Config supports separate relay lists for general/fetch/publish. `RelayConfig::resolved()` merges legacy `default_relays` into the new set structure.
- **Content modes**: Sections support Markdown, Org Mode, AsciiDoc, and PlainText, detected from event tags.
- **Nostr event kinds used**: 0 (metadata), 3 (contacts), 10002 (relay list), 30023 (long-form), 30040 (publication index), 30041 (publication section), 30817/30818 (wiki), 9802 (highlight).
