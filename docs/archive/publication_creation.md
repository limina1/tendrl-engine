# Publication, Comment, and Highlight Creation — Design Doc

This document describes how Alexandria turns user input into signed Nostr
events. It covers three pipelines:

1. **Publications** (kinds `30040` index / `30041` section) — built from
   AsciiDoc by a tree processor and published from the compose page.
2. **Comments** (kind `1` for note replies, kind `1111` everywhere else) —
   built by NIP-22 threading helpers.
3. **Highlights** (kind `9802`) — built inline from a DOM `Selection`.

The publications pipeline is the deepest piece; the parser for nested
`30040`s is documented section-by-section below.

---

## 1. Publications

### 1.1 End-to-end flow

```
AsciiDoc source
       │
       ▼
parseAsciiDocWithTree()                 src/lib/utils/asciidoc_publication_parser.ts:24
  └─ Asciidoctor.load() with registered tree processor extension
       │
       ▼
registerPublicationTreeProcessor()      src/lib/utils/publication_tree_processor.ts:59
  ├─ extractContentSegments()                                            :152
  ├─ detectContentType()  →  "article" | "scattered-notes" | "none"      :374
  └─ buildEventsFromSegments()                                           :400
       ├─ buildScatteredNotesStructure()  (flat 30041s)                  :427
       └─ buildArticleStructure()                                        :475
            ├─ buildLevel2Structure()        (parseLevel === 2: flat)    :507
            └─ buildHierarchicalStructure()  (parseLevel ≥ 3: nested)    :570
       │
       ▼
ProcessorResult { tree, indexEvent, contentEvents, eventStructure }
       │
       ▼
exportEventsFromTree()                  src/lib/utils/asciidoc_publication_parser.ts:114
  └─ NDKEvent → plain { kind, content, tags, created_at, pubkey, id, title }
       │
       ▼
ZettelEditor.handlePublish()            src/lib/components/ZettelEditor.svelte:271
  └─ JSON.parse(JSON.stringify(...))   ← guard against non-serializable proxies
       │
       ▼
+page.svelte handlePublishArticle()     src/routes/new/compose/+page.svelte:82
  └─ for each event: publishSingleEvent()                                :106 / :127
       │
       ▼
publishSingleEvent()                    src/lib/services/publisher.ts:118
  ├─ Fix a-tag :pubkey: placeholder    :164
  ├─ Auto-add `author` + `p` tags     :182
  ├─ ndkEvent.sign()                   :208   ← uses NDK signer (NIP-07 in browser)
  └─ ndkEvent.publish(relaySet)        :211   ← all pool relays
```

The compose page is **purely a publisher** — it never builds tag arrays.
All structural decisions happen inside the tree processor.

### 1.2 Parse level — the controlling setting

The single most important knob in the publication pipeline is
**`parseLevel`**, the third argument to
`parseAsciiDocWithTree(content, ndk, parseLevel = 2)`
(`asciidoc_publication_parser.ts:24`). It decides *how deep the AsciiDoc
heading hierarchy is turned into nested `30040` indices* versus folded
into `30041` content. Full nesting is always available but rarely
needed — most documents want the flat shape, and that is the default.

**Where it's set.** The compose UI binds it to a dropdown in
`ZettelEditor.svelte` (`bind:value={parseLevel}`, `:929` and `:1498`),
populated by `generateParseLevelOptions(MIN_PARSE_LEVEL, MAX_PARSE_LEVEL)`
(`:137`). The default is **2** (`:39`). The valid range is **2–5**
(`MIN_PARSE_LEVEL` / `MAX_PARSE_LEVEL`, `:63-64`; also `validateParseLevel()`
and `getSupportedParseLevels()` at `asciidoc_publication_parser.ts:149` /
`:156`). The processor module docstring mentions NKBIP-01 levels 2–7, but
the app clamps the selector to 2–5.

**Mental model.** `parseLevel` is the depth at which sections stop being
*indices* and become *terminal content*:

- A section **shallower** than `parseLevel` that has children becomes an
  index — it emits a `30040` **plus** a `30041` for its own prose.
