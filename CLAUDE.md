# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run Commands

```bash
# Build
cargo build                         # debug build
cargo build --release               # release build (in-process ONNX embeddings built in)
scripts/build-bundle.sh             # single-exe: SPA embedded, opens browser on run
scripts/build-portable.sh           # release artifact: manylinux glibc-2.28 floor, static onnxruntime, rustls — runs everywhere

# Releases (Cargo.toml [package] version is the single source of the release number)
scripts/build-portable.sh --bump patch   # bump (major|minor|patch|X.Y.Z), regen CHANGELOG, then build — same flag on build-bundle.sh
scripts/bump-version.sh minor            # bump the version only (also syncs Cargo.lock); prints current version with no args
scripts/release-notes.sh                 # prepend a CHANGELOG.md section: commits since the last v* tag (--print to preview)
scripts/build-android.sh                 # signed release APK (arm64) → target/android/ — bump syncs the mobile version;
                                         # one-time keystore setup: scripts/build-android.sh --init-signing
# then: review the diff, commit, and publish:
scripts/publish-release.sh               # tag v<version> if needed, push the tag, create the GitHub release
                                         # (CHANGELOG section = notes, portable tarball = asset; --dry-run to preview,
                                         #  --no-asset for notes-only, `publish-release.sh X.Y.Z --no-asset` to backfill an old tag)

# Run (no config file needed — defaults + <data_dir>/config.toml, written by the UI)
cargo run                           # engine on :3030 with defaults
cargo run -- -c config.toml         # run engine with a preset config
./target/debug/tendrl-engine --data-dir /tmp/tendrl-test --port 3131   # isolated test instance (first-run state); never reuse the live :3030
./start.sh                          # start all services (engine + web preview)
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
pnpm -C web test                    # vitest: rendering-layer tokenizers + segment invariants

# Dependency updates (cargo + pnpm + git-pinned nostrdb)
make                                # report outdated deps — changes nothing
make update                         # safe updates within version ranges
```

## Architecture

