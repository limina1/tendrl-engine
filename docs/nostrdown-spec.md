# Nostrdown — Inline Reference Syntax for Nostr

Nostrdown is a markup-agnostic inline syntax for referencing Nostr events from
prose. It rides on top of whatever markup a document is written in. A `{{ }}`
token names an event: it resolves to a coordinate, renders as a link / card /
quote, and is emitted as a tag on the signed event.

`{{ }}` is the reference delimiter because the pair is free across the common
prose markups, so a reference never collides with the surrounding text — and a
client that does not parse the host markup can still find every reference by
scanning for `{{ }}` alone. Markups keep their own links, images, and local-file
references untouched; nostrdown only adds the one thing they cannot express:
pointers to Nostr data.

A bare `nostr:` URI (NIP-21) is already a universal entity link, resolved by any
client. A `{{ }}` reference is the semantic layer over it — wrapping the same
target to add a role and a tag: use `nostr:` to link anywhere, `{{ }}` when the
reference should *do* something.

## Grammar

```abnf
reference = "{{" prefix ":" target [ "|" modifier ] "}}"
mention   = "{{@" target [ "|" modifier ] "}}"   ; profile mention — target is an npub / nprofile (@name reserved)
prefix    = "ref" / "wiki" / "embed" / "quote" / "slot" / "cite"
target    = text     ; what is referenced
modifier  = text     ; a display label, render directive, or quote's excerpt
text      = any characters up to the next "|" or "}}"
```

`target` is the only required part after the prefix. Its admissible form — a
title-slug, a NIP-19 entity (optionally `nostr:`-prefixed), or a
`kind:pubkey:d-tag` coordinate — depends on the prefix (see *Prefixes*); the
grammar only locates where it ends.

Parsing rules:

- Split on the *first* `:` only — colon-bearing targets (a coordinate, a
  `nostr:` URI) are preserved intact.
- `|` is optional and splits once; it runs to the closing `}}` as free text —
  a display label, or `quote`'s inline excerpt.
- A title-slug target is NIP-54 normalized (lowercased; spaces and separators
  collapse to `-`; other punctuation dropped). Entities and coordinates are
  matched verbatim.
- A token with an unknown prefix or a malformed body is left as literal text.
- `{{@target}}` is the mention shorthand — the `@` stands in for `prefix:`
  (see *Prefixes*).

To reference one section of a publication, target that section directly — a
30041 section is an addressable event with its own coordinate. There is no
anchor-into-parent syntax; reference the child, not "parent + child".

## `[[ ]]` — the wikilink alias

`[[ ]]` is the established cross-tool wikilink and travels in published content
regardless, so it is recognized — but *only* as a `wiki` reference, never as a
general Nostr pointer.

```abnf
wikilink = "[[" topic [ ( "][" / "|" ) display ] "]]"   ; topic, d-tag NIP-54 normalized
```

| form                 | meaning                      |
| -------------------- | ---------------------------- |
| `[[topic]]`          | wiki, label = topic          |
| `[[d-tag][display]]` | wiki, bracketed display form |
| `[[topic\|display]]` | wiki, piped display form     |

A target that is itself a link, scheme (`scheme:`, `://`), path, anchor, or
image / media file is left to the host markup and never claimed. Tag and
resolution are identical to `{{wiki:topic}}`.

## Prefixes

Each prefix shares the grammar above and differs only in resolution and tag.

A prefix names the reference's *role* — the human-facing vocabulary the author
writes. The *tag* it emits is the Nostr-native encoding, and need not share the
prefix's name: a single-letter tag where one fits (`wiki` → `w`), otherwise the
tag dictated by the target (an `embed` is `a` / `q` / `p` by entity type).
`{{wiki:topic}}` and `[[topic]]` are two ways to write the one reference.

| prefix  | example | resolves to | emitted tag |
| ------- | ------- | ----------- | ----------- |
| `ref`   | `{{ref:The Ascent}}` | a sibling section in the same publication | `["ref", "the-ascent"]` |
| `wiki`  | `{{wiki:proof of work}}` | kind 30818 (`d`), else 30040/41 (`T`), by title | `["w", "proof-of-work"]` |
| `embed` | `{{embed:naddr1…}}` | transclude the event inline, as a card | by target type (below) |
| `quote` | `{{quote:naddr1… \| And now, let me show in a figure…}}` | attributed blockquote; the excerpt is inline, the target resolves for attribution only | `["a", "30041:‹pk›:‹d›"]` `["p", "‹author›", "", "author"]` |
| `slot`  | `{{slot:naddr1…}}` *(on its own line)* | the event becomes a child node of the index (addressable 30040 / 30041 only) | `["a", "30041:‹pk›:‹d›"]` on the *30040 index* |
| `cite`  | `{{cite:smith-2024}}` *(reserved)* | a kind-30161 citation record | `["cite", "smith-2024", "30161:‹pk›:smith-2024"]` |

