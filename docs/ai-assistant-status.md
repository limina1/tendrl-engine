# AI Assistant — Status & Handoff

Status/handoff companion to **`docs/ai-tools-architecture.md`** (the original
design + rationale). That doc explains *why*; this one records *what exists now*
and *where to go next*, so a fresh agent can pick up without re-deriving the
state. Read the architecture doc first, then this.

---

## TL;DR

A provider-agnostic AI **writing assistant with server-side tool-calling** over
the Nostr corpus is built and working end-to-end: the user chats, the engine runs
an agentic loop (Anthropic Messages API, **API-key only**), the model calls tools
to search / read / curate context / compose / fetch from relays / publish, and the
web renders the turn live and folds tool results into its panels.

- **Code lives on branch `ai-context-tools`** (worktree
  `/home/user/Documents/Programming/tendrl-ai`), forked from `tendrl-web` and
  intended to **merge back into `tendrl-web` when done**
  (`git checkout tendrl-web && git merge ai-context-tools`, fast-forward).
- `tendrl-web` already contains everything below; `ai-context-tools` is where
  ongoing AI work continues so it stays off the shared main checkout.
- Nothing pushed to origin.

---

## How to run / verify

```bash
# fresh worktree needs setup once:
cd /home/user/Documents/Programming/tendrl-ai
pnpm -C web install && web/node_modules/.bin/svelte-kit sync
cargo build

# run (API key from env — the ONLY supported auth):
set -gx ANTHROPIC_API_KEY (string trim -- (head -1 ~/.apis/claude.key))   # fish
./start.sh --dev
```

Without `ANTHROPIC_API_KEY` the provider falls back to `NoopProvider` (echo) so
the stream still works for plumbing tests, but no real tool-use happens.

**Web checks in a fresh worktree** (pnpm's pre-run deps check errors on ignored
esbuild builds — bypass it):
```bash
cd web && node_modules/.bin/svelte-kit sync && node_modules/.bin/svelte-check --threshold error
```

---

## Architecture recap (the load-bearing decisions)

- **Auth: API key only.** `ANTHROPIC_API_KEY` → `x-api-key`. The OAuth/subscription
  path was explored and **dropped** (we can only use the API with Claude). No
  credential abstraction.
- **The loop runs server-side in Rust** (`src/api.rs::chat_agent_handler`), per the
  frontend/backend boundary rule (tool-calling is data access + derivation). The
  web renders + approves; it does not orchestrate.
- **Provider-neutral types** in `src/llm.rs`: `ContentBlock` (Text/Thinking/
  ToolUse/ToolResult), `AgentMessage`, `ToolDefinition`, `StopReason`,
  `run_turn()`. `chat()` is a provided default wrapper for legacy single-shot.
- **Tools are thin language→engine bridges** (`src/tools.rs`). They add no data
  logic — each calls an existing engine method and returns a **compact JSON
  handle** (id/kind/addr/title/snippet), so the model curates a working set by id
  and expands only what it needs.
- **Permission model:** `ToolPolicy` filters the catalog *before* the provider
  call (a disabled tool never reaches the model). Default = **everything but
  publish**. Surfaced in the AI Tools settings tab, grouped by category.
- **Gating reuses what exists.** Relay fetches and publishes gate themselves
  through the engine's pre-existing Confirm-mode flows (FetchConfirmModal /
  PublishConfirmModal) — no new approval machinery.
- **The context model is the web's `ContextItem` pool**, not engine state — see
  below.

### Key files
| Concern | File |
|---|---|
| Provider + neutral types | `src/llm.rs` |
| Tool catalog + dispatch | `src/tools.rs` |
| Agent loop (SSE) + chat/session/AI endpoints | `src/api.rs` |
| Chat state, `to_agent_messages` + sanitize | `src/chat.rs` |
| Saved sessions store | `src/sessions.rs` |
| `[ai]` config | `src/config.rs` |
| Editable system prompt | `prompt.md` (path resolved at boot) |
| Web chat + streaming + pool | `web/src/lib/state.svelte.ts`, `web/src/lib/api.ts` |
| Context panel UI | `web/src/lib/components/ContextPanel.svelte` |
| AI Tools settings tab | `web/src/lib/wm/renderers/SettingsBuffer.svelte` |

---

## What's built

### Foundation (prior sessions)
- **Provider core** (`llm.rs`), **tool registry** with the read/search/view +
  compose tools (`tools.rs`), the **agent loop** `POST /api/v1/chat/agent` (inline
  `async_stream` SSE), **web rendering** of live blocks, **saved chat sessions**
  (`sessions.rs` + `/chat/sessions`), an **editable `prompt.md`** loaded into the
  system prompt and live-reloaded each turn, and the **AI Tools settings tab**.
  Commits: `cd06fe8` (architecture), `fb4fb21` (server-side agent), `30d1e4c`
  (web chat/settings/sessions/prompt), `8600bef` (drop OAuth → API key only).

