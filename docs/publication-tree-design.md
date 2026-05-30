# Publication Tree — Design Doc

Design for porting the **notedeck publication tree** (lazy hierarchical loading +
visibility-driven prefetch + multi-mode reading) into the tendrl-engine `src/tree/`
module.

Reference implementation: `reference/notedeck/crates/notedeck_publications/` and
`reference/notedeck/crates/notedeck_reader/`.

> **Status (2026-05, Phase 3):** The `src/tree/` navigation engine this doc maps onto
> — `TreeState` / `TreeCommand` / `TreeEngine` (`state.rs` / `command.rs` / `engine.rs`
> / `render.rs`) — was **removed** in Phase 3 of the boundary-compliance refactor. The
> live publication tree is now assembled by `src/publication.rs` (a recursive depth-N
> loader + `stream_publication_tree` SSE stream) and rendered by the web's
> `ReaderBuffer.svelte`, which owns collapse/expand view-state. `src/tree/` now retains
> only the pure document-structure core (`node.rs` / `parser.rs` / `content.rs`); the
> compose payload types moved to `crate::publication::compose`. So the
> `TreeState` / `TreeCommand` mappings below are **historical design intent** — to be
> re-homed onto the `publication.rs` loader + web reader, not the deleted engine. The
> notedeck comparison and the prefetch/visibility reading model remain a genuine design
> record.

---

## 1. Goal

NKBIP-01 publications are hierarchical: a kind-30040 index references children
(sections 30041, or *nested* 30040 indices) via `a` tags. A publication can be
arbitrarily deep and large. We want to:

1. Open a publication knowing only its root index event.
2. Lazily resolve children — never block on fetching a whole publication.
3. Fetch only what the reader is actually looking at, plus a small prefetch buffer.
4. Render the same tree in several reading modes (Outline / Continuous / Paginated / Tree).
5. Cross into nested publications without unbounded inline nesting.

tendrl already has the *skeleton* for this. This doc specifies what to add and how
it maps onto the existing module.

---

## 2. What tendrl already has

`src/tree/` is interface-agnostic tree navigation, consumed by the web UI:

| Piece | File | Status |
|-------|------|--------|
| `TreeNode` = `Publication \| Section`, `NodeId`, `loaded`/`loading` flags | `node.rs` | ✅ exists |
| `TreeState` — flat `HashMap<NodeId, TreeNode>`, `roots`, `expanded`, `cursor` | `state.rs` | ✗ removed Phase 3 — see `publication.rs` loader + web reader |
| `TreeCommand` enum + `TreeEngine::execute()` synchronous executor | `command.rs`, `engine.rs` | ✗ removed Phase 3 — see `publication.rs` loader + web reader |
| Async boundary — `CommandResult::NeedsAsync(AsyncRequest)` → `apply_async_result()` | `command.rs`, `engine.rs` | ✗ removed Phase 3 — replaced by `stream_publication_tree` SSE |
| `ViewMode { Tree, Outline, Continuous, Paginated }` | `state.rs` | ✗ removed Phase 3 — view-mode state lives in the web reader |
| `visible_nodes()` → `Vec<VisibleNode>` for the UI | `render.rs` | ✗ removed Phase 3 — flattening lives in `ReaderBuffer.svelte` |
| `LoadStatus<T>` (`Pending → Loading → Loaded/Failed`) | `publication.rs` | ✅ exists |
| `PublicationEngine::load_publication / load_sections / load_section` | `publication.rs` | ✅ exists |

So the data model, the command/engine split, the async boundary, and the four mode
names are already present.

## 3. What's missing (the gap vs. notedeck)

notedeck's `PublicationTree` does five things tendrl's tree does **not** yet:

1. **Lazy a-tag-driven population.** A node starts `Pending` from its `a` tag and is
   `resolve`d when the event arrives. tendrl currently expects `load_sections` to
   return a fully-built `Publication`; it does not model per-node pending state
   inside the tree.
2. **Visibility-driven prefetch.** The UI reports which nodes are on screen; the
   engine fetches only those + a buffer of siblings/children, in relay-sized batches.
3. **A change-detection counter** (`resolved_version`) so the UI can cheaply tell
   whether the tree changed since the last frame/poll.
4. **Outline drill-down state** — a `current_node` cursor distinct from the tree
   cursor, with within-tree breadcrumbs and per-leaf expand/collapse.
