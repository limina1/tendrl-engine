# Discussion authoring (NIP-22 comments + NIP-84 highlights) — design spec

**Status:** IMPLEMENTED (2026-07-09, `feature/discussion-authoring`).
Every judgment call in this doc went through
`docs/discussions-authoring-worksheet.org` and was decided there; the
worksheet records the options and rationale, this doc records the
outcomes. Engine half curl-verified against a sandboxed engine; web half
driven end-to-end in the embedded SPA (headless CDP), including the
offset-pinned duplicate-phrase case.
This is the authoring half of
`docs/discussions-plan.md` — the read side of that plan has shipped
(`src/discussions.rs::build_thread` / `group_threads_by_address` /
`resolve_highlight_spans`, `POST /discussions/list|counts`,
`POST /highlights/resolve`, `CommentThread.svelte`, `HighlightsDrawer.svelte`).
This doc supersedes the plan's §2 ("authoring endpoints"): the tag layouts
there copied two Alexandria bugs verbatim (see *Lessons* below); the layouts
in **this** doc are checked against `nips/22.md` and `nips/84.md` and are the
source of truth.

Companion analysis: `reference/gc-alexandria/doc/publication_creation.md`
§2–3 documents Alexandria's own creation pipelines in detail.

Scope: not just publications. NIP-22 makes kind 1111 the comment layer for
**every** event kind except kind 1 (which keeps NIP-10), plus external
content ids (NIP-73: URLs, ISBNs, DOIs, podcasts, geohashes). §3.4 defines
the universal target routing; §5.4 maps the UI surfaces where the write
affordance attaches.

---

## 1. How Alexandria creates them (findings)

### Comments (kind 1111)

Alexandria has **two parallel comment subsystems** and builds the same
kind-1111 event inline in at least four places with diverging tag shapes:

- *Publication reader path*: `CommentButton.svelte:115-130`,
  `CardActions.svelte:301-312`, `Publication.svelte:787-794` (top-level),
  `SectionComments.svelte:369-386` (replies).
- *Generic event/profile path*: `CommentBox.svelte` →
  `nostrEventService.buildReplyTags` (`nostrEventService.ts:199-310`) — the
  more spec-faithful one.

Common shape: root scope = uppercase `A`/`K`/`P` pointing at the section
(30041) or publication (30040) **coordinate**; parent scope = lowercase
`a`/`e`/`k`/`p`. Replies keep `A` at the section and thread purely via a
lowercase `e` → parent-comment-id chain. Fetching uses one
`{kinds:[1111], "#A": [addresses]}` filter (uppercase-A index) so a whole
publication's comments — including nested replies — come back in one query.
Threading is rebuilt client-side: a comment is a reply iff it has a
lowercase `e` whose value is the id of another comment in the fetched set
(`SectionComments.svelte:56-133`).

### Highlights (kind 9802)

One component pair: `HighlightSelectionHandler.svelte` (capture + publish)
and `HighlightLayer.svelte` (fetch + render).

- Capture: document-level `mouseup` → `window.getSelection()`, reject
  collapsed / <3-char selections. The owning section is found via
  `target.closest('section[id]')` carrying `data-event-address` /
  `data-event-id`.
- Event: `content` = the selected text verbatim; tags = `["a", sectionAddr]`
  (or `["e", id]` fallback), optional `["context", enclosingParagraphText]`,
  `["p", author, "", "author"]`, optional `["comment", annotation]` (quote
  highlight — the annotation lives **inside** the 9802, no separate 1111).
- **No position data is written** — no offsets, no Range serialization.
  Re-anchoring on display is a multi-strategy fuzzy string match
  (`HighlightLayer.svelte:358-909`: normalized substring → exact →
  case-insensitive → flexible-whitespace regex, first occurrence only) with
  a 10-retry backoff loop for async-loaded content, mutating the DOM with
  injected `<mark>` elements.
- Per-author color: `hsla(pubkeyToHue(pk),70%,60%,.5)`; a fixed drawer
  groups highlights by author with click-to-scroll.

### Lessons — keep / avoid

