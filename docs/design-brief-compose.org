#+TITLE: Tendrl Engine — Compose View Designer Brief
#+SUBTITLE: Companion to design-brief.org, focused on the compose surface
#+DATE: 2026-04-27
#+STATUS: HANDOFF — for the compose-view design pass

* Why this doc exists

The whole-app mockups are landing, but compose has been left blank
deliberately — it's the highest-leverage surface and we have a specific
direction in mind. This doc gives you the model and constraints so you can
design compose from scratch with a real target instead of guessing.

Read =docs/design-brief.org= first if you haven't. This doc assumes you
already understand /section/, /publication/, and /zettel/.

* What you're being asked to design

A composition surface where a writer can:

1. Start a new publication, or open a draft
2. Write new sections, import existing ones from search, or fork imported
   ones to edit them
3. Reorder, retitle, retag
4. Add inline references to other Nostr events (citations, transclusions)
5. Preview the rendered result
6. Publish — as a local draft, signed locally, or signed + broadcast to
   relays

The hard part isn't any one of those — it's that the surface needs to
support four very different /modes of working/ on the same underlying
document, and switching between them must be lossless.

* The four modes

These exist today (or are planned). The mode is a /viewport/ on the same
document — switching modes does not change the data.

** 1. Button mode (the structured form)

A stack of section cards. Each card has explicit fields: title, tags, content
textarea. Add/remove/reorder buttons between cards.

This is the lowest-friction mode for a first-time user — every input has a
label. It's the highest-friction mode for a writer in flow, because moving
between fields requires clicks.

When designing: think "Notion's database row editor" or "Airtable expanded
view," not "Google Docs."

** 2. Plain mode (the full-document editor)

A single textarea. The user writes the whole publication as one stream of
plain text. Headings, tags, and section boundaries are detected by the
parser from delimiters (`#`, `*`, `=`, configurable) and inline tag lines
(`:tag-name: value`).

A side panel shows what the parser /detected/ — section titles, tags,
hierarchy — so the user has live feedback that their delimiters are working.

This is the writer's mode. Treat it like a code editor: monospace-friendly,
generous line height, minimal chrome.

** 3. WYSIWYG overlay (planned, not built)

This is the major design opportunity. The model:

- Plain text remains the source of truth — *the buffer is the document*
- The WYSIWYG layer is a /rendered viewport/ that sits on top of that buffer
- Keystrokes still hit the underlying text; the overlay updates to match
- The overlay provides a button bar for inserting markup constructs
- The user can drop down to plain mode at any time and trust what they see

This is the inverse of Google Docs and Notion (where the rich tree is
canonical and plain text is an export). It means undo, search, copy/paste,
and merge all work on text — never on a tree the user can't see.

The button bar inserts /semantic operations/, not literal characters:

- "Make this a heading" → emits `#`, `=`, or `*` depending on the section's
  content mode
- "Insert citation" → opens a picker, then writes `{{ref:...}}` (see
  Nostrdown below)
- "Insert sidebar / callout / admonition" → emits an AsciiDoc block macro

The toolbar is the /API/ to the underlying markup. The user shouldn't need
to remember syntax for any markup they've chosen.

** 4. Preview

Read-only render of the assembled publication. Useful as a final pass before
publishing.

** Mode switching

A mode bar at the top of the compose surface — currently four buttons.
Switching between any two modes is lossless. The user must always be able
to drop into plain mode and verify the source.

* The base case: AsciiDoc

We're starting with AsciiDoc as the canonical markup, with org-mode and
markdown as second-class targets. /Why AsciiDoc and not Markdown:/

- It has block-level constructs that long-form writers actually need:
  sidebars, admonitions, callouts, includes, attribute lists
- Tables and footnotes are first-class
- It has a real spec, unlike Markdown's many dialects

What this means for the WYSIWYG button bar: it's closer to *Confluence*
than to a Markdown editor. Plan for:

- Inline: bold, italic, code, link, citation, math
- Block: heading levels, ordered/unordered list, code block, sidebar,
  admonition (note/tip/warning/caution/important), callout, table, image,
  blockquote, horizontal rule
- Document: title, attribute (author, date, version), include, conditional

You will not have time to design every block in detail. Pick the 6–8 most
common (heading, bold/italic/code, list, link, citation, sidebar,
admonition) and treat the rest as "more" / overflow.

* The block model: what each section can be

Inside a publication, each section is one of three kinds. The visual
language has to make these distinguishable at a glance.

** Editable
A section the user wrote. Their content, their pubkey, their d-tag. Fully
mutable.

** Imported (read-only reference)
A section that exists somewhere on the network. The publication index
contains an `a` tag pointing to the original event — *no copy is made*.
The compose surface shows the imported content inline as a read-only block
so the user can see what's being referenced. Currently shown with a
lock icon and dimmed background.

** Forked
The user took an imported section and explicitly forked it. A new event is
created with a new d-tag, the user's pubkey, and `a`/`e` tags marking the
lineage (NIP-54 fork convention). Now editable, with attribution preserved.

** Design implications
- Imported sections are visually /quoted/, not authored — they look like
  someone else's words
