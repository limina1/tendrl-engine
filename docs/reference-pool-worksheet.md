# Reference Pool — design worksheet

Working doc, not a spec. The high-level design lives in
[workbench-architecture.org](workbench-architecture.org) — the "The
Reference Pool" section and Phase 7. This worksheet is for the open
design decisions that section deliberately left under-specified, so
they can be reasoned about one at a time.

Each item has the same shape:

- **What it is** — concrete description, with a sketch where helpful.
- **Why it matters (UX)** — what breaks if we get it wrong.
- **Options on the table** — discrete choices, with tradeoffs.
- **Decision** — locked-in choice when committed; *Pending* until then.

---

## Shared background

The reference pool is one `items: ContextItem[]` array. An item carries
independent membership flags — today `in_context` and `in_compose`,
Phase 7 adds `held`. `addToPool` *merges* flags (it never transfers),
and `gc()` reaps any item whose every flag is false.

One question sits underneath all four issues below and is flagged where
it bites: **what does the `refs` buffer actually show** — every pool
item (a master list), or only `held` ones (a filtered view)? It is not
assumed here; each issue notes how it depends on the answer, and issue 3
makes a recommendation.

---

## 1. Button soup

### What it is

Pool items today carry a cluster of glyph buttons:

```
[◂]  to chat context
[□]  to compose
[▸]  publish
[🗑] remove
```

Phase 7 adds at least three more destinations — hold/Refs, cite,
append-to-search. Naively that is a 7-glyph row per item:

```
[◂] [□] [▸] [🗑] [☆] [”] [⌕]      ← unreadable
```

The glyphs also encode a dead layout: `◂`/`▸` mean "push left / right"
— meaningful when chat was pinned left and compose centre in the frozen
three-panel design, meaningless now that WM buffers have no fixed
position.

### Why it matters (UX)

- Seven cryptic glyphs is unlearnable without a legend; every use is a
  guessing game.
- Directional glyphs that point at nothing actively mislead.
- It does not scale — every new pool route is another glyph fighting
  for the same row.

### Options on the table

**Option A — keep glyphs, add the new ones.** Status quo extended.
Zero design work; maximal button soup. Rejected on sight; listed for
completeness.

**Option B — membership chips.** Replace the additive-route glyphs
with labelled chips that show *current membership* and toggle on click:
`⟨ctx⟩ ⟨compose⟩ ⟨cited⟩`. Filled = member, outline = not. Honest
(shows state), symmetric (toggles both ways), scales (new pool = new
chip). See issue 2.

**Option C — one primary action + overflow menu.** Inline the single
most-common action; the rest behind a `⋮` kebab. Compact, but hides
state behind a click and is slow for frequent routing.

**Option D — hybrid.** Membership chips for the pool routes (state); a
small action menu for true *actions* (publish, cite, append-to-search,
remove). Separates "where does this item live" from "do something with
this item" — the real distinction the flat glyph row blurs.

### Decision

> *Pending:* recommendation is **D**. Memberships are state and want
> chips (issue 2); publish / cite / search / remove are one-shot
> actions and want a menu or a small action row. The flat
> equal-weight glyph cluster fails because it mixes the two.

---

## 2. Membership controls — move vs. toggle

### What it is

`◂` and `□` are framed as *moves* ("send to chat", "send to compose").
The code disagrees: `addToPool` does
`in_context: e.in_context || target.context` — it *adds* a membership,
never removes the source. There is no real "move" anywhere; an item can
be in context and compose at once.

The control should therefore be a **toggle of an independent
membership**, not a one-way push.

```
current:   [◂]  [□]                 one-way, additive-only, spatial
proposed:  ⟨ctx⟩  ⟨compose⟩  ⟨held⟩  toggles; filled = member
```

### Why it matters (UX)

- A control labelled "move" that actually "adds" trains the user to
  distrust it.
- Removal today is asymmetric — you add via `◂`/`□` but remove via a
  panel-specific `×` or the trash. Toggles make add and remove the
  same gesture.