Keep:
- The uppercase-`A` root-scope index (one filter fetches a publication's
  whole discussion graph, replies included).
- Content = selected text; `context` tag for disambiguation; `comment` tag
  for quote highlights (per NIP-84, avoids a second event).
- Per-author deterministic color shared between overlay and drawer.

Avoid (all present in Alexandria, first two copied into the old plan §2):
1. **Malformed reply parent coordinate** — `["a", "1111:<pubkey>:"]`
   (`SectionComments.svelte:380`). Kind 1111 is not addressable; a reply's
   parent scope is `e`/`k`/`p` only. Threading there survives only because
   the `e` tag is also emitted.
2. **Non-standard 4-element `A` tag** — `["A", addr, relay, authorPubkey]`.
   Per NIP-22 only `E`/`e` carry a pubkey in position 4.
3. **N duplicated inline tag builders.** Tendrl builds tags in exactly one
   place, in Rust (the frontend/backend boundary rule: payload emission is
   engine-owned).
4. **`setTimeout(refresh, 3000)` after posting** to wait for relay indexing
   (`Publication.svelte:745-754`). Tendrl's engine ingests locally before
   responding, so the follow-up read is a sub-50ms `local_only` call.
5. **Per-surface relay policy** (three publish paths, three relay sets).
   Tendrl publishes to the engine's `publish` relay set (`relay_store.rs`),
   one policy for every event kind.
6. **Write-path/read-path divergence** — Alexandria's reader honors
   `offset` tags that its own writer never emits. Tendrl's writer emits
   offsets and its resolver consumes them (§4).

---

## 2. What tendrl already has (build on, don't rebuild)

| Piece | Where |
|---|---|
| Thread forest builder (NIP-22, pure) | `src/discussions.rs::build_thread`, `group_threads_by_address` |
| Discussion fetch (events + threads) | `POST /api/v1/discussions/list` (`api.rs::discussions_list_handler`), defaults `kinds=[1111,9802]` |
| Per-address counts | `POST /api/v1/discussions/counts` |
| Highlight span resolution (UTF-16, engine-side) | `src/discussions.rs::resolve_highlight_spans` + `POST /api/v1/highlights/resolve` |
| Overlay renderer | `RichContent.svelte` + `buildSegments` (`web/src/lib/nostr/nostrdown.ts:103`) |
| Thread renderer (read-only) | `CommentThread.svelte` |
| Drawer / per-section lists (read-only) | `HighlightsDrawer.svelte`, `HighlightList.svelte` |
| Generic sign seam | `SigningController` (`signing.rs`) — in-process, NIP-07, NIP-46 via SSE round-trip |
| Sign + broadcast endpoints | `POST /api/v1/identity/sign`, `POST /api/v1/broadcast` |
| Web one-shot publish helper | `signAndBroadcast(template)` (`web/src/lib/identity/signer.ts:184`) |

The only genuinely missing pieces are: template construction for
1111/9802/5, two-and-a-half endpoints, selection capture, and reply UI.

---

## 3. Engine: template builders + endpoints

### 3.1 Module layout

Template builders live in `src/discussions.rs` next to the thread/span
logic they mirror — pure functions from typed inputs to `EventTemplate`
(`signing.rs:63`), unit-tested against the `nips/22.md` / `nips/84.md`
fixtures. Handlers in `api.rs` compose: **build → sign → ingest → broadcast
→ respond**.

