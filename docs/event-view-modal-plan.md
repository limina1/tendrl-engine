# Event View Modal — design plan

Bring Alexandria's interactive event inspector to tendrl's "View JSON" modal:
clickable tags/identifiers and an automatic "Containing Publications" list.

## Goals

1. Replace the flat `JSON.stringify` dump in the View JSON modal with a
   structured inspector: header, identifiers, tags, containing publications,
   raw JSON (collapsible).
2. Make every tag and identifier in the modal a hyperlink that does the
   right thing inside tendrl's search/reader flow.
3. Automatically surface kind-30040 indexes that reference the current event,
   so the user can jump from a section back to the publication that contains it.

Non-goals: editing events, navigating to external Nostr clients via bech32,
restyling the rest of the search UI.

## Reference: how Alexandria does it

Files (in `reference/gc-alexandria/`):

- `src/lib/components/EventDetails.svelte` — the parent component shown for
  every event in `/events`. Contains three relevant blocks.
- `src/lib/components/util/ContainingIndexes.svelte` — drop-in component that
  renders "Containing Publications" buttons.
- `src/lib/components/util/ViewPublicationLink.svelte` — "View Publication"
  button shown only when the event *itself* is the publication (or defers
  to one). Different feature; not part of this plan but worth noting.
- `src/lib/utils/event_search.ts:findContainingIndexEvents` — relay query
  that powers `ContainingIndexes`.

### Identifiers block (`EventDetails.svelte:270-315`, rendered `:463-499`)

`getIdentifiers(event)` returns a list of `{label, value, link?}` rows. For
kind 0: npub, nprofile, nevent, hex pubkey. For everything else: nevent,
naddr (if addressable), hex id. Each row renders as a `<button>` that
navigates to `/events?id=<encoded>`, plus a copy-to-clipboard control.

Encoding helpers live in `src/lib/utils/index.ts`: `neventEncode`,
`naddrEncode`, `nprofileEncode`, `toNpub`. All return bech32 strings.

### Tags block (`EventDetails.svelte:94-225`, rendered `:500-520`)

`getTagButtonInfo(tag)` is a per-tag switch:

| Tag | Treatment |
|-----|-----------|
| `a` | Decode `kind:pubkey:d` triplet → re-encode as `naddr`. |
| `e`, `note`, `q` | Validate hex64 event id → encode as `nevent`. |
| `p` | `toNpub(pubkey)`. |
| `d` | Pass through as `d:<value>` (search key, not encoded). |
| `t` | Pass through as `t:<value>` (hashtag search). |
| anything else | Renders as `{key}:{value}`, no link. |

Output flows through `handleTagGoto()` (`:227-251`), which routes by prefix:

- `naddr…` / `nevent…` / `npub…` / `nprofile…` / `note…` → `/events?id=<v>`
- `d:<x>` → `/events?d=<x>`
- `t:<x>` → `/events?t=<x>`
- bare hex64 → `navigateToEvent(value)`
- anything else → `/events?id=<v>`

### Containing publications block (`ContainingIndexes.svelte`)

Always rendered inside `EventDetails` (`:378`). On mount and whenever the
event id changes, calls `findContainingIndexEvents(event, ndk)`:

```ts
// event_search.ts:289
const contentEventKinds = [30041, 30818, 30040, 30023];
if (!contentEventKinds.includes(contentEvent.kind!)) return [];

const indexEvents       = await ndk.fetchEvents({ kinds:[30040], "#a":[event.tagAddress()] });
const indexEventsWithE  = await ndk.fetchEvents({ kinds:[30040], "#e":[event.id]         });

// dedupe, require title + d + (a-tags || e-tags)
```

Renders up to 3 title buttons. Click → `/publication/d/<dtag>` (fallback
`/publication/naddr/<naddr>`). No filtering by author — any author's index
that references this event counts.

## Reference: what tendrl already has

- **View JSON modal**: `web/src/routes/+layout.svelte:137-148`. Renders
  `<pre>{JSON.stringify(app.jsonModalData, null, 2)}</pre>` inside a fixed
  modal. Opened by `handleViewJson` in `web/src/lib/state.svelte.ts:955`,
  which fetches the full event via `api.getEvent(id)`.
- **Find-containing flow**: `web/src/routes/+layout.svelte:70-90` already
  defines `onFindContaining(r)`. It builds `a:<kind>:<pubkey>:<d_tag>` and
  routes through `app.handleSearch(query, { scopeToMe:false })`, then forces
  the search slot visible. Currently wired only to the
  `SearchActionModal` ("Find containing publications" row,
  `SearchActionModal.svelte:46`), not to the JSON modal. Notably it does
  **not** add `kind:30040` — it surfaces any event that references the
  address, not just publication indexes.
