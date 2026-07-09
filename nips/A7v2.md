NIP-A7 (v2)
===========

Spells
------

`draft` `optional`

Revision 2: integrates the composition extension (`$arg`/`$in`
variables, `param` declarations, `in` chaining, `PIPE` pipelines,
closures, relay-hinted references) into the base spell spec. Section
order mirrors [v1](A7.md) so the two read side by side; the delta alone
is in [A7-composition.md](A7-composition.md).

## Abstract

This NIP defines `kind:777` events ("spells") that encode Nostr relay
query filters as portable, shareable events. A spell stores a REQ or
COUNT filter with optional runtime variables and relative timestamps,
allowing users to publish, discover, and execute saved queries across
clients.

One spell holds one filter, and Nostr filters have no join or
dereference — "events tagged `#x`" and "the events those events point
at" are two queries, the second dependent on the first. v2 therefore
adds **composition**: spells can declare arguments, project values out
of another spell's result set, and connect to other spells either by
naming an input (`in` chaining) or as ordered pipeline stages (`PIPE`).
A composed feed — "the root events that people filed under a hashtag" —
becomes as portable as a single filter.

## Event Format

A spell is a regular (non-replaceable) event with `kind:777`.

The `content` field contains a human-readable description of the query
in plain text. It MAY be an empty string.

### Required Tags

| tag   | values                   | description        |
| ----- | ------------------------ | ------------------ |
| `cmd` | `REQ`\|`COUNT`\|`PIPE`   | Query command type |