```rust
pub struct DiscussionTarget {
    pub address: Option<String>,   // "kind:pubkey:d" — preferred when addressable
    pub event_id: Option<String>,  // pins the exact version (offsets, §4)
    pub kind: u32,
    pub pubkey: String,            // target author
    pub relay_hint: Option<String>,
}

/// What a comment is scoped to — any Nostr event, or a NIP-73 external id.
pub enum CommentScope {
    Event(DiscussionTarget),
    External { id: String, kind: String, hint: Option<String> }, // I/i + K/k values
}

pub fn build_comment_template(
    root: &CommentScope,
    parent: Option<&ParentComment>,   // id + pubkey + kind of the event being replied to
    content: &str,
    created_at: i64,
) -> EventTemplate;

/// Derive the root scope for a reply by chasing the parent's uppercase tags.
/// A 1111 parent already carries A/E/I + K + P — copy them verbatim; the
/// parent itself was never the root. A non-1111 parent IS the root.
pub fn root_scope_from_parent(parent_event: &Value) -> CommentScope;

pub fn build_highlight_template(
    target: &DiscussionTarget,
    content: &str,                    // the selected text, verbatim
    offset: Option<(u64, u64)>,       // UTF-16 units into the pinned version
    context: Option<&str>,
    comment: Option<&str>,
    created_at: i64,
) -> EventTemplate;

pub fn build_deletion_template(
    event_id: &str, event_kind: u32, address: Option<&str>, reason: &str,
    created_at: i64,
) -> EventTemplate;
```

### 3.2 Tag layouts (normative)

**Top-level comment** on an addressable target `A = k:pk:d`
(section 30041, publication index 30040, wiki 30818, long-form 30023 —
same shape for all):

```
["A", "k:pk:d", "<relay>"]          root scope = the target coordinate
["K", "<k>"]
["P", "<pk>", "<relay>"]
["a", "k:pk:d", "<relay>"]          parent scope = same as root
["e", "<target event id>", "<relay>", "<pk>"]   when the id is known
["k", "<k>"]
["p", "<pk>", "<relay>"]
content: plaintext (NIP-22: no markdown/HTML)
```

The `e` tag is filled from nostrdb (`get_addressable`, `local_only` — never
a surprise network fetch inside a publish handler); omitted if the target
event isn't cached locally.

**Reply to a comment** (parent = 1111 event `<pid>` by `<ppk>`; root
unchanged):

```
["A", "k:pk:d", "<relay>"]          root scope unchanged
["K", "<k>"]
["P", "<pk>", "<relay>"]
["e", "<pid>", "<relay>", "<ppk>"]  parent = the comment
["k", "1111"]
["p", "<ppk>", "<relay>"]
```

No lowercase `a` (1111 is not addressable) and no NIP-10 `"reply"` marker
(NIP-22 doesn't use markers) — both deliberate departures from Alexandria
and from the old plan §2. Replies-to-replies are the same shape with a
different `<pid>`; `build_thread` already reconstructs arbitrary depth
from the lowercase-`e` chain. The root scope of a reply is **copied from
the parent's uppercase tags** (`root_scope_from_parent`), never
re-derived — so a reply chain started under a 30023 stays scoped to that
30023 even if the replier's client never saw the article.

**Comment on a regular (non-addressable) event** — a highlight (9802), a
file (1063), a picture (20), another 1111, any kind without a `d` tag:

```
["E", "<event id>", "<relay>", "<pk>"]
["K", "<kind>"]
["P", "<pk>", "<relay>"]
["e", "<event id>", "<relay>", "<pk>"]
["k", "<kind>"]
["p", "<pk>", "<relay>"]
```

Commenting **on a highlight** is this shape with `K = 9802`: the highlight
is the root, not the section it highlights — the discussion is about the
annotation. (The highlight's own inline annotation stays a `comment` tag
per NIP-84; a *thread under* a highlight is 1111s rooted at it.)
Validated against Amethyst's wire format (worksheet A7 carries a captured
example): `E`/`K`=9802/`P` root scope, `e/k/p` parent at the same
highlight for a top-level comment. Because such replies carry no section
address, the read side compensates with a **second hop**: after
`discussions/list` fetches a section's 1111s + 9802s, it also fetches
1111s whose `#E` points at the returned highlight ids, and merges them —
so highlight-rooted threads still appear in section views and badge
counts.

**Comment on a replaceable event** (kind 0, 3, 10000–19999): same as the
addressable layout with the empty-`d` coordinate `"k:pk:"`.

**Comment on an external id** (NIP-73 — URL, ISBN, DOI, podcast GUID,
geohash, hashtag):