**tendrl-engine** is the engine of a local-first **reader, writer, and reference
manager** built on Nostr; it implements NKBIP-01 structured publications (indexes
and sections), structured + semantic search, and the nostrdown reference layer.
It runs as two cooperating processes: the Rust **engine**
(`src/`, port 3030) and a SvelteKit **web frontend** (`web/`, port 5173 dev / 5174
preview). The engine owns all data access; embeddings and document text
extraction both run in-process (no external services). (A ratatui TUI was the
original frontend; it has been
removed — there is no `ratatui`/`crossterm` dependency and a single `[[bin]]`
(`tendrl-engine`; the package/lib are still named `nostr-engine`/`nostr_engine` so
existing installs aren't orphaned). The web app is the only live frontend; the
engine-owns-data architecture keeps the door open to other frontends.)

### Frontend/backend boundary (the governing rule)
Keep this split when adding features — it's what makes the engine portable to any
future frontend without re-implementing logic per-frontend:

- **Rust owns** fetching, event storage/query, and **all algorithmic derivation of
  structured data from events**: parsing/classification, document-tree assembly +
  ordering, content-format detection, NIP-19/NIP-11 decode+encode, kind-routing, event
  dedup/merge, slug/d-tag/coordinate generation, publish-payload emission, NIP-22
  threading, NIP-84 highlight resolution, kind-0 author resolution.
- **TypeScript (`web/`) owns** rendering and **ephemeral view/interaction state**
  (focus, expansion set, scroll, selection, active view mode) — and calls Rust over
  HTTP/SSE.

When the same event-derivation logic appears in both Rust and TS, resolve it **toward
Rust** (expose/wire the Rust, delete the TS twin) — not the reverse. Document
*extraction* (PDF/DOCX/EPUB → text) now runs in-process in Rust (`src/document.rs`,
pure-Rust crates — no native libs), as does *classification* (text → structured
sections/kinds). The cross-language audit that drove this split has been completed and
its findings folded into the code; the boundary is now enforced (the TS twins were
deleted as the Rust transforms landed).

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

**Session relay overrides** (`SessionRelays`, in memory — the session is the
engine process, i.e. one app launch on Android): a confirm decision is folded
in whole — listed relays left unpicked go `off` (skipped by *every* later
fetch, Auto and Confirm, background included, via the `tracked_fetch` funnel),
relays picked but not listed go `on` (joined to every later user-initiated
fetch). `begin_operation` applies them in Auto and lists them in Confirm
(`Intent.session_off` = start unchecked). `POST /network/session/reset`
clears them; `NetworkStatus.session` shows them. Durable config is relay
management only — the modal never writes `relays.json`.

**Local-first, reactive (the display rule)**: a view renders from its local
query and re-reads when the engine announces `FetchEvent::Ingested { kinds,
count, profiles }` on the fetch-events stream (emitted from
`Engine::note_ingest_batch`, the single ingest funnel — relay fetches and local
saves alike). Web: `$lib/network/ingest.ts` (`onIngested` / `onKindsIngested`,
a leaf module). Relay-bound work that fires on its own must **not** hold its
HTTP request open — a browser has ~6 connections per origin, two of them
permanent SSE streams, and held requests queue every local read behind slow
relays. `POST /profiles/fetch` without `force` detaches (returns at once);
feed, timeline picker, bookshelf and the profile cache react to ingests.
Still request-held (user-initiated): feed sync, bookshelf sync, discussions,
wiki resolve, pull-user-data — convert them the same way when touched.

Pending intents and live fetch activity are pushed to clients over an SSE
`fetch-events` stream; the UI approves an intent via a confirm endpoint. The
`FetchGuard` RAII type tracks active relay fetches for the activity log.
One exception: the **provenance round** (`begin_operation_ungated`) — a
fetch aimed only at relays a document was already served from — emits
its intent with `needs_confirmation: false` and never blocks, in either
mode; session-off relays still apply.

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
**Status: pure core only.** This module was the navigation engine for the (now-removed)
ratatui TUI; the dead command/state/navigation half was deleted in the Phase 3 boundary
cleanup. What remains is the frontend-agnostic, IO-free core kept
as the source of truth for any future frontend (web today; others via the API):

- **`parser.rs`**: line-by-line classification for compose (headings/attributes/code
  blocks → which event kind 30040/30041 a line generates). Pure function, no UI state.
- (No content-format *detection* here any more: a section's markup is whatever its
  `m` tag declares — `src/mimetype.rs` — never sniffed from the text.)
- **`node.rs`**: `TreeNode`/`NodeId` structure + accessors. Pure data.

The **compose payload structs** (`ComposeState`, `SectionCompose`, `TagEntry`,
`BlockKind`, `ComposeBlock`, `ComposeBlockState`) — which feed slug/coordinate/payload
emission — now live in **`src/publication/compose.rs`** (`crate::publication::compose`),
next to the publishing code that consumes them, not in the tree module.

What was deleted: `TreeState`/`TreeEngine` + the `CommandResult::NeedsAsync` async
boundary, `command.rs` (`TreeCommand`), `render.rs` (`TreeRenderer`/`VisibleNode`
flatten), `ViewMode`/window/dialog/palette/clipboard view-state, and the never-wired
`undo.rs` (`UndoStack`). All held view/interaction state that, per the
frontend/backend boundary above, belongs in the frontend — and the web owns its own.

Note the live publication tree is **loaded/assembled by `publication.rs`** (the
recursive depth-N loader + `build_toc` + the `stream_publication_tree` SSE stream) —
and this is correctly engine-side: `PubLoadEvent::Index` ships
`depth` + an ordered `children` list, parent-before-child, so `a`-tag resolution,
ordering, dedup, and the in-horizon walk all run in Rust. The web (`ReaderBuffer.svelte`)
re-accumulates those events into an addr-keyed map (mostly to track per-node load status
and flatten to an outline), with collapse/expand kept as frontend view state. That
re-accumulation is a thin twin — an optional refinement (stream flattened TOC rows to
drop it), not a duplicated algorithm.

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
- **`stats.rs`**: Local-DB inventory (`GET /api/v1/stats/inventory`) — totals,
  kind/author/relay histograms, archive span, disk cost. **Walks per kind on
  purpose**: a filter with no ids/kinds/authors/tags routes nostrdb to its
  created-at plan, whose 32-byte start key is truncated (`{0xFF}` sets one
  byte), silently skipping ~0.4% of notes. The kind index has no such
  truncation, so the cheap walk only *enumerates* kinds and each is then
  recounted exactly. `count:` scans get the same treatment via
  `Engine::known_kinds` (5-min cache); other whole-DB scans (keyword, `has:`)
  still take the ~0.4% hit deliberately — see `docs/commands.org`
- **`embedding.rs`**: HNSW vector index (usearch) with in-process ONNX embeddings
  (fastembed); the model loads lazily on first embed and is cached next to the index
- **`document.rs`**: in-process document text extraction (PDF via `pdf-extract`, DOCX
  via `zip`+`quick-xml`, EPUB via `epub`, HTML via `html2text`, plus the plain-text
  family) — replaces the former Python sidecar `/parse`; emits ordered `{title,
  content}` pages
- **`config.rs`**: TOML config — `initial_relays` (bootstrap seed only),
  `timeout_ms`, authors, embedding, identity. The live `general`/`fetch`/`publish`
  URL sets are populated at runtime from `relay_store.rs`, not from TOML.
- **`identity.rs`**: Nostr key management — ncryptsec decryption, keyring storage, NIP-49
- **`signing.rs`**: Pluggable `Signer` trait + `InProcessSigner`. The engine is the
  signing orchestrator; callers turn an `EventTemplate` into a `SignedEvent` without
  knowing the key source (engine ncryptsec, NIP-07, future NIP-46).
- **`mimetype.rs`**: the content-format contract, pure. `canonical()` (alias table:
  `text/x-…` spellings, short names, legacy `format` values → the four canonical
  mimetypes; `application/json` and other non-text → `None`), `from_tags` /
  `from_event` (`m` first, `format` fallback, never sniffs). Publishing declares a
  format at the compose-state boundary (`ComposeState::stamp_section_mimetypes` /
  `ComposeBlockState::stamp_block_mimetypes` — every 30041 gets `["m", …]`, indexes
  never; `PublishRequest.format` is required → 400 otherwise). The publication
  stream (`PubLoadEvent::Index`/`Leaf`) ships `format` (own), `effective_format`
  (own → nearest enclosing `text/*` index → none), `format_source`, `format_from`.
- **`nip11.rs`**: NIP-11 relay information document fetch + cache (process-wide,
  normalized URL keys, 1-hour TTL)
- **`nip42.rs`**: NIP-42 relay auth, pure: the kind-22242 `auth_template`,
  `RelayMessage` (typed parse of `AUTH`/`CLOSED`/`OK`/`EVENT`/`EOSE`/`NOTICE`)
  and `Refusal::classify` (`auth-required:` / `restricted:`). The handshake
  runs in `relay.rs` (`RelayConn`); policy is `Engine::relay_auth()` — the
  user's per-relay **opt-in `auth` set** in `relays.json` plus
  `signing::AuthSigner`, a capability that can sign *only* 22242 as the
  **user's** active source (engine key / NIP-07 / NIP-55; never the
  assistant). A relay in the set is answered silently; one outside it has
  its challenge ignored and its refusal surfaced as
  `RelayStatusValue::AuthRequired` / `PublishResult.refusal` — status only,
  nothing prompts. AUTH is the third signing purpose next to snapshot and
  broadcast: it produces no stored event. Publish opens one socket per
  relay per batch so a batch costs one AUTH (`publish_batch`).
  Plan/decisions: `docs/nip42-relay-auth-plan.org`.