A `REQ` or `COUNT` spell MUST contain at least one filter tag (see
below). A `PIPE` spell contains no filter tags of its own — it MUST
carry either `stage` tags or a parent reference (see
[Composition](#composition)). Clients that implement only v1 skip
unknown commands; discovery stays uniform because every spell is still
`kind:777`.

### Filter Tags

Filter tags encode the fields of a Nostr REQ filter.

| tag       | values                                  | REQ filter field | notes                              |
| --------- | --------------------------------------- | ---------------- | ---------------------------------- |
| `k`       | `<kind number>`                         | `kinds`          | One tag per kind for queryability  |
| `authors` | `<pubkey1>`, `<pubkey2>`, ...           | `authors`        | Single tag, multiple values        |
| `ids`     | `<id1>`, `<id2>`, ...                   | `ids`            | Single tag, multiple values        |
| `tag`     | `<letter>`, `<val1>`, `<val2>`, ...     | `#<letter>`      | See [Tag Filters](#tag-filters)    |
| `limit`   | `<integer>`                             | `limit`          |                                    |
| `since`   | `<timestamp>` or `<relative>`           | `since`          | See [Relative Timestamps](#relative-timestamps) |
| `until`   | `<timestamp>` or `<relative>`           | `until`          | See [Relative Timestamps](#relative-timestamps) |
| `search`  | `<query string>`                        | `search`         | [NIP-50](50.md)                    |
| `relays`  | `<wss://url1>`, `<wss://url2>`, ...     | —                | Target relay URLs                  |

All filter tag values are strings. Numeric values (kinds, limit,
timestamps) MUST be encoded as decimal strings.

### Tag Filters

Filter conditions on event tags are encoded as `["tag", <letter>,
<value>, ...]` rather than using the tag letter directly (e.g. `["e",
...]` or `["p", ...]`). This prevents semantic collision — a `["p",
<pubkey>]` tag on a Nostr event normally means "this event references
this pubkey," which would cause relays and clients to misinterpret
filter parameters as social graph references.

The `k` tag is the exception: it uses the tag letter directly (`["k",
"1"]`) to enable relay-side indexing and discovery of spells by the
kinds they query.

Examples:

```
["tag", "t", "bitcoin", "nostr"]   → filter: {"#t": ["bitcoin", "nostr"]}
["tag", "p", "abcd...", "ef01..."] → filter: {"#p": ["abcd...", "ef01..."]}
["tag", "e", "abcd..."]           → filter: {"#e": ["abcd..."]}
```

### Metadata Tags

| tag              | values     | description                                                  |
| ---------------- | ---------- | ------------------------------------------------------------ |
| `name`           | `<string>` | Human-readable spell name                                    |
| `alt`            | `<string>` | [NIP-31](31.md) alternative text                             |
| `t`              | `<topic>`  | Topic tag for categorization (multiple allowed)              |
| `close-on-eose` | none       | Clients SHOULD close the subscription after EOSE             |
| `e`              | `<event-id>` | Fork provenance: references the parent spell event         |

Note: `["t", "bitcoin"]` as a top-level tag categorizes the spell
itself, while `["tag", "t", "bitcoin"]` is a filter condition matching
events with `#t = bitcoin`. Both may appear in the same event. Clients
SHOULD render a spell's topics distinctly from its filter so the
difference stays legible.

### Composition Tags (v2)

| tag     | values                                            | description                                          |
| ------- | ------------------------------------------------- | ---------------------------------------------------- |
| `param` | `<name>`, `<prompt>`                              | Declares a runtime argument for `$arg.<name>`        |
| `in`    | `<spell-event-id>`, `<relay-url>`…                | Chain: this spell applies to that spell's results    |
| `stage` | `<spell-event-id>`, `<combinator|"">`, `<relay-url>`… | One ordered pipeline stage (`PIPE` spells only)  |
| `arg`   | `<name>`, `<value>`                               | Closure binding: a durable default for a parameter   |

Trailing values on `in` and `stage` are optional **relay hints**: where
to find the referenced spell when it isn't already available. Clients
composing a reference from an `nevent` SHOULD carry its relay hints
here. A source stage carrying hints uses `""` as a combinator
placeholder to keep positions stable.

## Runtime Variables

The `authors`, `ids`, and `tag` filter values MAY contain runtime
variables that are resolved at execution time. Three namespaces
complete the grammar — a spell is a function
`(args, identity, input) → filter`:

| variable           | context    | resolves to                                               |
| ------------------ | ---------- | --------------------------------------------------------- |
| `$me`              | identity   | The executing user's pubkey                               |
| `$contacts`        | identity   | All pubkeys from the executing user's kind 3 contact list |
| `$arg.<name>`      | invocation | A value supplied at run time (see `param`)                |
| `$in.<projection>` | pipeline   | Values projected from the input result set                |

```
projection = "ids" | "pubkeys" | "tag." letter [":" marker]
```

- `$in.ids` — the input events' ids
- `$in.pubkeys` — the input events' author pubkeys, deduplicated
- `$in.tag.<letter>` — the first value of every `<letter>` tag on each
  input event; with `:marker`, only tags whose fourth element equals
  the marker (e.g. `$in.tag.e:root` for NIP-10 root-marked `e` tags)

Variables are case-sensitive and MUST be lowercase (parameter names are
whatever `param` declared).

**Expansion rule:** variables expand in place; a filter tag's values
concatenate and deduplicate. `["ids", "$in.tag.E", "$in.tag.e:root"]`
unions NIP-22 roots with NIP-10 root markers — no new mechanism, filter
tags already take multiple values. Implementations SHOULD cap a single
expansion (this implementation: 500 values) and report truncation.

If a client cannot resolve a variable (no logged-in user for `$me`, no
contact list for `$contacts`, an unbound `$arg.<name>`, or `$in.*` with
no input), it MUST NOT send the REQ and SHOULD display a message
explaining the unresolved dependency.

### Parameters

```
["param", <name>, <prompt>]
```

Declares an argument consumed by `$arg.<name>` — a machine-readable
signature. Clients prompt for unbound parameters before executing; the
`prompt` string is the question to ask. This is how "Filed under
#asknostr" generalizes to "Filed under #{tag}".

### Partial Spells

A spell that references `$in.*` without naming its own input (no `in`
tag) is **partial**: it is runnable only as a pipeline stage. Partial
spells are the shared library of composition — "kind 0s of
`$in.pubkeys`" or "roots of `$in`" get written once, network-wide, and
referenced by id from any pipeline.

## Relative Timestamps

The `since` and `until` tags MAY contain relative time expressions
instead of Unix timestamps.

Grammar:

```
value = unix-timestamp / relative-time / "now"
relative-time = 1*DIGIT unit
unit = "s" / "m" / "h" / "d" / "w" / "mo" / "y"
```

| unit | meaning | seconds   |
| ---- | ------- | --------- |
| `s`  | seconds | 1         |
| `m`  | minutes | 60        |
| `h`  | hours   | 3600      |
| `d`  | days    | 86400     |
| `w`  | weeks   | 604800    |
| `mo` | months  | 2592000   |
| `y`  | years   | 31536000  |

Months and years use approximate fixed durations (30 days and 365 days
respectively).

`now` resolves to the current Unix timestamp. A relative time like `7d`
resolves to `now - 7 * 86400`.

Clients MUST resolve relative timestamps to absolute Unix timestamps
before constructing a REQ message.

## Composition (v2)

Two forms, one semantics. Both replace-with-referents steps use the
same `map` rules (below), so results, pass-through, and provenance
behave identically whichever form authored them.

### `in` Chaining (minimal form)

```
["in", <spell-event-id>, <relay-url>…]
```

A REQ or COUNT spell MAY reference another spell as its **input**. To
execute a chained spell:

1. Resolve the input spell (see [Reference
   Resolution](#reference-resolution)) and execute it in full — it may
   itself be chained, a closure, or a pipeline. Caller-supplied
   arguments flow down the chain.
2. Resolve this spell's `$in.*` projections against the input's result
   set, then run this spell's filter.
3. Replace input events with the referents they matched (`map`
   semantics), deduplicated by id.

Constraints:

- A chained spell MUST reference `$in.*` somewhere in its filter — an
  input nothing consumes is invalid.
- `in` applies to REQ/COUNT spells; a PIPE composes via stages instead.
- A spell carrying `in` is complete (runnable standalone); the same
  filter without `in` is partial.
- Implementations MUST bound chain depth (this implementation: 4 hops)
  to fail legibly on cycles.
- A chained spell SHOULD re-declare its input's `param` tags so one
  prompt binds the whole chain (clients SHOULD copy them at authoring
  time).

### `PIPE` Pipelines (general form)

A pipeline is a `kind:777` with `["cmd", "PIPE"]` and ordered stage
tags:

```
["stage", <spell-event-id>]                        first stage: the source
["stage", <spell-event-id>, <combinator>]          later stages
["stage", <spell-event-id>, <combinator|"">, <relay-url>…]
```

Combinators — deliberately tiny (every combinator is something all
clients must implement or degrade around):

- `map` (default) — replace the running result set with the referents
  this stage fetches.
- `join` — keep the running result set; this stage's results are
  carried alongside as auxiliary enrichment (kind 0 profiles beside
  publications).

A stage spell MUST NOT be a PIPE and MUST NOT carry its own `in` tag.
Unbound `$arg.*` in stages escalates to the pipeline's signature via
re-declared `param` tags.

Choose `in` when one spell feeds one spell (one event, no new spells
published). Choose `PIPE` when you need `join`, several consumers of
one source, or stages maintained as independent shared spells.

### `map` Semantics

**Pass-through rule:** an upstream event that contributes no projection
values is its own referent and passes through to the output. (In a live
labels→roots run, most labeled events were top-level posts — dropping
non-replies would discard most of the set.)

**Provenance:** executors SHOULD keep the inverse mapping (referent ←
the upstream events that pointed at it) for presentation ("filed by 3
people"). Provenance is derived, not wire format.

### Closures

A **closure** binds parameters durably: a fork (`e` tag → parent spell,
per fork provenance above) with `["cmd", "PIPE"]`, `["arg", <name>,
<value>]` bindings, and no stages of its own. Executing a closure
executes the parent with those bindings as defaults — caller-supplied
arguments win. "Referents of `#devstr`" becomes a one-tap feed.
Implementations MUST bound closure-chain depth (this implementation:
4).

## Executing a Spell

To execute a spell, a client:

1. Parses the event tags; follows closure chains to the spell they
   close over, collecting `arg` defaults (caller arguments win)
2. For `PIPE`: resolves and executes each stage in order, applying its
   combinator. For a spell with `in`: executes the input spell first
   (recursively), then continues below with its results as `$in`
3. Resolves runtime variables (`$me`, `$contacts`, `$arg.*`, `$in.*`)
   and applies the expansion rule; refuses (with an explanation) if any
   variable is unresolvable
4. Resolves relative timestamps to absolute Unix timestamps
5. Constructs a REQ or COUNT message with the resolved filter
6. Determines target relays (see [Relay Resolution](#relay-resolution))
7. Sends the REQ or COUNT message to the resolved relays
8. If `close-on-eose` is present, closes the subscription after
   receiving EOSE from all connected relays
9. Applies `map`/pass-through and deduplicates by id where composition
   is involved

Pagination note: a client paging a composed feed ("load older") SHOULD
apply its `until` cursor to the *source* stage only — later stages
query referents, whose timestamps don't follow the source's pagination.

### Relay Resolution

If the spell contains a `relays` tag, the client SHOULD send the query
to those relays.

If no `relays` tag is present, the client SHOULD use [NIP-65](65.md)
relay lists to determine where to send the query, falling back to the
executing user's NIP-65 read relays.

### Reference Resolution (v2)

`in` and `stage` reference spells by event id. To resolve one: check
local storage first; if absent, fetch the id from the reference's relay
hints; fall back to the client's relay set when a reference carries no
hints. Relay hints are what keep a composed spell executable for
someone whose relays never saw its parts.

## Discovering Spells

Clients can discover spells using standard Nostr queries:

- By author: `{"kinds": [777], "authors": ["<pubkey>"]}`
- By topic: `{"kinds": [777], "#t": ["bitcoin"]}`
- By queried kind: `{"kinds": [777], "#k": ["1"]}`

Pipelines, chained spells, and closures are `kind:777` like everything
else, so discovery stays uniform; their `in`/`stage`/`e` references
resolve by event id.

## Examples

A spell that finds recent notes about Bitcoin from the user's contacts
(unchanged from v1):

```json
{
  "kind": 777,
  "content": "Notes about Bitcoin from my contacts",
  "tags": [
    ["cmd", "REQ"],
    ["name", "Bitcoin from contacts"],
    ["alt", "Spell: notes about Bitcoin from contacts"],
    ["k", "1"],
    ["authors", "$contacts"],
    ["tag", "t", "bitcoin"],
    ["since", "7d"],
    ["limit", "50"],
    ["t", "bitcoin"],
    ["t", "social"]
  ]
}
```

When executed by a user with 3 contacts, `$contacts` resolves to their
pubkeys and `7d` resolves to 7 days before the current time:

```json
["REQ", "<sub-id>", {
  "kinds": [1],
  "authors": ["aabb...", "ccdd...", "eeff..."],
  "#t": ["bitcoin"],
  "since": 1740000000,
  "limit": 50
}]
```

A COUNT spell with multiple kinds and an absolute timestamp (unchanged
from v1):

```json
{
  "kind": 777,
  "content": "",
  "tags": [
    ["cmd", "COUNT"],
    ["k", "1"],
    ["k", "6"],
    ["k", "7"],
    ["authors", "$me"],
    ["since", "1704067200"],
    ["close-on-eose"]
  ]
}
```

A parameterized finder — post-hoc classification, where users file
existing events under a hashtag by labeling them:

```json
{
  "kind": 777,
  "content": "labels under a hashtag",
  "tags": [
    ["cmd", "REQ"],
    ["name", "Filed under #{tag}"],
    ["param", "tag", "hashtag to collect labels for"],
    ["k", "1"],
    ["k", "1111"],
    ["tag", "t", "$arg.tag"],
    ["since", "30d"],
    ["limit", "100"]
  ]
}
```

A chained spell that dereferences those labels to the events they filed
(`<finder-id>` is the event id above; the relay hint says where to find
it):

```json
{
  "kind": 777,
  "content": "",
  "tags": [
    ["cmd", "REQ"],
    ["name", "…their roots"],
    ["param", "tag", "hashtag to collect labels for"],
    ["in", "<finder-id>", "wss://relay.example.com"],
    ["k", "1"],
    ["ids", "$in.tag.E", "$in.tag.e:root"]
  ]
}
```

Executing it with `tag = asknostr`: run the finder, union the labels'
NIP-22 `E` roots with NIP-10 root-marked `e` tags, fetch those ids,
pass top-level labeled posts through unchanged, deduplicate. The same
composition as a two-stage pipeline keeps the deref spell partial (no
`in` tag, reusable as a library spell):

```json
{
  "kind": 777,
  "content": "",
  "tags": [
    ["cmd", "PIPE"],
    ["name", "Referents of #{tag}"],
    ["param", "tag", "hashtag to collect labels for"],
    ["stage", "<finder-id>"],
    ["stage", "<deref-id>", "map"]
  ]
}
```

A closure that pins the argument — a one-tap feed forked from the
pipeline:

```json
{
  "kind": 777,
  "content": "",
  "tags": [
    ["cmd", "PIPE"],
    ["name", "Referents of #devstr"],
    ["e", "<pipeline-id>"],
    ["arg", "tag", "devstr"]
  ]
}
```