5. **Depth-overflow handoff** — when a nested index is deeper than `MAX_INLINE_DEPTH`,
   open it as a *new* publication view rather than nesting inline forever.

This doc adds those five things.

---

## 4. Data model

### 4.1 Node resolution status

Add an explicit per-node resolution status. tendrl's `node.rs` has `loaded: bool` /
`loading: bool`; promote that to a tri-state mirroring notedeck's `NodeStatus`:

```rust
// src/tree/node.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NodeStatus {
    /// Known from a parent's a-tag, event not fetched yet
    #[default]
    Pending,
    /// Fetch in flight
    Loading,
    /// Event fetched, metadata + children extracted
    Resolved,
    /// Event not found / fetch failed
    Error,
}
```

Keep `loaded`/`loading` as derived accessors during migration, or replace them with
`status`. `NodeStatus` is preferable: it makes `Pending` (never attempted) distinct
from `Error` (attempted, failed) — needed for retry logic and the progress bar.

A node's *type* (branch vs leaf) is **only known after resolution**: a 30040 with
`a` tags is a Branch; everything else is a Leaf. Until resolved, treat a pending
node as a leaf placeholder (notedeck does exactly this in `new_pending`).

### 4.2 Why keep tendrl's `HashMap<NodeId, TreeNode>`

notedeck uses an index-based `Vec<PublicationTreeNode>` + `HashMap<EventAddress, usize>`.
tendrl already keys nodes by `NodeId` (a hash of the `NAddr`). Keep tendrl's map —
it gives the same O(1) address lookup notedeck's `address_to_index` provides, since
`NodeId::from_addr` is deterministic. The dedup guarantee notedeck gets from
`add_pending_node` ("return existing if already present") is free: inserting a node
whose `NodeId` already exists is a no-op / merge.

**One requirement:** child ordering must be preserved. `PublicationNode.children`
is a `Vec<NodeId>` — populate it in `a`-tag order and never sort it. (notedeck
carries an explicit `order: usize`; tendrl can rely on `Vec` order as long as
populate-from-a-tags appends in document order.)

### 4.3 Resolution version counter

Add to `TreeState` (or to a per-publication sub-state):

```rust
/// Increments every time a node transitions Pending/Loading → Resolved/Error.
resolved_version: u64,
```

The web API exposes it; the client passes its last-seen value back and skips
re-rendering when unchanged. This replaces polling-diffs with a single integer
compare. notedeck uses it in `PublicationTreeState::check_changed()`.

### 4.4 Per-publication view state

notedeck has one `PublicationTreeState` per open publication, held in
`Publications: HashMap<NoteId, PublicationTreeState>`. tendrl's `TreeState` is
currently a single shared tree. Two options:

- **A — single tree, multiple roots.** Keep one `TreeState`; each opened publication
  is a root in `roots`. `selected_publication` already scopes Reader mode to one
  subtree. Simplest; reuses existing code.
- **B — per-publication sub-state.** A `PublicationView` struct per root holding the
  fetch bookkeeping (`pending_fetch`, `sub_id`, `visible_nodes`, `resolved_version`).

**Recommendation: B, but lightweight.** The fetch bookkeeping is genuinely
per-publication and does not belong in the global `TreeState`. Add:

```rust
// src/tree/state.rs
pub struct PublicationView {
    pub root: NodeId,
    pub resolved_version: u64,
    /// Addresses with a fetch in flight (dedup guard)
    pending_fetch: HashSet<NAddr>,
    /// Node ids the client last reported as on-screen
    visible_nodes: HashSet<NodeId>,
    /// Outline drill-down cursor (see §7.2)
    outline_node: NodeId,
    expanded_leaves: HashSet<NodeId>,
}
```

`TreeState` keeps the node map (shared — nested publications can share authors and
even sections); `PublicationView` keeps the loading bookkeeping. The node map being
shared is a feature: a nested publication referenced twice resolves once.

---

## 5. Lazy loading & visibility-driven prefetch

This is the core behavior to port. The principle (notedeck §1): **the network is a
side-effect that feeds the local store; the tree is a pure function of the store.**

### 5.1 Open

`open_publication(root_index_id)`:

1. Load the root 30040 from nostrdb (`FetchPolicy::LocalFirst`).
2. Build the root `PublicationNode` (`Resolved`), parse its `a` tags, create one
   `Pending` child node per tag in tag order.