```
["I", "<external id>", "<url hint>"]
["K", "<id kind>"]            e.g. "web", "isbn", "doi", "podcast:item:guid"
["i", "<external id>", "<url hint>"]
["k", "<id kind>"]
```

No `P`/`p` — there is no Nostr author. The id **must** be normalized per
NIP-73 (URLs stripped of fragments/trackers, ISBNs without hyphens, DOIs
lowercase) — normalization is engine-side (`nip73.rs` or a
`discussions.rs` helper), since two clients disagreeing on normalization
split the thread. For a reference manager this is the quietly important
case: a comment scoped to `["I", "doi:10.1038/..."]` is a discussion
attached to *the paper*, portable across every Nostr client, independent
of any publication event that embeds it.

**Comment on a kind-1 note** — same regular-event shape (`E`/`K`=1/`P` +
`e/k/p`), i.e. **1111 for kind 1 too**. This deliberately follows the
ecosystem's direction (worksheet A5: "Nostr is moving to kind 1111 for
all comments") over the vendored NIP-22's "MUST NOT be used to reply to
kind 1 notes". Known cost, accepted: clients that only implement NIP-10
threading won't show these replies under the note; that cost shrinks as
the convergence proceeds, and no NIP-10 template code exists to maintain.
(Kind-1-adjacent legacy comment kinds — 1244 voice, 2004 torrent — are
likewise ignored; 1111 is the one general layer.)

**Highlight** on section `30041:pk:d`, version `<eid>`:

```
content: <selected text, verbatim slice of the section content>
["a", "30041:pk:d", "<relay>"]
["e", "<eid>", "<relay>"]           pins the content version offsets refer to
["p", "<pk>", "<relay>", "author"]
["k", "30041"]                      extension, harmless; aids kind filtering
["offset", "<start>", "<end>"]      tendrl extension — see §4
["context", "<enclosing paragraph>"]  optional (NIP-84)
["comment", "<annotation>"]           optional → quote highlight (NIP-84)
```

Highlights always tag the **section** (30041) the selection lives in, never
the 30040 root — the capture UI knows the section, so the
publication-root→section cascade in `ReaderBuffer.highlightsForSection` is
kept only for legacy/foreign events. Quote-highlight extras (NIP-84:
`p`-mentions and `r`-urls inside the comment get a `"mention"` marker) are
out of scope for v1 — we don't parse the annotation.

**Deletion** (NIP-09 kind 5):

```
["e", "<event id>"]
["a", "<coordinate>"]               when the target had one (not for 1111/9802)
["k", "<kind>"]
content: <reason>
```

### 3.3 Endpoints

```
POST /api/v1/discussions/comment
{
  "root":    { "address": "k:pk:d" }
           | { "event_id": "<hex>", "kind": n, "pubkey": "<pk>" }
           | { "external": "<nip-73 id>", "id_kind": "web|isbn|doi|...", "hint": "https://..." },
  "parent":  { "event_id": "<hex>", "kind": n, "pubkey": "<pk>" },  // optional; omit = top-level
  "content": "...",
  "relays":  ["wss://..."]                                 // optional broadcast override
}
→ 200 { "event": <signed 1111>,
        "broadcast": { "successful": n, "total": m, "results": [...] } }

POST /api/v1/discussions/highlight
{
  "target":  { "address": "30041:pk:d", "event_id": "<hex>" },
  "content": "<selected text>",
  "offset":  [start, end],          // optional but the web always sends it
  "context": "...",                 // optional
  "comment": "...",                 // optional
  "relays":  ["wss://..."]
}
→ 200 { "event": <signed 9802>, "broadcast": {...} }

POST /api/v1/discussions/delete
{ "event_id": "<hex>", "reason": "..." }
→ 200 { "event": <signed kind-5>, "broadcast": {...} }
```

Handler flow, identical for all three:

1. Validate (non-empty content; comment ≤ some sane cap; for delete:
   target exists locally and `target.pubkey == active identity pubkey`).
2. `build_*_template(...)` with `created_at = now`.
3. `SigningController::sign(template)` — NIP-07/NIP-46 transparently
   round-trip through the existing SSE signer channel. No active identity →
   `401`; signer timeout → `504`.
