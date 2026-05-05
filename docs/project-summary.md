#+TITLE: Tendrl Engine: Project Summary
#+SUBTITLE: Nostr-Native Knowledge Workbench — Design, Architecture, and Implementation Status
#+DATE: 2026-04-27
#+STATUS: CURRENT — reflects implemented state as of commit 5c92b81

* What This Project Is

Tendrl Engine is a composition environment for long-form writing on the Nostr
protocol. It treats Nostr events as a zettelkasten — atomic, tagged, addressable
notes that can be searched, discussed with an LLM, and assembled into
publications.

The system implements NKBIP-01, a convention for structured publications using
two Nostr event kinds:

- *kind 30040* — a publication index (table of contents, ordered list of
  section references)
- *kind 30041* — a section (an individual content block: a chapter, a code
  listing, a note)

A publication is an ordered assembly of sections. Sections are independently
addressable, versionable, and can appear in multiple publications. This is the
zettelkasten property: content is composed, not copied.

* Architecture

The system has three layers:

#+begin_example
┌────────────────────────────────────────────────────────────────┐
│  Frontends                                                     │
│  ┌──────────┐  ┌──────────────────┐  ┌───────────────────┐    │
│  │ TUI      │  │ Svelte Web App   │  │ Emacs / Nvim      │    │
│  │ (ratatui)│  │ (primary UI)     │  │ (via HTTP API)    │    │
│  └────┬─────┘  └────────┬─────────┘  └────────┬──────────┘    │
│       │                 │                      │               │
│       ▼                 ▼                      ▼               │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  HTTP API  (Axum)                                       │   │
│  │  /api/v1/search, /chat, /publications, /events          │   │
│  └────────────────────────┬────────────────────────────────┘   │
│                           │                                    │
│  ┌────────────────────────▼────────────────────────────────┐   │
│  │  Engine Layer                                           │   │
│  │  nostrdb (event storage) + relay I/O + search + LLM    │   │
│  └─────────────────────────────────────────────────────────┘   │
└────────────────────────────────────────────────────────────────┘
#+end_example

** Rust Backend

- *nostrdb* — embedded C database for Nostr events, accessed via Rust bindings.
  Events are ingested from relays and stored locally. All queries start here.
- *Relay layer* — NIP-01 WebSocket client. Fetches from multiple relays in
  parallel, deduplicates, ingests into nostrdb. Publishes signed events.
- *Engine* — query coordinator with three fetch policies:
  - =LocalOnly= — nostrdb only (instant, offline)
  - =LocalFirst= — local then relay backfill if under limit (default)
  - =FetchAlways= — relays first, then merge local
- *Search* — structured query parser (tag, kind, author, text, semantic) that
  compiles to NIP-01 filters with post-filter text matching. Semantic search
  via =~:concept= / =~:"phrase":k= with usearch HNSW. OR/union queries via =|=.
- *Chat / LLM* — conversation state with fragment model, context injection,
  edit mode. Provider trait with Claude (Anthropic Messages API) and noop
  implementations.
- *Publication builder* — constructs 30040/30041 event sets from compose state,
  handles block types (editable, imported reference, fork with lineage).

** HTTP API

All state manipulation flows through a REST API so any frontend can drive it:

| Endpoint                                      | Method | Purpose                                |
|-----------------------------------------------+--------+----------------------------------------|
| =POST /api/v1/search=                           | POST   | Structured query (tags, text, kinds)   |
| =GET /api/v1/publications=                       | GET    | List root publications                 |
| =GET /api/v1/publications/:pubkey/:d_tag=        | GET    | Publication detail with TOC            |
| =POST /api/v1/publications/:pubkey/:d_tag/sections= | POST | Load all sections for a publication   |
| =GET /api/v1/events/:id=                         | GET    | Single event by hex ID                 |
| =GET /api/v1/chat=                               | GET    | Current conversation state             |
| =POST /api/v1/chat/message=                      | POST   | Send message, receive LLM response     |
| =POST /api/v1/chat/edit=                         | POST   | Enter edit mode (collapse to buffer)   |
| =PUT /api/v1/chat/edit=                          | PUT    | Exit edit mode (re-parse buffer)       |
| =POST /api/v1/chat/system=                       | POST   | Set system prompt                      |
| =PUT /api/v1/chat/context=                       | PUT    | Replace all injected context notes     |
| =POST /api/v1/publish=                           | POST   | Build + sign + broadcast 30040/30041   |
| =POST /api/v1/ingest=                            | POST   | Stream-ingest events from JSONL upload |
| =GET /api/v1/export[/manifest]=                  | GET    | Export local DB as JSONL + manifest    |
| =POST /api/v1/import=                            | POST   | Parse PDF/DOCX/EPUB/HTML/TXT to events |
| =GET /api/v1/profile/:pubkey=                    | GET    | Profile lookup (local-first w/ relay) |
| =POST /api/v1/profiles/fetch=                    | POST   | Batch profile prefetch                 |
| =GET /api/v1/identity=                           | GET    | Current identity status (locked/open)  |
| =POST /api/v1/identity/login=                    | POST   | Decrypt ncryptsec, unlock signer       |
| =POST /api/v1/identity/lock=                     | POST   | Lock signer, clear in-memory key       |
| =GET/POST/DELETE /api/v1/ignore=                 | —      | Ignore list CRUD                       |
| =GET /api/v1/network/status=                     | GET    | Online/offline + active fetch tally    |
| =POST /api/v1/network/mode=                      | POST   | Toggle online/offline                  |
| =GET /api/v1/relays=                             | GET    | Resolved relay sets (general/fetch/pub)|
| =POST /api/v1/config/update=                     | POST   | Edit config.toml from UI               |
| =POST /api/v1/fetch[/authors|/sections]=         | POST   | Per-relay/per-author/per-section pulls |
| =GET /api/v1/embed/status=                       | GET    | Embedding sidecar + index health       |
| =POST /api/v1/embed/sync=, =/reindex=              | POST   | Sync new events / full reindex         |

** Svelte Web Frontend

The primary UI is a SvelteKit 2 / Svelte 5 application (~30 components, ~10K
LOC) with a three-panel workbench layout:

#+begin_example
┌─────────────────┬──────────────────────┬──────────────────┐
│   Chat          │   Document / Compose │   Search         │
│                 │                      │                  │
│ system prompt   │ ┌──────────────────┐ │ ┌─ Query ──────┐ │
│ context panel   │ │ = Publication    │ │ │ t:python     │ │
│                 │ ├──────────────────┤ │ └──────────────┘ │
│ [user] ...      │ │ == Section 1     │ │                  │
│ [assistant] ... │ │ (editable)       │ │ ☐ Result A       │
│ [user] ...      │ ├──────────────────┤ │ ☑ Result B       │
│                 │ │ == Section 2     │ │ ☐ Result C       │
│ [edit] [□→]     │ │ (from search,🔒) │ │                  │
│                 │ └──────────────────┘ │ [◂ context]      │
│ ┌────────────┐  │                      │ [□ compose]      │
│ │ input...   │  │ Full | Plain | Prev  │                  │
│ └────────────┘  │                      │                  │
└─────────────────┴──────────────────────┴──────────────────┘
#+end_example

Each panel collapses to a 32px vertical bar. The center panel (document /
compose) gets 2fr; side panels get 1fr each.

** TUI

A ratatui terminal interface provides the same capabilities through a
vim-flavored keybinding model: feed browsing, reader modes (tree, outline,
continuous, paginated), compose (structured and editor modes), command palette
(=M-x= / =Space=), identity management, and draft storage. The TUI was the
original interface and remains functional, though the web app has become the
primary development target for the workbench features.

* Key Design Decisions

** Unified Item Pool

The web frontend manages a single =ContextItem[]= pool. Each item has
=in_context= and =in_compose= boolean flags that route it to the appropriate
panel. Items are deduplicated by source event identity (=source_event_id= /
=source_addr=). Editing in one panel is immediately visible in the other since
both reference the same pool entry.

