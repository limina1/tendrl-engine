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
  matched verbatim — a `kind:pubkey:d-tag` coordinate is equivalent to its
  `naddr` and is admissible wherever an entity is (a parser may canonicalize
  the coordinate to that naddr internally; it is never slug-normalized).
- Prefixes are written lowercase; matching is case-sensitive (`{{Ref:…}}` is
  literal text).
- A token with an unknown prefix or a malformed body is left as literal text.
- `{{@target}}` is the mention shorthand — the `@` stands in for `prefix:`
  (see *Prefixes*).

To reference one section of a publication, target that section directly — a
30041 section is an addressable event with its own coordinate. There is no
anchor-into-parent syntax; reference the child, not "parent + child".

## `[[ ]]` — the wikilink alias

`[[ ]]` is the established cross-tool wikilink and travels in published content
regardless, so it is recognized. A bare topic is a `wiki` reference; a pasted
NIP-19 entity or coordinate target is a *general Nostr pointer* — it resolves
as a direct link to that event and emits the pointer tag for its target type
(`a` / `q` / `p`), so an `[[naddr…][My Doc]]` copied from anywhere keeps
working instead of being slug-mangled into an unresolvable topic.

```abnf
wikilink = "[[" target [ ( "][" / "|" ) display ] "]]"   ; topic (NIP-54 normalized), entity, or coordinate
```

| form                  | meaning                              |
| --------------------- | ------------------------------------ |
| `[[topic]]`           | wiki, label = topic                  |
| `[[d-tag][display]]`  | wiki, bracketed display form         |
| `[[topic\|display]]`  | wiki, piped display form             |
| `[[naddr1…][display]]`| direct event link (entity target)    |

A target that is itself a link, scheme (`scheme:`, `://`), path, anchor, or
image / media file is left to the host markup and never claimed. For topic
targets, tag and resolution are identical to `{{wiki:topic}}`.

## Prefixes

Each prefix shares the grammar above and differs only in resolution and tag.

A prefix names the reference's *role* — the human-facing vocabulary the author
writes. The *tag* it emits is the Nostr-native encoding, and need not share the
prefix's name: a single-letter tag where one fits (`wiki` → `w`), otherwise the
tag dictated by the target (an `embed` is `a` / `q` / `p` by entity type).
`{{wiki:topic}}` and `[[topic]]` are two ways to write the one reference.

| prefix  | example | resolves to | emitted tag |
| ------- | ------- | ----------- | ----------- |
| `ref`   | `{{ref:The Ascent}}` | a sibling section (or nested index) of the same publication — and *only* a sibling; a ref never leaves its document | `["ref", "the-ascent"]` |
| `wiki`  | `{{wiki:proof of work}}` | a sibling of the containing document first; else kind 30818 by `d`; else 30040/41 by `T` title-slug | `["w", "proof-of-work"]` |
| `embed` | `{{embed:naddr1…}}` | transclude the event inline, as a card | by target type (below) |
| `quote` | `{{quote:naddr1… \| And now, let me show in a figure…}}` | attributed blockquote; the excerpt is inline, the target resolves for attribution only | `["a", "30041:‹pk›:‹d›"]` `["p", "‹author›", "", "author"]` |
| `slot`  | `{{slot:naddr1…}}` | an editor / compose-time operation: the referenced event (addressable 30040 / 30041) becomes a child node of the enclosing index — inclusion into the tree, not the prose. Conventionally written on its own line; renders as a preview card | `["a", "30041:‹pk›:‹d›"]` on the *30040 index* (nothing inline) |
| `cite`  | `{{cite:smith-2024}}` *(reserved)* | a kind-30161 citation record | `["cite", "smith-2024", "30161:‹pk›:smith-2024"]` |

`ref` accepts all three target forms, but resolution is always against the
sibling index: a title-slug or d-tag matches by handle; an naddr / coordinate
matches only if it addresses a sibling, and stays unresolved otherwise (point
at arbitrary events with `embed` or an entity wikilink). The entity form's tag
carries the sibling's d-tag: `["ref", "‹d-tag›"]`.