- **`nip19.rs`**: bech32 TLV decoders for `nevent`/`naddr`/`nprofile` (npub/nsec live
  in `identity.rs`); unified `decode()` returns a tagged enum for the API
- **`nostrdown.rs`**: pure, IO-free tokenizer for the `{{ }}` inline-reference
  layer (see `docs/nostrdown.org`). Scans content for
  `{{ref|wiki|embed|quote|slot:target#fragment|display}}` (the Nostr-*event*
  layer) **and** `[[topic]]` / `[[d-tag][display]]` / `[[topic|alias]]` (the
  de-facto Nostr *wikilink* → a `wiki` ref) — but a `[[ ]]` whose target is a
  markup-native link/image/path (`scheme:`, `://`, `foo.png`, …) is skipped
  (`is_markup_link_target`) so it never overrides Markdown/Org/AsciiDoc's own
  links. Returns typed `NostrdownRef`s with byte offsets + NIP-54 slug normalization.
  Resolution is engine-side (`PublicationEngine::resolve_refs` →
  `POST /api/v1/nostrdown/resolve`): `ref:`→**sibling-only** by title-slug (d-tags
  are opaque nanoids, so the human slug matches the `T` tag/normalized title;
  an naddr/coordinate ref resolves iff it addresses a sibling),
  `wiki:`→sibling first, else 30818 by `d` ONLY (the 30040/30041
  `T`-title-slug fallback was removed 2026-08-12 —
  `docs/zettel/decision-wikilinks-30818-only.org`),
  `embed:`→naddr/coordinate/sibling transclusion. A `kind:pubkey:d-tag`
  coordinate is admissible wherever an entity is (canonicalized to its naddr,
  never slug-normalized). The web (`RichContent.svelte`) merges resolved spans with NIP-84
  highlight spans onto one overlay — the same parse/resolve/render split as
  highlights. Rendering-layer style spans (`web/src/lib/layers/`, default-on
  decorated-source bases for Markdown / Org / AsciiDoc keyed by the section's
  `m` tag) are applied as a post-pass over that merge, never inside it, so refs
  keep their offsets — a `[[topic]]` is nostrdown's atom first, Org/AsciiDoc
  link styling second. Publish emits `["w", topic]` tags (single-letter, relay-indexable)
  for `{{wiki:}}` / `[[ ]]` refs, plus `ref`/`a`/`q`/`p` tags per reference kind
  (`reference_tags()`).