3. Do **not** recurse — children stay `Pending`.
4. Create a `PublicationView { root, outline_node: root, .. }`.

### 5.2 Visibility reporting

The UI must tell the engine what is on screen. In notedeck (egui, immediate mode)
each frame pushes rendered node indices into `state.rendered_nodes`, then
`set_visible_nodes()`. In tendrl the UI is web + REST, so visibility arrives as a
request parameter:

```
POST /api/v1/publications/{root}/visible   { "node_ids": ["node:..", ...] }
```

or it is implicit in whatever slice the client requested (the Paginated mode
endpoint inherently declares "this one section is visible"). Store it in
`PublicationView::visible_nodes`.

### 5.3 What to fetch — `visible_pending_addresses()`

Port notedeck's algorithm (`state.rs:72-113`). Given `visible_nodes`, collect
`NAddr`s of pending nodes from:

- each visible node itself,
- the children of each visible **branch** node (so drilling down is instant),
- the immediate previous/next siblings of each visible node (smooth scroll).

If `visible_nodes` is empty (first poll, no UI info yet), fall back to the first
`VISIBILITY_PREFETCH_BUFFER` (≈5) pending addresses — enough to paint something.

Exclude any address already in `pending_fetch`.

### 5.4 Batching

Group the addresses to fetch by `(kind, pubkey)` and emit one Nostr filter per group
(notedeck `build_filters_for_addresses`, `state.rs:573`). A 30040 publication can mix
authors across `a` tags, so do **not** assume one author.

Batch size = the minimum `max_event_tags` (NIP-11 limit) across connected relays, so
a single REQ never exceeds what a relay will answer. tendrl's `relay.rs` /
`RelayConfig` should expose this; default to 25 if unknown (notedeck's
`PublicationRequest::batch_size`).

### 5.5 Resolution loop

A `poll_publication(view)` step, run on a tick or after each relay EOSE:

1. For every address in `pending_fetch`, query nostrdb. If found, `resolve_node`:
   set the node `Resolved`, extract `title`, detect branch-vs-leaf, and **populate
   that node's own pending children from its `a` tags**. Bump `resolved_version`.
   Remove from `pending_fetch`.
2. Recompute `visible_pending_addresses()`; for any not in flight, emit a batched
   subscription and add them to `pending_fetch`.
3. Return whether anything resolved (drives the version bump / UI refresh).

Resolving a branch *adds new pending nodes*, so the tree grows outward one ring at a
time, paced by what the reader looks at. This is the whole mechanism.

### 5.6 Async-boundary fit

This maps directly onto tendrl's existing pattern. `TreeEngine::execute` already
returns `CommandResult::NeedsAsync(AsyncRequest)`; add request/result variants:

- `AsyncRequest::FetchAddresses { view: NodeId, filters: Vec<Filter> }`
- `AsyncResult::AddressesFetched { view: NodeId }`  → triggers `poll_publication`

The engine never does IO; the API layer runs the relay fetch and calls
`apply_async_result()`. Identical to how `load_sections` is wired today.

---

## 6. Rendering — `VisibleNode` is already the contract

`render.rs::visible_nodes()` already flattens the tree to `Vec<VisibleNode>` for the
web UI. Extend `VisibleNode` with the fields the reader needs:

```rust
pub status: NodeStatus,        // replaces / augments is_loading/is_loaded
pub node_kind: NodeKind,       // Branch | Leaf | PendingUnknown
pub order: usize,              // position within parent
```

The four modes are then *different flattenings of the same tree* — pure functions,
no new fetching logic:

| Mode | Flattening |
|------|-----------|
| **Tree** | current `visible_nodes()` — roots + expanded children, indented. |
| **Outline** | children of `outline_node` only (one level), + within-tree breadcrumbs. §7.2 |
| **Continuous** | all `Resolved` leaves of the selected publication in reading order. |
| **Paginated** | one leaf at a time, indexed by `current_leaf_index`. |

"Reading order" = depth-first leaf traversal. Port notedeck's `LeafIterator`
(`tree.rs:383`): push children reversed onto a stack, yield nodes with no children.

The progress indicator (`render_publication`, `reader.rs:498`) is
`resolved_count / node_count` — both already cheap to compute over the node map.

---

## 7. Outline mode — drill-down navigation

