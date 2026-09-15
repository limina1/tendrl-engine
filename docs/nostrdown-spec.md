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

## Roles, not scopes

A reference declares a *role* — what the author wants done with the event:

| role | prefix | the event is… |
| ---- | ------ | ------------- |
| point | `ref` | linked, in place |
| define | `wiki` | the topic's definition, linked |
| transclude | `embed` | shown, whole |
| excerpt | `quote` | attributed for a passage the author carries inline |
| include | `slot` | adopted into the document's own tree |
| mention | `@` | a person, linked by handle |

The syntax never decides *where* a reference may reach. Its **target form**
does: a title-slug or d-tag is a handle, and handles are local to the document
the reference lives in; a NIP-19 entity or a `kind:pubkey:d-tag` coordinate is
a pointer, and a pointer may name any event anywhere. An author who wants to
stay inside the document writes a handle; an author who wants to reach outside
writes the address. The same prefix serves both — the reader's client decides
only how to *present* the result (a sibling navigates in place; an outside
event opens).

## Grammar

```abnf
reference = "{{" prefix ":" target [ "|" modifier ] "}}"
mention   = "{{@" target [ "|" modifier ] "}}"   ; profile mention — target is an npub / nprofile (@name reserved)
prefix    = "ref" / "wiki" / "embed" / "quote" / "slot" / "cite"
target    = text     ; what is referenced
modifier  = text     ; what is shown in place (label, excerpt, or render directive)
text      = any characters up to the next "|" or "}}"
```

`target` is the only required part after the prefix. Its admissible form — a
title-slug, a NIP-19 entity (optionally `nostr:`-prefixed), or a
`kind:pubkey:d-tag` coordinate — is the same for every prefix; the grammar only
locates where it ends, and the form chooses the scope (see *Roles, not scopes*).

Parsing rules:

- Split on the *first* `:` only — colon-bearing targets (a coordinate, a
  `nostr:` URI) are preserved intact.
- `|` is optional and splits once; it runs to the closing `}}` as free text.
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

### The modifier is what shows in place

The `|modifier` is the author's answer to "what should the reader see here":

- on a pointer (`ref`, `wiki`, `@`) it is the link text; without it the client
  shows the resolved title (or the handle as written);
- on `embed` it turns the transclusion into an in-place reference: the label
  is shown inline and unfolds to the whole event on demand; **without** a
  modifier the embed *is* the whole event, rendered where the token stands;
- on `quote` it is the excerpt itself — the passage travels in the prose, the
  target attributes it;
- on `cite` it is a render directive (`foot`, `end`, `inline`, `quote`).

To reference one section of a publication, target that section directly — a
30041 section is an addressable event with its own coordinate. There is no
anchor-into-parent syntax; reference the child, not "parent + child".

## `[[ ]]` — the wikilink alias

`[[ ]]` is the established cross-tool wikilink and travels in published content
regardless, so it is recognized. A bare topic is a `wiki` reference; a pasted
NIP-19 entity or coordinate target is a *general Nostr pointer* — it resolves
as a direct link to that event and emits the pointer tag for its target type
(`a` / `q` / `p`), so an `[[naddr…][My Doc]]` copied from anywhere keeps
working instead of being slug-mangled into an unresolvable topic. The `{{ }}`
spelling of the same pointer is `{{ref:naddr…|My Doc}}`; prefer it in Org,
where `[[ ]]` is also Org's own link syntax.

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
tag dictated by the target (an entity is `a` / `q` / `p` by type, whatever the
prefix). `{{wiki:topic}}` and `[[topic]]` are two ways to write the one reference.