- **`user_data.rs`**: NIP-01/02/51/65 profile data — parses kind 0/3/10000/10002/
  10003/10006/10007/30002 from nostrdb `Note`s. **Partially wired**: `Metadata`
  (kind 0) is the single source of truth for the profile endpoint + profile search
  (`api.rs::profile_from_event`, `query.rs::find_profiles_matching`). The relay-list
  kinds (10002/10007/30002) are still parsed client-side in `RelaysBuffer.svelte`
  because the hard part (NIP-44 decrypt of private lists) is signer-dependent —
  under NIP-07 the key lives in the browser extension, so that decrypt is rightly
  client-side, not Rust. The follow/mute/bookmark parsers (3/10000/10003/10006)
  remain dormant — no consumer UI yet; wire them when there is one, don't build
  UI-less plumbing.
- **`bookshelf.rs`**: kind-30045 bookshelves (format from
  `reference/bookshelf-app`): addressable, d-tag `my-book-collection` by
  default plus named shelves under other d-tags, `a` tags →
  `30040:pubkey:d` (+ relay hint, event-id hint), newest per d-tag wins.
  Pure parser + `to_tags`; `BookshelfEngine::load` / `load_all` resolve
  books against the store (a `fetch_always` is Confirm-gated and backfills
  missing indexes). API: `GET /api/v1/bookshelf` (`?policy=&shelf=&pubkey=`),
  `GET /bookshelf/all`, `POST /bookshelf/template` (add/remove/create →
  unsigned template, engine slugs the d-tag) → sign → `POST /bookshelf/save`
  (ingest, optional broadcast, local-until-broadcast on the coordinate).
  Web: feed bookshelf mode, `EventViewModal` Shelve action, profile
  Bookshelves tab.
