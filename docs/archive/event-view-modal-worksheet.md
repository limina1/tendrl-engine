# Event View Modal — implementation worksheet

Working doc, not a spec. The
[main plan](event-view-modal-plan.md) covers the modal redesign at a
high level; the
[workbench-architecture](workbench-architecture.md) "Search
Invariants" section codifies the search rules everything below
depends on. This worksheet is for the open design decisions —
written out so they can be reasoned about one at a time, not all in
one head.

Each item has the same shape:

- **What it is** — concrete description, with a sketch where helpful.
- **Why it matters (UX)** — what breaks if we skip it.
- **Options on the table** — discrete choices, with tradeoffs.
- **Decision** — locked-in choice when committed.

---

## 1. Search history surface (global modeline strip + popover list)

### What it is

A persistent, app-level surface for the search history stack with two
display states:

**Collapsed state — a small icon + count in a global status strip.**

```
┌─────────────────────────────────────────────────────────┐
│  workbench shell …                                      │
│  ┌─chat─┐ ┌─work──────────┐ ┌─research──┐               │
│  │      │ │               │ │           │               │
│  │      │ │               │ │           │               │
│  └──────┘ └───────────────┘ └───────────┘               │
│  🔍 7    ← global status strip (only when count > 0)    │
└─────────────────────────────────────────────────────────┘
```

The strip is a new piece of chrome — separate from the per-pane
modelines, sitting at the app level. It shows nothing when the search
history stack is empty (no chrome, no whitespace). The moment the
user runs their first search, the strip appears with `🔍 1`.

**Expanded state — vertical popover list anchored to the strip.**

```
┌─ Search history · 7 entries ───────────────────────────┐
│ ░ query    bitcoin                        (12 events)  │ ← lighter
│   nevent   "The 21 Million Limit"         (1 event)    │
│   naddr    bitcoin-standard (kind 30040)  (3 versions) │
│   query    t:rust k:30041                 (47 events)  │
│   query    ~:proof of work                (10 events)  │
│   query    by:me                          (108 events) │
│   query    t:bitcoin                      (24 events)  │
└────────────────────────────────────────────────────────┘
        🔍 7
```

Clicking the icon toggles the popover. Each row replays its entry
when clicked: `query` entries call `handleSearch(q, opts)`; `nevent`
entries call `api.getEvent(id)` and wrap as a 1-row result; `naddr`
entries run the equivalent `k:K by:<npub> #d:<d>` query.

The most-recent entry (top of the list, the depth-1 target) is
rendered in a **lighter color** — not because it's clickable
differently, but because it indicates "this is the search whose
results are currently displayed" (or "this is what 'one step back'
would land on" when the modal is open one chain deep).

### Why it matters (UX)

- After several searches, the user wants to revisit one without
  retyping. Without a history surface, every revisit is a memory
  exercise.
- The "back from chained drill-down" affordance lives here, not in
  the modal. The user looks at the strip to see where they came from,
  clicks the top entry (lighter color) to step back one. No
  modal-internal chrome needed.
- When new events arrive (via publish, fetch, or import), the user
  can replay a prior query to see the impact — the freshest-local
  property of the search invariant makes this cheap.

### Decisions

- **Display when collapsed:** icon + count only (`🔍 N`). No latest
  query shown; the latest is implicit (it's whatever the search
  panel currently shows).
- **Most-recent entry highlight:** lighter color in the expanded
  list, indicating "this is what is displayed now" (or, with the
  modal open one level deep, "this is what going back lands on").
- **Visibility rule:** the global status strip is hidden entirely
  when `searchHistory.length === 0`. It appears the moment the first
  search is run and persists for the session.
- **Placement:** a new global status strip, separate from per-pane
  modelines. Not duplicated across panes; not attached to any single
  slot's modeline.
- **Implementation reuse:** the popover list reuses the minibuffer's
  list-rendering and row-selection infrastructure where practical
  (add a `'history'` mode + a non-leader trigger), but the trigger
  is a click on the strip, not a leader key. A leader keybinding
  can be added later without changing the data layer.

### Open mechanics (small, can defer)

- Cap on `searchHistory.length`. Suggest 50 entries; drop oldest
  when full. Tunable later. user: this shouldn't be a problem, it is in memory and resets each session