`embed` tag by target type — the same `{{embed:…}}` token, tagged according to the target:

| target          | example | emitted tag |
| --------------- | ------- | ----------- |
| naddr / coordinate | `{{embed:naddr1…}}` / `{{embed:30040:‹pk›:‹d›}}` | `["a", "30040:‹pk›:‹d›", "‹relay›"]` |
| nevent / note   | `{{embed:nevent1…}}` | `["q", "‹id›", "‹relay›", "‹pubkey›"]` |
| npub / nprofile | `{{embed:npub1…}}`   | `["p", "‹pubkey›", "‹relay›"]` |
| title-slug      | `{{embed:The Ascent}}` | *(none)* — transcludes a sibling; the index's own `a` tags already address it |

A slug `{{embed:…}}` and a `{{ref:…}}` are deliberately distinct roles — a ref
*links* to a sibling and tags `["ref", slug]`; a slug embed *transcludes* one
and emits no tag of its own.

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
finds the topic's *definitional* event, and the tag semantics split by kind —
this separation matters: for a kind-30818 wiki article the `d`-tag **is** the
topic name, so 30818 resolves by `d`; publication indexes and sections
(30040/30041) mint opaque `d`-tags and carry their title-slug in the **`T`**
tag, so they resolve by `T`. (A topic that names a sibling of the containing
document resolves document-locally first — an author's own section title beats
a same-named global article.) The citing event itself emits **`["w", topic]`**
— an *outgoing* usage marker recording that this event *uses* the term,
carrying the topic only: author-agnostic, no pinned version, the way a wiki
interlinks every page that invokes a concept. All three are single-letter and
relay-indexed: query `d` (wiki) or `T` (publications) to reach what *defines*
a term, `w` to find every event that *uses* it.

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

{{embed:naddr1…}}   ≡   {{embed:30041:‹pubkey›:‹d-tag›}}
  → transclude a publication / section / article / wiki inline, as a card
    (an naddr and its coordinate are interchangeable)
{{embed:nevent1…}}
  → quote-repost a note inline (NIP-18 `q` tag), as a card
{{embed:npub1…}}
  → a profile card: name, picture, bio
{{embed:The Ascent}}
  → transclude the sibling section titled "The Ascent" (no tag emitted)
{{@npub1…}}   ≡   {{@nprofile1…}}
  → an inline @handle mention of the profile (not a card)

[[naddr1…][The Republic]]
  → a direct link to the event behind the naddr, shown as "The Republic"
{{ref:naddr1…}}
  → a sibling link, iff that naddr addresses a section of this document;
    unresolved otherwise (a ref never leaves its publication)

{{quote:naddr1… | And now, let me show in a figure how far our
nature is enlightened or unenlightened… }}
  → an attributed, collapsible blockquote; the excerpt is the inline text

{{slot:naddr1…}}                                   (conventionally on its own line)
  → make that 30040 / 30041 a child node of the enclosing publication index
    (a compose-time operation — the tag lands on the index, not the section)

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

- *Shipped*: `ref` (sibling-only, all three target forms), `wiki` (`{{ }}` and
  `[[ ]]`, incl. entity/coordinate wikilinks as direct event links), `embed`
  (naddr / coordinate / nevent / note / npub / nprofile / sibling slug),
  `quote`, `slot`, `@` mention (npub / nprofile).
- *Reserved*: `cite` (kind 30161), `book`, `@name` (mention by name).
- *Known clash*: prefix matching is case-sensitive while ABNF string literals
  are case-insensitive by convention — case-insensitive prefixes are a planned
  fix (tracked in `docs/bugs.org`).

## Related

- NKBIP-01 (30040 / 30041)
- NKBIP-03 (`cite`; kind 30161)
- NKBIP-08 (`book`)
- NIP-19 (entities)
- NIP-54 (wiki; normalization; kind 30818)
- NIP-84 (highlights; the `quote` model)
- NIP-18 (the `q` tag)
- NIP-23 (kind 30023)
