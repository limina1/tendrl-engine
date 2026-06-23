# Event Relay Provenance — design plan

Show, per event, the set of relays it has been seen on or successfully
broadcast to — accumulating across sessions. Unlike Amethyst (where the
"seen on" list is in-memory and resets on app restart), tendrl persists
this set in nostrdb and grows it over time whenever Auto mode brings new
events through.

## Goals

1. The Event View Modal (and any future event-detail surface) displays a
   "Found on" block: every relay that has served or accepted this event
   id, with the local cache shown distinctly from network relays.
2. The set is **append-only** at the per-event level. In Auto mode, every
   relay that delivers an event id contributes; in Confirm mode, only
   user-approved fetches contribute. No background process ever removes
   entries.
3. Every ingest site — inbound fetch, outbound broadcast, profile prefetch,
   address-target fetch, search-relay fetch — records relay provenance.
   No "silent" ingest paths that drop the source relay.
4. The data survives engine restart (nostrdb LMDB) and is queryable from
   the API for any event id without re-fetching.

Non-goals: tracking relay hints from event tags (a separate concept used
for outbound routing, not for "where did we actually find this"); per-
relay timestamps (nostrdb doesn't expose them today — see open questions);
ranking or sorting beyond stable insertion order.

## Reference: how Amethyst does it

Detailed in the survey done against `reference/amethyst/`. Summary:

- **Storage**: `Note.relays: List<NormalizedRelayUrl>` on the in-memory
  `Note` model (`commons/.../model/Note.kt:161`).
- **Capture**: `LocalCache` calls `note.addRelay(relay)` from every
  relay-EVENT consumer. Dedup at insertion (`Note.kt:482-487`).
- **Display**: `RelayBadges` composable
  (`amethyst/.../ui/note/RelayListBox.kt:64-82`) collects a StateFlow of
  `Note.relays`, renders first 3 collapsed / all expanded, each badge
  click → relay info screen.
- **Hints are separate**: a `HintIndexer` tracks relay hints scraped from
  `nevent`/`nprofile`/`naddr` tags, used only for **outbound** routing
  (`Note.relayUrls()` = `authorRelay + relays`). The "seen on" UI shows
  only `relays`, never hints.
- **Session-scoped**: the in-memory `Note` model is rebuilt on every app
  launch. Amethyst does not persist per-event seenOn across sessions.

The key behavioral property tendrl wants to match: **display only relays
that actually served the event**, not aspirational hints.

The key behavioral property tendrl wants to *change*: **persist** the set
so that every Auto-mode fetch monotonically expands what we know.

## Reference: what tendrl already has

The persistence layer is in place. The work is mostly plumbing.

### Capture (write path) — partial coverage today

`nostrdb::IngestMetadata::relay(url)` records a relay against every
ingested event id, deduplicated by id internally. `note.relays(txn)`
reads it back. The data lives in nostrdb's LMDB store, so it persists
across restarts and grows monotonically.

Current ingest sites:

| Site | File | Records relay? |
|---|---|---|
| WebSocket REQ → EVENT consumer | `src/relay.rs:90-95` | Yes (`relay(relay_url)`) |
| Broadcast OK ack recorder | `src/engine.rs:935-943` (`record_event_relay`) | Yes |
| Local-author `ingest_event` | `src/engine.rs:912-924` | No — `client(true)`, no relay (correct: locally authored, never on a relay yet) |
| API generic ingest | `src/api.rs:2462`, `:2770`, `:3016`, `:3043` | No — `engine.ingest_event` only |
| Publication recursive load | `src/publication.rs:2262` | No — `engine.ingest_event` only |

The api.rs ingest paths look like an audit hazard: events coming back
from search/fetch into `ingest_event` won't have their source relay
attributed. **The fix is to route those through `record_event_relay`
when we know the relay**, or document why the relay is unknown there.

### Query (read path) — already wired

`src/query.rs:913-919` (`note_to_value`):

```rust
let relays: Vec<Value> = note
    .relays(txn)
    .map(|r| Value::String(r.to_string()))
    .collect();
// ...
"relays": relays
```

So `GET /api/v1/events/:id` already returns a `relays` array. Every event
fetched through `note_to_value` carries this field. No new backend
endpoint is required for the modal; the data already arrives.

`SearchResult` (`src/search.rs`) deliberately does **not** carry
`relays`. That's fine — search results are summaries; the modal fetches
the full event via `getEvent(id)` when it opens, and that response has
the field. Don't add `relays` to `SearchResult` unless a search-time
display case appears.

### Network mode interaction

`NetworkMode::Auto` (`src/network.rs:29-32`) means relay fetches proceed
without confirmation; `Confirm` gates each one behind a UI modal. This
maps cleanly onto the persistence model the user wants:

- **Auto** — every background sync, profile prefetch, search-relay fetch
  contributes to the seenOn set. The set grows whenever the engine talks
  to a relay successfully.
- **Confirm** — only fetches the user approved contribute. (The local
  cache is still always read; that's not a network fetch.)

The data structure makes no distinction — both modes append the same
way. The difference is purely in *whether* a fetch happens, not in
whether it records.

### UI — gap

`web/src/lib/components/EventViewModal.svelte` is the canonical event
detail surface. Inspection confirms it currently has **no** relay
provenance block. The `event.relays` field is plumbed through many other
places (publication summaries, profile views) but not here. This is the
primary UI gap.

## Design

### Data model: keep it where it is

No new table, no sidecar store. nostrdb's per-event relay set is already
the right structure: append-only, deduped, persistent, indexed by event
id. Everything we add is on top of this.

The set conflates two things on purpose:

- "Relay R served event E to us at least once" (inbound, recorded by
  `relay.rs`)
- "Relay R accepted our broadcast of event E with OK=true" (outbound,
  recorded by `engine.rs:record_event_relay` via api `:2510`, `:2805`,
  `:3695`)

Both mean "R now has E"; both are useful when answering "where can I
find this?". The UI doesn't need to distinguish them. If we ever want
to, the disambiguation belongs on a sibling field (e.g. a separate
"published to" event store), not on this one.

### Local cache as a first-class entry

When an event lives in nostrdb but `note.relays(txn)` is empty, that
means "we wrote it locally and have never seen it on any relay" — a
real and useful state. Surface it as a virtual entry in the UI block,
not as an empty list. The Rust side stays unchanged; the modal renders
"Local cache only" when `relays.length === 0`, plus an explicit "Local
cache" pill alongside the relay chips otherwise.

### Auto-mode growth contract

Document the invariant explicitly:

> While `NetworkMode::Auto` is set, every successful EVENT message from
> any relay records that relay against the event id, including events
> we've seen before. A re-fetch of an event we already have appends new
> relays to the existing set; it never replaces or shrinks it.

This is already how nostrdb behaves. The contract just makes it
guaranteed and testable.

### UI: a new "Found on" block in EventViewModal

Position: between the IDENTIFIERS block and the TAGS block, or after
TAGS — both are reasonable; pick whichever leaves the modal scannable.

Layout:

```
FOUND ON
  ┌──────────────┐ ┌────────────────┐ ┌──────────────────┐
  │ Local cache  │ │ relay.damus.io │ │ relay.noswhere…  │ …
  └──────────────┘ └────────────────┘ └──────────────────┘
  3 relays · last updated when this event was last fetched
```

- One chip per entry. Chronological by insertion order (matches Amethyst
  and matches what nostrdb returns — no sort).
- Local cache renders as its own chip when the event exists locally
  (always true inside the modal — the modal can't open without the
  event), styled distinctly (no host, tinted with `--id-local` or
  similar token).
- Each relay chip is clickable → opens the relay's NIP-11 info via
  whatever route `RelaysBuffer` already uses, or a tooltip if no route
  exists yet.
- Collapsed by default if `relays.length > 5`; expand-all link below.
- An "Add more" affordance: a small button that, on click, kicks off a
  fetch of this event id against the configured fetch relays. Subject
  to NetworkMode — Auto runs it, Confirm raises an intent. On success,
  the modal refreshes the event (chained nav already does this) and
  the new relays appear. **This is the user-facing realization of
  "auto mode keeps adding"**.

Empty-state copy: "Only in local cache — not seen on any relay yet."

### Capture: close the audit holes

For every `engine.ingest_event(...)` call site that *does* know the
source relay, switch to `record_event_relay(..., relay_url)`. Audit
list:

- `src/api.rs:2462` — used by `POST /api/v1/events`. Often the caller
  is publishing locally; relay url is not known at that point. Leave
  as-is, but verify the broadcast paths that follow (`:2510`, `:2805`,
  `:3695`) record once OK comes back. **Confirmed**: they do.
- `src/api.rs:2770`, `:3016`, `:3043` — same shape. Audit each
  individually.
- `src/publication.rs:2262` — this is the recursive publication loader.
  When it descends into a child event fetched from a relay, the relay
  url is available in the calling context. **This site should pass it
  through.** Likely the biggest real fix.

Add an audit test in `src/engine.rs` test module: after a fetch from a
fake relay, assert `note.relays(txn)` contains that relay url.

### API: no new endpoints

`GET /api/v1/events/:id` already returns `relays`. Web reads it.
That's the whole API contract for v1.

A possible future endpoint `GET /api/v1/events/:id/relays` is a
convenience (smaller payload) but not needed for the modal — it always
has the full event in hand. **Defer.**

### Web types

`NostrEvent` and the response from `getEvent` already have a place for
`relays?: string[]`. Verify the type definition in
`web/src/lib/types.ts` and add `relays?: string[]` to `NostrEvent` if
absent. The modal's `normalize()` function then surfaces it on the
`Normalized` shape; render it from there.

`SearchResult` stays without `relays` — the modal fetches the full
event via `api.getEvent(id)` when opened (it already does this for the
"chained nav" path), and that response carries the relays. No new
field on the search wire format.

## Phasing

Each phase ships independently.

**Phase A — capture audit & test.** Walk every ingest site, record the
relay url where it's known, leave it unset where it isn't. Add the test
that asserts the relay url lands on `note.relays(txn)` after a real
relay fetch round-trip. No UI changes. This is the foundation for B —
no point displaying the set if half the ingest paths skip it.

**Phase B — Modal "Found on" block.** Add the block to
`EventViewModal.svelte`. Render local-cache + relay chips. No "Add more"
button yet; the existing modal already refreshes on chained-nav, so a
manual refresh round-trip works in practice. Wire chip clicks to the
relay info surface (or tooltip stub if none exists).

**Phase C — "Add more" affordance.** A button that triggers a re-fetch
of this event id against fetch relays, threaded through `NetworkMode`.
Auto runs it silently; Confirm raises the standard fetch-intent modal
listing this event id as the target. On completion, the modal
re-fetches the event and re-renders. Closes the "auto mode keeps
adding" UX loop: any user who wants to expand the seenOn set right now
has a one-click way to do it.

**Phase D — Documentation pass.** Update `docs/data-lifecycle-roadmap.org`
to note the persistent seenOn invariant. Update CLAUDE.md "Key Patterns"
to mention the `IngestMetadata::relay` contract — currently only
`relay.rs:88` comments on it. Add a row to whatever event-shape table
exists describing the `relays` field.

Phases A and B together are MVP. C is the "feels great" addition. D is
hygiene.

## Open questions

1. **Per-relay timestamps**: nostrdb stores `(event_id, relay)` pairs
   but no "first seen at" or "last seen at" per pair (verify by reading
   the nostrdb crate — the doc comment we have only describes the set
   API). If we ever want "this relay served it 3 minutes ago", we need
   an external table. Out of scope for v1; leaves the door open.
2. **Drop-on-eviction semantics**: nostrdb has its own retention rules.
   If nostrdb evicts an event id under storage pressure, the seenOn set
   for that id is lost with it. That's fine — the set is meaningless
   without the event itself — but worth noting in the data-lifecycle
   doc.
3. **Relay normalization**: `relay.rs` records the raw URL the engine
   connected to. Amethyst normalizes to a canonical form
   (`NormalizedRelayUrl`). If two configured relays differ only in
   trailing slash / case, they'd appear as separate chips. Likely
   harmless for now; document if it becomes a UX issue.
4. **Outbound disambiguation**: do users care that
   `wss://relay.damus.io` in the chip strip might mean "we received it
   from there" versus "we published it there and they ack'd"? My read
   is no — both mean "the event is at that address" — but if user
   testing says otherwise, the disambiguation goes on a separate sibling
   field, not on this one. See "Data model" above.
5. **Confirm-mode ergonomics for "Add more"**: should the intent the
   button raises list the relays it's about to try, so the user can
   uncheck some? `FetchConfirmModal.svelte` already takes a relay
   override (`resolveConfirm(true, relays)`), so the plumbing exists.
   Decision: yes, pass the full relay set; user can prune.

## Files touched (estimate)

Rust:
- `src/engine.rs` — `record_event_relay` already exists; audit
  callers, add a test.
- `src/publication.rs:2262` — switch to relay-attributed ingest in the
  recursive loader.
- `src/api.rs` — audit the four `ingest_event` sites; rewire those that
  know their relay.
- No new endpoints.

Web:
- `web/src/lib/types.ts` — verify/add `relays?: string[]` on `NostrEvent`.
- `web/src/lib/components/EventViewModal.svelte` — new "Found on" block,
  ~60 lines including styles.
- `web/src/lib/state.svelte.ts` (Phase C only) — "refresh this event from
  relays" action that threads through NetworkMode.

Docs:
- `docs/data-lifecycle-roadmap.org` — persistent seenOn invariant.
- `docs/event-view-modal-plan.md` — add a v3 addendum referencing the
  new block, so future readers find it.

## Acceptance

- Opening any non-local event in the modal shows at least one relay
  chip if the engine has ever fetched it from a relay.
- Opening a locally-authored draft that's never been published shows
  "Local cache only".
- Restarting the engine and re-opening the same event shows the same
  relay chips — no in-memory loss.
- Fetching the same event again in Auto mode from a different relay
  adds a new chip on next modal open (no shrink, no replace).
- Test in `engine.rs` proves a fetched event lands on
  `note.relays(txn)` with the expected url.
