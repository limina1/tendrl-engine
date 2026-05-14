# Discussions (NIP-22 + NIP-84) — plan

**Status:** PLAN. Read-side scaffolding is shipped: `discussions/counts`
endpoint, two-phase hydration in the reader, per-section badges, and a
single-event `DiscussionViewBuffer` that surfaces root/parent refs and
loads sibling threads on demand. Highlight overlays land via the
reader's `?highlight=<id>` buffer suffix. This doc captures the work
needed to make discussions a first-class citizen — authoring, listing,
threading, and live updates — without unbalancing the WM model.

## Goals

1. **Read everything that's already been published.** A user opening a
   reader buffer can see, for each section and for the publication
   index, the full set of NIP-22 comments (kind 1111) and NIP-84
   highlights (kind 9802) referencing it — not just counts.
2. **Write back.** A signed-in user can post a comment (top-level or
   threaded reply) against any addressable event, and capture a text
   selection as a highlight. Tag construction is centralized so the
   wire format stays uniform.
3. **Local-first.** Authored events land on the local relay first and
   are visible immediately in the reader. Broadcast to externals
   happens through the same `PublishController` pipeline that
   publications use, so per-relay status (`accepted | rejected |
   rate-limited`) is observable.
4. **NIP-22 threading, correctly.** Uppercase `A/E/K/P` = root scope;
   lowercase `a/e/k/p` = immediate parent. Replies-to-replies work.
   Display orders by time within a tree; collapses by default.
5. **Highlights pin to position, not phrase.** New highlights carry
   `["offset", start, end]` so a recurring phrase doesn't ambiguate
   the overlay. Read path falls back to substring match for events
   without offsets (Alexandria's older payloads).
6. **Cheap to repeat.** Repeated visits to the same reader buffer
   don't re-roundtrip relays. The engine caches per-address counts
   and invalidates on ingest of a matching 1111/9802.

Out of scope for this pass: NIP-25 reactions (kind 7), notifications
(kind 1 mentions, follow alerts), encrypted comments. Those have
their own surface area and shouldn't get bolted onto this work.

## Reference

The Alexandria web client (`reference/gc-alexandria/`) implements a
working version of all of this on top of NDK. It's the wrong shape
for tendrl — Alexandria does its own per-relay WebSocket fanout from
the browser because "NDK subscriptions mysteriously returned 0
events" (their words, copied verbatim into both `HighlightLayer.svelte`
and `CommentLayer.svelte`). In tendrl the engine *is* the
relay-fetcher; the web is a thin client. But the data model,
tag rules, and UI affordances are sound. Specifically borrow:

- `src/lib/components/publications/CommentLayer.svelte` — `#A` vs
  `#e` filter selection.
- `src/lib/components/publications/SectionComments.svelte:56-133` —
  NIP-22 parent-detection (lowercase `e` ⇒ parent, normalized
  lowercase ids on both ends of the map).
- `src/lib/components/publications/CommentButton.svelte:109-132` —
  NIP-22 tag construction for a top-level comment.
- `src/lib/components/publications/SectionComments.svelte:373-386` —
  NIP-22 tag construction for a reply.
