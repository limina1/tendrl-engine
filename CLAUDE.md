# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
# Build
cargo build                         # debug build
cargo build --release               # release build
cargo build --features onnx         # with in-process ONNX embeddings

# Run
cargo run -- -c config.toml         # run engine with config
./start.sh                          # start all services (sidecar + engine + web preview)
./start.sh --dev                    # use the Vite dev server (hot reload) for the web UI
./start.sh --build                  # rebuild web/build/ before starting preview

# Tests
cargo test                          # all tests
cargo test <test_name>              # single test by name
cargo test --lib search             # tests in a specific module
cargo test -- --nocapture           # show println output

# Lint
cargo clippy
cargo fmt --check
pnpm -C web check                   # Svelte / TypeScript checks

# Dependency updates (cargo + pnpm + uv + git-pinned nostrdb)
make                                # report outdated deps — changes nothing
make update                         # safe updates within version ranges
```

## Architecture

**tendrl-engine** is a local-first Nostr backend implementing NKBIP-01 (publication
indexes and sections). It runs as three cooperating processes: the Rust **engine**
(`src/`, port 3030), a SvelteKit **web frontend** (`web/`, port 5173 dev / 5174
preview), and an optional Python **embedding sidecar** (`sidecar/`, port 3031). The
engine owns all data access; the web UI consumes the engine's interface-agnostic
tree logic.

### Core Engine (`src/engine.rs`)
The `Engine` struct owns the `nostrdb::Ndb` instance, relay config, embedding index,
ignore list, NIP-11 cache, and a `NetworkActivity` tracker. All data access goes
through Engine. It is wrapped in `Arc<Engine>` and shared across the HTTP server and
background tasks.

**FetchPolicy** controls data retrieval: `LocalOnly` (nostrdb only), `LocalFirst`
(local then relay backfill), `FetchAlways` (always hit relays). When the engine's
`NetworkMode` is `Confirm`, user-initiated relay fetches are gated behind a
confirmation step instead of running automatically.

### Network Module (`src/network.rs`)
Engine-level `NetworkMode` toggle and a ring buffer of recent relay fetch activity.

- **`Auto`** — fetch from relays automatically (the former "Online").
- **`Confirm`** — every user-initiated relay fetch emits an *intent* the UI must
  approve before the engine proceeds (the former "Offline").

Pending intents and live fetch activity are pushed to clients over an SSE
`fetch-events` stream; the UI approves an intent via a confirm endpoint. The
`FetchGuard` RAII type tracks active relay fetches for the activity log.

### Publication System (`src/publication.rs`)
Implements NKBIP-01 structured documents:
- **Kind 30040**: Publication indexes (table of contents referencing sections via `a` tags)
- **Kind 30041**: Publication sections (individual content blocks)
- **NAddr**: `kind:pubkey:d_tag` addressing for replaceable events
- **LoadStatus<T>**: Generic `Pending → Loading → Loaded { data } / Failed { error }`
  state machine used throughout for lazy-loading

The `PublicationEngine` trait (implemented on `Engine`) provides `load_publication()`,
`load_sections()`, `load_section_versions()`, a recursive depth-N publication tree
loader, and publishing operations. `publication.rs` also contains `PublishPayload`
for signing and broadcasting events.

### Tree Module (`src/tree/`)
Interface-agnostic tree navigation for publications, with strict separation:
- **`state.rs`**: Pure `TreeState` — all data, no IO. `AppMode` is `Feed`, `Reader`,
  or `Compose`.
- **`command.rs`**: `TreeCommand` enum — every possible user action
- **`engine.rs`**: `TreeEngine` — executes commands synchronously on TreeState
- **Async boundary**: When IO is needed, `TreeEngine::execute()` returns
  `CommandResult::NeedsAsync(AsyncRequest)`. The UI layer handles the async work and
  feeds results back via `apply_async_result()`. This is the key architectural
  pattern — tree logic never does IO directly.
- **`node.rs`**: `TreeNode` enum (`Publication` | `Section`), identified by `NodeId`
- **`render.rs`**: Flattens tree into `Vec<VisibleNode>` for display, consumed by the web UI
- **`content.rs`**: `ContentDetector` — detects content format from event tags/heuristics
- **`parser.rs`**: Line-by-line classification for compose mode (headings, attributes,
  code blocks; determines which event kind — 30040/30041 — a line generates)
- **`undo.rs`**: `UndoStack` for tree operations (stub, expanded incrementally)

The tree logic is interface-agnostic — the web UI is its consumer, and the design
keeps room for other front-ends (e.g. Emacs).

### HTTP API (`src/api.rs`, `src/main.rs`)
Axum-based REST API. State is `Arc<Engine>`. Routes are mounted under `/api/v1/`;
`main.rs` wires the router (including separate `chat`, `identity`, and `signing`
sub-routers merged in). The `EngineError` type in `src/error.rs` implements
`IntoResponse` for automatic HTTP error codes. A background task fetches missing
sections and embeds new events on a 60-second interval when embeddings are enabled.

### Supporting Modules
- **`query.rs`**: Local nostrdb NIP-01 filter querying. **All nostrdb reads are
  serialized through a process-wide mutex** — concurrent `ndb_query` calls corrupt
  nostrdb's heap and abort the process; do not remove this lock.
- **`relay.rs`**: WebSocket relay fetching, NIP-01 REQ/EVENT/EOSE protocol
- **`search.rs`**: Structured query parser — see *Search syntax* below
- **`embedding.rs`**: HNSW vector index (usearch) with dual backends: Python sidecar
  (HTTP at port 3031) or in-process ONNX (`--features onnx`)
- **`config.rs`**: TOML config — `initial_relays` (bootstrap seed only),
  `timeout_ms`, authors, embedding, identity. The live `general`/`fetch`/`publish`
  URL sets are populated at runtime from `relay_store.rs`, not from TOML.
- **`identity.rs`**: Nostr key management — ncryptsec decryption, keyring storage, NIP-49
- **`signing.rs`**: Pluggable `Signer` trait + `InProcessSigner`. The engine is the
  signing orchestrator; callers turn an `EventTemplate` into a `SignedEvent` without
  knowing the key source (engine ncryptsec, NIP-07, future NIP-46).
- **`nip11.rs`**: NIP-11 relay information document fetch + cache (process-wide,
  normalized URL keys, 1-hour TTL)
- **`nip19.rs`**: bech32 TLV decoders for `nevent`/`naddr`/`nprofile` (npub/nsec live
  in `identity.rs`); unified `decode()` returns a tagged enum for the API
- **`user_data.rs`**: NIP-01/02/51/65 profile data — parses kind 0/3/10000/10002/
  10003/10006/10007/30002 from nostrdb `Note`s on login
- **`chat.rs`**: Pure state logic for LLM chat fragments, edit mode, context
  injection, message serialization (no IO)
- **`llm.rs`**: Async `LLMProvider` trait — `NoopProvider` (testing) and
  `ClaudeProvider` (Anthropic Messages API)
- **`claude_sessions.rs`**: Reader for Claude Code conversation JSONL files in
  `~/.claude/projects/`
- **`drafts.rs`**: Local JSON draft storage for unsigned NKBIP-01 publications before
  they are signed and published
- **`relay_store.rs`**: Persistent runtime relay sets (`general` / `fetch` /
  `publish`) backed by `<data_dir>/relays.json`. The TOML config only carries the
  bootstrap `initial_relays` seed; `relays.json` is authoritative for the live
  working sets and is rewritten on every UI relay add/remove. Never publishes
  Nostr events — relay-list mutations are local-only state by design.

## Search syntax

The parser (`search.rs`) tokenizes a query string into typed filters:
- `k:N` — kind filter
- `by:me` / `by:assistant` / `by:npub1…` / `by:<64-hex>` / `by:name:<partial>` /
  `by:<word>` — author filter (a bare word resolves as a name partial)
- `id:<64-hex>` — exact event-id lookup
- `~:concept` or `~:"phrase":k` — semantic search (default k=10)
- `"exact phrase"` — exact text match
- `note1…` / `nevent1…` / `naddr1…` / `npub1…` / `nprofile1…` (optionally
  `nostr:`-prefixed) — NIP-19 entities decoded to precise filters
- `since:<ts>` / `until:<ts>` — NIP-01 time bounds
- `has:NAME` — tag-presence operator (matches any event carrying a `NAME` tag)
- `NAME:value` — generic tag filter (**bare key — no `#` prefix**; `#NAME:value` is
  NIP-01 raw-filter notation and parses as a literal text token here)