Outline is the default and most distinctive mode. It shows **one level at a time**
and lets the reader walk the hierarchy.

### 7.1 State

`PublicationView::outline_node` — the node whose children are currently displayed.
Independent of the tree `cursor`. `expanded_leaves` — leaf nodes shown with full
content inline instead of collapsed.

### 7.2 Commands

Add to `TreeCommand` (these already have UI affordances in notedeck's
`PublicationNavAction`):

| Command | Effect |
|---------|--------|
| `OutlineDrillDown(NodeId)` | branch child → `outline_node = child` |
| `OutlineUp` | `outline_node = parent` |
| `OutlinePrevSibling` / `OutlineNextSibling` | move `outline_node` among siblings |
| `OutlineToggleLeaf(NodeId)` | add/remove from `expanded_leaves` |
| `OutlineExpandAll` / `OutlineCollapseAll` | toggle every leaf child of `outline_node` |

`siblings(node)` — port notedeck `tree.rs:225`: look up the parent's child `Vec`,
find position, return `[pos-1]` / `[pos+1]`. Root returns `(None, None)`.

### 7.3 Within-tree breadcrumbs

`hierarchy(node)` — walk `parent` pointers to the root, reverse. Render the path of
titles (`reader.rs:855` `render_outline_breadcrumbs`). This is the *intra-publication*
breadcrumb; distinct from the *inter-publication* history in §8.

### 7.4 Leaf vs branch in the child list

For each child of `outline_node`:

- **Branch** → render as a folder; clicking it `OutlineDrillDown`s.
- **Leaf** → render collapsed; clicking toggles `expanded_leaves`; when expanded,
  inline its content.
- **Pending** (unresolved) → render with a spinner/⏳ marker. Reporting it as visible
  is what triggers its fetch (§5.2).

---

## 8. Depth overflow & cross-publication navigation

A publication can nest 30040 indices inside 30040 indices without bound. Rendering
that fully inline is unusable. Port notedeck's rule:

```rust
pub const MAX_INLINE_DEPTH: usize = 5;
```

`should_open_as_new_publication(node)` = node is a **Branch** AND
`depth(node) >= MAX_INLINE_DEPTH`. When the reader drills into such a node, instead
of `OutlineDrillDown` within the current view, open a **new publication view** rooted
at that node and push the current root onto a navigation history.

### 8.1 Navigation history

Port `PublicationSelection` (`reference/notedeck/crates/notedeck_reader/src/nav.rs`):

```rust
pub struct PublicationSelection {
    pub root: NodeId,         // current publication root
    history: Vec<NodeId>,     // parent publications, oldest first
}
```

`navigate_into(new_root)` pushes current; `navigate_back()` pops; `breadcrumbs()`
returns the history. This is the *inter-publication* trail — shown in the reader
header alongside the intra-publication breadcrumb of §7.3.

In tendrl this likely belongs in the reader/UI state, not `TreeState`, since
`TreeState` already supports multiple roots and `selected_publication`. Switching the
selected publication = `navigate_into`.

---

## 9. N-level eager expansion (the depth knob)

§5 fetches reactively — one ring at a time, paced by what's on screen. That is the
right *default* for huge publications, but the reader often wants the opposite: a
deliberate "show me N levels now, indented, so I can see the shape." This section
adds that as an explicit, bounded prefetch policy layered **on top of** §5 — it does
not replace lazy resolution, it front-loads it.

**Decisions** (stated, not asked — they follow directly from the described flow):

- *N counts 30040 nesting depth.* Sections (30041) are leaves at their parent index's
  level; they don't consume a level. Depth 1 = "the index and its direct children";
  depth 2 = "…and every nested index's children".
- *A depth-N load is full within N.* It resolves every index **and** every section
  inside the N levels. Below depth N, nodes stay `Pending` and §5's visibility-driven
  resolution takes over. So N is a prefetch horizon, not a hard wall.
- *Refocus re-roots in place.* Clicking a nested 30040 makes it the new focus root,
  re-runs the depth-N load from it, and pushes the previous root onto a focus stack.
  The stack is the breadcrumb. This is the *same* mechanism as §8's automatic
  `MAX_INLINE_DEPTH` overflow — overflow is just an auto-triggered refocus.
- *Indented display is `ViewMode::Tree`* with a depth bound, not a new mode.

### 9.1 The recursive loader