| prefix  | example | resolves to | emitted tag |
| ------- | ------- | ----------- | ----------- |
| `ref`   | `{{ref:The Ascent}}` / `{{ref:naddr1…\|NIP-42}}` | a handle → a sibling section (or nested index) of the same publication; a pointer → sibling-first, else the event it names, anywhere | handle: `["ref", "the-ascent"]`; pointer: `a` / `q` / `p` by target type |
| `wiki`  | `{{wiki:proof of work}}` | a sibling of the containing document first; else kind 30818 by `d` | `["w", "proof-of-work"]` |
| `embed` | `{{embed:naddr1…}}` | transclude the event inline; with a `\|label`, an in-place reference that unfolds to it | by target type (below) |
| `quote` | `{{quote:naddr1… \| And now, let me show in a figure…}}` | attributed blockquote; the excerpt is the modifier, authored inline (not fetched), the target resolves for attribution only | `["a", "30041:‹pk›:‹d›"]` `["p", "‹author›", "", "author"]` |
| `slot`  | `{{slot:naddr1…}}` | a compose-time operation: the addressable 30040 / 30041 it names becomes a child node of the enclosing index — inclusion into the tree, not the prose; no new event is minted. Conventionally written on its own line; renders as a preview card | `["a", "30041:‹pk›:‹d›"]` on the *30040 index* (nothing inline) |
| `cite`  | `{{cite:smith-2024}}` *(reserved)* | a kind-30161 citation record | `["cite", "smith-2024", "30161:‹pk›:smith-2024"]` |

`ref` accepts all three target forms, and the form the author writes decides
where the reference may reach — the syntax does not choose for them:

- a *title-slug or d-tag* is a handle, and handles are document-local: it
  matches a sibling by title-slug, `T` tag, or d-tag (both slug forms of each),
  and stays unresolved otherwise. A global slug match is deliberately not
  attempted — any same-titled section by anyone would become a target.
- an *naddr, coordinate or nevent* is a pointer and needs no scope: it is
  resolved sibling-first (an entity addressing a section or nested index of the
  containing document is an internal link, which a reader navigates in place),
  else as a direct link to the event it names, wherever it lives — the same
  resolution an entity wikilink gets, a not-yet-local event coming back
  `pending` until fetched.

A `ref` *links*; it never transcludes. `{{ref:naddr…|label}}` and
`[[naddr…][label]]` are therefore two spellings of one reference; use `embed`
for the event's content inline, `quote` for a passage of it.

`embed` tag by target type — the same `{{embed:…}}` token, tagged according to the target:

| target          | example | emitted tag |
| --------------- | ------- | ----------- |
| naddr / coordinate | `{{embed:naddr1…}}` / `{{embed:30040:‹pk›:‹d›}}` | `["a", "30040:‹pk›:‹d›", "‹relay›"]` |
| nevent / note   | `{{embed:nevent1…}}` | `["q", "‹id›", "‹relay›", "‹pubkey›"]` |
| npub / nprofile | `{{embed:npub1…}}`   | `["p", "‹pubkey›", "‹relay›"]` |
| title-slug      | `{{embed:The Ascent}}` | *(none)* — transcludes a sibling; the index's own `a` tags already address it |