Cross-panel actions flip flags rather than cloning:
- =◂= (to context) sets =in_context=true=, =in_compose=false=
- =□= (to compose) sets =in_compose=true=, =in_context=false=
- Items with neither flag are garbage-collected from the pool

Each item carries independent =content= and =context_content= fields so that
compose edits and context edits can diverge. Badge colors indicate sync state:
green (matched), yellow (diverged), blue (readonly/locked).

** Markup-Agnostic Compose

The compose panel does not impose a markup language. Three editing modes are
available:

1. *Full mode* — structured section cards with title, tags, and content fields
2. *Plain mode* — a single textarea with configurable delimiters. The user
   chooses what character denotes a heading (=#=, =*=, ===, or custom). Tags are
   serialized as =:key: value= lines. The system parses structure from the
   delimiter and tag patterns, but the content itself can be Markdown, Org-mode,
   AsciiDoc, or anything else.
3. *Preview mode* — read-only rendering of the assembled document

Switching between modes is lossless. Plain mode parses back into sections;
full mode serializes into plain text. The delimiter is reactive — changing it
re-renders all headings immediately.

** Chat as a Structured Buffer

The LLM conversation is a sequence of typed fragments (system, user, assistant).
Each fragment is individually selectable, removable, and pushable to compose.

Edit mode collapses the entire conversation into a single text buffer with =---=
delimiters and =[role]= headers. The user can freely restructure, merge, split,
reorder, or rewrite fragments. Exiting edit mode re-parses the buffer back into
typed fragments. This makes the conversation a malleable editing surface rather
than an append-only log.

** Imported Content and Forking

When a search result is sent to compose, it arrives as a read-only reference.
The publication index will contain an =a=-tag pointing to the original event — no
new event is created. The publication /assembles/ existing content.

If the user wants to modify imported content, they explicitly fork it. Forking
creates a new kind 30041 event with:
- A new =d=-tag
- The user's pubkey as author
- =a= and =e= tags with ="fork"= marker (NIP-54 convention) for lineage
- Content copied from the original, now editable

This preserves attribution and makes the derivation chain traceable.

** LLM Provider Abstraction

The chat backend is provider-agnostic. An =LLMProvider= trait defines a single
=chat(messages) -> Result<String>= method. Two implementations exist:

- *ClaudeProvider* — calls the Anthropic Messages API (claude-sonnet-4-20250514
  default, configurable via =ANTHROPIC_MODEL= env var)
- *NoopProvider* — echo mode for testing when no API key is set

Provider selection happens at startup from environment variables. The web UI
shows animated loading dots during LLM responses. Context notes are injected
into the system message so the LLM sees them as reference material.

* What Has Been Built

The project has progressed through four implementation phases defined in the
workbench architecture document, plus foundational work predating the workbench
design.

** Foundation (Pre-Workbench)

| Feature                        | Description                                                    |
|--------------------------------+----------------------------------------------------------------|
| Nostr engine + nostrdb         | Event storage, relay I/O, NIP-01 query, fetch policies         |
| TUI application                | Feed, reader (4 view modes), compose, command palette          |
| Identity system                | npub/nsec/ncryptsec login, NIP-49 decryption, OS keyring       |
| User data fetching             | NIP-01 profile, NIP-02 contacts, NIP-51 lists, NIP-65 relays  |
| Draft storage                  | Local JSON drafts with sync status indicators                  |
| Editor compose mode            | Single-buffer vim-style editor with structure detection         |
| Content parser                 | Heading/tag/code-block detection for Markdown, Org, AsciiDoc   |
| Publication builder            | 30040/30041 event construction with local creation + signing   |
| JSON/structured preview        | Event inspection with syntax highlighting                      |

** Phase 1: Search + Query Engine (Complete)

- Structured query parser: =t:tag=, =k:kind=, =by:author=, ="exact text"=, bare
  keywords (AND), =~:semantic= (syntax ready, backend stub)
- Compiles to NIP-01 filters with text post-filtering over nostrdb content
- =POST /api/v1/search= endpoint
- Web search panel with result list, kind badges, tag pills, tag inspector
- Per-result checkboxes with select all / invert
- Per-result =◂= (to context) and =□= (to compose) actions
- Bulk actions on checked results

** Phase 2: Compose Block Model (Complete)

- =ComposeBlock= types: Editable, Imported (read-only reference), Forked (with
  lineage tags)
- Publication builder handles all three block types
- Block reordering, insertion, removal
- Three compose modes: full (structured cards), plain (delimiter-based
  textarea), preview (read-only)
- Configurable delimiter input for plain mode
- Tag serialization (=:name: value=, =:tags: a, b, c=)
- Per-section checkboxes across all modes
- Unified toolbar with =All=, =Inv=, =◂=, =□=, =▸=, =🗑= actions
- Two-step trash: first press removes from current panel, second press
  (with 10-second countdown and opacity fade) permanently deletes
- Modified detection (yellow highlight) with per-section reset

** Phase 3: Chat + LLM Integration (Complete)

- =ChatState= with typed fragments, selection, context injection
- =LLMProvider= trait + Claude provider (Anthropic Messages API) + noop fallback
- Full chat API: send, edit mode (collapse/expand), system prompt, context CRUD
- Web chat panel with fragment display, system prompt editor, context panel
- Per-fragment checkboxes with bulk =□= (to compose) and =▸= (publish) actions
- Unified =ContextItem= pool with =in_context= / =in_compose= routing flags
- Event identity deduplication (=source_event_id= / =source_addr=)
- Cross-panel editing is reactive (same pool item, shared state)
- Badge system showing origin, sync state, and cross-panel presence
- Loading indicator with animated dots during LLM response

** Phase 4: Chat Edit Mode + Fragment Flow (Complete)

- Edit mode: collapse all fragments into single buffer with =---= delimiters
  and =[role]= headers
- Re-parse on exit: split on =---=, detect role headers, infer from position
- Fragment → compose flow: checked messages push to compose as editable sections
- Chat fragments show compose badge when in compose panel, with edit indicator
  when content has diverged

** Phase 5: Feed, Lazy Loading, Publish, Semantic Search (Complete)

- Publication feed: local-first loading, relay sync, cursor-based pagination
- Lazy section loading: outline auto-loads all, paginated prefetches adjacent,
  continuous uses IntersectionObserver with read-ahead
- Progressive publication opening via TOC (includes nested 30040 sub-publications)
- Context-aware search: defaults to =k:30040= on feed view, =by:me= from config
- Publish API: =POST /api/v1/publish= creates 30040+30041 events from compose
  state, unsigned (draft) or signed (keyring), local ingest + optional relay
  broadcast
- Compose ▸ / Doc ▸ / Chat ▸ publish handlers wired
- OR/union queries with =|= operator: =k:30041 python | k:30040 by:me=
- Semantic search via embedding sidecar:
  - Python sidecar (sentence-transformers) for dev, Rust ONNX (=--features onnx=) for prod
  - usearch HNSW index (in-memory, persisted to disk as =vectors.idx= + =vectors.map=)
  - =~:concept=, =~:"multi word":k= syntax with text+semantic intersection
  - Over-fetch k×10 from HNSW when kind/author filters present, truncate after
  - Toolbar progress bar with polling during sync
  - Semantic score badges on search results
- =~:"phrase":k= syntax (quotes after prefix, consistent with other patterns)
- Config: =[embedding]= section (backend, model, dimensions), =[identity]= pubkey

** Phase 6: Operations, Identity, and Import (Complete)

Work since the Phase 5 milestone (commit 98c0ee4) has focused on making the
app survive real-world use: bigger libraries, real identities, real imports,
real network failures.

- *NetworkMode (online/offline)* with =FetchGuard= RAII tracking of in-flight
  relay requests; =FetchPolicy= clamps to =LocalOnly= when offline regardless
  of the caller's preference (=src/network.rs=)
- *ncryptsec login* with idle lock timer and auto-signing while unlocked
  (commit c22bec1)
- *Engine concurrency fix*: CPU-bound DB scans wrapped in =spawn_blocking= so
  the Tokio runtime stops freezing under load (commit 1d3752d)
- *Document import*: PDF, DOCX, EPUB, HTML, and TXT parsing into kind 30041
  sections, with per-page embeddings and split search results (commits
  69b01af, 7359251, 21440a4)
- *Streaming JSONL ingest* with duplicate skip, progress UI, and an export
  manifest for round-trip backup/restore (commits 0250959, a10c6d9)
- *Profile pipeline*: batch prefetch on every event fetch, reactive updates
  after batch completes, fall back to general relays when local-only misses
  (commits ef48970, 5793712, 76ca450, 36e5fa7)
- *Ignore list end-to-end*: filtered in search, semantic results, and the
  feed reactively; "ignored" view with unblock and purge (commits bff97a7,
  3566ce9, cd429c0, 4944bd1)
- *Relay sets restructured* into general / fetch / publish with per-set kind
  filters; editable from the UI (commits dae3e94, 35af34b, 2ae9a0e, 193e98a)
- *SPA route structure*: =/p/[pubkey]/[d_tag]=, =/compose=, =/profile=,
  =/ignored= with deep-linkable state (commits f0312d6, b45d95e)
- *Sidecar reliability*: persistent venv, =HF_HUB_OFFLINE=1=, and a
  =start.sh= that waits for sidecar health and detects embedding readiness
  before starting the engine (commits 14977ac, e1174ee, plus the start.sh
  series)
- *Publication query limit raised to 5000* for large libraries (commit 78f7165)
- *Search "everything" mode* that auto-routes 30040 hits to the feed and
  splits document-page results into a separate tab with badges (commits
  6f23a00, 21440a4)

** Not Yet Started

| Feature                        | Phase | Notes                                           |
|--------------------------------+-------+-------------------------------------------------|
| LLM tool calling               | 7     | LLM autonomously searches knowledge base         |
| Zettelize action               | 4     | Edit buffer → compose blocks (chat → zettel)     |
| TUI workbench panels           | Def.  | Deferred — web is primary                        |
| TUI draft flags                | Def.  | local-only / unsigned / signed / broadcast flags |
| Stability roadmap items        | —     | See =docs/stability-roadmap.org=                 |

* Technology Stack

| Layer     | Technology                                                         |
|-----------+--------------------------------------------------------------------|
| Backend   | Rust, Axum (HTTP), nostrdb (C via FFI), tokio (async)              |
| Frontend  | Svelte 5, SvelteKit 2, TypeScript, Vite 6                         |
| Database  | nostrdb (events), filesystem JSON (drafts), usearch HNSW (embeddings)    |
| Embedding | Python sidecar (sentence-transformers) or Rust ONNX (fastembed)          |
| LLM       | Anthropic Messages API (Claude), provider-trait abstraction        |
| Relay I/O | tungstenite WebSocket, NIP-01 protocol                             |
| TUI       | ratatui, crossterm                                                 |
| Identity  | NIP-49 ncryptsec (scrypt + XChaCha20-Poly1305), OS keyring        |

* Codebase

| Directory        | Contents                                        | Approx. Size  |
|------------------+-------------------------------------------------+---------------|
| =src/=             | Rust backend                                    | ~13,000 lines |
| =src/tree/=        | TUI state, commands, widgets                    | ~6,000 lines  |
| =web/src/=         | Svelte frontend (~30 components + lib)          | ~10,000 lines |
| =docs/=            | Architecture docs (org-mode)                    | ~3,000 lines  |
| =tests/=           | Integration tests (search, query, engine)       | ~800 lines    |

100 commits from initial commit to current HEAD (5c92b81). All co-authored
with Claude.

* Summary

The project delivers a working three-panel composition workbench where a user
can search their Nostr knowledge base, discuss findings with an LLM, and
assemble publications from a mix of original writing, imported references, and
AI-generated content — all without leaving a single interface. The markup-
agnostic design means writers can use whatever syntax they prefer. The
read-only-by-default import model with explicit forking preserves attribution
across the Nostr network. Six implementation phases are complete (search,
compose blocks, chat, edit mode, feed/publish/semantic, ops/identity/import).
Remaining major work: LLM tool-calling and the stability items in
=stability-roadmap.org=.