This belongs in `publication.rs`: `Publication.nested: Vec<Publication>` is already a
recursive owned tree and `from_event` already classifies `a` tags by kind. The gap
the prior analysis identified — "nothing loads the nested 30040 stubs" — is exactly
one method:

```rust
// PublicationEngine
pub async fn load_publication_tree(
    &self,
    addr: &NAddr,
    max_depth: usize,        // 0 = index only, 1 = + direct children, ...
    policy: FetchPolicy,
) -> Result<Publication>
```

Algorithm:

1. `get_addressable(30040, addr)` → `Publication::from_event(ev, is_root)` — yields
   the placeholder TOC (`sections` = 30041 stubs, `nested` = 30040 stubs), as today.
2. `load_sections(&mut pub_)` — resolve this level's 30041 leaves (full load).
3. If `max_depth == 0`, return. Otherwise for each `nested[i]`, recurse
   `load_publication_tree(&nested[i].addr, max_depth - 1)` and **replace** the stub
   with the filled `Publication`.
4. Return the depth-N `Publication`. `is_root` is `true` only for the top call.

Two things to get right:

- **Cycle / revisit guard.** Carry a `HashSet<NAddr>` of indices already on the
  current path — a 30040 that references an ancestor (or itself) must not recurse.
  On a hit, leave the stub `Pending` and stop.
- **Concurrency.** Sibling `nested` loads at one level are independent — fetch them
  concurrently (`FuturesUnordered` / `join_all`) under a small semaphore so a wide
  index doesn't open hundreds of relay requests at once. Depth is sequential per
  branch; breadth is parallel.

`build_toc` already recurses over `nested` producing per-level `depth` — once the
loader fills `nested`, `build_toc` output *is* the indented N-level view, for free.
The one `build_toc` fix: a nested index left `Pending` (depth horizon or cycle) must
render with a visible "load more" affordance instead of the silent childless
`"Nested Publication"` fallback.

### 9.2 The depth knob

`max_depth` is a viewer setting — a slider/stepper in the reader header. Default 1
(index + direct children: cheap, always safe). Changing it re-runs
`load_publication_tree` from the current focus root.

Because §5's lazy layer still exists, the knob is not load-bearing for correctness —
set it to 0 and the reader still works, one drill at a time. It is a *convenience
horizon*: deep levels you didn't eagerly load fall through to lazy resolution when
scrolled into view. That is why "full vs structure-only" was never a hard fork.

### 9.3 Refocus + breadcrumb

Clicking a nested 30040 in the indented view:

1. push the current focus root onto `PublicationSelection::history` (§8.1),
2. set focus root = clicked node's `addr`,
3. re-run `load_publication_tree(addr, max_depth)`.

The breadcrumb is `history` rendered as a clickable path; clicking crumb *k* pops
back to it. This is *inter-publication* navigation; the within-tree `hierarchy()`
breadcrumb of §7.3 still applies inside a single focus. The header shows both:
`Root › Sub-pub › Sub-sub-pub` (focus stack) and, beneath it, the outline path
inside the current focus.

`MAX_INLINE_DEPTH` (§8) then reduces to: if a depth-N load would render a branch
deeper than `MAX_INLINE_DEPTH`, stop indenting and present it as a refocus target —
auto-applying steps 1–3 the moment the reader drills in.

### 9.4 Confirm-gate integration

A depth-N load has a knowable cost *after step 1 of each level* — every index lists
its children before they're fetched. So `load_publication_tree` can, before
committing the fetch, emit a plan: "level 1: 1 index + 12 sections · level 2: 3
indexes + 40 sections · …". That plan is the `steps` of a single `FetchOperation`
Intent — one confirm gate for the whole expansion rather than one per event.

---

## 10. Web API surface