- A section **at** `parseLevel` becomes a leaf `30041` content event.
- A section **deeper** than `parseLevel` is never its own event — its text
  is folded verbatim into the nearest ancestor's `30041` content, child
  headers and all.

Two cooperating cutoffs enforce this:

- `collectSectionsAtLevel(hierarchy, targetLevel)`
  (`publication_tree_processor.ts:230`) only collects sections with
  `level >= 2 && level <= parseLevel`, so deeper headings never become
  their own segments.
- `extractSegmentContent()` (`:257`) slices each segment from its header to
  the next sibling-or-shallower header, and `parseSegmentContent()` (`:311`)
  keeps child headers in `content` — so the absorbed deeper text round-trips
  as AsciiDoc inside the ancestor.

**What each level produces** for a document using `==` / `===` / `====`
headings:

| `parseLevel`    | Shape             | `30040` indices                       | `30041` content                                                |
| --------------- | ----------------- | ------------------------------------- | -------------------------------------------------------------- |
| **2** (default) | Flat — one tier   | 1 (root only)                         | one per `==`; each carries its `===`/`====` subtree **inline** |
| **3**           | One nested tier   | root + each `==` *with children*      | each `==` (own prose) + each `===`; `====` folded into `===`   |
| **4**           | Two nested tiers  | root + `==`/`===` *with children*     | `==`,`===` own prose + each `====`                             |
| **5**           | Three nested tiers| root + `==`/`===`/`====` *w/ children* | leaf `=====` content + each tier's own prose                   |

A `==` section with **no** children is always a plain `30041`, regardless
of level (`processHierarchicalGroup`, `:1003`).

**Level 2 and Level 3+ are two distinct code paths.**
`buildArticleStructure()` (`:475`) branches on `parseLevel === 2`:

- **Level 2 → `buildLevel2Structure()` (`:507`)** — the flat "single
  `30040` over a flat list of `30041` documents" shape that the user
  usually wants. `groupSegmentsByLevel2()` (`:878`) re-serialises every
  deeper subsection back into its parent `==` section's content
  (`combinedContent += "\n\n" + "=".repeat(level) + …`), so the entire
  subtree lives inside one content event.
- **Level 3+ → `buildHierarchicalStructure()` (`:570`)** — the recursive
  nested build documented in Stage 4 (§1.3.4). `buildHierarchicalGroups()`
  buckets only levels `2..parseLevel` (`:924`), and `buildNodeHierarchy()`
  stops recursing once `child.level >= parseLevel` (`:963`).

So: a user who wants a **flat publication** picks Level 2; a user who wants
the table of contents to fan out into sub-indices picks the depth they care
about and lets everything below it collapse into prose.

### 1.3 Parser: nested 30040 publications

The parser lives in two files. The public entry point thinly wraps
Asciidoctor and registers a tree processor extension:

```ts
// src/lib/utils/asciidoc_publication_parser.ts:24
export async function parseAsciiDocWithTree(
  content: string,
  ndk: NDK,
  parseLevel: number = 2,
): Promise<PublicationTreeResult>
```

The processor extension (`publication_tree_processor.ts:59`) runs inside
Asciidoctor's `treeProcessor` hook and owns all event construction.

#### 1.3.1 Stage 1 — Asciidoctor parse + AST hierarchy

`parseAsciiDocWithTree()` constructs a fresh `Asciidoctor()` processor,
creates an extension registry, and registers the tree processor with the
**original source string** captured in the closure
(`publication_tree_processor.ts:91`).

Inside `process(doc)`:

- `doc.getTitle()` → publication title
- `doc.getAttributes()` → document-level attributes (author, version, …)
- `doc.getSections()` → top-level AST sections

`buildSectionHierarchy()` (`publication_tree_processor.ts:182`) converts
Asciidoctor's nested AST into our `SectionNode` tree, normalising the
level by `+1` (Asciidoctor counts `==` as level 1; the app counts it as
level 2 — the convention is consistent across `asciidoc_ast_parser.ts:66`
and `publication_tree_processor.ts:186`).