- **Search backend**: `src/search.rs`. `TagFilter { tag_name, values }` is
  generic over single-letter tags; `a:<v>` and `t:<v>` and `#x:<v>`
  tokens all become `{"#a":[v]}` / `{"#t":[v]}` / etc. via
  `to_nip01_filters` (`:232`). `kind:N` is a separate `kind_filter`.
- **Event API**: `web/src/lib/api.ts:114` `getEvent`, `:118` `queryEvents`
  (arbitrary NIP-01 filters), `:261` `search` (string query).
- **Search result hooks**: `handleViewJson` and `handleSearch` are both on
  `app`, plus a `getActiveStore()` helper for opening WM buffers.

## Design

### Component split

Extract the modal out of `+layout.svelte` into
`web/src/lib/components/EventViewModal.svelte`. The layout keeps the
`{#if app.jsonModalData}` mount point but renders the new component.

The modal becomes structured:

```
EventViewModal
├─ Header        : title (or fallback), kind label, author (ProfileName)
├─ Identifiers   : id, addr (if any), nevent/naddr/npub (encoded) — copy + click
├─ Tags          : every tag as a chip; clickable when we can map it
├─ Containing    : list of kind-30040 indexes (lazy / auto on open)
└─ Raw JSON      : collapsed by default; toggle to expand
```

The modal accepts an `event: NostrEvent` (the same payload that currently
lands in `jsonModalData`). It must work for events fetched via
`api.getEvent` as well as raw `SearchResult` fallbacks — both paths exist
in `handleViewJson`.

### Tag → action mapping (tendrl-flavored)

Tendrl does not navigate by bech32 inside its own UI; it routes everything
through `handleSearch` and `spawnReader`/`spawnEventReader`. Skip bech32
encoding entirely. Map tags to actions instead:

| Tag | On click |
|-----|----------|
| `a` (kind:pubkey:d) | If `kind=30040` → open publication via `spawnReader(pubkey, d)`. Else `handleSearch('a:<aRef>', { scopeToMe:false })`. |
| `e` / `q` / `note` (hex64) | `handleViewJson({ event_id })` — chain to the referenced event by fetching it and replacing the modal contents. |
| `p` (hex64 pubkey) | `handleSearch('by:<npub>')` — or open profile if a profile route exists. |
| `d` | `handleSearch('#d:<value>')`. |
| `t` | `handleSearch('#t:<value>')`. |
| anything else where value looks like a single token | `handleSearch('#<key>:<value>')` (best-effort). |
| anything else | Render as plain `key:value`, no link. |

Validation: hex64 is `/^[0-9a-fA-F]{64}$/`, the same regex Alexandria uses.
Treat the `a` triplet as `kind:hex64:d_tag`. If a tag fails validation,
fall through to plain rendering — never crash the modal.