Note `author:` is a *tag* filter (events with an `["author", …]` tag), **not** an
alias for the `by:` publishing-pubkey filter.

## Project Layout

- `src/` — Rust engine (library + the `nostr-engine` binary)
- `web/` — SvelteKit frontend (Svelte 5, pnpm, static adapter, served from `web/build/`)
- `sidecar/` — Python embedding server (sentence-transformers, Flask, uv for venv)
- `config.example.toml` — reference config; copy to `config.toml`
- `scripts/` — utility scripts (MCP server, publishing helpers)
- `knowledgebase/` — local documents for import
- `nips/` — Nostr protocol specs (NIP references and custom NKBIPs)
- `docs/` — design notes, roadmaps, and `commands.org` (verify/test command reference)

## Key Patterns

- **Error handling**: `EngineError` (thiserror) with variants for Database, Relay,
  NotFound, etc. `Result<T>` is aliased to `Result<T, EngineError>`.
- **Relay sets**: The TOML config carries only `initial_relays` (a bootstrap seed
  list used **once** on first boot). The live working sets — `general` / `fetch` /
  `publish` — live in `<data_dir>/relays.json`, mutated by `engine.add_relay` /
  `engine.remove_relay` and the `POST /api/v1/config/update` handler. The state
  file is **never** broadcast as a Nostr event (no auto-publishing kind 10002 /
  30002 from UI edits — see `project_publishing_philosophy`).
- **Content modes**: Sections support Markdown, Org Mode, AsciiDoc, and PlainText,
  detected from event tags.
- **nostrdb read lock**: Never run nostrdb queries concurrently — `query.rs` holds a
  global mutex for a reason (see module docs).
- **Nostr event kinds used**: 0 (metadata), 3 (contacts), 10000/10002/10003/10006/
  10007 (NIP-51/65 lists), 30002 (relay sets), 30023 (long-form), 30040 (publication
  index), 30041 (publication section), 30817/30818 (wiki), 9802 (highlight).