4. **Ingest into nostrdb** before responding — local-first: the event is
   queryable by the next `discussions/list?policy=local_only` call even if
   every relay rejects it.
5. Broadcast to the `publish` relay set (`relay_store.rs`), or the request
   override. This is the same fan-out `broadcast_handler` uses. Publishing
   is a deliberate user action — `NetworkMode::Confirm` gates *fetches*,
   not this.
6. Respond with the signed event + per-relay results (same
   accepted/rejected shape publications render).

Why dedicated endpoints instead of the web calling
`signAndBroadcast(template)` with tags built in TS: the boundary rule.
Tag/coordinate emission is algorithmic event derivation — it lives in Rust,
once, with tests, and any future frontend gets it for free. The generic
sign/broadcast endpoints stay as the escape hatch they already are.

`discussions_list_handler` additionally learns to drop events tombstoned by
a local kind-5 from the same pubkey (tombstone filter), and to drop
ignore-listed pubkeys — both at that single chokepoint. The tombstone
filter is mandatory, not belt-and-suspenders: nostrdb never physically
removes events (worksheet B4), so without it a deleted comment would
reappear on every query forever. Deletion is also surfaced as a
*universal* affordance — EventViewModal Actions → `(d) delete` for any
own event — with the `relays` override for targeted broadcast; the
per-comment kebab (§5.3) is the same endpoint.

Parent handling is hybrid (worksheet B1): the request carries the parent
id *and* may carry the full parent event JSON; the handler prefers its
own nostrdb copy (`local_only` lookup) and falls back to the supplied
event when the lookup misses — so "Reply failed: event not found" can't
happen on a comment the user is literally reading. Either way the engine
calls `root_scope_from_parent` and builds from there — the web never
assembles root scopes.

Relay hints (tag position 3) are filled with the first configured
`general` relay from `relay_store.rs`, empty string when none (worksheet
B2) — a hint should name a relay where the *target* is readable, which
is the fetch side, not the publish set. Upgrade to real per-event
provenance only if the engine grows that tracking for other reasons.

### 3.4 Target routing — one rule table

`build_comment_template` dispatches on the root's nature; this table is
the whole decision surface (and the unit-test matrix):

| Root target | Root scope tags | Kind produced |
|---|---|---|
| Addressable event (30000–39999): 30040/30041, 30023, 30818/30817, 30402, 31922… | `A` = `k:pk:d`, `K`, `P` (+ `e` version pin when cached) | 1111 |
| Replaceable event (0, 3, 10000–19999) | `A` = `k:pk:` (empty d), `K`, `P` | 1111 |
| Regular event, incl. kind 1: 9802, 1111, 1, 1063, 20, 21/22, 1222… | `E` = id (+pk in pos 4), `K`, `P` | 1111 |
| External id (NIP-73): web / isbn / doi / geo / podcast / `#hashtag` | `I` = normalized id (+url hint), `K` = id kind | 1111 |

Parent scope: lowercase mirror of whichever row the *parent* falls in —
same value as root for top-level; `e`/`k`/`p` at the parent 1111 for
replies. Root scope for replies is always `root_scope_from_parent`, never
recomputed.

**Fetch-filter consequence (read-side fix, ships with this):** per the
layouts above, a *reply* carries the root only in its **uppercase** tag —
a REQ filtering on `"#a"` alone returns top-level comments but misses
every reply. Alexandria filters on `"#A"` for exactly this reason
(`CommentLayer.svelte:79-88`). `discussions/list` already queries
`#a` ∪ `#A` ∪ `#e` (`api.rs:3538-3564`); **`discussions/counts` does not**
— its single filter is `{"kinds":[1111,9802], "#a": addresses}`
(`api.rs:3189-3192`), so today's badges undercount by exactly the reply
set. Fix: mirror the list handler's filter trio there (dedup before
tally, as list already does). Regular and external targets additionally
need `#E` ∪ `#e` / `#I` — all single-letter, hence relay-indexable per
NIP-01.