- **`history.rs`**: reading / listening history — `<data_dir>/history.json`,
  one entry per document coordinate (30040 with the last section shown,
  30023, 30818; a 30041 only inside its publication), plus the narration
  place (section, recording id, piece, position). Local state only, never
  an event. API: `GET/POST /api/v1/history`, `POST /history/remove`
  (`coord:"*"` clears). Web: `history.svelte.ts` (debounced reads,
  throttled listens, optimistic mirror), reader resume, feed *history* tab.
- **`attached.rs`**: what hangs off a document — NIP-22 comments, NIP-84
  highlights, kind-32176 narration indexes and leaves, the containing
  30040 — bucketed per target (`classify`, pure). `GET /api/v1/attached?
  coord=|id=&policy=`: `local_first` runs the **provenance round**
  (`Engine::provenance_for` = the index's + its sections' relay
  provenance; `Engine::provenance_round` → `NetworkActivity::
  begin_operation_ungated`, an intent with `needs_confirmation: false`
  that never blocks — the relays a document came from already know the
  reader has it); `fetch_always` is the gated widening round. Narration
  discovery runs the same round first. Web: the event menu's publication
  row + coloured chips, `NarrationStrip` under every chapter / article
  with audio, `AudiobookBuffer` (`audiobook:<pk>:<d>`, a 32176 opened
  when its document is not local), profile Narrations / Audio tabs.
  Decision: `docs/zettel/decision-provenance-first-fetch.org`.
- **`git.rs`**: NIP-34 git (`nips/34.md`) — kind-30617 repository
  announcements (name/description, multi-valued `web`/`clone`/`relays`/
  `maintainers`, `r … euc`, `u` fork pointer, `t`), kind-30618 state
  (`HEAD` + `refs/*`), kind-1621 issues (`a` → repo coordinate, `subject`,
  `t` labels), kinds 1630–1633 status. Pure parsers + `issue_template` /
  `status_template`; status resolution per spec (newest by the issue author
  or a maintainer wins, default Open). `GitEngine::list` / `load` are
  Confirm-gated like bookshelves (one modal: announcement + issues +
  statuses + state, then comments under the found issues); `load` folds
  status + NIP-22 comment counts onto each issue. API: `GET /api/v1/git/
  repos` (`?pubkey=&policy=`), `GET /git/repo/:pubkey/:d_tag`, `POST /git/
  issue` (+ `/preview`), `POST /git/status` (author or maintainer only).
  Issue replies are ordinary NIP-22 comments via `/discussions/comment`
  (`root.kind: 1621`); the comment handler unions the repo's `relays`
  into the broadcast set for any NIP-34 root. Web: profile Repositories
  tab → `repository:<pk>:<d>` buffer (`RepositoryBuffer.svelte`).
  `query.rs` matches all-hex values of any tag filter (`#E`, `#q`) as
  packed ids — nostrdb packs 64-hex tag values regardless of tag name.
- **`spell.rs`**: NIP-A7 kind-777 "spells" — saved queries as events — plus the
  tendrl composition extension (spec: `nips/A7-composition.md`, beside the
  vendored `nips/A7.md`; design: `docs/zettel/idea-spells.org`): `param`
  declarations bound via `$arg.*`, pipeline projections (`$in.ids/pubkeys/
  tag.<letter>[:marker]`), `in` chaining (`["in", <spell-id>]` — a REQ/COUNT
  spell names its input spell and applies its filter to that spell's results;
  map semantics, recursive, depth-capped at 4, input params escalate at
  compose), `PIPE` spells whose stages reference other spells by
  id with `map` (replace with referents; pointer-less events pass through) /
  `join` (auxiliary enrichment) combinators, and closures (arg-binding `e`
  forks). Parsing/resolution pure; `SpellEngine` executes via
  `get_events_with_options` (FetchPolicy + Confirm gating apply). The
  search↔spell mapping is lossless and bidirectional: `to_clauses()`/
  `query_string()` render a spell as search-DSL clauses (the UI preview),
  `from_search_query()` + `parameterize()` compile a search into a 777
  template (`~:` refuses — no silent semantic→text degrade). Spellbooks =
  kind 30777 addressable `e`-tag sets referencing spells by any author,
  local-until-broadcast on the coordinate. API: `POST /api/v1/spell/`
  `inspect` / `execute` / `list` / `compose` / `book` / `book/template` /
  `book/save`. Web: profile Spells tab (merged book ∪ authored list,
  run-in-place, chevron unpacks PIPE stages and `in` chains), dual wand on
  the search panel (`+🪄` saves the typed query, `🪄` opens the form-driven
  spell builder incl. pipeline mode and chain-to-a-spell projections).