#### 1.3.2 Stage 2 — Segment extraction from original text

`extractContentSegments()` walks `SectionNode`s collected by
`collectSectionsAtLevel()` (`:230`) and, for each section, locates its
literal header line in the original source using:

```ts
new RegExp(`^${"=".repeat(section.level)}\\s+${escapeRegex(section.title)}`)
```

It then scans forward until it hits a sibling-or-higher header
(`levelMatch[1].length <= section.level`) to find the end of the section
(`:283-290`).

Each resulting `ContentSegment` carries:

```ts
{ title, content, level, attributes, startLine, endLine }
```

Section attributes (`:key: value` lines immediately following the
header) are stripped into `attributes` and excluded from `content`
(`parseSegmentContent`, `:311`). Crucially, **child headers ARE
included** in `content` — content events embed their full subtree as
AsciiDoc text, while index events get child references via `a` tags
(`:347-353`).

#### 1.3.3 Stage 3 — Content type detection

`detectContentType()` (`:374`):

| Has doc title | Has sections | Title == first section | Result            |
| ------------- | ------------ | ---------------------- | ----------------- |
| yes           | yes          | no                     | `article`         |
| any           | yes          | yes (or no doc title)  | `scattered-notes` |
| —             | no           | —                      | `none`            |

The "title == first section" check prevents single-section docs from
emitting a `30040` index that just points at one `30041`.

#### 1.3.4 Stage 4 — Event construction

##### Scattered notes (flat 30041)

`buildScatteredNotesStructure()` (`:427`) emits one `30041` per segment,
no `30040`. The first event becomes the `PublicationTree` root and
subsequent events are added under it. No `indexEvent`.

##### Article, Level 2 — single tier

`buildLevel2Structure()` (`:507`) emits:

- **One root `30040`** for the publication
- **One `30041` per `==` section**, with all `===`/`====` subsections
  re-serialised into its content by `groupSegmentsByLevel2()` (`:878`)

`groupSegmentsByLevel2()` walks segments, and for each `level === 2`
section finds all deeper segments whose `startLine` falls before the
next level-2 boundary. They're appended back into the parent's content
verbatim:

```ts
combinedContent += `\n\n${"=".repeat(nested.level)} ${nested.title}\n${nested.content}`;
```

So at Level 2 the AsciiDoc subtree round-trips through the parser.

##### Article, Level 3+ — nested 30040s

`buildHierarchicalStructure()` (`:570`) is the core of nested
publications. It runs in three passes:

1. **`buildHierarchicalGroups()` (`:916`)** — converts the flat segment
   list into a `HierarchicalNode` tree by re-deriving parent/child
   relationships from `startLine` ranges (`buildNodeHierarchy()`,
   `:940`). A segment `s` is a direct child of `segment` if
   `s.level === segment.level + 1` and `s.startLine` is between
   `segment.startLine` and the next sibling-or-higher segment.

2. **`processHierarchicalGroup()` (`:992`)** — recursive walk that
   emits events per node:

   ```
   For each node:
     IF node.hasChildren AND node.level < parseLevel:
       emit 30040 index for this node          ← createIndexEventForHierarchicalNode (:1083)
       emit 30041 content for this node        ← createContentEvent (:657)
       recurse into children
     ELSE:
       emit 30041 content only
   ```

   Every intermediate node gets **two events**: a `30040` index whose
   first `a` tag points at its own content `30041`, plus the `30041`
   that holds the section's prose. This lets readers fetch the index
   alone for navigation and pull the prose lazily.

3. **Event structure tracking** — an `EventStructureNode` tree is built
   in lockstep with the events for the preview UI
   (`:1027-1045`). The structure mirrors the event tree exactly, with
   `30040` nodes containing their own `30041` content node as the first
   child, followed by descendant index/content pairs.

#### 1.3.5 Stage 5 — d-tag namespacing

Section `d` tags are namespaced by the publication's abbreviation to
prevent collisions across publications (`generateTitleAbbreviation()`,
`:740`):

```
"My Test Article"  →  "mta"
"Untitled"          →  "u"   (fallback)
```