- Deduplication: should running the same query twice push two
  entries or update-in-place? Update-in-place is cleaner (the
  history doesn't bloat with repeated `t:rust` calls). Each entry
  carries a `lastRunAt` timestamp for tiebreaking. user: yes always deduplicate
- Persistence across reload: in-memory only for v1. Localstorage
  later if it proves useful. user: in memory

---

## 2. Modal positioning constraint

### What it is

The View Event modal must be positioned so the global status strip
(item 1) remains visible while the modal is open. The modal does not
take over the full viewport.

Concretely: if the strip sits at the bottom of the viewport at
`bottom: 0`, the modal's overlay/backdrop must stop short of the
strip — either by leaving a gap at the bottom (`bottom: <strip-height>`)
or by being a non-fullscreen panel positioned in a slot/area that
doesn't touch the strip's vertical band.

### Why it matters (UX)

- The whole point of the history strip is "see where you came from
  at all times." If the modal obscures the strip during chained
  navigation — exactly when the user is most likely to want to step
  back — the strip's value evaporates.
- It also keeps the modal feeling less heavy: a full-overlay modal
  is a context-switch; a modal that respects the chrome reads as
  "an inspector layer over the workbench," which matches what it
  actually is.

### Decision

- Modal is rendered such that the global status strip is always
  visible. Implementation detail: modal overlay clips to
  `bottom: <strip-height>` when the strip is visible, full-viewport
  when the strip is hidden (empty history case).
- The backdrop click-to-dismiss area is the same clipped region —
  clicking on the still-visible strip area should interact with the
  strip (open the popover), not dismiss the modal.

---

## 3. Audit: search invariant vs. current code

### Findings

Audit complete. Summary:

**Web search bar matches the invariant (✅).**
- `web/src/lib/api.ts:261` — `api.search()` defaults `policy = 'local_only'`.
- `web/src/lib/state.svelte.ts:753` — sole caller `handleSearch` uses the default.
- No other web-side code calls `api.search` with a different policy.

**Rust engine is capable of relay fetching, but the web doesn't trigger it (⚠️).**
- `src/engine.rs:23-28` — `FetchPolicy` defaults to `LocalFirst` (relay-augmented).
- `src/engine.rs:785` — `Engine::search()` takes a policy parameter; routes
  through `get_events()` which can hit relays for non-`LocalOnly` policies.
- `src/api.rs:175-178` — HTTP `/api/v1/search` falls back to
  `FetchPolicy::default()` (= `LocalFirst`) if the request body omits `policy`.

**NIP-50: no references anywhere in the Rust source.** The previous
workbench doc line about "optionally queries NIP-50 search-capable
relays" was aspirational; already fixed in the invariant section.

**Other consumers:**
- TUI (`src/tree/tui/app.rs:1821`) passes its own `policy` variable — user: forget TUI, our MVP functionality with the web interface is our guiding set for other implementations.
- Unit tests pin `LocalOnly` explicitly.
- No scripts or MCP found that hit search with a non-default policy.

### What this means

The invariant is true for the user-facing search bar today. It's not
yet enforced at the Rust API boundary — a third-party caller of
`POST /api/v1/search` that omits `policy` would get `LocalFirst`
behavior. The doc and the code have a seam.

we will figure this pattern out later - we anticipate if not found in search to look on relays - if we find on relays, that's fetching behavior, so we need to note this pattern later on, there's fetching feed, fetching events, fetching publications, fetching by tag.... etc. fetching + optional storage into db should be the anticipated pattern

### Options on the table

**Path A — Tighten API default to `LocalOnly`.** Change the search
handler's fallback from `FetchPolicy::default()` to an explicit
`LocalOnly`. Web is unaffected (it already overrides). External
callers that rely on the current default behavior would break.

**Path B — Soften the invariant.** Reword to "search-bar UX is
local-only; the engine API accepts a policy." Honest but weaker.

**Path C — Treat the seam as intentional.** The invariant lives on
the UX surface (search bar), not the engine API. The engine offers
`Engine::search(policy)` as a primitive; the search bar always uses
`LocalOnly`; future relay-augmented features get their own UX entry
points and use `LocalFirst`/`FetchAlways` deliberately. Doc this
boundary explicitly.

### Decision

> *Pending:* recommendation is **C** — it's what the code already
> does and it cleanly separates "engine capability" from "search-bar
> UX." Needs a small clarification in the invariant section saying
> the invariant binds the *UX*, not the engine primitive. If the
> user prefers A (cleaner doc, slight risk of breaking external
> callers), the change is a two-line edit to `src/api.rs:175-178`.

user:  yes use C separate engine capability from search bar
---

## 4. Addendum to the main plan

### What it is

The main plan (`event-view-modal-plan.md`) was written before this
conversation. The decisions made since — three-way stack types
(`query` / `nevent` / `naddr`), app-level `searchHistory` instead of
modal-scoped, query-replay instead of result-snapshotting,
version-aware `a`-tag handling, decode via a Rust `/api/v1/decode`
endpoint instead of adding `nostr-tools`, modeline-based history
surface instead of modal-internal breadcrumb — need to flow into the
plan or it goes stale.

### Why it matters (implementer UX)

If someone (you, me, or a future contributor) builds this feature
later and reads only the original plan, they'll build the wrong
thing. The addendum prevents the next round of design re-litigation.

### What goes in it

1. Replace the modal-internal back-stack design with: app-level
   `searchHistory`, modeline popover surface, modal-positioning
   constraint.
2. Document the three entry shapes and their replay rules.
3. Update tag-click mapping: `a` → version-aware fetch with
   "+N older versions" badge; `e`/`q`/`note` → direct fetch + modal
   swap; `p` → `by:` search, no modal reopen; `t`/`d`/`#x:val` →
   `handleSearch` only.
4. Replace "add nostr-tools" open question with "add
   `POST /api/v1/decode` endpoint in Rust."
5. Add "not found" handling: loading state in modal, relay-hint
   preference (use nevent/naddr TLV relays first), retry/close stub
   on truly-not-found.
6. Reference the new "Search Invariants" section in
   workbench-architecture, and the audit decision (path C, pending).
7. Cross-link this worksheet for any items still open.

### Decision

> *Pending:* append addendum sections to the existing plan with
> clear "supersedes" markers on outdated paragraphs. Promote to a
> full plan rewrite once the design stops shifting and we're
> entering implementation. To be done after item 3 audit decision
> is locked in.

---

## Order of operations

1. **Lock in audit direction (item 3).** Path A, B, or C. Two-line
   code change if A; doc tweak only if C; nothing if B.
2. **Write the addendum (item 4)** with all decisions to date.
3. **Implement `searchHistory` data structure + global status strip
   (item 1, foundation).** Stack lives at `app` level. Every
   `handleSearch` appends. Visibility-when-nonempty rule. Icon-plus-
   count rendering. No popover yet.
4. **Implement the popover list (item 1, full).** Click toggles, row
   rendering per-kind, lighter color on most-recent, replay on click.
5. **Implement modal positioning constraint (item 2).** Clip
   overlay to leave strip visible.
6. **Implement the modal redesign per the addendum** (items not in
   this worksheet — extract `EventViewModal`, identifiers/tags
   blocks, containing-publications block, chained navigation hooks).

---

## Glossary

- **Modeline** — Emacs-style status strip at the bottom of a buffer
  showing its state. In tendrl, modelines are per-pane. The "global
  status strip" introduced in item 1 is *not* a modeline — it's new
  chrome at the app level.
- **Status strip / global status strip** — the new app-level chrome
  that hosts the search-history icon and count. Visible only when
  the history stack is non-empty.
- **Popover** — a small floating panel anchored to a triggering
  element, dismissed by clicking outside. The expanded history list
  is a popover anchored to the status strip icon.
- **Minibuffer** — Emacs-style modal selection surface. In tendrl,
  triggered by leader keys (`SPC b b`, `SPC :`, etc.). The history
  popover *reuses* the minibuffer's list-rendering machinery
  internally but is triggered by mouse click, not leader key.
- **Leader key** — a modal keybinding prefix (`SPC` here) that
  starts a chord. Subsequent keys narrow to a command. Out of scope
  for v1 of the history surface; mouse-only for now, leader binding
  later.
- **Stack** — append-only on new searches, pop-or-pick on history
  navigation. Bounded at ~50 entries; oldest dropped when full.
- **Replay** — re-running a prior history entry. Cheap because
  search is local-only (per the invariant). Yields fresh local
  state — newly-arrived events show up.
- **Audit** — read the relevant code paths end-to-end and verify
  what they do vs. what the doc claims they do. Done for the search
  invariant; findings in item 3.
- **Addendum** — follow-up section appended to a plan that captures
  decisions made after the plan was originally written.
- **Invariant** — a property the system commits to upholding.
  "Search is local-only" is the invariant the modal and history
  features depend on; the audit shows it's true for the UX, with a
  seam at the engine API to be addressed by item 3's decision.
