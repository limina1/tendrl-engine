#+TITLE: Tendrl Engine — Features
#+AUTHOR: tendrl-engine contributors
#+DATE: 2026-05-06
#+STATUS: CURRENT — reflects implemented state as of commit 0b98195

* Scope

Tendrl is a local-first Rust engine for Nostr publications (NKBIP-01) with a
web frontend and an HTTP API:

- *Web app* (SvelteKit, primary) — three-panel workbench at
  =http://localhost:3030= when the engine is running
- *HTTP API* (Axum, =/api/v1/*=) — drives the web UI and is the integration
  surface for the planned Emacs and Nvim frontends

(A ratatui TUI was the original interface; it has since been removed — the web
app is the only live frontend.)

Both sit on the same engine: nostrdb storage, NIP-01 relay client,
structured search, embedding sidecar, identity manager, and publication
builder.

* Core Engine

** nostrdb Integration
- Local LMDB-backed event storage via the nostrdb C library
- Events are cached for offline use and millisecond-time queries
- Author-scoped queries to keep large libraries snappy
- CPU-bound scans run on =spawn_blocking= so the async runtime stays responsive

** Relay Connections
- NIP-01 WebSocket client; multiple relays in parallel; deduplicates results
- Local relay auto-detection at =ws://localhost:3334=
- Structured *relay sets*:
  - =[relay.general]= — profile, contacts, NIP-65 relay lists
  - =[relay.fetch]= — publication content (kinds 30040/30041, articles, wikis)
  - =[relay.publish]= — where signed events get pushed
- Each set has its own URL list and kind filter; editable from the web UI via
  =POST /api/v1/config/update=

** Fetch Policies
- =LocalOnly= — nostrdb only (instant, offline)
- =LocalFirst= — local then relay backfill if under limit (default)
- =FetchAlways= — always hit relays, merge with local

** Network Mode (Online / Offline)
- Single =NetworkMode= toggle clamps any policy to =LocalOnly= when offline
- =FetchGuard= RAII tracks in-flight relay requests so the UI shows live
  activity (=GET /api/v1/network/status=)
- Toggling online → offline cancels nothing but stops new fetches

** NKBIP-01 Support
- Full kind 30040 (publication index) and kind 30041 (section) handling
- Defensive validation: rejects malformed events (e.g. 30040 with non-empty
  =content=)
- Address-based identity (=kind:pubkey:d_tag=) for replaceable events
- Versioning preserved (=load_section_versions=)

** Structured Search
- Query language: =k:30041 by:npub1... t:python "exact phrase" ~semantic= and
  =|= for OR/union
- Compiles to NIP-01 filters with text post-filtering over nostrdb content
- Semantic search via embedding sidecar; over-fetches k×10 from HNSW when
  kind/author filters are present
- Search results split by type: publications routed to the feed, document
  pages into their own tab with badges

** Embedding Sidecar
- Two backends:
  - *Python sidecar* (sentence-transformers / Flask) — default, dev-friendly,
    runs at =http://localhost:3031= via =start.sh=
  - *Rust ONNX* (=--features onnx=) — in-process, single-binary deployment
- usearch HNSW index, persisted to =vectors.idx= + =vectors.map=
- =POST /api/v1/embed/sync= for incremental, =/reindex= for full
- =GET /api/v1/embed/status= for health + index size
- Reliability fixes: persistent venv, =HF_HUB_OFFLINE=1= so the sidecar
  doesn't block on Hugging Face Hub on startup

** Document Import
- Parsers for PDF, DOCX, EPUB, HTML, TXT (=POST /api/v1/import=)
- Each document becomes a kind 30041 with per-page chunks for embedding
- Streaming JSONL ingest (=POST /api/v1/ingest=) with duplicate skip and
  progress UI
- JSONL export (=GET /api/v1/export=) plus an export manifest endpoint for
  round-trip backup/restore

** Publishing
- =POST /api/v1/publish= builds + *signs* 30040 + 30041 events from compose
  state and ingests them. The db only ever holds *signed snapshots* — the
  passive unsigned-event path was removed; =sign:false= is rejected.
- Three separable steps:
  - *Save draft* (no identity) — unsigned working state in =<data_dir>/drafts/=
    (=POST /api/v1/drafts=), survives refresh, resumable.
  - *Sign* (identity required) — a signed local snapshot in the db. The
    signature *is* the snapshot; nostrdb keeps every version. Re-signing a
    same-title publication reuses its nanoid d-tag (republish-diff) so it's a
    version update, not a fork.
  - *Broadcast* — push a signed snapshot to the publish relays
    (=POST /api/v1/publications/:pubkey/:d_tag/broadcast=), one op for the whole
    publication. A =LocalPublicationTracker= marks publications local vs.
    published; the feed renders a "local" pill until broadcast.
- Local ingest of signed events for instant feedback in the UI

* Identity

** Supported Key Formats
- *npub* — public key only (read-only)
- *nsec* — unencrypted secret (rare; not recommended)
- *ncryptsec* — NIP-49 encrypted secret (scrypt + XChaCha20-Poly1305), the
  default flow

** Login & Lock
- Web: ncryptsec entry modal; password decrypts in-memory, never persisted
- *Idle lock timer*: signer auto-locks after configurable inactivity
- *Auto-signing while unlocked*: publish flows don't re-prompt for password
  during a session
- =GET /api/v1/identity= returns status (none / locked / unlocked); =/login=
  and =/lock= flip state

** Storage
- OS keyring integration for the encrypted blob (not the plaintext key)
- TUI also supports session restoration on startup via the keyring

* Web App

** WM Shell (tiling-window-manager UI)
=web/src/routes/+page.svelte= owns the shell root; full spec in
=docs/wm-shell.md=. The legacy three-panel workbench has been
retired — the shell is now the only frontend layout.

- *Class-typed slots*: =chat= (left), =work= (center), =research=
  (right). Each slot is =open= or =rail= (32px collapsed bar).
- *Buffers* with stable IDs: =reader:30040:<pk>:<dtag>=,
  =reader:event:<id>= (standalone-section reader, paginated view),
  =composer:current=, =draft-reader:current=, =profile:<pk>=, plus
  singletons (=chat=, =feed=, =search:default=, =ignored=, =settings=,
  =refs=, =kb=).
- *Splits*: =SPC w s= picks another same-class buffer and splits the
  focused leaf horizontally; =j= / =k= cycles between leaves.
- *Single base layout* (chat rail / work center / research rail);
  collapse / expand individual slots to reshape on the fly. The
  earlier =chat= preset and named-layout list are gone — user-savable
  perspectives are deferred.
- *Buffer kill* (=SPC b k=): removes from the open list, pushes onto
  recently-closed (recall via =SPC b r=, cap 20), replaces the focused
  leaf with another same-class buffer or the class default singleton.
- *Leader popup* (=SPC=) — Doom-style which-key with descend / up /
  cancel; resolves to engine commands or further prefixes.
- *M-x palette* — fuzzy-searchable command list keyed off =:= or the
  M-x button in the header.
- *Settings buffer* (=settings= kind) — opens via =SPC s s=,
  =M-x tendrl-open-settings=, or the header settings button. Editor
  options (line numbers, vim mode, insert-from-search behavior) and
  compose options (default mode, sync mode, button labels) live here.
- *Modeline* shows layout name, focused class, focused buffer with
  modified marker, network mode, identity badge.

** Modal Navigation
- *Two modes*: normal (vim-style nav) and insert (free typing). The
  modeline shows =-- NORMAL --= or =-- INSERT --=.
- *Editable focus = implicit insert*: focus a textarea / input /
  contenteditable and the shell auto-flips to insert. Esc / =C-[= /
  =C-g= blurs and returns to normal.
- *Ranger nav* across feed, reader (outline / paginated), search, and
  compose: =j= / =k= / arrows walk a per-buffer cursor with
  bounds-checked scroll (the scrollbar only moves when the cursor
  would leave the visible rows).
- *=gg= / =G=* — first / last cursor row (or scroll-top / -bottom in
  reader continuous mode). =gg= uses a 700ms pending window.
- *=h= / =l= drill axis*: in reader cycles outline → paginated →
  continuous; in compose toggles full ↔ plain. Outline =l= is special
  — drills into paginated with the cursored section loaded.
- *=i= / =o=* enters insert. Buffers can target specific inputs via
  =dispatchNav('insert')=; otherwise the shell focuses the first
  =[data-entry]= element in the focused slot.
- *Slot navigation* under =SPC w=: =SPC w h= / =l= focuses the
  prev/next slot, =SPC w c= toggles rail/open, =SPC w s= splits.
- *Vim search loop* in the search bar: type query → =Enter= commits
  and exits insert → =j= / =k= walks results → =Enter= or =l= opens →
  =i= re-enters the field. Mirrors vim's =/=-search semantics.

** Routes
The shell mounts at =/=. Legacy per-route pages (=/p/=, =/compose=,
=/profile/=, =/ignored=) have been deleted; their functionality lives
inside the corresponding buffer renderer. Deep-link routing
(=/p/<pubkey>/<dtag>= spawning the right buffer) is on the roadmap.

** Feed
- Publication cards with title, author, summary, section count
- =Sync all= pulls fresh data from fetch relays
- Per-item hamburger menu: open, copy address, ignore
- Pagination raised to 5000 items for large libraries

** Reader
- Three view modes cycled with =h= / =l=: outline (cursored TOC,
  shows titles + provenance borders), paginated (one section at a
  time with prev/next bar), continuous (full doc, page-scroll on
  =j= / =k=).
- Eager-loads all sections after the TOC arrives — outline shows
  titles immediately, content fills in as each =GET /sections/i=
  resolves. Continuous's IntersectionObserver still drives loads
  for very long publications.
- Paginated prefetches adjacent sections on navigate.
- =Edit= / =Edit §= buttons import the publication (or just the
  cursored section) into the compose pool and open the composer.
- *Always pristine* — opening a published 30040 from the feed
  always shows the engine's view, even if the user has a draft
  seeded from the same publication. Draft preview lives in its own
  =draft-reader:current= buffer, opened via composer's =Read=
  button.
- *Standalone-section reader*: =reader:event:<id>= synthesizes a
  one-section publication from any event id and forces paginated
  view — used by the search action modal's "Read section" path so
  a 30041 result opens just that section, no parent walk.
- TOC handles nested 30040 sub-publications.

** Compose
- Block model: editable, imported (read-only reference), forked (with =a/e=
  lineage tags, NIP-54 marker)
- Two editing modes (default lives in settings; =h= / =l= in normal
  mode toggles): full (structured cards with collapsible header +
  sections), plain (single CodeMirror 6 buffer over the delimited
  document). The visible Full/Plain toggle is gone now that the
  default is a setting; mode transitions run from a =$effect= on the
  bindable mode prop. Read view ships as its own =draft-reader=
  buffer.
- *Plain mode runs CodeMirror 6 + vim* (=@replit/codemirror-vim=).
  Double-Esc stack: first Esc — vim insert→normal (handled by the
  plugin); second Esc while already normal — blurs the editor and
  returns the shell to normal mode. CM6 is attached via a Svelte
  =use:= action so the editor instance is stable for the buffer
  lifetime regardless of upstream re-renders. Line numbers and vim
  mode each live in a Compartment so settings toggles reconfigure
  live without losing cursor or undo history.
- *Ranger nav over sections* in full mode: =j= / =k= / =gg= / =G=
  walk a cursor across section blocks with the same inset-bar +
  tinted-bg highlight as feed/reader/search; =Enter= / =i= focuses
  the cursored section's textarea.
- *Reorder controls*: ↑ / ↓ buttons in each section's header in
  full mode (calls =reorderComposeSection=), and on each detected
  section row in plain mode (parse → swap → reserialize the
  =plainText=, no pool round-trip).
- *Default-locked sections*: =+ Section= and search-Insert both
  create =readonly: true= entries — unlock-to-edit matches the
  transclusion model. Bulk =Unlock all= / =Lock all= still available.
- Configurable delimiter for plain mode (=#=, =*=, ===, custom)
- Tag serialization: =:name: value=, =:tags: a, b, c=
- Bulk actions across sections: =All=, =Inv=, =◂= (to context),
  =▸= (publish), =🗑= (two-step trash with countdown)
- *Unlock all* / *Lock all* bulk buttons mirror the reader's lock
  affordance, available from the compose mode bar
- Modified detection (yellow highlight) with per-section reset

** Chat
- Typed fragments (system, user, assistant), individually selectable
- Edit mode collapses everything into a single buffer with =---= delimiters
  and =[role]= headers, freely editable, re-parses on exit
- =LLMProvider= trait with Claude (Anthropic Messages API) and noop backends
- Context notes injected into system message for the LLM

** Search Panel
- Keyword + tag + kind + author filters
- =~semantic= prefix for HNSW search; semantic score badges on results
- Per-result checkboxes, select all / invert
- *Action modal* on Enter or row click — opens a three-action chooser
  (=r= Read section/publication, =f= Find containing publications,
  =i= Insert into compose). =◂= chat button retained for quick send;
  =□= compose button dropped (Insert covers it).
- *Find containing publications* runs an =a:KIND:PUBKEY:DTAG= query
  against nostrdb (translated to the =#a= tag filter) — opt-out from
  =handleSearch='s default =by:me= scoping so cross-author parents
  surface. Force-focuses the search slot when results land.
- *Insert into compose* respects the =editorInsertMode= setting:
  =cursor= dispatches a CM6 insert at the active plain-mode caret;
  =append= writes to end of doc or appends to the section pool.
  Inserted blocks use a default-locked =import= origin and an ====
  heading so the plain parser sees them as sections.

** Profiles
- Local-first profile lookup; falls back to general relays on miss
- Batch prefetch on every event fetch, so usernames resolve without UI churn
- =ProfileView= is a full route with profile metadata + that author's works
- =ProfileName= component used everywhere; reactive across batch updates

** Ignore List
- Identifier: =kind:pubkey:d_tag= so it survives event re-fetches
- Filtered in feed, search, and semantic results
- =/ignored= page lists everything blocked, with unblock + permanent purge

** Settings & Operations
- Relay sets editable from the UI
- Per-relay / per-author / per-section manual fetch buttons
- Reindex button + live event-count indicator for the embedding sidecar
- Network-mode toggle visible in the header

* Reader View Modes (formerly the TUI)

The ratatui TUI was the original frontend and has since been removed; the web
app is the only live frontend. The reader view modes it introduced (below) now
live in the web reader, and remain the model for the planned emacs/nvim
frontends.

** Reader View Modes
| Mode       | Description                    | Primary Navigation     |
|------------+--------------------------------+------------------------|
| Tree       | Hierarchical expand/collapse   | j/k nav, h/l fold      |
| Outline    | Sections displayed as cards    | j/k nav, preview panel |
| Continuous | Scrollable full content        | j/k scroll             |
| Paginated  | One section at a time          | j/k scroll, J/K section|

Cycle modes with =v=.

** Vim-Style Keybindings
| Key     | Action                                    |
|---------+-------------------------------------------|
| j/k     | Navigate down/up or scroll                |
| J/K     | Next/prev section (Paginated) or move     |
| h/l     | Collapse/expand (Tree mode)               |
| Enter   | Open publication / expand / load          |
| Esc     | Back to feed                              |
| v       | Cycle view mode                           |
| Tab     | Toggle preview panel                      |
| q       | Quit                                      |
| c       | Compose new publication                   |
| i       | Login dialog                              |
| :       | Relay configuration dialog                |
| ?       | Command palette                           |
| Ctrl+d  | Save current as draft                     |
| Ctrl+u  | Filter to show only drafts                |
| Ctrl+t  | Toggle tag editing (compose)              |
| Ctrl+p  | Toggle JSON event preview                 |

** Command Palette
- =?= or =Alt+x= (or =Space= in Doom style)
- Categorized commands (Navigation, Manipulation, View, Compose, etc.) with
  fuzzy search and inline keybinding hints

** Sync Status Indicators
| Status    | Color  | Meaning                     |
|-----------+--------+-----------------------------|
| Remote    | Cyan   | Fetched from relay          |
| LocalOnly | Yellow | Only in local database      |
| Draft     | Red    | Unsigned local draft        |

** Draft Storage
- Unsigned 30040/30041 events stored as JSON in =<data_dir>/drafts/=
- Drafts auto-load on startup, displayed in feed with a red sync bar

* Operations

** start.sh
=./start.sh= launches sidecar + engine; =--dev= also starts the SvelteKit dev
server. Waits for sidecar health, detects embedding readiness, applies
=HF_HUB_OFFLINE= so it doesn't hang on a missing network.

** Purge & Reingest
- =POST /api/v1/purge= clears local data
- TUI =--purge-db -y= flag for a clean slate
- Combined with JSONL export, gives a backup → wipe → reingest loop

** Claude Code Sessions
- =GET /api/v1/claude-sessions= lists local Claude Code session JSON
- Useful for surfacing prior conversations as context inside the workbench

* Known Limitations

** nostrdb Deletion
nostrdb does not support deleting individual events. Workarounds:
- Session-only ignore (events stay in DB, UI hides them)
- NIP-09 deletion requests (network signal only)
- Full purge + selective reingest from JSONL backup

See =docs/nostrdb-deletion-architecture.org=.

** Bulk Ingest Throughput
=sync_embeddings()= per-event can overwhelm the sidecar around ~350 events.
Tracked separately; batching is the planned fix.

** ncryptsec Compatibility
NIP-49 decryption works against the official test vectors but may show edge
cases with some external implementations.

* Running the Application

#+begin_src bash
# Full stack (sidecar + engine)
./start.sh

# Full stack + web dev server
./start.sh --dev

# Engine only
cargo run -- -c config.toml

# Tests
cargo test
#+end_src