### This session (on `ai-context-tools`, today)
| Commit | What |
|---|---|
| `203edd8` | **Overwrite-save + edit-repair.** `session_save` takes an optional `id`; the web tracks the active session (Save→"Update", "Save as new"). `to_agent_messages` runs `sanitize_tool_pairs` so deleting blocks can't orphan a `tool_use`/`tool_result` and 400 the API. |
| `6f9ad54` | **`add_to_context` tool.** The assistant pulls a found event (by id/addr) or a synthesized idea into the **shared** context the user also sees/edits. Emits a structured item; the web folds it into the pool (boundary-clean). |
| `bcfdc9f` | **`fetch_from_relays` + `publish` tools (P6).** `fetch_from_relays` (Network, on by default) runs a plain Nostr query against relays, gated by the FetchConfirmModal. `publish` (Publish, **off by default**) broadcasts an **already-local, already-signed** event by id — it does NOT sign. |
| `343ac64` | **Peek/compact context UI.** Pristine context items render as a 2-line snippet you can skim; click/▸ to expand into the editable textarea. Modified items stay expanded. |
| `8f2c472` | **Profile → context entry point.** A "+ Context" button on the profile bar captures a kind-0 profile as a note (profiles aren't addressable, so they have no pool pill). |

---

## The context model (most important mental model)

"Context" = the working set of reference notes handed to the model as a system
message (`ChatState.injected_context`, rendered in `to_agent_messages`). But the
**source of truth is the web's `ContextItem` pool**, not the engine.

- The web holds one unified `items[]` pool with flags `in_context` / `in_compose`
  / `held` and an `origin` (`chat`/`search`/`compose`/`import`). `ContextPanel`
  shows `in_context` items; `syncContext()` pushes `{title, context_content}` of
  those items to the engine via `PUT /api/v1/chat/context` (`replaceContext`).
- This split is correct per the boundary rule: routing/edit/expansion state is
  view state (web); the engine just receives the resolved notes.

**Entry points into context (all already wired):**
- **Search results, reader / publication-tree sections, event detail** — the
  unified `PoolStateBadges` pill / `togglePoolMembership` / `pillActionByAddr`,
  which call `syncContext()` on context toggles.
- **Profile detail** — the "+ Context" button (added this session).
- **The AI** — the `add_to_context` tool. The agent *emits a proposal*; the web's
  `applyToolResult` folds it into the pool and syncs. The agent never writes
  engine context directly (which `syncContext` would clobber).

So a context item can be added by the user *or* the assistant, and both see the
same set — which is exactly the intended "shared space."

---

## Tool catalog (current — 15 tools)

| Tool | Category | Default | Notes |
|---|---|---|---|
| `search_events` | Search | on | local DSL search |
| `semantic_search` | Search | on | embeddings; needs index |
| `search_profiles` | Search | on | |
| `get_event` / `get_addressable` | Read | on | full event JSON |
| `list_publications` / `view_publication` / `view_publication_tree` / `list_section_versions` | Read | on | NKBIP-01 |
| `fetch_from_relays` | Network | on | relay query, FetchConfirm-gated |
| `add_to_context` | Context | on | curate the shared context |
| `propose_section` / `edit_section` / `save_draft` | ComposeWrite | on | feed the composer / DraftStore |
| `publish` | Publish | **off** | broadcast an existing local event, PublishConfirm-gated |

`ToolCategory` enum: `Read, Search, Network, Context, ComposeWrite, Publish`.

---

## Where to go next (open items)

1. **`publish` semantics — decision pending.** Today it's **broadcast-only**: it
   rebroadcasts an already-signed local event by id and deliberately cannot sign
   ("signing is the snapshot; broadcast is a separate step"). If AI-authored
   *signing* is wanted, that needs the `SigningController` threaded into the agent
   handler (it's an axum `Extension`, not on `Engine`) and a proper finalize path.
   Get the user's call before building it.
2. **Context model — only snapshot + peek shipped.** The user liked all three
   models (snapshot, live-reference re-resolve, compact-handle-via-tools). We built
   snapshot + a peek/skim affordance. Optional future: a "live reference" mode that
   re-resolves an `addr` each turn, and/or a compact-handle mode where the item
   stores id+snippet and the model expands on demand.
3. **Chat-selection → context.** Listed as a desired entry point; chat fragments
   already exist as pool items (`chatFragmentItems`). Verify/finish promoting a
   selected chat message into a pinned context note if not already smooth.
4. **Boundary refinement:** stream a flattened TOC from the engine to drop
   `ReaderBuffer`'s addr-keyed re-accumulation twin (see
   `docs/eval/08-frontend-backend-boundary.org`).
5. **End-to-end verification with a real key + live relays** — exercise
   `fetch_from_relays` in Confirm mode (modal pops, decline → `approved:false`
   fallback) and a `publish` round-trip.

---

## Gotchas / operational notes

- **Shared main checkout is used by concurrent agents.** Working directly in
  `/home/user/Documents/Programming/tendrl-engine` caused a branch collision and
  swept another agent's staged files into a commit. **Work in a dedicated worktree**
  (this is why `ai-context-tools` / `tendrl-ai` exists). When committing anywhere
  shared, stage explicitly and use `git commit -- <paths>`.
- **Fresh worktrees lack `web/node_modules` and `web/.svelte-kit`** — run
  `pnpm -C web install` and `svelte-kit sync` before `svelte-check`/builds.
- **rustfmt only leaf files, with the edition flag:** `rustfmt --edition 2021
  src/<file>.rs`. NEVER bare `cargo fmt` (HEAD isn't fmt-clean; it rewrites ~30
  files), and never rustfmt `src/lib.rs` (pulls every module).
- **nostrdb has a process-wide query lock** (`query.rs`); never hold it across
  `.await`. Tools inherit it transparently by calling engine methods.
- **Cross-turn block ordering** is the highest correctness risk: every assistant
  `tool_use` must be answered by a matching `tool_result` in the next message.
  `sanitize_tool_pairs` (`chat.rs`) now guarantees this regardless of edits — keep
  it.
- **Opus is adaptive-thinking-only:** omit `temperature`/`top_p`/`top_k` for Opus
  4.x in the request body or it 400s.