- `src/lib/components/publications/HighlightSelectionHandler.svelte` —
  mouseup → range → offset → 9802 template (skim for the offset
  math; the rest is NDK-flavored and won't port).
- `src/lib/components/publications/HighlightLayer.svelte:914-993` —
  the offset-then-fallback-text rendering loop.

Where tendrl's WM model diverges from Alexandria's all-in-one page
layout, ignore the visual treatment and keep only the data flow.

## What exists today

### Engine
- `POST /api/v1/discussions/counts` (`src/api.rs:1288-1415`): batch
  count of 1111 + 1112 — sorry, 1111 + 9802 — events referencing a
  list of `a` tag values. Goes through the standard `FetchPolicy`
  with optional `bypass_offline` for user-initiated refreshes.
  Combined single-filter relay REQ (`{kinds: [1111, 9802], "#a":
  addresses}`) keeps round-trips to one.
- Generic primitives ready to compose against: `get_events`,
  `get_by_id`, `get_addressable`, `EventTemplate`, `SigningController`
  (in-process, NIP-07, NIP-46), `publish_events_to_relays`,
  `/api/v1/identity/sign`, `/api/v1/broadcast`.

### Web
- `ReaderBuffer.svelte`: two-phase hydration (`local_only` then
  `fetch_always` if online), per-publication summary badges, per-
  section `c N h N` chips, "Refresh discussions" with offline-bypass.
- `DiscussionViewBuffer.svelte`: opens a single 1111 or 9802 event,
  extracts uppercase A/E (root) vs lowercase a/e (parent), provides
  "Open root/parent in reader" and "Show in source" for highlights;
  lazily loads sibling thread via a `#A` / `#E` query.
- Reader's `?highlight=<id>` suffix: fetches the 9802 event, drops
  into paginated view, auto-jumps to the section containing the
  match, overlays a `<mark>` around the substring (substring search,
  no offsets).
- Search routes 1111/9802 hits to `DiscussionViewBuffer` instead of
  the reader.

## Critical gaps

| # | Gap | Impact |
|---|-----|--------|
| 1 | No way to list the actual comments/highlights for a section | The `c 3 h 2` badge is a dead-end; users can only browse discussions if they happen to hit one via search. |
| 2 | No authoring path for kind 1111 | Can't comment, can't reply. Tendrl is read-only on the discussion layer. |
| 3 | No authoring path for kind 9802 | Can't highlight. Selection-to-event is the killer affordance for a long-form reader; missing it is the loudest gap. |
| 4 | Threading is flat siblings, not a tree | `DiscussionViewBuffer.loadThread` sorts by time. Reply-to-reply structure is lost. |
| 5 | Highlights are substring-matched, not offset-pinned | Repeating phrases highlight the wrong instance; whitespace differences miss. |
| 6 | One overlay color, no per-author identity | Multiple readers' highlights are indistinguishable. |
| 7 | Counts re-walk events on every reader open | Cheap individually, but the same publication browsed across a session re-queries needlessly. |
| 8 | Ignore-list pubkeys aren't filtered out of counts | A muted noisemaker still inflates badges. |
| 9 | No deletion / hide flow | Even your own comments are permanent from the UI's perspective. |

## Design

### 1. Engine: `GET /api/v1/discussions/list`

```
GET /api/v1/discussions/list
    ?address=<kind:pubkey:d_tag>      (repeatable)
    &event_id=<hex>                   (repeatable; alternative to address)
    &kinds=1111,9802                  (default: both)
    &policy=local_first               (local_only | local_first | fetch_always)
    &limit=500
    &since=<unix-ts>                  (optional, for incremental refresh)

→ {
    events: [<full event json>, ...],
    source: { local_count, relay_count },
    refreshed_at: <unix-ts>
  }
```

Reuses the same filter shape as `discussions/counts` but returns
events. Implementation is `engine.get_events_with_options(...)`
followed by light projection. The same call also warms nostrdb, so
the next `discussions/counts` hits local.

Why a separate endpoint and not a generic `/query`: it's the natural
caching boundary (see #7 below), it enforces the `kinds` filter
server-side so the web can't accidentally ask for everything, and it
gives us a place to apply the ignore-list filter (#8) without bleeding
that policy into `/query`.

### 2. Engine: authoring endpoints

NIP-22 tag construction is non-trivial and inconsistent across
clients in the wild. Keep it in one place. Two endpoints:

```
POST /api/v1/discussions/comment
{
  "root":   { "address": "<k:pk:d>" }  | { "event_id": "<hex>", "kind": <n>, "pubkey": "<pk>" },
  "parent": <same shape, optional — omit for top-level>,
  "content": "...",
  "relay_hint": "wss://..."  (optional; engine fills from relay config)
}
→ { event: <signed kind-1111 event>, broadcast_results: [...] }
```

```
POST /api/v1/discussions/highlight
{
  "target": { "address": "<k:pk:d>" } | { "event_id": "<hex>", "kind": <n>, "pubkey": "<pk>" },
  "content": "<highlighted text>",
  "offset": [<start_char>, <end_char>],         (optional but encouraged)
  "context": "<surrounding paragraph>",         (optional, per NIP-84)
  "comment": "<annotation>",                    (optional; goes in `comment` tag)
  "relay_hint": "wss://..."
}
→ { event: <signed kind-9802 event>, broadcast_results: [...] }
```

Internally both build an `EventTemplate`, route through
`SigningController` (so NIP-07 / NIP-46 work transparently), then
publish to the configured general relay set via the
`PublishController` pipeline when that lands — until then, direct
`publish_events_to_relays`. Local relay is written first, externals
second; both records flow into the same response shape so the web
can render a per-relay accepted/rejected matrix identical to
publications.

Tag layout the engine produces (mirrors what Alexandria does, with
the names spelled out so the source of truth is *this doc*, not three
.svelte files):

For a **top-level kind-1111 comment** against an addressable target
`A = k:pk:d`:
```
A k:pk:d <relay> <author_pk>        (uppercase: root scope = target)
K <kind>                            (root kind)
P <author_pk> <relay>               (root author)
a k:pk:d <relay>                    (parent scope = same as root)
k <kind>
p <author_pk> <relay>
```

For a **reply to comment `<parent_id>` on the same target**:
```
A k:pk:d <relay> <target_author>    (root unchanged)
K <target_kind>
P <target_author> <relay>
a 1111:<comment_author>:           (parent now points at comment scope)
k 1111
p <comment_author> <relay>
e <parent_id> <relay> reply         (NIP-10 marker; parent comment id)
```

For a **kind-9802 highlight**:
```
a k:pk:d <relay>                    (NIP-84 uses lowercase only)
e <target_event_id> <relay>         (when available — engine fetches it)
k <target_kind>
p <target_author> <relay>
offset <start> <end>                (when provided)
context <paragraph>                 (when provided)
comment <annotation>                (when provided)
alt "Highlighted text"
```

`content` for 9802 is the highlighted text itself.

### 3. Web: a `discussions-list` buffer

New buffer kind, opened by clicking the badge in the reader toolbar
or on a section badge. Renders all 1111 + 9802 events for the
target(s), threaded for comments, grouped-by-author for highlights.

Buffer id shape:
```
discussions:<k>:<pk>:<d>              single-address list
discussions:publication:<k>:<pk>:<d>  publication-wide (index + every section)
```

The publication-wide variant queries every section address in one
call; the engine flattens them into one event list.

UI inside the buffer:

```
┌───────────────────────────────────────────────────┐
│ Discussions on  <title>                  refresh  │
├───────────────────────────────────────────────────┤
│ Comments (12)                                     │
│ ▾ <author> 2d ago                                 │
│   <content>                                       │
│   ↳ <reply author> 1d ago                         │
│     <content>                              reply  │
│   ↳ <reply author> 1d ago                         │
│ ▾ <author> 4h ago                                 │
│   ...                                             │
│                                                   │
│ Highlights (5)                                    │
│ ● <author A>  3 highlights              expand    │
│ ● <author B>  2 highlights              expand    │
└───────────────────────────────────────────────────┘
```

#### Thread-tree builder

Pure function in `web/src/lib/discussions/thread.ts`. Ported from
`SectionComments.svelte:56-133` with the same lowercase-id
normalization on both ends of the map — the Alexandria notes call
this out as a deliberate bugfix worth keeping.

```ts
type CommentNode = { event: NostrEvent; replies: CommentNode[] };

export function buildThread(events: NostrEvent[]): CommentNode[] {
  const idSet = new Set(events.map(e => e.id.toLowerCase()));
  const nodes = new Map<string, CommentNode>();
  for (const e of events) nodes.set(e.id.toLowerCase(), { event: e, replies: [] });

  const childrenOf = new Map<string, CommentNode[]>();
  const roots: CommentNode[] = [];

  for (const e of events) {
    const node = nodes.get(e.id.toLowerCase())!;
    // NIP-22: lowercase e = immediate parent. Take the first lowercase
    // e tag whose value is the id of another comment in our input set.
    const parentId = e.tags
      .filter(t => t[0] === 'e')
      .map(t => t[1]?.toLowerCase())
      .find(pid => pid && idSet.has(pid));

    if (parentId) {
      const bucket = childrenOf.get(parentId) ?? [];
      bucket.push(node);
      childrenOf.set(parentId, bucket);
    } else {
      roots.push(node);
    }
  }

  // Attach children depth-first, oldest-first within each level.
  function attach(node: CommentNode) {
    const kids = (childrenOf.get(node.event.id.toLowerCase()) ?? [])
      .sort((a, b) => a.event.created_at - b.event.created_at);
    for (const k of kids) attach(k);
    node.replies = kids;
  }
  for (const r of roots) attach(r);

  roots.sort((a, b) => b.event.created_at - a.event.created_at); // newest root first
  return roots;
}
```

#### Recursive renderer

Svelte 5 snippet that calls itself by name. Tendrl can use unbounded
recursion here — Alexandria's hand-unrolled three levels was a
workaround for older Svelte. CSS caps the *visual* indent at depth 6
so a runaway thread doesn't push content offscreen, but the data
remains flat in the DOM (only `--depth` differs):

```svelte
<!-- CommentThread.svelte -->
{#snippet CommentNode(node: CommentNode, depth: number)}
  <article class="cn" style="--depth: {Math.min(depth, 6)}">
    <header class="cn-head">
      <ProfileChip pubkey={node.event.pubkey} />
      <time>{relative(node.event.created_at)}</time>
      <button class="cn-menu" use:popover>⋮</button>
    </header>
    <div class="cn-body">{node.event.content}</div>
    <footer class="cn-foot">
      <button onclick={() => replyTo = node.event.id}>Reply</button>
    </footer>
    {#if replyTo === node.event.id}
      <ReplyBox parent={node.event} root={rootRef} onposted={onreply} />
    {/if}
    {#each node.replies as child (child.event.id)}
      {@render CommentNode(child, depth + 1)}
    {/each}
  </article>
{/snippet}
```

```css
.cn { padding-left: calc(var(--depth) * 14px); }
.cn:not(:first-child) {
  border-left: 1px solid var(--border);
  margin-left: calc(var(--depth) * 14px - 14px);
}
```

A single `replyTo: string | null` state at the thread root tracks
which comment's reply box is open — only one at a time. On post,
the new event is optimistically appended (see §10) so the thread
re-renders without a refetch.

Top-level click on a comment opens the existing
`DiscussionViewBuffer` for that single event. Top-level click on a
highlight in the highlights section opens the reader at
`?highlight=<id>` (the same routing the existing "Show in source"
button uses).

### 4. Reader: inline thread disclosure per section

Below each section in the continuous/paginated views, render a
collapsed disclosure: `▸ N comments`. Expanding shows the top three
threads inline (root + first two replies, "show more" if truncated).
This mirrors the way Alexandria nests its threads under each section
without committing to Alexandria's full sidebar UI — tendrl's reader
already has the right vertical rhythm for this.

#### Where it mounts

`SectionDisclosure.svelte` is rendered as a *child* of each section
container, after the section content. Two integration points:

- `ContinuousView.svelte:78-110` — inside `.continuous-section`, after
  the existing `<pre class="section-content">`.
- `PaginatedView.svelte` — inside each pager page, after the
  `<SectionCard>` body.

It is **not** added to `SectionCard.svelte` itself, because that
component is also used in the outline view — threads under titles in
outline mode would be noise. The two consumers explicitly opt in by
mounting `SectionDisclosure` next to their `<SectionCard>` calls.

#### DOM shape

```svelte
<details class="thread-disclosure" data-section-addr={addrKey(section.addr)}>
  <summary>
    <span class="td-marker">▸</span>
    <span class="td-count">{N} comment{N === 1 ? '' : 's'}</span>
    {#if highlightCount > 0}
      <span class="td-hcount">· {highlightCount} highlight{highlightCount === 1 ? '' : 's'}</span>
    {/if}
  </summary>
  <div class="td-body">
    {#each visibleRoots as root (root.event.id)}
      {@render CommentNode(root, 0)}
    {/each}
    {#if rootCount > visibleRoots.length}
      <button onclick={openListBuffer}>Show all {rootCount} →</button>
    {/if}
    {#if signedIn}
      <ReplyBox root={section.addr} parent={null} onposted={onposted} />
    {:else}
      <p class="td-signin-hint">Sign in to comment</p>
    {/if}
  </div>
</details>
```

`visibleRoots` is `threadStructure.roots.slice(0, 3)`; the "Show all"
button opens the full `DiscussionsListBuffer` for that section
address. The `data-section-addr` attribute is what `HighlightCapture`
(§5) uses to know which section a selection belongs to.

Comments derive per-section: `commentsFor(addr) =
allEvents.filter(e => e.kind === 1111 && hasRootRef(e, addr))`.
"Root ref" is satisfied by either `A=<addr>` or `a=<addr>` (we accept
both so legacy clients work).

Authoring controls inside the disclosure:
- `Reply` button on every `CommentNode` (sign-in required).
- An inline `<ReplyBox>` at the bottom of the disclosure for new
  top-level comments. Posting against the section address, no
  `parent`.

The section-level highlight badge (today's `c N h N` chips) stays —
that part already works.

### 5. Highlight capture

New component `HighlightCapture.svelte` mounted once at the reader
level. Listens for `selectionchange` on `document`, debounces 200ms
so it fires on selection *settle* rather than every keystroke /
mouse-move, then renders a small floating action anchored to the
selection rect:

```
┌──────────────────────┐
│ ✎ Highlight    💬 …  │
└──────────────────────┘
```

#### Selection → section

A valid selection is one whose anchor node lives inside an element
with a `data-section-addr` attribute (set by `SectionDisclosure`'s
parent container). If not, the action stays hidden — selections in
the toolbar, sidebar, etc. don't trigger.

```ts
function activeSection(sel: Selection): HTMLElement | null {
  if (sel.rangeCount === 0 || sel.isCollapsed) return null;
  const anchor = sel.anchorNode;
  if (!anchor) return null;
  const el = anchor.nodeType === Node.TEXT_NODE ? anchor.parentElement : (anchor as Element);
  return el?.closest('[data-section-addr]') ?? null;
}
```

#### Offset math

The 9802 event's `["offset", start, end]` is in characters, relative
to the section's plain-text content (`section.content` from the
engine response). To compute it from a DOM `Range`, walk the
section's text nodes left-to-right and sum lengths until we reach the
range's start/end node.

```ts
export function domSelectionToOffsets(
  sectionEl: HTMLElement,
  range: Range
): [number, number] {
  const startIdx = textOffsetWithin(sectionEl, range.startContainer, range.startOffset);
  const endIdx   = textOffsetWithin(sectionEl, range.endContainer,   range.endOffset);
  return [Math.min(startIdx, endIdx), Math.max(startIdx, endIdx)];
}

function textOffsetWithin(root: HTMLElement, node: Node, offsetInNode: number): number {
  let total = 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  let n: Node | null;
  while ((n = walker.nextNode())) {
    if (n === node) return total + offsetInNode;
    total += (n.textContent ?? '').length;
  }
  // Edge case: range endpoint is an element node (selection ends right
  // after the last char of a child). Fall back to total text length.
  return total;
}
```

Critical: the walker descends into existing `<mark class="hl-overlay">`
elements because they're emitted as part of the same text stream
(no nested editable containers, no shadow DOM). That means offsets
are stable regardless of which other highlights are currently
rendered.

#### Action positioning

```ts
const rect = range.getBoundingClientRect();
const actionEl = ...; // measured after render
const top  = Math.max(8, rect.top - actionEl.offsetHeight - 6);
const left = Math.min(
  rect.left + rect.width / 2 - actionEl.offsetWidth / 2,
  window.innerWidth - actionEl.offsetWidth - 8
);
actionEl.style.transform = `translate(${left}px, ${top}px)`;
```

#### Posting

`Highlight` calls `POST /api/v1/discussions/highlight` with
`{ target: <section addr>, content: <selected text>, offset: [s, e] }`.
On 200 OK, the signed event in the response is optimistically
appended to `ReaderBuffer.discussions.events` (see §10) so the
overlay renders in the same tick. No refetch needed.

The `💬` button (future) seeds a comment compose with the selected
text quoted — out of scope for this pass.

### 6. Read-side highlight rendering

Today's overlay (`SectionCard.svelte:88-93`, `ContinuousView.svelte:92-98`,
analogous in PaginatedView) splits the section content into three
strings — `{before}<mark>{match}</mark>{after}` — by `String.indexOf`.
That handles exactly one highlight at a time and breaks when:
- the same phrase appears more than once in the section,
- whitespace in the highlight differs from the rendered text,
- two highlights cover overlapping ranges,
- two authors highlight the same range and want distinct attribution.

Replace with a single `<HighlightedText>` component that takes
`content: string` and `highlights: Highlight[]` and emits an
interleaved sequence of text nodes and `<mark>` elements via Svelte's
template — no imperative DOM mutation, no TreeWalker on the rendered
output. The renderers stay reactive.

```ts
type Highlight = {
  id: string;                  // 9802 event id
  pubkey: string;
  start: number;               // char offset into section.content
  end: number;
  // Resolved offset for events that arrived without offset tags
  // (substring match; see step 2 below).
};
```

#### Segmentation algorithm

For each section, compute a sorted list of "boundary events"
(`open`/`close` of each highlight), then sweep left-to-right
producing segments. Each segment carries the set of highlights
*active* at that span:

```ts
type Segment = { text: string; active: Highlight[] };

export function segment(content: string, highlights: Highlight[]): Segment[] {
  type Boundary = { at: number; kind: 'open' | 'close'; h: Highlight };
  const bs: Boundary[] = [];
  for (const h of highlights) {
    bs.push({ at: h.start, kind: 'open',  h });
    bs.push({ at: h.end,   kind: 'close', h });
  }
  // Process closes before opens at the same index so a touching pair
  // doesn't get merged into one segment.
  bs.sort((a, b) => a.at - b.at || (a.kind === 'close' ? -1 : 1));

  const segs: Segment[] = [];
  const active = new Map<string, Highlight>();
  let cursor = 0;

  for (const b of bs) {
    if (b.at > cursor) {
      segs.push({
        text: content.slice(cursor, b.at),
        active: [...active.values()]
      });
      cursor = b.at;
    }
    if (b.kind === 'open') active.set(b.h.id, b.h);
    else active.delete(b.h.id);
  }
  if (cursor < content.length) {
    segs.push({ text: content.slice(cursor), active: [] });
  }
  return segs;
}
```

#### Rendering segments

```svelte
<!-- HighlightedText.svelte -->
<pre class="section-content">{#each segments as seg, i (i)}{#if seg.active.length === 0}{seg.text}{:else}<mark
        class="hl-overlay"
        style={overlayStyle(seg.active)}
        data-hl-ids={seg.active.map(h => h.id).join(',')}
        onclick={(e) => onHighlightClick(e, seg.active)}
        title={seg.active.map(h => `@${short(h.pubkey)}`).join(', ')}
      >{seg.text}</mark>{/if}{/each}</pre>
```

(The lack of whitespace inside `{#each}` is deliberate — `<pre>`
preserves it, so any indentation in the template leaks into the
rendered text. Same trick the current code uses.)

#### Per-author color

Stable hue from pubkey. Port `pubkeyToHue` from Alexandria
(`src/lib/utils/nostrUtils.ts`) — it's a one-liner:

```ts
export function pubkeyToHue(pubkey: string): number {
  // First 6 hex chars of pubkey, mod 360.
  return parseInt(pubkey.slice(0, 6), 16) % 360;
}
```

Single highlight: tinted background, narrow inset stripe.
```css
mark.hl-overlay {
  background: hsla(var(--hue), 70%, 60%, 0.22);
  box-shadow: inset 3px 0 0 hsla(var(--hue), 70%, 50%, 0.9);
  padding: 1px 2px; border-radius: 2px;
}
```

Overlapping highlights from multiple authors stack stripes via
`box-shadow` (each stripe shifts 3px right). Background uses the
most-recently-opened highlight's hue:

```ts
function overlayStyle(active: Highlight[]): string {
  const top = active[active.length - 1];  // most-recently-opened
  const stripes = active
    .map((h, i) => `inset ${(i + 1) * 3}px 0 0 hsla(${pubkeyToHue(h.pubkey)}, 70%, 50%, 0.9)`)
    .join(', ');
  return `--hue: ${pubkeyToHue(top.pubkey)}; box-shadow: ${stripes};`;
}
```

This is why we don't *nest* `<mark>` elements — flat segments
preserve text selection across boundaries and let the user re-
highlight inside an existing highlight without the browser fighting
us. The cost is paid in segment count, not nesting depth.

#### Click on a highlight

`onHighlightClick(e, active)`:
- If `active.length === 1`, open `DiscussionViewBuffer` for that id.
- Else, render a small popover listing each author with click-to-open
  per highlight.

#### Offset back-fill for legacy events

Highlights from Alexandria-era clients arrive without an `offset`
tag — only `content`. The reader resolves their offset locally on
ingest:

```ts
function resolveHighlight(content: string, ev: NostrEvent): Highlight | null {
  const offsetTag = ev.tags.find(t => t[0] === 'offset');
  if (offsetTag) {
    return {
      id: ev.id, pubkey: ev.pubkey,
      start: parseInt(offsetTag[1], 10),
      end:   parseInt(offsetTag[2], 10),
    };
  }
  // Whitespace-normalized first-occurrence match.
  const needle = (ev.content ?? '').trim().replace(/\s+/g, ' ').toLowerCase();
  if (!needle) return null;
  const haystack = content.replace(/\s+/g, ' ').toLowerCase();
  const idx = haystack.indexOf(needle);
  if (idx < 0) return null;
  return { id: ev.id, pubkey: ev.pubkey, start: idx, end: idx + needle.length };
}
```

Resolved offsets are cached in a `Map<eventId, Highlight>` keyed on
the section content's hash so they don't recompute on every render.

#### Highlights drawer

Small fixed-position panel in the reader, opened from the discussion
summary chip. Lists highlights grouped by author, click-to-scroll to
the rendered `<mark>` in the article. Matches Alexandria's pattern
(`HighlightLayer.svelte:1229-1339`) but the data flow is trivial once
§1 (`discussions/list`) is in place — no relay logic in this
component, just a `$derived` view over the same `discussions.events`
state the renderers read from.

##### Grouping derivation

Sweep all highlights across all sections of the open publication,
bucket by author pubkey, sort each bucket newest-first:

```ts
type AuthorGroup = {
  pubkey: string;
  hue: number;                // pubkeyToHue, computed once
  highlights: HighlightEntry[];
};
type HighlightEntry = {
  id: string;                 // 9802 event id
  sectionAddr: string;        // which section the highlight lives in
  preview: string;            // truncated content for the row
  createdAt: number;
};

const authorGroups = $derived.by<AuthorGroup[]>(() => {
  const byPubkey = new Map<string, HighlightEntry[]>();
  for (const e of discussions.events) {
    if (e.kind !== 9802) continue;
    // Find which of our section addresses this highlight references.
    const aTag = e.tags.find(t => (t[0] === 'a' || t[0] === 'A') && t[1]);
    if (!aTag) continue;
    const entry: HighlightEntry = {
      id: e.id,
      sectionAddr: aTag[1],
      preview: truncate(e.content ?? '', 80),
      createdAt: e.created_at,
    };
    const bucket = byPubkey.get(e.pubkey) ?? [];
    bucket.push(entry);
    byPubkey.set(e.pubkey, bucket);
  }
  return Array.from(byPubkey.entries())
    .map(([pubkey, highlights]) => ({
      pubkey,
      hue: pubkeyToHue(pubkey),
      highlights: highlights.sort((a, b) => b.createdAt - a.createdAt),
    }))
    .sort((a, b) => b.highlights.length - a.highlights.length); // most prolific first
});
```

The `hue` here is the **same value** the inline `<mark>` overlays
compute — so the swatch in the drawer and the stripe on the
highlight in the article are guaranteed to match. This is the only
reason `pubkeyToHue` is deterministic and pure: two independent
renderers must agree.

##### DOM shape

```svelte
<aside class="hl-drawer" class:open={drawerOpen}>
  <header>
    <h3>Highlights ({totalCount})</h3>
    <button onclick={() => drawerOpen = false} aria-label="Close">×</button>
  </header>
  <ul class="hl-authors">
    {#each authorGroups as group (group.pubkey)}
      {@const expanded = expandedAuthors.has(group.pubkey)}
      <li class="hl-author">
        <button class="hl-author-row" onclick={() => toggleAuthor(group.pubkey)}>
          <span class="hl-swatch" style="--hue: {group.hue}"></span>
          <ProfileChip pubkey={group.pubkey} />
          <span class="hl-count">{group.highlights.length}</span>
          <span class="hl-chevron" class:open={expanded}>▸</span>
        </button>
        {#if expanded}
          <ul class="hl-entries">
            {#each group.highlights as entry (entry.id)}
              <li>
                <button
                  class="hl-entry"
                  onclick={() => scrollToHighlight(entry.id, entry.sectionAddr)}
                  title={entry.preview}
                >
                  <span class="hl-stripe" style="--hue: {group.hue}"></span>
                  <span class="hl-preview">{entry.preview}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
  </ul>
</aside>
```

```css
.hl-drawer {
  position: fixed;
  right: 16px; bottom: 16px;
  width: 320px; max-height: 60vh;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: var(--r-md);
  display: flex; flex-direction: column;
  transform: translateX(calc(100% + 32px));
  transition: transform 200ms;
}
.hl-drawer.open { transform: none; }

.hl-swatch {
  width: 12px; height: 12px; border-radius: 3px;
  background: hsla(var(--hue), 70%, 60%, 0.85);
  flex-shrink: 0;
}
.hl-stripe {
  width: 3px; align-self: stretch;
  background: hsla(var(--hue), 70%, 50%, 0.9);
}
.hl-preview {
  text-align: left;
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  font-size: var(--t-xs);
}
.hl-chevron { transition: transform 120ms; }
.hl-chevron.open { transform: rotate(90deg); }
```

The swatch and the stripe both bind their hue via the CSS custom
property `--hue` set inline from `group.hue` — identical math to
`overlayStyle()` in the renderer, so the drawer color and the
in-article overlay color are visually unified.

##### Click-to-scroll

A highlight may span multiple `<mark>` segments when other highlights
overlap it (segmentation breaks it up). `document.querySelector`
returns the first match in document order — that's the leftmost
segment of the highlight, which is what we want to scroll to:

```ts
function scrollToHighlight(highlightId: string, sectionAddr: string) {
  // Step 1: if the target section isn't currently visible (paginated
  // view, current page is elsewhere), jump to it first.
  if (currentSection !== sectionIndexFor(sectionAddr)) {
    currentSection = sectionIndexFor(sectionAddr);
  }
  // Wait a tick for the DOM to update if we changed pages.
  requestAnimationFrame(() => {
    const mark = document.querySelector<HTMLElement>(
      `mark.hl-overlay[data-hl-ids~="${highlightId}"]`
    );
    if (!mark) return;
    mark.scrollIntoView({ behavior: 'smooth', block: 'center' });
    mark.classList.add('hl-flash');
    setTimeout(() => mark.classList.remove('hl-flash'), 1200);
  });
}
```

Note the `~=` attribute selector — `data-hl-ids` is a comma-separated
list, but `~=` matches *whitespace-separated* tokens, so we use a
space separator in the rendered attribute (`data-hl-ids="id1 id2"`,
not `"id1,id2"`). Update the segmentation render template
accordingly:

```svelte
data-hl-ids={seg.active.map(h => h.id).join(' ')}
```

##### Flash animation

A pulse on the targeted `<mark>` so the user's eye can find it after
scroll completes. Uses brightness/saturation rather than a color
swap so it works with whatever hue the highlight already has:

```css
@keyframes hl-flash {
  0%, 100% { filter: brightness(1) saturate(1); }
  30%      { filter: brightness(1.4) saturate(1.6); }
}
mark.hl-overlay.hl-flash {
  animation: hl-flash 1.2s ease-in-out;
}
```

##### Open/close

The drawer's `drawerOpen` boolean lives on `ReaderBuffer`. Toggled
by the discussion summary chip in the toolbar:

```svelte
<button onclick={() => drawerOpen = !drawerOpen}>
  {totalDiscussion.highlights} highlight{totalDiscussion.highlights === 1 ? '' : 's'}
</button>
```

Closes on `Esc` when focused, and on click-outside via a top-level
listener that ignores clicks inside `.hl-drawer` or on any
`mark.hl-overlay` (so clicking a highlight in the article doesn't
dismiss the drawer that surfaced it).

`expandedAuthors: Set<string>` tracks which author rows are
expanded; clicking another author's row doesn't auto-collapse the
first — readers often want to compare two authors' highlights side
by side.

### 7. Engine: per-address count cache

Add `DiscussionCache` to `Engine`:

```rust
struct DiscussionCache {
    // key: (address, kind) -> count
    counts: DashMap<(String, u16), CachedCount>,
}
struct CachedCount {
    value: usize,
    refreshed_at: Instant,
    // bumped on any ingest of a 1111/9802 with this `a` tag
    invalidated: bool,
}
```

Populated lazily by `discussion_counts_handler`. Invalidated on event
ingest in `relay::ingest_event` whenever the event is kind 1111 or
9802 and carries an `a` tag (or `A` tag) — walk the tags, look up
matching keys, mark invalidated. `discussions/counts` with
`policy=local_only` and a still-fresh entry returns the cached value
without a nostrdb query; otherwise it recomputes and refreshes.

TTL is implicit: the cache only invalidates on actual ingest, so
"fresh" means "no new matching event since the count was taken."
Repeated reader-buffer opens of the same publication during a quiet
period are O(1).

### 8. Ignore list integration

`discussion_counts_handler` and the new `discussions/list` filter out
events whose `pubkey` is in the engine's ignore list before counting/
returning. Single chokepoint, both endpoints honor the same policy.
The ignore list is already queried via `/api/v1/ignore`; add a
sync-friendly accessor on `Engine`.

### 9. Deletion

Wire kind-5 deletion (NIP-09):

```
DELETE /api/v1/discussions/:event_id
{ "reason": "user deleted comment" }
→ { event: <signed kind-5 event>, broadcast_results: [...] }
```

Only allowed when `event.pubkey == identity.pubkey`. UI: a kebab on
each comment / highlight with `Copy id | Delete (mine only)`.
Engine respects local kind-5 events when serving `discussions/list`
(tombstone filter).

### 10. Reactivity model: from network to pixels

All rendering paths above feed off a single source of truth at the
reader level. No per-component fetching, no imperative DOM updates.

#### State location

`ReaderBuffer.svelte` owns:

```ts
let discussions = $state<{
  events: NostrEvent[];           // all 1111 + 9802 for this publication
  refreshedAt: number | null;
  source: { local_count: number; relay_count: number } | null;
}>({ events: [], refreshedAt: null, source: null });
```

Derived selectors (also at `ReaderBuffer` level, passed down as
props):

```ts
const byAddress = $derived.by(() => {
  const m = new Map<string, NostrEvent[]>();
  for (const e of discussions.events) {
    for (const t of e.tags) {
      if ((t[0] === 'a' || t[0] === 'A') && t[1]) {
        const bucket = m.get(t[1]) ?? [];
        bucket.push(e);
        m.set(t[1], bucket);
      }
    }
  }
  return m;
});

function commentsFor(addr: string): NostrEvent[] {
  return (byAddress.get(addr) ?? []).filter(e => e.kind === 1111);
}
function highlightsFor(addr: string, sectionText: string): Highlight[] {
  return (byAddress.get(addr) ?? [])
    .filter(e => e.kind === 9802)
    .map(e => resolveHighlight(sectionText, e))
    .filter((h): h is Highlight => h !== null);
}
```

#### Three-phase hydration

1. **Phase A — local instant.** On reader open: counts hit
   `/api/v1/discussions/counts?policy=local_only` (already shipped)
   for badges, plus `/api/v1/discussions/list?policy=local_only` for
   the events. Sub-50ms because nostrdb is local.
2. **Phase B — relay backfill.** If online: same `list` call with
   `policy=fetch_always`. Result replaces `discussions.events`. New
   events show up; the engine's nostrdb writes happen as a side
   effect so the next reader open also benefits.
3. **Phase C — optimistic append.** When the user posts via
   `discussions/comment` or `discussions/highlight`, the *response*
   already contains the signed event (the engine writes local before
   responding). The web prepends it to `discussions.events` in the
   same tick. The derived selectors recompute, the affected section's
   `SectionDisclosure` and `HighlightedText` re-render. No spinner.
4. **Phase D — background reconciliation.** 5s after a post, fire
   a debounced `list?policy=fetch_always` to pick up relay echoes
   and any concurrent events. Idempotent merge on event id.

#### Why this matters for rendering

Every render path is a pure function of `discussions.events`:

- `commentsFor(addr)` → `buildThread(...)` → recursive snippet → DOM.
- `highlightsFor(addr, content)` → `segment(content, ...)` → flat
  `<mark>` interleavings.

Optimistic append + derived state means the gap between "I clicked
post" and "I see my comment/highlight" is one tick, not a relay
round-trip. The current Alexandria flow has a `setTimeout(refresh,
3000)` to wait for relay indexing (Publication.svelte:745-754) —
tendrl skips that entirely because the local relay is the source of
truth and the engine wrote it before responding.

#### Cleanup

When `buffer.id` changes (reader switches publication):

```ts
$effect(() => {
  buffer.id;
  untrack(() => {
    discussions = { events: [], refreshedAt: null, source: null };
  });
});
```

This is mechanically identical to the existing `discussionCounts`
reset effect in `ReaderBuffer.svelte:321-328`.

## Wire format additions — summary

| Endpoint | Method | Status |
|----------|--------|--------|
| `/api/v1/discussions/counts` | POST | exists |
| `/api/v1/discussions/list` | GET | new |
| `/api/v1/discussions/comment` | POST | new |
| `/api/v1/discussions/highlight` | POST | new |
| `/api/v1/discussions/:event_id` | DELETE | new (kind-5) |

No breaking changes to anything that exists.

## Module layout

```
src/
├── discussions.rs           (new — tag construction + cache)
│   ├── pub fn build_comment_template(...) -> EventTemplate
│   ├── pub fn build_highlight_template(...) -> EventTemplate
│   ├── pub fn build_deletion_template(...) -> EventTemplate
│   ├── DiscussionCache
│   └── tests for tag layout against NIP-22 + NIP-84 fixtures
├── api.rs                   (extend with list/comment/highlight/delete handlers)
└── engine.rs                (own DiscussionCache; hook into ingest)

web/src/lib/
├── api.ts                   (add getDiscussionList, publishComment,
│                             publishHighlight, deleteDiscussion)
├── discussions/             (new)
│   ├── thread.ts            (NIP-22 tree builder, ported from
│   │                         SectionComments.svelte:56-133)
│   ├── highlight-position.ts (offset math)
│   └── colors.ts            (pubkeyToHue)
├── components/
│   ├── HighlightCapture.svelte   (new — selection → 9802)
│   ├── CommentThread.svelte      (new — recursive renderer)
│   └── SectionDisclosure.svelte  (new — inline N-comments expander)
└── wm/renderers/
    ├── DiscussionsListBuffer.svelte  (new — buffer kind 'discussions')
    ├── DiscussionViewBuffer.svelte   (extend with reply UI)
    └── ReaderBuffer.svelte           (mount HighlightCapture +
                                       SectionDisclosure)
```

## Build order

1. **`discussions/list` endpoint** — unlocks every read-side UI below
   without any auth or signing concerns.
2. **NIP-22 thread builder** (`web/src/lib/discussions/thread.ts`).
   Pure logic, testable in isolation.
3. **`DiscussionsListBuffer`** — wired to (1) + (2). Now the badges
   become navigable; #1 (read everything) is done.
4. **Inline `SectionDisclosure`** — reuses (2). Reader gains inline
   thread visibility.
5. **`discussions/comment` endpoint** + signing wiring. Unlocks reply
   UI in (3) and (4).
6. **`discussions/highlight` endpoint** + offset-position math +
   `HighlightCapture`. Killer feature; depends only on (5)'s signing
   wiring being clean.
7. **Offset-aware overlay** (upgrades existing renderers). Pulls
   forward Alexandria's `highlightByOffset` algorithm.
8. **Per-author color + drawer.** Cosmetic but high-impact.
9. **`DiscussionCache`** in the engine. Pure perf; can ship anytime
   after (1).
10. **Ignore-list filter** in handlers.
11. **Deletion (kind-5)** + tombstone filter.

(1)–(4) are read-side and ship before any signing changes are
needed. (5)–(7) are the authoring half. (8)–(11) are polish + perf.

## Open questions

- **Where does the comment compose actually live?** Inline-under-section
  is the lowest-friction. A dedicated `compose:comment` buffer would
  be consistent with the WM pattern but is heavier. Inline first,
  graduate to a buffer if the textarea is getting unhappy at large
  sizes.
- **Should highlights without offsets back-fill on read?** If we
  receive a 9802 with no `offset` and the substring matches in
  exactly one place, we could synthesize the offset locally and store
  it client-side. Worth doing if Alexandria-era highlights end up
  noisy.
- **Do highlights count against a section if the only ref is to the
  publication root?** Today `discussions/counts` matches `a` or `A`
  literally against the requested address. A highlight scoped to the
  publication index (kind 30040) doesn't show on any section. Should
  it cascade down to all sections? Alexandria does this in
  `HighlightLayer.svelte:79-113`. Decision needed before (7).
- **Live updates.** Polling on refresh works. A streaming endpoint
  (`GET /api/v1/discussions/subscribe?address=...` → SSE) would let
  the badge increment without a refresh click. Defer until the use
  case is felt; the cache (#7) already makes polling cheap.