- **`blossom.rs`**: Blossom blob storage, pure above the client — BUD-11
  kind-24242 auth template + `Authorization: Nostr <b64>` header, BUD-02
  `PUT /upload` / BUD-04 `PUT /mirror` / BUD-01 `HEAD`, blob descriptor,
  BUD-03 kind-10063 server-list parse, `BlossomStore` at
  `<data_dir>/blossom.json` (ordered: first = upload target, rest = mirror
  targets; the user's 10063 is read as a *suggestion*, never auto-published).
  The 24242 is signed as the **user** through the `SigningController`, like
  NIP-42 AUTH: short-lived, scoped to the blob hash, never stored.
- **`narration.rs`**: publications read aloud — kind **32176** NKBIP-07
  "Blossom piece index" trees mirroring the 30040 tree: one leaf per section
  (one `b` piece = the joined chapter MP3) under one root (`a` children in
  TOC order). Binding to the narrated node is tendrl's proposed NKBIP-07
  extension and needs all three letters: `A` (coordinate) + `K` (kind) span
  every version by every narrator, `E` pins the exact event whose text was
  spoken; `content` carries the spoken text (`transcript txt`, default on)
  so another version's recording can be diffed against the current text.
  Pure: `spoken_text` (markup + nostrdown → prose, by `m` mimetype),
  `chunk_text` (sentence-boundary chunks ≤ the model's cap), `join_mp3`
  (frame concat; strips ID3 + Xing), `rank` (exact → other version →
  unpinned, newest replaceable wins). `NarrationEngine`: `plan` / `discover`
  (`#A` per tree node, LocalFirst, relay round Confirm-gated) / `generate`
  (ppq TTS via `ProviderRegistry::ppq_tts` → Blossom → sign+ingest per
  section → root → optional broadcast; SSE `GenEvent`s; skips sections
  already narrated for the same `E` + voice) / `mirror`. API under
  `/api/v1/narration/*` (`models`, `voices`, `servers`, `estimate`,
  `generate` SSE, `for/:pubkey/:d_tag`, `node/:kind/:pubkey/:d_tag` —
  one document on its own, `event/:pubkey/:d_tag` — a 32176 opened as
  a book: chapters in `a` order, version pin, the local publication
  listing the narrated section, `mirror`). ppq returns MP3 only
  (`response_format` ignored, probed 2026-11-09). Web: `narration.svelte.ts`
  store (one global player, recordings cache), `NarrationPlayer` strip,
  `NarrateModal`, reader `NarrationDrawer` + per-section ▶, Settings ›
  Narration. Design: `docs/zettel/idea-audiobook-blossom.org`.
- **`chat.rs`**: Pure state logic for LLM chat fragments, edit mode, context
  injection, message serialization (no IO)
- **`llm.rs`**: Async `LLMProvider` trait — `NoopProvider` (testing) and
  `OpenAiCompatProvider` (chat-completions wire format: tools ↔ `tool_calls`).
  The engine holds **no** model API key: the in-app assistant runs on an
  *agent service* from **`providers.rs`** — ppq.ai (account + Lightning
  funding) or a routstr node (kind-38421 discovery, Cashu token → key) —
  persisted in `<data_dir>/providers.json` (0600), chosen/funded in the UI
  (drawer → agent services → `AgentServicesModal`). A turn with no active
  service emits `needs_service`; one below the per-provider ceiling emits
  `needs_funding` (nothing recorded). Claude Code reaches tendrl only as an
  external harness over MCP (`docs/zettel/feat-mcp.org`). Design:
  `docs/paid-providers.org`, `docs/agent-service-flows.org`.
- **`claude_sessions.rs`**: Reader for Claude Code conversation JSONL files in
  `~/.claude/projects/`
- **`drafts.rs`**: Local JSON draft storage for unsigned NKBIP-01 publications before
  they are signed and published. **`DraftStore` is wired** behind
  `/api/v1/drafts` (POST save / GET list / GET `:id` / DELETE) — it persists the full
  compose state to `<data_dir>/drafts/` so a draft survives a refresh and is
  resumable; the composer's "Save draft" + "Saved drafts" list drive it.
  `LocalPublicationTracker` (the local-only a-tag tracker) is **now wired**: the
  publish handlers mark a signed snapshot local until a relay accepts it (then
  published), and `list_publications` exposes `local` so the feed renders a
  "local" pill. The publishing model: drafts are unsigned (DraftStore); **signing
  is the snapshot** (the only way into the db — no passive unsigned events);
  broadcast is a separate step.
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
- `limit:N` — per-query result cap (token wins; request-level `limit` fills absence)
- `relay:<url>` — relay override, repeatable; bare domains normalize to `wss://`
  (per-branch in `|` compounds; a Confirm-modal-approved set wins over tokens)
- `has:NAME` — tag-presence operator (matches any event carrying a `NAME` tag)
- `count:NAME` — histogram of distinct values of tag `NAME` across the matches
  (computed after all filtering). Two reserved names aggregate an event *field*
  instead of a tag: `count:kind` and `count:by` (pubkey), each bucket labelled
  engine-side with the kind name / kind-0 display name. `count:author` is still
  the `author` **tag**, mirroring the `by:` / `author:` split.
- `NAME:value` — generic tag filter (**bare key — no `#` prefix**; `#NAME:value` is
  NIP-01 raw-filter notation and parses as a literal text token here)

Note `author:` is a *tag* filter (events with an `["author", …]` tag), **not** an
alias for the `by:` publishing-pubkey filter.

Tag-filter values match slug variants automatically (`tag_value_variants`): each
typed value is expanded to {as-typed, lowercase, whitespace→dash, full NIP-54
normalization via `nostrdown::normalize`} and OR'd, so `T:"What's Up?"` hits the
stored `whats-up` — no need to hand-normalize; quote values containing spaces.

## Project Layout

- `src/` — Rust engine (library + the `tendrl-engine` binary). `src/server.rs`
  is the embeddable boot (`server::start`): the binary and the Android host
  both call it; `main.rs` is a thin CLI wrapper.
- `web/` — SvelteKit frontend (Svelte 5, pnpm, static adapter, served from `web/build/`)
- `mobile/src-tauri/` — Tauri 2 Android host (app id `io.github.limina1.tendrl`):
  engine in-process on a token-gated loopback port (stable 41347 — the WebView
  origin keys the SPA's persisted storage), WebView at `?shell=mobile`, in-app
  NIP-55 (Amber) Kotlin plugin, and a `media` plugin (foreground service +
  MediaSession so narration keeps playing with the screen off; the WebView
  stays the player). Independent crate, NOT in the root package;
  build commands + Android toolchain env in `docs/commands.org` ("Android build").
  The engine crate stays platform-agnostic: mobile builds use
  `--no-default-features --features embed-assets` (features gate
  embeddings/keyring/desktop).
- `emacs/` — `tendrl.el` (engine client: `tendrl-local-fetch` / `tendrl-relay-fetch`
  open an event's content read-only, `tendrl-event-json` shows the raw event apart;
  nak fallback) and `nostrdown-mode.el` (minor mode layering the `{{ }}` reference
  syntax over Org/Markdown/AsciiDoc via `POST /api/v1/nostrdown/resolve`); live
  tests `emacs/*-test.el`, usage in `emacs/README.org`
- `config.example.toml` — reference config; copy to `config.toml`
- `scripts/` — utility scripts (MCP server, publishing helpers)
- `knowledgebase/` — local documents for import
- `nips/` — Nostr protocol specs (NIP references and custom NKBIPs)
- `docs/` — design notes, roadmaps, `commands.org` (verify/test command reference),
  and `docs/zettel/` — the tendrl zettelkasten (mission, feature map, ideas,  comparisons; start at `docs/zettel/index.org`). After confirming with the user upon a feature/task completion - you should do a mini recap of the ideas and work you added, as a proposed outline of topics you might include or edit within the zettelkasten.

## Key Patterns

- **Error handling**: `EngineError` (thiserror) with variants for Database, Relay,
  NotFound, etc. `Result<T>` is aliased to `Result<T, EngineError>`.
- **Relay selection (one rule)**: a fetch for kinds K goes to the **read relays ∪
  the search relays serving K** (`Engine::relays_for_kinds` / `relays_for_filters`
  / `plan_resolve_fetch`), plus relay hints (local `e`/`a` tag hints, an
  `nevent`/`naddr`'s own relay TLVs via `search::entity_relay_hints` or the
  `relays=` query param), then session overrides. A relay is one row with five
  flags — auth, read, write, search, broadcast. **Search kinds**
  (`search_kinds`, formerly `resolve_kinds`) narrow the search flag: none
  listed = any kind; a kindless filter reaches only unrestricted search relays.
  There is no indexer class, no default/fallback tier, no `exclusive` flag —
  a profile relay is a search relay with `relay_store::INDEXER_KINDS`
  (generation-2 migration in `RelaySets::apply_default_upgrades`).
- **Relay sets**: The TOML config carries only `initial_relays` (a bootstrap seed
  list used **once** on first boot). The live working sets — `general` / `fetch` /
  `publish` / `broadcast` / `auth` (NIP-42 opt-in) / `search` — live in `<data_dir>/relays.json`, mutated by `engine.add_relay` /
  `engine.remove_relay` and the `POST /api/v1/config/update` handler. The state
  file is **never** broadcast as a Nostr event (no auto-publishing kind 10002 /
  30002 from UI edits — see `project_publishing_philosophy`).
- **Content modes**: Sections support Markdown, Org Mode, AsciiDoc, and PlainText,
  declared by the `["m", <mimetype>]` tag (canonical `text/markdown` / `text/org` /
  `text/asciidoc` / `text/plain`; legacy `["format", …]` read as an alias) — see
  `mimetype.rs`. **Tag or spec or user, no heuristics**: the engine refuses to publish
  a section without a declared format and stamps `m` on every 30041 (never on a
  30040). Reading precedence (`format_source`): the event's own `m` → the format its
  kind fixes by spec (`mimetype::for_kind`: 30023 Markdown, 30818 AsciiDoc; 30041
  never) → the nearest enclosing index's `text/*` `m` → the user's "untagged
  sections" preference → their per-buffer pick. The three bases are on by default
  (`web/src/lib/layers/`). Decisions: `docs/zettel/decision-mimetype-tag.org`,
  `docs/zettel/decision-layers-default-on.org`.
- **nostrdb read lock**: Never run nostrdb queries concurrently — `query.rs` holds a
  global mutex for a reason (see module docs).
- **Nostr event kinds used**: 0 (metadata), 3 (contacts), 777 (spells — saved
  queries, NIP-A7 draft), 10000/10002/10003/10006/
  10007 (NIP-51/65 lists), 30002 (relay sets), 30023 (long-form), 30040 (publication
  index), 30041 (publication section), 30045 (bookshelf), 30817/30818 (wiki),
  9802 (highlight), 30617/30618 (git repository + state), 1621 (git issue),
  1630–1633 (git status), 32176 (NKBIP-07 narration / Blossom piece index),
  24242 (Blossom auth, never stored), 10063 (Blossom server list, read only).