- Membership chips double as a status display: glance at an item, see
  every pool it is in. The glyph row shows only what you *can do*,
  never what *is*.

### Options on the table

**Option A — inline membership chips.** Each item renders its chips
inline; click toggles. Always-visible state. Costs horizontal room
(~3 chips).

**Option B — membership popover.** One `•••` per item opens a
checklist of pools. Compact, discoverable (labelled checkboxes), but
state is one click away and routing is two clicks.

**Option C — drag between buffers.** Drag an item from `refs` into the
chat or compose buffer. Spatially intuitive, demo-friendly, but heavy
to build, poor for keyboard/a11y, and ambiguous past two destinations.

### Decision

> *Pending:* recommendation is **A** for the `refs` buffer (room to
> spare) and **B** as the fallback wherever the row is tight (search
> result rows, feed cards). Both drive the same `addToPool` /
> membership-clear calls; chip vs. popover is pure presentation. **C**
> is a possible later enhancement, not a foundation.
>
> Either way: drop the `◂`/`▸` directional glyphs. Use text labels
> (`ctx`, `compose`) or neutral non-directional icons — nothing that
> implies a fixed panel position.

---

## 3. Drop from Refs — and what Refs contains

### What it is

A pool needs an exit. The user can hold an event, route it, cite it —
but with `held` keeping items off the `gc()` chopping block, there must
be an explicit "drop this from Refs" affordance or the pool only ever
grows.

What "drop" *means* depends on the unresolved question of what `refs`
shows:

```
Model 1 — refs = items where held === true
   "drop from Refs"  →  held = false
   item still in compose? it just leaves the refs view, survives.

Model 2 — refs = the whole pool (master list)
   "drop from Refs"  →  must remove from the pool entirely
   (clear every flag) — there is nowhere "lesser" to drop to.
```

### Why it matters (UX)

- Without an explicit drop, Refs becomes a roach motel: events check
  in, `held` keeps them, nothing leaves. The buffer turns to noise
  within a session.
- The meaning of "drop" must be unsurprising. Nuking an item's
  `in_compose` membership because the user clicked "drop" in the Refs
  buffer — while that section is live in their draft — is a
  data-loss-grade surprise.

### Options on the table

**Option A — drop = unpin (Model 1).** "Drop from Refs" clears `held`
only. If the item is still in context or compose it survives and stays
visible in those panels; `gc()` reaps it once every flag is false.
Soft, safe, never destroys in-use work. Needs Model 1.

**Option B — drop = remove from pool (Model 2).** "Drop" clears all
flags and evicts the item. Simple if refs is the master list, but
destructive — must be guarded.

**Option C — two-tier, reuse the existing trash pattern.** A soft
"drop"/unpin (Option A) *plus* a hard "remove from pool" on the
existing two-step trash with its 10s undo countdown. The soft action is
one click and safe; the hard action is deliberate and reversible.

### Decision

> *Pending:* recommendation is **Model 1 + Option C**. `refs` shows
> `held` items; "drop from Refs" unpins; the hard eviction is the trash
> control already built for context/compose, with its undo window.
> This also settles the cross-issue question: `refs` is a
> *held-filtered view*, a peer of the context and compose views, not a
> master list.

---

## 4. Provenance — original vs. forked / live draft

### What it is

Once an event is collected, two versions of it exist and the pool item
must hold both:

- **origin** — the immutable source event as collected, pinned to a
  specific event id (not just the addressable coordinate — a
  replaceable event's "latest" drifts). What you transclude, cite, and
  diff against.
- **draft** — the live, editable working copy. Starts identical to
  `origin`; diverges the moment the user edits it in compose.

The current schema gestures at this but does not name it: `ContextItem`
has `source_event_id` / `source_addr`, a `readonly` flag, a `modified`
flag, and — tellingly — *two* content fields, `content` and
`context_content`, with a `⇄` "cross-panel copy" badge to shuffle text
between them. That is two *consumer-specific* drafts with no notion of
a shared origin. It does not scale: a third consumer wants a third
content field.

### Why it matters (UX)

- **Transclusion** points an `a` tag at the origin, not the user's
  edit. Without a clean origin, an imported section silently
  transcludes the wrong thing.
- **Forking** (NIP-54 markers) needs a diff base: origin content vs.
  draft content decides imported / forked, and the `e` tag must pin the
  exact origin version.
- **Citation** (NKBIP-03 `kind:30`) cites the origin.
- The compose `sectionState()` — imported / claimed / forked / original
  — is *derivable* from an origin/draft model (`draft === origin &&
  hasSource → imported`; `draft !== origin && hasSource → forked`; `no
  source → original`). Today it is tracked separately and can drift
  from the content fields.

### Options on the table

**Option A — `origin` + single `draft`.** One pair of fields. `origin`
frozen at collect time; one shared `draft` for all consumers.
`modified = draft !== origin`. Kills `content` / `context_content` /
the `⇄` badge. If a consumer genuinely needs different text, that is a
*second collected item* (an explicit fork), not a hidden content slot.

**Option B — `origin` pointer only, no snapshot.** Store
`source_event_id` + `source_addr`; re-resolve origin content from
nostrdb on demand. Lighter memory; but origin is unavailable if the
event was never indexed locally, and you must pin the event id (not
just addr) so a replaceable update does not silently move the origin.

**Option C — keep per-consumer drafts, just rename.** Formalise
`content` / `context_content` as named per-panel drafts. Rejected: it
is the current mess, and it scales by one field per consumer.

**Option D — origin = `{ addr, pinned_event_id, snapshot }`.** Pin both
the addressable coordinate (follows latest — for "is there a newer
version?" prompts) *and* the exact event id collected (the fork / cite
/ diff base). Orthogonal to A/B; pairs naturally with A.

### Decision

> *Pending:* recommendation is **A + D** — one frozen `origin`
> (carrying both the addr and the pinned event id), one shared editable
> `draft`, `modified` derived, `sectionState()` derived. Retire
> `content` / `context_content` and the `⇄` badge. Option B's
> pointer-only origin is a viable memory optimisation later, but a
> cached snapshot keeps citation/transclude working offline, which
> matches the local-first invariant.

---

## Open threads

- **What `refs` contains** — issue 3 recommends Model 1 (held-filtered
  view). If that is rejected, issues 1–3 shift; revisit together.
- **Per-panel divergence** — issue 4 Option A says context and compose
  share one `draft`. If real use shows they must differ (a terse chat
  summary vs. a full compose section), that is a deliberate fork — but
  the UX of "fork for a different purpose" is unspecified here.
- **`held` semantics** — explicit flag (issue 3, Model 1) vs. implied
  by pool membership. Recommended explicit; still flagged open in the
  workbench doc's Reference Pool section.
- **Multi-collect** — collecting the same event twice is deduped by
  `source_event_id` / `source_addr` already; forking it intentionally
  twice (two drafts of one origin) needs a distinct-id story.

---

## Glossary

- **Pool / reference pool** — the one `items: ContextItem[]` array; the
  substrate under the context, compose, and refs views.
- **Membership** — an independent boolean on a pool item (`held`,
  `in_context`, `in_compose`) saying which view it appears in. Not a
  state machine; an item may hold several at once.
- **Route** — an operation that sets or clears a membership, or acts on
  an item (cite, publish, append-to-search).
- **Chip** — a small labelled toggle showing one membership; filled
  when the item is a member.
- **Origin** — the immutable source event a pool item was collected
  from, pinned to a specific event id.
- **Draft** — the live editable copy of a pool item; starts equal to
  origin, diverges on edit.
- **Drop** — soft removal from Refs (clear `held`); distinct from trash
  (hard eviction from the pool, with undo).
- **gc()** — prunes pool items whose every membership flag is false.