---

## 4. Offsets: pinning highlights to position

### 4.1 Wire format

`["offset", "<start>", "<end>"]` — decimal UTF-16 code-unit offsets into
the `.content` of the **event version pinned by the highlight's `e` tag**.
UTF-16 because that's the unit `resolve_highlight_spans` already returns
and JS slices in; the tag is advisory and self-verifying (below), so
foreign clients that ignore it lose nothing.

### 4.2 Resolver extension (`resolve_highlight_spans`)

`Highlight` gains `pub offset: Option<(usize, usize)>` (the API handler
extracts it from the event's tags; current callers pass `None`).
Resolution order per highlight:

1. **Offset, verified.** If `offset` is present and
   `content[start..end]` (UTF-16 slice), trimmed + case-folded, equals the
   highlight's `content` trimmed + case-folded → use it verbatim. The
   verification is what makes stale offsets safe: if the section was
   re-published with edits, the slice no longer matches and we fall through.
2. **Substring with context disambiguation.** Today's case-insensitive
   first-occurrence scan, extended: when the needle occurs more than once
   and the event carries a `context` tag, prefer the occurrence whose
   surrounding window best contains the context text.
3. **Substring, first occurrence** — current behavior, final fallback
   (covers Alexandria-era and foreign events).

Overlap arbitration (longer-first, non-overlapping spans) is unchanged, but
offset-verified spans claim their range **before** substring-resolved ones —
an exact pin should never lose its spot to a fuzzy match.

### 4.3 Why not fragment selectors / Ranges

W3C-style selectors (XPath + offsets) anchor to rendered DOM, which differs
per frontend and per content mode (Markdown/Org/AsciiDoc). The section's
raw `.content` string is the one representation every consumer shares and
the engine already resolves against. Offsets into it are frontend-agnostic,
one tag, and degrade to NIP-84 baseline.

---

## 5. Web: capture + compose

### 5.1 Highlight mode + selection → offsets (`HighlightCapture.svelte`)

Highlighting is an explicit **mode**, not an always-on selection listener
(worksheet C3). Two entries, one state: a `highlight-mode` command
invocable from the command palette (`Spc :`) and a **modeline pill** shown
on any buffer rendering highlightable content (Reader, Doc). While the
mode is on: the pill renders active, and completing a selection — both
endpoints inside the same `[data-section-addr]` container — opens an
Alexandria-style confirm popover: the selected text as a preview, an
optional one-line annotation input (the quote-highlight `comment` field,
A8), and post/cancel. Leaving the mode (pill, `Esc`, or the command
again) restores normal copy-selection behavior untouched. Selections
outside a section container, or while the mode is off, do nothing.

Offset mapping (worksheet C2 — **still open**, the one undecided item;
option 1 below is the working default, to be verified visually before
step 6 commits to it): the naive TreeWalker-sum-text-lengths approach
(old plan §5) is **wrong** here, because `RichContent` doesn't render raw
content — nostrdown refs render as chips/`EmbedCard`s whose DOM text
differs from their source span, so DOM text offsets ≠ content offsets.
Instead, use the segmentation the renderer already has:

- `buildSegments` (`nostrdown.ts:103`) tracks a source cursor internally;
  extend `ContentSegment` with `srcStart`/`srcEnd` (it's emitted in source
  order, so this is two extra fields at emission time, no new algorithm).
- `RichContent` wraps text segments in `<span data-src-start={srcStart}>`
  (visually inert inside the `<pre>`) and stamps the same attribute on
  highlight `<mark>`s; ref/token segments get `data-src-start`/`data-src-end`
  and are treated as atomic.
- Mapping an endpoint `(node, offsetInNode)`: walk up to the nearest
  `[data-src-start]`; for text/mark segments the DOM text **is** the content
  slice, so `src = srcStart + utf16OffsetWithinSegment(node, offsetInNode)`
  is exact. An endpoint inside a ref chip clamps to that segment's
  `srcStart`/`srcEnd`.
- `content` for the event is `sectionContent.slice(start, end)` — sliced
  from the **source**, not `selection.toString()`, so the offset tag and
  the content string agree byte-for-byte (whitespace fidelity guaranteed;
  §4.2 step 1 then always verifies on unedited sections).

Post: `api.publishHighlight({ target: { address, event_id }, content,
offset: [start, end], context, comment? })`. On 200, append the returned
event to the reader's `discussions.events`; the existing `$effect` →
`api.resolveHighlights` → `RichContent` pipeline re-renders the overlay.
The `context` tag is emitted **sparingly**: only when the selected text
repeats within the section (so offset-ignorant readers can disambiguate),
and then as a tight sentence window — never the enclosing paragraph.
Field-tested reason: clients like Amethyst render `context` as the quote
body, so a paragraph-wide context displays as if the whole section were
highlighted.
Sign-in gating per worksheet C7: the modeline pill (a persistent
affordance) renders disabled with a "sign in to highlight" tooltip when
`identityCanSign` is false; the mode itself can't be entered.

### 5.2 Comments (`CommentThread.svelte` + `ReplyBox`)

- `CommentThread` (currently read-only) gains a `Reply` button per node and
  one `replyTo: string | null` at the thread root (one open box at a time),
  plus a top-level `ReplyBox` under each section's thread disclosure /
  in `DiscussionViewBuffer`.
- `ReplyBox`: plain `<textarea>` (NIP-22 content is plaintext — no toolbar,
  no markdown preview), Cmd/Ctrl-Enter to post, disabled + "sign in to
  comment" hint when `identityCanSign` is false.
- Post: `api.publishComment({ root: { address }, parent?, content })`.
- **After posting, refetch instead of client-side tree surgery**: one
  `discussions/list { policy: local_only, threaded: true }` call. The
  engine ingested the event before responding, so the refetch includes it,
  costs sub-50ms, and keeps thread building in Rust (no TS twin of
  `build_thread` creeping back for optimistic inserts).
- A later `fetch_always` refresh may return the relay echo of the same
  event; `discussions/list` already dedups by id.

### 5.3 Deletion

Kebab on own comments/highlights (`event.pubkey === active pubkey`) →
confirm → `api.deleteDiscussion({ event_id })` → refetch as above. The
tombstone filter (§3.3) makes it disappear locally regardless of relay
acceptance.

### 5.4 Where the affordance attaches (surface map)

Every surface below already renders the target read-only; the affordance
is one `ReplyBox` mount + one API call each, because tag construction and
kind routing live behind the endpoint. In rollout order:

| Surface | Target | Wiring |
|---|---|---|
| `CommentThread.svelte` (per node, after `.ct-body` at :74) | reply to that 1111 | `parent = node.event`; root chased engine-side. Serves Reader, Doc, and DiscussionView at once since they all render threads through it |
| `ReaderBuffer` comment blocks (:2109-2122) + per-section disclosures | top-level on 30040 / 30041 | `root = { address }` — the publication case, §3.2 first layout |
| `DocBuffer` comments section (:272-288) | top-level on 30023 / 30818 / 30817 | `root = { address }` — same call, different kind; no new code beyond the mount |
| `DiscussionViewBuffer` (next to "Pull thread", :433-527) | reply to the subject 1111 **or comment on the subject 9802** | the **primary** experience for viewable events (worksheet C4): a social-style thread with a reply box, not a form. `parent = subject` for comments; `root = { event_id, kind: 9802, pubkey }` for highlights — the E-tag row of §3.4 |
| `EventViewModal` actions chord (`a` section, :390-405) | **any** event of any kind | the **universal fallback** (worksheet C4): Actions → `(c) comment` expands an inline textarea in the modal; engine routes (addressable/replaceable/regular/external). Sibling action `(d) delete` for own events (B4). This is the escape hatch for kinds with no dedicated view |
| Refs/import surfaces (later) | external ids | a nostrdown `{{ref:}}`/DOI/URL context could offer "discuss" → the `I`-tag row. Deferred until a concrete surface wants it; the endpoint already accepts it |

`ProfileView`'s comments/highlights tabs need no compose affordance — rows
route to `DiscussionViewBuffer`, which has one.

Read-side prerequisite per surface: the thread shown must include the new
comment's scope. Reader/Doc already fetch by address; `EventViewModal` and
regular-event targets need `discussions/list` queried by `#E`/`#e` (§3.4),
which the request shape already allows (`event_ids`).

---

## 6. Failure + edge cases

| Case | Behavior |
|---|---|
| No active identity / locked key | Endpoint 401; UI never shows compose affordances (`identityCanSign`) |
| NIP-07 user rejects signature | Signer round-trip returns error → 4xx; UI keeps draft text in the box |
| All relays reject / offline | Event is still in nostrdb (ingested pre-broadcast); response shows 0/N accepted; UI renders it with a "local" pill (same pattern as `LocalPublicationTracker`) — rebroadcast later via the existing broadcast endpoint |
| Reply target not in local db | `parent` carries id+pubkey from the rendered event, so no lookup needed; root `e` tag simply omitted if the target version isn't cached |
| Selection spans two sections | Capture rejects (endpoints must share one `data-section-addr`) |
| Selection inside a ref chip / EmbedCard | Endpoints clamp to the segment's source span |
| Section re-published after highlight | Offset verification fails → context/substring fallback (§4.2) |
| Duplicate phrase in section | Offset pins the right occurrence for tendrl-authored highlights; context disambiguation for foreign ones |
| Muted (ignore-listed) author | Filtered in `discussions/list` + `counts`, single chokepoint |
| Comment on a kind-1 note | 1111 like everything else (§3.2, worksheet A5); NIP-10-only clients won't thread it — accepted cost of following the ecosystem's convergence on 1111 |
| Reply to a 1111 whose parent isn't cached locally | `root_scope_from_parent` needs the parent event; the surfaces offering "Reply" render the parent, so the web sends the full parent event in the request as fallback when the engine misses it |
| Comment on an event with no dedicated view (calendar, listing, repo…) | Works via `EventViewModal`'s universal action — routing is kind-range-based, not a kind allowlist |
| External-id normalization drift | Engine normalizes per NIP-73 before tagging; the web never constructs `I` values |

---

## 7. Build order

1. `src/discussions.rs`: `build_comment_template` (full §3.4 routing table:
   addressable / replaceable / regular incl. kind 1 / external) +
   `root_scope_from_parent` / `build_highlight_template` /
   `build_deletion_template`, with fixture tests against the `nips/22.md`,
   `nips/84.md`, and `nips/73.md` examples plus the Amethyst
   highlight-reply capture in worksheet A7. Pure, no IO — smallest
   reviewable unit.
1b. Read-side filter fix: `discussions/counts` adopts the `#a` ∪ `#A` ∪
   `#e` filter trio `discussions/list` already uses, so reply comments
   stop being invisible to the badges (§3.4). One-hunk fix in
   `discussion_counts_handler` (`api.rs:3189`); independent, shippable
   first.
2. `Highlight.offset` + resolver steps 1–2 in `resolve_highlight_spans`
   (+ tests: verified offset, stale offset falls back, context picks the
   right occurrence).
3. The three handlers (`comment` / `highlight` / `POST /discussions/delete`)
   + tombstone and ignore-list filters and the highlight second-hop fetch
   (§3.2) in `discussions_list_handler`. Verifiable with curl against a
   local engine before any UI exists.
4. `segments`→`srcStart` plumbing in `nostrdown.ts` + `RichContent`
   (mechanical, read-side only, independently shippable).
5. `ReplyBox` + `CommentThread` reply wiring (uses 3) — one component
   serves Reader, Doc, and DiscussionView (§5.4). Then the `DocBuffer` /
   `ReaderBuffer` top-level mounts and the `EventViewModal` universal
   "Comment" action, in that order — each is a mount + one call.
6. `HighlightCapture` (uses 3 + 4). The killer affordance, lands last
   because everything under it is then already tested.

Steps 1–3 are engine-only; 4 is web read-side; 5–6 are the visible feature.
