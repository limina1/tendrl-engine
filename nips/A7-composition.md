NIP-A7 Composition Extension
============================

Composable Spells
-----------------

`draft` `optional` `tendrl extension to` [NIP-A7](A7.md)

## Abstract

[NIP-A7](A7.md) spells encode one REQ/COUNT filter per event. Nostr filters
have no join or dereference: "events tagged `#x`" and "the events those
events point at" are two queries, the second dependent on the first. This
extension adds **composition** as its own small layer — argument
parameters, input projections, and two ways to connect spells: `in`
chaining (the minimal form — one spell names another as its input) and
`PIPE` pipelines (the general form — ordered stages with combinators).

A spell remains one filter. Composition is expressed entirely in tags, so
REQ-only NIP-A7 clients degrade cleanly: they can still display any spell,
and skip commands or tags they don't implement.

## Variable Namespaces

NIP-A7 defines the identity namespace (`$me`, `$contacts`). This extension
completes the grammar — a spell is a function `(args, identity, input) →
filter`:

| namespace          | context    | resolves to                                    |
| ------------------ | ---------- | ---------------------------------------------- |
| `$me`, `$contacts` | identity   | executing user (NIP-A7 as written)             |
| `$arg.<name>`      | invocation | a value supplied at run time                   |
| `$in.<projection>` | pipeline   | values projected from the input result set     |

```
projection = "ids" | "pubkeys" | "tag." letter [":" marker]
```

- `$in.ids` — the input events' ids
- `$in.pubkeys` — the input events' author pubkeys, deduplicated
- `$in.tag.<letter>` — the first value of every `<letter>` tag on each
  input event; with `:marker`, only tags whose fourth element equals the
  marker (e.g. `$in.tag.e:root` for NIP-10 root-marked `e` tags)

**Expansion rule:** variables expand in place; a filter tag's values
concatenate and deduplicate. `["ids", "$in.tag.E", "$in.tag.e:root"]`
unions NIP-22 roots with NIP-10 root markers — no new mechanism, filter
tags already take multiple values. Implementations SHOULD cap a single
expansion (tendrl caps at 500 values) and report truncation.

NIP-A7's unresolved-variable rule extends unchanged: a client that cannot
resolve a variable MUST NOT send the REQ and SHOULD explain why.

## Parameters

```
["param", <name>, <prompt>]
```

Declares an argument consumed by `$arg.<name>`: a machine-readable
signature. Clients prompt for unbound parameters before executing (the
`prompt` string is the question to ask). A spell whose `$arg.*` references
are all bound — by user input or by closure `arg` tags — resolves like any
other.

## `in` Chaining (minimal form)

```
["in", <spell-event-id>]
```

A REQ or COUNT spell MAY reference another spell as its **input**. To
execute a chained spell:

1. Execute the input spell in full (it may itself be chained, a closure,
   or a pipeline). Arguments supplied by the caller flow down the chain.
2. Resolve this spell's `$in.*` projections against the input's result
   set, then run this spell's filter.
3. Replace input events with the referents they matched (`map`
   semantics, below), deduplicated by id.

**Pass-through rule:** an input event that contributes no projection
values is its own referent and passes through to the output. (In a live
labels→roots run, most labeled events were top-level posts — dropping
non-replies would discard most of the set.)

Constraints:

- A chained spell MUST reference `$in.*` somewhere in its filter —
  an input nothing consumes is invalid.
- `in` applies to REQ/COUNT spells; a PIPE composes via stages instead.
- A spell carrying `in` is complete (runnable standalone). The same
  filter *without* `in` is **partial** — runnable only as a pipeline
  stage.
- Implementations MUST bound chain depth (tendrl: 4 hops) to fail
  legibly on cycles.
- Executors SHOULD keep the inverse mapping (referent ← input events
  that pointed at it) as provenance — presentation data, not wire format.

A chained spell escalates its input's parameters: clients SHOULD copy the
input spell's `param` declarations into the chained spell at authoring
time, so one prompt binds the whole chain.

## `PIPE` Pipelines (general form)

A pipeline is a kind 777 with `["cmd", "PIPE"]` and ordered stage tags:

```
["stage", <spell-event-id>]                  first stage: the source
["stage", <spell-event-id>, <combinator>]    later stages
```

Combinators (deliberately tiny — every combinator is something all
clients must implement or degrade around):

- `map` (default) — replace the running result set with the referents
  this stage fetches, applying the pass-through rule.
- `join` — keep the running result set; the stage's results are carried
  alongside as auxiliary enrichment (kind 0 profiles beside publications).

Stage spells are referenced by event id, which makes small generic spells
a shared library ("kind 0s of `$in.pubkeys`", "roots of `$in`"). A stage
spell MUST NOT be a PIPE and MUST NOT carry its own `in` tag. Unbound
`$arg.*` in stages escalates to the pipeline's signature via re-declared
`param` tags.

Choose `in` when one spell feeds one spell (the common case: one event,
no new spell kinds published). Choose `PIPE` when you need `join`, more
than one downstream consumer of the same source, or stages maintained as
independent shared spells.

## Closures

A **closure** binds a pipeline's or spell's parameters durably: a fork
(`e` tag → parent spell, per NIP-A7 fork provenance) with `["cmd",
"PIPE"]`, `["arg", <name>, <value>]` bindings, and no stages of its own.
Executing a closure executes the parent with those bindings as defaults
(caller-supplied arguments win). "Referents of `#devstr`" becomes a
one-tap feed. Implementations MUST bound closure-chain depth (tendrl: 4).

## Worked Example: labels → roots

Post-hoc classification: users file existing events under a hashtag by
replying with a label; the feed shows what was *filed*, not the labels.

Finder (parameterized):

```json
{"kind": 777, "content": "labels under a hashtag", "tags": [
  ["cmd", "REQ"], ["name", "Filed under #{tag}"],
  ["param", "tag", "hashtag to collect labels for"],
  ["k", "1"], ["k", "1111"],
  ["tag", "t", "$arg.tag"], ["since", "30d"], ["limit", "100"]]}
```

Chained root-deref (`<finder-id>` = the event id above):

```json
{"kind": 777, "content": "", "tags": [
  ["cmd", "REQ"], ["name", "…their roots"],
  ["param", "tag", "hashtag to collect labels for"],
  ["in", "<finder-id>"],
  ["k", "1"], ["ids", "$in.tag.E", "$in.tag.e:root"]]}
```

Executing the chained spell with `tag = asknostr`: run the finder, union
the labels' NIP-22 `E` roots with NIP-10 root-marked `e` tags, fetch
those ids, pass top-level labeled posts through unchanged, deduplicate.
The same composition as a two-stage `PIPE` uses `["stage", <finder-id>]`
and `["stage", <deref-id>, "map"]` with the deref spell kept partial (no
`in` tag) as a reusable library spell.

## Discovery

Unchanged from NIP-A7 (`#k`, `#t`, authors). Chained spells and pipelines
are discoverable like any spell; the `in`/`stage` references resolve by
event id. Spellbooks (curated cross-author spell sets) are a separate
tendrl proposal: kind 30777 addressable events whose `e` tags reference
spells by any author.