This keeps the click contract internal. We can always add bech32 export
later behind a copy-to-clipboard menu (Alexandria's `CopyToClipboard`
already exists conceptually in the existing modal's "Close" UX area).

### Identifiers block

For tendrl's purposes the identifiers users actually need are:

- `id` (hex64 event id) — copyable, click opens the event in the modal
  (chained `handleViewJson`).
- `addr` (`kind:pubkey:d_tag`) — only when `event.tags` contains a `d` tag.
  Click runs `handleSearch('a:<addr>')`.
- `author` (npub) — click runs `handleSearch('by:<npub>')`.

We can skip nevent/naddr/nprofile bech32 unless there's a downstream
consumer that needs them. If we ever want them, encode client-side with
`nostr-tools` (already a transitive dep via NDK in Alexandria — tendrl
would need to add it; see "Open questions").

### Containing publications

Add a new method `findContainingIndexes(event)` on `app` (in
`state.svelte.ts`). Reuses `api.queryEvents` with NIP-01 filters; runs
two queries in parallel and dedupes:

```ts
async function findContainingIndexes(event: NostrEvent) {
  const allowed = new Set([30041, 30818, 30040, 30023]);
  if (!allowed.has(event.kind)) return [];

  const aRef = addressOf(event);   // `${kind}:${pubkey}:${d_tag}` if d-tag
  const [byA, byE] = await Promise.all([
    aRef
      ? api.queryEvents([{ kinds:[30040], "#a":[aRef] }], 'local_first')
      : Promise.resolve({ events: [] }),
    api.queryEvents([{ kinds:[30040], "#e":[event.id]  }], 'local_first'),
  ]);

  const seen = new Set<string>();
  return [...byA.events, ...byE.events]
    .filter(e => !seen.has(e.id) && seen.add(e.id))
    .filter(isValidIndex);     // has title + d + (#a || #e)
}
```

Trigger automatically when the modal opens, with a `loading | error | data`
state. Cache by event id within the modal lifetime to avoid re-fetching
on re-render (Alexandria gates this with `lastEventId`, same trick).

Render each result as a button. Click → `spawnReader(pubkey, d_tag, title)`
(same call `onReadEvent` uses for 30040s in `+layout.svelte:62`).
Show first N (3 in Alexandria; 5 here is fine — modal has more room than
an inline section).

### Reusing the existing `onFindContaining`

The `onFindContaining` handler in `+layout.svelte` already does the right
broad query. **Keep it as the "see everything that references this event"
escape hatch** — wire a "Show all references" link at the bottom of the
Containing Publications section that calls `onFindContaining(result)` to
populate the search buffer (the alexandria-style auto-list shows only
indexes; the search buffer shows quotes, replies, etc.).

Recommended: also tighten `onFindContaining` to include `kind:30040` when
it's invoked from the Containing block; leave the SearchActionModal entry
broad. Implementation: pass an optional `{ indexesOnly?: boolean }`
parameter.

### State plumbing

`jsonModalData` is currently typed `unknown`. Tighten to
`NostrEvent | { buffer: ... } | null` and let the modal narrow at the
boundary. The existing PublishProgressBuffer (`:60`) and `+page.svelte`
(`:282`) callers always set either a raw event or a buffer object, so
this is safe.

Add `containingIndexes` state on `app`:

```ts
let containingIndexes = $state<{ event_id: string; loading: boolean;
                                 error: string | null;
                                 indexes: NostrEvent[] } | null>(null);
```

Keyed by event id so opening a chained modal correctly resets state.

## Implementation order

1. **Extract modal** to `EventViewModal.svelte` rendering the existing
   `<pre>` JSON dump inside a `Raw JSON` section. No behavior change.
   Verify all three open-paths (`handleViewJson`, PublishProgressBuffer,
   `+page.svelte:282`) still work.
2. **Identifiers block** — id / addr / author rows, click → `handleSearch`
   or `spawnReader`. Add `CopyToClipboard` controls.
3. **Tags block** — port `getTagButtonInfo`, map to tendrl actions per the
   table above. Non-mappable tags render as plain chips.
4. **Containing publications** — `findContainingIndexes` on `app`,
   auto-load on modal open, render up to 5 buttons, "Show all references"
   link wired to `onFindContaining(..., { indexesOnly: false })`.
5. **Chained navigation** — clicking an `e`/`q`/`note` tag replaces modal
   contents with the referenced event (call `handleViewJson({ event_id })`
   from inside the modal). Add a tiny back-stack (array of seen ids) for
   `Esc`-to-pop behavior.

Steps 1–4 each ship independently. Step 5 is nice-to-have; defer if it
costs too much.

## Open questions

1. **Bech32 encoding**: do we ever need to display npub/naddr/nevent
   strings in the modal (for copying out to other Nostr clients)? If yes,
   add `nostr-tools` (or implement bech32 in the existing utils — it's
   ~150 lines). If no, skip and keep hex/triplet throughout.
2. **Profile route**: is there a per-profile buffer/route in the WM? If
   so, `p`-tag clicks should open it instead of running a `by:` search.
3. **Auto-load policy**: `local_first` matches the rest of the app, but
   for indexes that almost always live on relays, `fetch_always` might be
   needed. Pick `local_first` initially; revisit if results feel stale.
4. **Modal lifetime**: should the back-stack persist across `Close`, or
   reset every time the modal opens? Reset is simpler and matches the
   "modal == single drill-down" mental model.

## Files touched (estimate)

- `web/src/lib/components/EventViewModal.svelte` — new, ~250 lines.
- `web/src/routes/+layout.svelte` — replace inline modal block with
  `<EventViewModal />`. Loosen `onFindContaining` signature to take an
  options bag.
- `web/src/lib/state.svelte.ts` — add `findContainingIndexes`,
  `containingIndexes` state, tighten `jsonModalData` type.
- `web/src/lib/api.ts` — no change (uses existing `queryEvents`).
- `src/search.rs`, `src/api.rs` — no change. The Rust backend already
  exposes everything we need.