`embed` tag by target type — the same `{{embed:…}}` token, tagged according to the target:

| target          | example | emitted tag |
| --------------- | ------- | ----------- |
| naddr           | `{{embed:naddr1…}}`  | `["a", "30040:‹pk›:‹d›", "‹relay›"]` |
| nevent / note   | `{{embed:nevent1…}}` | `["q", "‹id›", "‹relay›", "‹pubkey›"]` |
| npub / nprofile | `{{embed:npub1…}}`   | `["p", "‹pubkey›", "‹relay›"]` |

`@` is the profile-mention shorthand — `{{@npub1…}}` (or `{{@nprofile1…}}`)
renders an inline `@handle` link rather than the full card `{{embed:npub1…}}`
gives, and emits the same `p` tag. A mention is not kind-1-only; it carries into
any event kind.

| shorthand | example | emitted tag |
| --------- | ------- | ----------- |
| `{{@…}}`  | `{{@npub1…}}` / `{{@nprofile1…}}` | `["p", "‹pubkey›", "‹relay›"]` |

`embed` transcludes a whole event; `quote` carries an excerpt inline; `cite`
references an excerpt held in a reusable record. `slot` is the only block-level
prefix — inclusion by reference into the index tree, not the prose.

`{{wiki:topic}}` involves two tags that point in opposite directions. Resolution
*follows* **`d`**: the topic addresses its *definitional* event — the kind-30818
article (or a 30040 / 30041 section by its `T` title) whose `d`-tag is the
normalized topic. The citing event itself emits **`["w", topic]`** — an *outgoing*
usage marker recording that this event *uses* the term, carrying the topic only:
author-agnostic, no pinned version, the way a wiki interlinks every page that
invokes a concept. Both are single-letter and relay-indexed: query the `d`-tag to
reach what *defines* a term, the `w`-tag to find every event that *uses* it.

## Examples

```
{{ref:The Ascent}}
  → inline link to the sibling section titled "The Ascent"
{{ref:The Ascent|see above}}
  → the same link, shown as "see above"

{{wiki:proof of work}}   ≡   [[proof of work]]
  → link to the wiki article on the topic (d-tag "proof-of-work")
{{wiki:proof of work|PoW}}
  → the same link, shown as "PoW"

{{embed:naddr1…}}
  → transclude a publication / section / article / wiki inline, as a card
{{embed:nevent1…}}
  → quote-repost a note inline (NIP-18 `q` tag), as a card
{{embed:npub1…}}
  → a profile card: name, picture, bio
{{@npub1…}}   ≡   {{@nprofile1…}}
  → an inline @handle mention of the profile (not a card)

{{quote:naddr1… | And now, let me show in a figure how far our
nature is enlightened or unenlightened… }}
  → an attributed, collapsible blockquote; the excerpt is the inline text

{{slot:naddr1…}}                                   (on its own line)
  → make that 30040 / 30041 a child node of the enclosing publication index

{{cite:smith-2024}}                                (reserved)
  → a citation, rendered in the document's default style
{{cite:smith-2024|foot}}
  → the same citation, forced to a footnote
```

## Presentation

A reference declares *what*, not *how*. Render style (link, card, footnote,
endnote, blockquote) is decided downstream, in order of precedence:

1. a per-reference modifier (`{{cite:smith-2024|foot}}`);
2. a publication-level tag on the 30040 index (`["citation-style", "chicago"]`);
3. reader preference.

## Status

- *Shipped*: `ref`, `wiki` (`{{ }}` and `[[ ]]`), `embed` (naddr / nevent /
  note / npub / nprofile), `quote`, `slot`, `@` mention (npub / nprofile).
- *Reserved*: `cite` (kind 30161), `book`, `@name` (mention by name).

## Related

- NKBIP-01 (30040 / 30041)
- NKBIP-03 (`cite`; kind 30161)
- NKBIP-08 (`book`)
- NIP-19 (entities)
- NIP-54 (wiki; normalization; kind 30818)
- NIP-84 (highlights; the `quote` model)
- NIP-18 (the `q` tag)
- NIP-23 (kind 30023)