- The "fork" affordance is a deliberate, visible action, not an accident
  (e.g. an explicit checkbox on the imported block)
- A forked section should look like an editable section /with a lineage
  badge/, not a hybrid of imported and editable

* Nostrdown: the citation picker

Nostrdown is our markup-agnostic syntax for referencing other Nostr events
inline. The literal syntax is `{{ }}` — chosen because it doesn't conflict
with Markdown, AsciiDoc, or Org. Examples:

#+begin_example
{{ref:chapter-3}}                       # internal section reference
{{ref:chapter-3|Display Text}}          # with custom display
{{wiki:target-topic}}                   # wiki entry
{{embed:naddr1...}}                     # full event embed from network
#+end_example

The full spec is in =docs/nostrdown.org=. For design purposes, what matters
is the /insertion UX/.

When the user clicks the "insert citation" button (or types a trigger like
`{{`), they shouldn't have to remember syntax. The natural flow:

1. Cursor is in the buffer
2. User triggers citation
3. A small picker opens near the cursor — search input + result list
4. User searches the right-panel knowledge base (sections, publications,
   profiles, wiki entries, highlights)
5. Picks one
6. The picker writes the correctly-formed `{{...}}` into the buffer at the
   cursor and closes

The reference points: *Notion's `@`-mention*, *Roam's `[[`-link*, *Obsidian's
quick switcher with insert*. The right-panel search already exists — this
is just piping it through a small inline overlay.

Important: the picker should support all of nostrdown's reference types
(=ref:=, =wiki:=, =embed:=) — probably as tabs or a type selector inside
the picker.

* Constraints

These come from the data model, not preference.

- *Plain text is canonical at every level.* Whatever the WYSIWYG shows, the
  buffer is the truth. Don't design a representation that can't round-trip
  to text.
- *Markup-agnostic.* Today AsciiDoc; tomorrow org and markdown. Buttons
  emit semantic operations, not literal characters.
- *Async loading is constant.* Imported sections load lazily — design the
  loading and failed states for them, not just the loaded state.
- *No live collaboration.* Single-user compose. You don't need to design
  multi-cursor, presence, or conflict resolution.
- *No autosave to network.* Drafts save locally; publishing is an explicit
  action with consequences (signs an event with the user's key).
- *Mode switching is free.* Any mode → any mode, no warning, no data loss.

* What exists today (the floor, not the ceiling)

Files in =web/src/lib/components/=:

- =ComposeView.svelte= — the mode bar, toolbar, sections list, plain-mode
  editor, preview
- =ComposeSection.svelte= — a single section card in button mode
- =EditView.svelte= — the plain-mode textarea with parser highlighting
- =Toolbar.svelte=, =WorkbenchToolbar.svelte= — the action button rows

The current visual language is ad-hoc. Treat all of it as legacy. The only
things worth preserving structurally:

- The mode bar pattern (button / plain / preview switcher)
- The detected-sections side panel in plain mode
- The two-step trash with countdown (delete is destructive — friction is good)
- The modified indicator (yellow highlight) on sections diverged from their
  source

* Open design decisions (your call)

1. *Where does the WYSIWYG button bar live?* Floating near the cursor like
   Medium? Pinned at the top like Confluence? Inline above each section?
2. *How does the user switch a section's content mode?* (asciidoc / org /
   markdown) — global toggle or per-section?
3. *What does "imported but failed to load" look like?* The block exists
   in the publication index but the network can't fetch it. Placeholder
   with retry?
4. *Should sections have visible boundaries in plain mode?* Or only in
   button/WYSIWYG?
5. *Block-level operations (drag to reorder, duplicate, split)* — gestures
   or buttons or both?
6. *Where does publish live?* Single button on the toolbar, or a publish
   panel with target relays / draft-vs-broadcast options?

* What to deliver

In rough priority order:

1. *Button mode* — refined card layout, field affordances, add/remove,
   reorder
2. *Plain mode* — editor chrome, detected-structure panel, content density
3. *WYSIWYG overlay* — button bar contents, button states, citation picker
   overlay, how it renders block macros (sidebar, admonition)
4. *Mode bar* — how the four modes are presented and switched
5. *Block kinds* — visual distinction between editable / imported / forked,
   plus the fork-this-section affordance
6. *Publish flow* — local draft / signed local / broadcast, target-relay
   selection, the moment of "this is going live"
7. *Empty state* — what compose looks like before any sections exist

A Loom or 5-minute walkthrough of how a writer would use it end-to-end is
worth more than a polished single screen. We'd rather see a complete flow
in low-fidelity than one perfect button.

* Glossary (compose-specific)

- *Block / Section* — interchangeable here; one kind 30041 event
- *Index / Publication* — the kind 30040 that orders the sections
- *Content mode* — Markdown / AsciiDoc / Org / PlainText, set per section
- *Fork* — turn an imported reference into an editable copy with lineage
- *Nostrdown* — the `{{ }}` citation syntax (see =docs/nostrdown.org=)
- *NIP-54* — the convention for marking forked content with `a`/`e` tags