For a section titled "Background" inside "My Test Article" the section
`d` tag becomes `mta-background` and the index `a` tag is:

```
["a", "30041:<pubkey>:mta-background"]
```

**Index events keep an un-namespaced `d` tag** (`generateDTag(title)`
only). Only the child `a` tags that point at `30041`s use the namespace.
Child indices (`30040`) are referenced by their bare `d` tag
(`:1117-1119`). This is intentional: indices are publication-level
identifiers; sections are publication-scoped.

#### 1.3.6 Stage 6 — Tag assembly

Index events (`:616`, `:1083`):

```
["d", dTag]
["m", mime], ["M", mimeMajor]     ← getMimeTags(30040)
["title", title]
…document/section attribute tags  (author, version, t, summary, etc.)
["p", pubkey]                      (root index only — :783)
["a", "30041:<pubkey>:<ns-dtag>"]  (own content event)
["a", "30040:<pubkey>:<dtag>"]    (child indices)
["a", "30041:<pubkey>:<ns-dtag>"] (child content)
```

Content events (`:657`):

```
["d", "<abbrev>-<section-dtag>"]
["m", mime], ["M", mimeMajor]      ← getMimeTags(30041)
["title", section.title]
…section attribute tags
…wiki link tags from content       ← extractWikiLinks() (:684)
```

NKBIP-01 requires **index events to have empty content** — enforced at
`:649` and `:1127`.

System attributes injected by Asciidoctor (`asciidoctor-version`,
`localdate`, `doctitle`, etc.) are filtered out by `addCustomAttributes()`
(`:808`).

#### 1.3.7 Stage 7 — Async tree relationships

After the tree processor returns, `buildTreeRelationships()`
(`asciidoc_publication_parser.ts:82`) walks `contentEvents` and calls
`tree.addEvent(child, parent)` on the `PublicationTree`. For articles,
all content events are attached to the root index event; for scattered
notes, subsequent events attach to the first one. The `tree` field is
**deliberately omitted** from `exportEventsFromTree()` (`:120`) because
it carries non-serialisable state and the publish step only needs the
flat event objects.

### 1.4 Publish step

`publishSingleEvent()` (`src/lib/services/publisher.ts:118`) is the
single choke point for signing and broadcasting:

- `VITE_MOCK_PUBLISH === "true"` short-circuits with a fake hex event ID
  for UI tests (`:130`).
- **a-tag placeholder fix-up** (`:164`): the parser emits
  `30041:<pubkey>:…` using `ndk.activeUser?.pubkey || "preview-placeholder-pubkey"`.
  At publish time the placeholder string `pubkey` (legacy path,
  `asciidoc_ast_parser.ts:171`) is replaced with the real pubkey. The
  newer processor already inlines the real pubkey, but this safety net
  remains for events constructed elsewhere.
- **Identity injection** (`:182-198`): if no `author` tag is present,
  the active user's display name is added; if no `p` tag is present,
  their pubkey is added.
- **Signing** is delegated to NDK (`ndkEvent.sign()`, `:208`). NDK in
  turn uses whatever signer was attached to the instance — in practice
  a NIP-07 signer (browser extension) wired through `ndk.ts`.
- **Publishing** broadcasts to **every relay in the pool**
  (`Array.from(ndk.pool?.relays.values())`). Success is determined by
  `publishedToRelays.size > 0`.

The compose page publishes the index event first
(`+page.svelte:106`), then iterates content events sequentially
(`:122-140`). Sequential publishing is intentional — relays need the
referenced `30041`s to exist when readers resolve `a` tags from the
`30040`.

---

## 2. Comments

Comments use **NIP-22 threading** for everything except kind-1 replies.
The pipeline is fully separate from publications.

### 2.1 Flow