Concrete endpoints under `/api/v1/` (the engine stays interface-agnostic; these are
the reader-app's calls):

| Endpoint | Returns |
|----------|---------|
| `POST /publications/{naddr}/open` | root metadata + `resolved_version` + first ring of (pending) children |
| `GET  /publications/{root}/tree?since_version=N` | `Vec<VisibleNode>` if version advanced, else `304`/empty |
| `GET  /publications/{root}/outline/{node}` | children of `node` + breadcrumbs |
| `GET  /publications/{root}/leaf/{index}` | one section's content (Paginated) |
| `POST /publications/{root}/visible` | `{ node_ids }` → updates prefetch set |
| `POST /publications/{root}/close` | drops `PublicationView`, unsubscribes relays |

`since_version` is the `resolved_version` mechanism: cheap long-polling without
diffing trees.

---

## 11. Implementation phases

1. **Node status.** Add `NodeStatus` to `node.rs`; thread it through `VisibleNode`.
   Migrate `loaded`/`loading` call sites. *No behavior change yet.*
2. **PublicationView + resolved_version.** Add the per-publication struct and the
   version counter. `open_publication` builds root + pending ring.
3. **Lazy resolution loop.** Port `visible_pending_addresses`, `build_filters`,
   `resolve_node`, `poll_publication`. Wire to `AsyncRequest::FetchAddresses`.
4. **Recursive depth-N loader.** `load_publication_tree` with cycle guard and
   bounded-concurrency breadth fetch; the `max_depth` knob; `build_toc` "load more"
   affordance for unexpanded indices. Lives in `publication.rs`, parallel to 1–3.
5. **Reading modes.** `LeafIterator`; Continuous + Paginated flattenings; progress.
6. **Outline drill-down.** `OutlineDrillDown`/`Up`/sibling/expand commands;
   `hierarchy()`, `siblings()`, breadcrumbs.
7. **Depth overflow + refocus.** `MAX_INLINE_DEPTH`, `PublicationSelection` history,
   refocus-on-click, cross-publication navigation.
8. **API + visibility reporting.** Endpoints in §10.

Phases 1–3 are the tree-module foundation. Phase 4 is a parallel `publication.rs`
track and can land **first** if recursive N-level expansion is the priority — it
needs none of 1–3. 5–7 are independent of each other. 8 follows once the rest is
stable.

---

## 12. Open questions

- **Visibility without an immediate-mode UI.** notedeck gets `rendered_nodes` for
  free each frame. tendrl's web client must report it explicitly or accept that the
  requested slice *is* the visibility signal. Pick one before phase 8.
- **`SyncStatus` interaction.** tendrl's `node.rs` already has `SyncStatus`
  (Remote/LocalOnly/Draft/…) for compose/publish. `NodeStatus` (fetch resolution) is
  orthogonal — a node can be `Resolved` + `Draft`. Keep both; don't conflate.
- **Cache eviction.** notedeck keeps every resolved node for the session. For very
  large publications tendrl may want to drop content (not structure) of off-screen
  leaves. Defer until measured.
- **Section versioning.** tendrl's `Section.alternates` / `load_section_versions` has
  no notedeck equivalent. The tree should expose `alternate_count` (already on
  `VisibleNode`) but version selection stays outside this design.
- **Subscription churn.** notedeck unsubscribes the previous batch before each new
  one (`state.rs:444`). Confirm tendrl's relay layer is happy replacing subscriptions
  every poll, or coalesce.

---

## 13. File-by-file map

| tendrl file | Change |
|-------------|--------|
| `src/tree/node.rs` | `NodeStatus` enum; branch/leaf-after-resolve |
| ~~`src/tree/state.rs`~~ (removed Phase 3) | `PublicationView`, `resolved_version`, visibility set — re-home onto `src/publication.rs` + web reader |
| ~~`src/tree/command.rs`~~ (removed Phase 3) | `Outline*` commands; `FetchAddresses` async req/result — re-home onto `src/publication.rs` + web reader |
| ~~`src/tree/engine.rs`~~ (removed Phase 3) | `open_publication`, `resolve_node`, `poll_publication`, `visible_pending_addresses`, `siblings`, `hierarchy`, `LeafIterator` — re-home onto `src/publication.rs` |
| ~~`src/tree/render.rs`~~ (removed Phase 3) | extend `VisibleNode`; Continuous/Paginated/Outline flattenings — re-home onto `src/publication.rs` + web reader (`ReaderBuffer.svelte`) |
| `src/publication.rs` | `load_publication_tree` (recursive depth-N loader, cycle guard, bounded concurrency); `build_toc` "load more" for `Pending` nested indices; `MAX_INLINE_DEPTH`; reuse `NAddr`, `LoadStatus` |
| `src/api.rs` | endpoints in §10 |

Reference files to read alongside:
`reference/notedeck/crates/notedeck_publications/src/{tree,node,address,fetcher}.rs`,
`reference/notedeck/crates/notedeck_reader/src/{state,nav}.rs` and
`ui/reader.rs`.