A slug `{{embed:…}}` and a slug `{{ref:…}}` are deliberately distinct roles — a
ref *links* to a sibling and tags `["ref", slug]`; a slug embed *transcludes*
one and emits no tag of its own.

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
tag, which is how a *handle* finds a sibling. (A topic that names a sibling of
the containing document resolves document-locally first — an author's own
section title beats a same-named global article; a topic naming no sibling and
no 30818 stays unresolved rather than matching a stranger's section title.) The
citing event itself emits **`["w", topic]`** — an *outgoing* usage marker
recording that this event *uses* the term, carrying the topic only:
author-agnostic, no pinned version, the way a wiki interlinks every page that
invokes a concept. All three are single-letter and relay-indexed: query `d`
(wiki) or `T` (publications) to reach what *defines* a term, `w` to find every
event that *uses* it.

## Examples

```
{{ref:The Ascent}}
  → inline link to the sibling section titled "The Ascent"
{{ref:The Ascent|see above}}
  → the same link, shown as "see above"
{{ref:naddr1…|NIP-42}}   ≡   [[naddr1…][NIP-42]]
  → a link to the event behind the naddr, shown as "NIP-42": in place if it
    is a section of this document, otherwise it opens that event
{{ref:naddr1…}}
  → the same, shown as the event's title

{{wiki:proof of work}}   ≡   [[proof of work]]
  → link to the wiki article on the topic (d-tag "proof-of-work")
{{wiki:proof of work|PoW}}
  → the same link, shown as "PoW"

{{embed:naddr1…}}   ≡   {{embed:30041:‹pubkey›:‹d-tag›}}
  → transclude a publication / section / article / wiki inline, whole
    (an naddr and its coordinate are interchangeable)
{{embed:naddr1…|NIP-55 — Android signer}}
  → the same transclusion folded behind its label; unfolds in place
{{embed:nevent1…}}
  → quote-repost a note inline (NIP-18 `q` tag), as a card
{{embed:npub1…}}
  → a profile card: name, picture, bio
{{embed:The Ascent}}
  → transclude the sibling section titled "The Ascent" (no tag emitted)
{{@npub1…}}   ≡   {{@nprofile1…}}
  → an inline @handle mention of the profile (not a card)

{{quote:naddr1… | And now, let me show in a figure how far our
nature is enlightened or unenlightened… }}
  → an attributed, collapsible blockquote; the excerpt is the inline text,
    written by the author, and the target says whose words they are

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
endnote, blockquote, folded / unfolded) is decided downstream, in order of
precedence:

1. a per-reference modifier (`{{cite:smith-2024|foot}}`, a label on an embed);
2. a publication-level tag on the 30040 index (`["citation-style", "chicago"]`);
3. reader preference.

Navigation is likewise the client's: a pointer that resolves to a sibling of
the document being read is an in-place jump; one that resolves elsewhere opens
the event. The reference itself is the same either way.

## Revision notes

**Revision 2 (2026-09-15).** Written after using nostrdown to draft design
notes that cite specs and notes by other people. What clashed with the
intent, and how each was resolved:

- *A `ref` could not leave its document.* Revision 1 made `ref` sibling-only
  in every target form, so `{{ref:naddr…|label}}` to an outside event stayed
  unresolved and the only `{{ }}`-free way to point outside was the `[[ ]]`
  alias — the one syntax that collides with Org. Resolved: scope follows the
  target form (handle = local, pointer = anywhere, sibling-first). Entity refs
  now emit the pointer tag (`a` / `q` / `p`); `["ref", handle]` is reserved for
  the handle form, the only one that needs a resolution hint.
- *The modifier meant different things per prefix without saying so.*
  Resolved: the modifier is what shows in place. On an embed it folds the
  transclusion behind a label; no modifier means the whole event.
- *`quote` read as if it fetched the passage.* Clarified: the excerpt is the
  author's, carried inline (the NIP-84 model); the target attributes it.
- *`slot` read as if it minted something.* Clarified: it adopts an existing
  30040 / 30041 into the tree; nothing new is signed.
- *The `wiki` row still described a fallback to 30040 / 30041 by title-slug*,
  removed on 2026-08-12 because any same-titled section by anyone became a
  link target. The row now says 30818 by `d` only.
- *Still open:* prefix matching is case-sensitive while ABNF string literals
  are case-insensitive by convention.

**Revision 1 (2026-06-29)** — first published text (kind 30817, `d:nostrdown`).

## Status

- *Implemented* (tendrl engine): `ref` (handle and pointer forms, scope by
  target), `wiki` (`{{ }}` and `[[ ]]`, incl. entity / coordinate wikilinks as
  direct event links), `embed` (naddr / coordinate / nevent / note / npub /
  nprofile / sibling slug), `quote`, `slot`, `@` mention (npub / nprofile).
- *Specified, not yet implemented*: the folded rendering of an `embed` that
  carries a modifier (clients still render every embed as a card).
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
