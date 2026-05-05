#+TITLE: Reading / Editing / Knowledge Workflow
#+SUBTITLE: Transclusion, nested publications, and search-to-source navigation
#+DATE: 2026-05-04
#+STATUS: DRAFT — design vision; implementation pending

* Scope

This doc captures the next horizon for the reading and authoring workflow
on top of the WM shell: how transclusion, nested 30040 publications, and
search results compose into a coherent navigation model.

It is a *design vision*, not an implementation plan. Concrete tasks are
listed at the bottom and tracked in =wm-shell.md= as roadmap items.

Adjacent docs:
- =docs/nostrdown.org= — the ={{ }}= reference syntax and parser model
- =docs/wm-shell.md= — the running WM shell roadmap
- =docs/publication-architecture.org= — kind 30040 / 30041 semantics

* Three composable primitives

** Transclusion (={{ }}=)

Nostrdown's ={{ }}= is the reference primitive. A section's body can
contain ={{kind:pubkey:dtag}}= or ={{event:id}}= and the renderer pulls in
the referenced content inline.

- *Resolution policy* mirrors =FetchPolicy=: =LocalOnly= renders only
  in-cache references; =LocalFirst= triggers a relay backfill.
- *Render modes*:
  - =inline= — the referenced content replaces the marker
  - =quoted= — content is shown in a callout block with an attribution
  - =collapsed= — only the title + author + a one-line preview, click to
    expand
- *Cycles* are detected at parse time (a section can't transclude itself,
  directly or indirectly).
- *Editing* — in compose, a transcluded reference is read-only. Forking
  it copies the content into a new editable block and adds an =a= tag for
  lineage (NIP-54 marker), the same flow we already use for imported
  sections.

** Orgdown / format adapters

Sections carry a =content_mode= tag (=md=, =org=, =adoc=, =plain=). The
={{ }}= syntax is markup-agnostic; format adapters render the surrounding
text per its mode while leaving references unchanged.

The default authoring target is *AsciiDoc* (admonitions, structured
attributes, transclusion-friendly include syntax). The wysiwyg editor
will write AsciiDoc; ={{ }}= references survive round-tripping because
they're not part of the host markup.

** Nested publications (30040-in-30040)

A 30040 index can list other 30040s alongside 30041 sections. This
gives us hierarchy:

#+begin_example
collection (30040)
├── book A (30040)
│   ├── chapter 1 (30040)
│   │   ├── §1.1 (30041)
│   │   └── §1.2 (30041)
│   └── chapter 2 (30040)
└── book B (30040)
    └── §1 (30041)
#+end_example

We support arbitrary depth, but UX optimizes for *=n=3= levels* (the
deepest the user typically sees in one viewport): collection → book →
chapter, with sections as the leaves.

* The reading model

** Default packed state

Opening a 30040 shows its TOC. Nested 30040 children render as **packed
cards** — just the title, author, child count, and a small =▸= chevron.
Direct 30041 children render their preview as today.

Packed cards do *not* trigger child loads. Only the cursored or
explicitly-clicked card unpacks.

** Unpacking

Two unpacking gestures:

1. *Inline expand* — click =▸= or press =l= / =Enter= on the cursored
   card. The child TOC drops in below the row, in a nested outline frame.
   Cursor descends into it. Pressing =h= / =Esc= collapses back.

2. *Drill in* — double-click or press =L= on the cursored card. The
   reader buffer's view replaces with the child publication, with a
   breadcrumb at the top. Cursor starts at child's first row. =h= /
   =Backspace= ascends.

Inline expand is for browsing structure; drill-in is for reading. The
WM-shell ranger nav primitives (cursor, =gg= / =G=, drill axis) already
exist — nested unpacking is just another layer of the same model.

** Navigation tree (full-screen mode)

When drilled into a child, a left-rail navigation tree shows the path
from root with siblings at each level. Clicking a node drills there.

The tree is stateless — it derives from the current path + the cached
TOCs of each ancestor. No persistent UI state to manage.

** Cross-document navigation

From any 30041 section view, =SPC g p= ("go parent") jumps the reader
buffer to the containing 30040. From a 30040, =SPC g c= jumps to the
parent collection if one exists.

Parent lookup uses the =a= tag on the 30040 → query for any 30040 that
references this address. If multiple, prompt; if exactly one, jump.

* Search-to-source

A search result for a 30041 section is currently a dead end — clicking
it opens the section in isolation. The new workflow:

1. Result row shows *containment breadcrumb* below the title:
   =collection › book › chapter=. Each segment is clickable.
2. Default click on the row → opens the section *in the context of its
   chapter* (the parent 30040 unpacked, cursor on the result section).
3. Alt-click → opens the section standalone (current behavior).

This requires resolving each result's parent chain at search time. We
already have the data — the section's =a= tag points to its index — so
this is a UI-side enrichment, not new engine work.

* Implementation outline

** Engine

- =GET /api/v1/publications/:addr/parents= — returns ancestors by walking
  =a= tag references. =LocalOnly= by default; can hit relays to find
  collections we don't have cached.
- =GET /api/v1/publications/:addr/children/meta= — for a 30040, return
  child kind + title + section count without loading section bodies.
  Lets packed cards render without paying the cost of fetching every
  descendant.

** Web

- =LazyTreeNode= type alongside =LazySection=: shape is similar but the
  node's =children= field is itself a =LazyTreeNode[]= (or a
  =LoadStatus<LazyTreeNode[]>= for lazy expansion).
- ReaderBuffer extended: outline view recurses into nested 30040 cards,
  rendering them packed by default. =l= / =Enter= triggers an inline
  expand (loads child TOC) or a drill-in (replaces the buffer's root
  publication address) depending on modifier.
- DraftReader gets the same packed/unpacked rendering for consistency
  when authoring nested publications.
- SearchPanel result row gains a breadcrumb resolved from a new
  =resolve_breadcrumb()= helper that walks =a= tags up.

** Compose

- The =+= transclusion picker (already on the Polish list) becomes the
  primary way to nest a publication inside another: select a 30040 →
  composer adds it as a packed child instead of as an editable section.
- Authoring mode bar gains a =Nest= button that prompts for an existing
  30040 to embed as a child entry of the current 30040.

* Open questions

- *Loading discipline.* Eager-loading every level of a deep tree is
  expensive. Probably: eager-load only the cursored path (root TOC →
  cursored child's TOC) and lazy-load siblings on cursor-enter. We
  already use that pattern in ReaderBuffer at the section level.
- *Cycles in nested 30040s.* A 30040 can transclude an ancestor by
  accident or by design (e.g., a "hub" page). Decide: detect at render
  time and replace with a placeholder, or allow but cap depth.
- *Search context.* Where does the result-breadcrumb resolve from when
  the parent 30040 isn't local? Probably: best-effort local lookup; if
  missing, show =? › ? › chapter= and let the user click to fetch.
- *URL deep-linking.* When a user shares a link to "section 1.2 in
  chapter 1 of book A," what does that look like? Likely:
  =/p/<book>/<chapter>/<section>= as path segments, with the WM shell
  spawning a reader buffer that drills to the right depth.

* Roadmap touchpoints

These items are tracked in =docs/wm-shell.md=:

- DraftReader transclusion =+= picker (existing item; expand to support
  packed-child insertion)
- Nested-publication packed/unpacked rendering in ReaderBuffer (new)
- Parent / breadcrumb endpoints + UI surfacing (new)
- Drill-in / drill-out navigation with breadcrumb chrome (new)
- Search result containment breadcrumb (new)