```
Reply UI (CommentBox, Publication, SectionComments, CardActions)
       │
       ▼
extractRootEventInfo(parent)            src/lib/utils/nostrEventService.ts:71
extractParentEventInfo(parent)                                          :120
buildReplyTags(parent, rootInfo, parentInfo, kind)                      :199
       │
       ▼
createSignedEvent(content, pubkey, kind, tags)                          :315
  ├─ prefixNostrAddresses(content)      (inline nevent/naddr fix-up)
  ├─ if kind === 24: add ["expiration", …]                              :326
  ├─ Try window.nostr.signEvent()       (NIP-07)                        :349
  └─ Fall back to local signEvent()
       │
       ▼
publishEvent(signedEvent, relayUrls, ndk)                               :378
```

### 2.2 Tag construction (`buildReplyTags`)

`kind` selection (`CommentBox.svelte:185`):

```ts
const kind = parent.kind === 1 ? 1 : 1111;
```

Kind 1 replies stay on the legacy `e/p` shape (`:213-228`):

```
["e", parent.id, parentRelay, "root"]
["p", parentPubkey]
["a", "<kind>:<pubkey>:<d>", "", "root"]   ← only for addressable parents
```

Kind 1111 follows NIP-22, with **uppercase** tags for the *root scope*
(the original article being commented on) and **lowercase** for the
*parent scope* (what we're directly replying to):

```
top-level comment on an addressable event (parent IS root):
  ["A", parentAddress, parentRelay]
  ["K", rootKind]
  ["P", rootPubkey, rootRelay]
  ["a", parentAddress, parentRelay]
  ["e", parent.id, parentRelay]
  ["k", parentKind]
  ["p", parentPubkey, parentRelay]

reply to a comment (parent ≠ root):
  ["A", parentAddress, parentRelay]   ← still the original article
  ["K", rootKind]
  ["P", rootPubkey, rootRelay]
  ["e", parent.id, parentRelay]       ← but the comment we reply to
  ["k", parentKind]
  ["p", parentPubkey, parentRelay]
```

When the parent is non-addressable, `E/e` substitute for `A/a`
(`:267-289`).

`extractRootEventInfo()` chases the root through the parent's tags: if
the parent already has `E`/`A`/`I` tags (because it is itself a reply),
we use those; otherwise the parent IS the root (`:86-114`).

### 2.3 Signing

`createSignedEvent()` (`:315`) is reused for several event kinds
including comments and DM-style kind-24 replies. Behaviour:

- Inline `nostr:` prefixing of embedded references in `content`
  (`prefixNostrAddresses`).
- Kind 24 gets an `expiration` tag (NIP-40) computed from
  `EXPIRATION_DURATION`.
- Tag values are stringified and normalised to a 4-tuple shape
  (`tag[0..3]`) before hashing (`:338-343`).
- Prefers `window.nostr.signEvent()` (NIP-07); falls back to a local
  `signEvent()` via `nostrUtils.ts`.

The returned envelope `{ id, sig, event }` is then passed to
`publishEvent()` (`:378`), which wraps it in `NDKEvent` if needed and
broadcasts with a 5s timeout.

---

## 3. Highlights

Highlights (`kind 9802`) are constructed inline in
`HighlightSelectionHandler.svelte` (`:139-203`) — they don't go through
any shared service.

### 3.1 Flow

1. The handler captures a DOM `Selection`, finds the enclosing
   `section[id]` element (a `PublicationSection`), and reads
   `data-event-address` + `data-event-id` off the DOM (`:104-105`).
2. On confirm, an `NDKEvent` is built with:

   ```
   kind:    9802
   content: selectedText
   pubkey:  $userStore.pubkey
   tags:
     ["a", "<kind>:<pubkey>:<d>", ""]     (preferred — addressable section)
     OR
     ["e", sectionEventId, ""]            (fallback)
     ["context", surroundingText]?        (paragraph/section text)
     ["p", authorPubkey, "", "author"]    (extracted from address[1])
     ["comment", userComment]?            (quote highlights)
   ```

3. To dodge proxy issues when serialising for the signer, the event is
   first cloned into a plain object (`:183-189`), then signed via
   NIP-07 (`window.nostr.signEvent`) with a fallback to the user's NDK
   signer (`:191-203`).
4. Relays are the union of `communityRelays`, `activeOutboxRelays`, and
   `activeInboxRelays` (`:206-209`).

Highlights are **not** addressable (no `d` tag), so encoding them for
sharing uses `nevent` rather than `naddr`
(`src/lib/utils/highlightUtils.ts:60-90`).

---

## 4. Common concerns

### 4.1 Signing

| Path                 | Signer                                                                |
| -------------------- | --------------------------------------------------------------------- |
| Publications         | `ndkEvent.sign()` — NDK delegates to its configured signer (NIP-07).  |
| Comments / kind-24   | `window.nostr.signEvent()` → falls back to local `signEvent()`.       |
| Highlights           | `window.nostr.signEvent()` → falls back to `event.sign(userSigner)`.  |

There's no shared "signer" abstraction in the web UI — each path picks
its own. The Rust engine has a `Signer` trait (`src/signing.rs`), but
the web client signs locally.

### 4.2 d-tag generation

Three sites generate `d` tags; all produce identical output for the
same input:

- `asciidoc_ast_parser.ts:226`
- `asciidoc_publication_parser.ts` (re-exported)
- `publication_tree_processor.ts:724`
- `services/publisher.ts:384` (simpler form — keeps `_` and `-`)

```ts
title.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "-").replace(/-+/g, "-").replace(/^-|-$/g, "")
```

`publisher.ts:384` uses a stricter `[^\w\s-]` strip and only collapses
spaces, so it's not interchangeable with the others. That's fine
because `publisher.ts` only generates `d` tags for the legacy
`publishZettel()` path, not for tree-built publications.

### 4.3 Relay selection

| Path                   | Relay set                                                     |
| ---------------------- | ------------------------------------------------------------- |
| `publishSingleEvent`   | Every relay in `ndk.pool` (broadcast-everywhere).             |
| `CommentBox`           | `$activeOutboxRelays`, optionally + `$activeInboxRelays`.     |
| `HighlightSelectionHandler` | `communityRelays ∪ activeOutboxRelays ∪ activeInboxRelays`. |

There's no central relay policy — each surface picks. Worth
consolidating later (see *Future work*).

---

## 5. Known issues / future work

- **Two parser implementations coexist.** `asciidoc_ast_parser.ts`
  (flat) and `asciidoc_publication_parser.ts` (tree processor) overlap;
  `doc/compose_tree.md` flags this. The flat AST parser is still used
  by `createPublicationTreeFromAST()` but has no live caller from the
  compose page — `ZettelEditor.svelte:103` exclusively uses the tree
  processor.
- **`asciidoc_parser.ts::generateNostrEvents`** is a third nested-event
  generator with its own a-tag construction; it isn't on the active
  compose path but may still be referenced by tests / scattered notes
  flows. Confirm and remove if dead.
- **Root index over-references content events at Level 3+.**
  `buildArticleStructure()` builds the root `30040` via
  `createIndexEvent(title, attributes, segments, …)` (`:487`) using the
  *full flat* segment list, and that function emits an `["a", "30041:…"]`
  tag for *every* segment (`:640-644`). At Level 3+ this means the root
  index points at the namespaced `30041` **content** d-tag of intermediate
  sections that are *also* published as their own `30040` index — so the
  root references the prose copy rather than the sub-index, and level-3
  sections end up referenced twice (once by the root, once by their `==`
  parent index). The nested `createIndexEventForHierarchicalNode()`
  (`:1083`) discriminates `30040` vs `30041` correctly; only the root index
  is built flat. Reconcile so the root references its direct children as
  indices.
- **Index `d` tags aren't namespaced.** Child `30040` indices in a
  deeply-nested publication will collide if two publications use the
  same section title (e.g. "Introduction"). Either namespace them too
  or scope by `pubkey:d_tag` consistently at lookup time.
- **Relay policy fragmentation.** Three publishers, three policies. A
  shared `selectRelays(intent)` helper would let the engine's
  `NetworkMode` (`src/network.rs`) gate user-initiated fetches and
  publishes uniformly.
- **Signer abstraction.** The engine already has `signing::Signer`;
  the web UI doesn't. A web-side analog would let highlights and
  comments share the publication signing path (and make NIP-46 wiring
  straightforward).

