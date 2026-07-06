#+TITLE: WM Shell — Active Spec
#+SUBTITLE: Scoped working spec for the tiling-WM web UI redesign
#+DATE: 2026-05-06
#+STATUS: WORKING — Phase 2 polish + Phase 3 prep

* Scope

This is the *active spec* for the web UI redesign as an Emacs/i3-flavored
tiling window manager. It captures resolved decisions, what we're
building right now, and what's deferred.

For high-level context (what tendrl is, why a redesign, the engine/
interface separation, the data model), read =docs/archive/design-brief.org=.
This doc is the working spec; the brief is the orientation packet.

* Roadmap

Tick boxes as work lands; don't delete the line. The roadmap is the
running ledger of what was promised vs what shipped.

** Phase 0 — Artboard scaffolding [X]

- [X] Class-typed slots, layouts, modal editing, leader popup, M-x
- [X] Mock buffers prove out the WM mechanics

** Phase 1 — Wire to real engine data [X] (commit ~f5d6cc3~)

- [X] BufferStore, registry, BufferRenderer
- [X] Per-kind renderers: Chat, Feed, Reader (multi-instance), Search, Profile, Composer (singleton), Ignored, Knowledgebase, Refs
- [X] AppState init lifted for =/design/*=; JSON modal scope fixed
- [X] Engine commands: =SPC t n=, M-x show-event-json, login/logout
- [X] =home= layout default (chat rail + feed center + work rail)

** Phase 1.5 — Layout simplification + split-create [X]

- [X] Trim named layouts to one base (=chat= / =work= / =research=, all collapsible) + =chat= preset; defer the named-layout list to a user-savable-perspectives feature
- [X] Wire =SPC w s= same-class split-create (horizontal split with class-scoped buffer picker; new leaf takes focus)
- [X] Intra-slot leaf navigation (=j= / =k= cycles between leaves when slot has multiple; focuses one buffer at a time)
- [X] Class taxonomy revision: =feed= moves from =research= to =work= so feed/reader/composer cycle the same center slot (the "main content surface"). Research class is now auxiliary tools only (search/refs/kb).
- [X] =chat= preset dropped — base covers it via slot toggle (=SPC w c=). Header layout pills replaced with a single =settings= button.

** Phase 2 — Promote to root [ ]

- [X] Replace =+layout.svelte= three-column chrome with the WM shell — layout is now minimal (init AppState, mount JSON modal, render children)
- [X] Existing routes (=/p/=, =/compose=, =/profile/=, =/ignored=) deleted; root =/+page.svelte= mounts the workbench at full viewport; =/design/shell= removed (was a duplicate of root)
- [ ] Extract Shell into a reusable component (currently inlined in =/+page.svelte=; the deleted =/design/shell= can return as the artboard wrapper around it)
- [ ] AppState shrink — remove dead per-route fields (=publication=, =sections=, =viewMode=, =searchResults=, etc.) once renderers fully own state
- [ ] Delete unused legacy chrome components (=WorkbenchToolbar=, =PanelFrame=)
- [ ] Deep-link routing: =/p/<pubkey>/<dtag>= etc. spawn the right buffer on load instead of 404

** Modal nav + ranger nav (shipped this session) [X]

Tracked here as a single line per work cluster — see commits
~872d1cb..3852a74~ for the per-feature granularity, and
=docs/zettel/index.org= for the user-facing summary.

- [X] Per-buffer NavHandler registry on BufferStore; non-reactive Map +
      reactive snapshot for diagnostics; singleton on =globalThis= so
      HMR module duplication can't fork it
- [X] Ranger-style cursor over feed, reader (outline / paginated /
      continuous), search, compose-full — bounds-checked scroll, same
      inset-bar + tinted-bg highlight everywhere
- [X] =gg= / =G= motion (700ms pending-=g=) wired into every ranger
      buffer; in continuous reader, snaps =scrollTop= to 0 / =scrollHeight=
- [X] =h= / =l= drill axis: reader cycles outline → paginated →
      continuous; compose toggles full ↔ plain
- [X] Editable-focus implicit insert (=focusin= / =focusout= flips
      modal nav); =Esc= / =C-[= / =C-g= safety net to escape from
      inside any input
- [X] =i= / =o= dispatch through =NavAction='insert'= so buffers can
      target a specific input vs. the generic first-=[data-entry]=
      fallback
- [X] Search "commit-and-exit" loop on =Enter= (vim =/=-search
      semantics); ranger over results; result-row data-cursor wrapper
- [X] Reader eager-loads all sections after the TOC arrives — fixes
      empty-outline regression on entering reader from feed
- [X] Plain compose runs CodeMirror 6 + =@replit/codemirror-vim= via
      a Svelte =use:= action; doc-compare sync (no cursor reset on
      bindable round-trip); double-Esc stack to leave the editor

** Polish (parallel) [ ]

- [ ] Free-text find prompt for =SPC f e/d/p=
- [X] Statusbar pill chrome — modeline now renders =network=,
      =embedding=, and =identity= as segmented pills with state dots
      (=dot--online= / =dot--offline= / =dot--fetching=). Active
      relay fetches drive the pulsing fetching dot. Buffer / leader /
      minibuffer info stays as text segments — they're transient and
      don't fit the stable-status metaphor.
- [ ] Rail keybinding glyphs (=rail-key= chips)
- [ ] Scroll position retention across layout switches
- [ ] Cross-tab =BroadcastChannel= sync for buffer list
- [X] Stop AppState navigation handlers from goto-ing routes when a shell is mounted (=app.setNavigationHandlers=); search-result clicks and add-to-compose now spawn buffers instead of breaking out to =/p/...= or =/compose=
- [X] ReaderBuffer "Edit" / "Edit §" — import loaded sections (or just the focused section in paginated mode) into the compose pool and jump to composer
- [X] DraftReader: composer's =preview= tab body replaced with the same outline/continuous/paginated rendering as ReaderBuffer, fed by an adapter that turns =ComposeState.sections= into =LazySection[]=. Per-entry lock toggle in outline (yellow accent when unlocked).
- [ ] DraftReader: drag-to-reorder when section is unlocked
- [X] DraftReader: transclusion affordance — search action modal's "Insert into compose" path covers this (a-tag preserved via =source_addr=, default-locked import)
- [X] DraftReader: per-entry remove (plumbed via =onremove= → =handleDeleteFromCompose=)
- [X] Pane header =N↻= cycle button — clicking opens the class-scoped switcher (lets user back out of composer to the reader without knowing =SPC b b=)

** Search → compose, settings, draft separation (shipped 2026-05-06) [X]

Tracked here as a single line per work cluster — see commit ~0b98195~
for the bundled implementation.

- [X] *Search action modal* on Enter/click — three actions (Read
      section/publication, Find containing publications, Insert into
      compose). Replaces the inline =◂= / =□= buttons (=◂= chat retained;
      =□= dropped). =j= / =k= / =Enter= keyboard nav, letter shortcuts
      (=r= / =f= / =i=).
- [X] *Find containing publications* uses an =a= tag query against
      nostrdb (=a:KIND:PUBKEY:DTAG=, parsed as =#a= filter). New
      =scopeToMe= opt-out on =handleSearch= so cross-author parents
      aren't filtered to =by:me=. Force-focuses the search slot when
      results land.
- [X] *Insert into compose* writes a default-locked section block
      (origin: import) with ===Title= heading so the plain-mode parser
      sees it; respects =editorInsertMode= setting (=cursor= dispatches
      CM6 insert at the active plain-mode caret; =append= goes to end
      of doc or pool).
- [X] *Standalone-section reader* — =reader:event:<id>= buffer-id
      format; ReaderBuffer.parseEventId synthesizes a single-section
      publication via =api.getEvent= and forces =viewMode = 'paginated'=.
- [X] *Reader / draft separation* — =isDraftMode= ripped out of
      ReaderBuffer; opening a published 30040 from feed always shows
      the pristine API view. =DraftReaderBuffer= registered as
      =draft-reader:current= in registry + BufferRenderer; compose's
      =Read= button opens it instead of navigating to source pub.
- [X] *Settings buffer* (=kind: 'settings'=, opens via =SPC s s=, =M-x
      tendrl-open-settings=, or the new header button). Editor: line
      numbers, vim mode, insert mode (cursor / append). Compose:
      default mode (full / plain), sync mode, button labels.
- [X] *CodeMirror Compartments* — =lineNumbers()= and =vim()= live in
      compartments so toggles reconfigure without remounting (cursor
      and undo stack survive).
- [X] *Compose mode toggle dropped* — default lives in settings; =h= /
      =l= in normal mode still flips. A =$effect= on the bindable
      =mode= prop runs the transition (serialize on entering plain,
      commit on leaving).
- [X] *Reorder controls* — =↑= / =↓= buttons in ComposeSection (full
      mode, wired to =reorderComposeSection=) and in plain mode's
      detected sidebar (parse → swap → re-serialize on plainText, no
      pool round-trip). New empty sections from =+ Section= default
      to =readonly: true= matching transclusion semantics.
- [X] *Buffer kill* — =BufferStore.killFocused= was a flash-only stub;
      now removes from openBuffers, pushes onto recentlyClosed (cap
      20), replaces the focused leaf with another same-class buffer
      or restores the class default singleton (=feed= for work, =chat=
      for chat, =search= for research).
- [X] Drop =@codemirror/lang-markdown= — no syntax highlighting on
      plain mode (the asciidoc-wysiwyg direction will own that later).

** Phase 3 — Reading / editing / knowledge workflow [ ]

Design vision: =docs/reading-editing-workflow.md=. This phase makes the
reader and composer first-class for nested publications and inline
references; pairs with the eventual AsciiDoc wysiwyg editor.

Engine:
- [ ] =GET /api/v1/publications/:addr/parents= — walk =a= tag references
      to resolve the containment chain
- [ ] =GET /api/v1/publications/:addr/children/meta= — child kind +
      title + section count without loading section bodies (for packed
      cards)

Web — nested publications:
- [ ] ReaderBuffer outline renders nested 30040 children as packed
      cards by default (title + author + child count + =▸=)
- [ ] Inline expand: =l= / =Enter= on a packed card drops in the child
      TOC below the row (cursor descends; =h= / =Esc= collapses)
- [ ] Drill-in: =L= / double-click replaces the buffer's root with the
      child publication; breadcrumb chrome at the top; =h= / =Backspace=
      ascends
- [ ] Left-rail navigation tree showing the path from root + siblings
      at each level when drilled into a child
- [ ] =SPC g p= / =SPC g c= jump to parent publication / parent
      collection
- [ ] DraftReader gets the same packed/unpacked rendering for nested
      authoring

Web — search-to-source:
- [ ] Search result row gains a containment breadcrumb
      (=collection › book › chapter=) resolved client-side from each
      result's =a= tag
- [ ] Default click on a section result opens it *in context* (parent
      30040 unpacked, cursor on the result section); =Alt=-click keeps
      the current standalone behavior

Compose / transclusion:
- [ ] =+= picker between sections (existing Polish item) — selecting a
      30040 inserts as a *packed child* (nested publication), selecting
      a 30041 inserts as an editable / imported section
- [ ] =Nest= button in the compose mode bar — prompts for an existing
      30040 to embed as a child entry of the current publication
- [ ] Nostrdown ={{ }}= rendering in reader (inline / quoted /
      collapsed modes) — pairs with =docs/nostrdown.org=

Editor (parallel work):
- [ ] AsciiDoc wysiwyg editor with toolbar buttons for inline syntax;
      replaces the current per-section textarea inside Full mode
- [ ] Per-section CM6 instance ("repotting" pattern) for power-user
      editing as a fallback / alternate mode
- [ ] CM6 syntax decoration for =:tag:= lines and the configured
      delimiter heading style (lost when we dropped =lang-markdown=)

** Deferred indefinitely (low priority / open) [ ]

- [ ] Timeout-based leader popup suppression (Doom muscle-memory mode)
- [ ] Sub-prefix hover/help discoverability
- [ ] User-customizable prefix tree
- [ ] Multi-instance composer — likely unnecessary: a draft is itself a list of kind:30041 sections, so the composer already organizes "multiple drafts" internally
- [ ] Layout persistence backend (localStorage vs per-identity Nostr replaceable event)
- [ ] Deep-link routing (=?event=<naddr>=)
- [ ] =Rail.svelte= naming clash resolution

* Resolved decisions

** Class-typed slots

| Class      | Buffer types                                              | Splits?             |
|------------+-----------------------------------------------------------+---------------------|
| =chat=     | chat                                                      | no — singleton      |
| =work=     | feed, reader, composer, profile, ignored                  | yes (=SPC w s=)     |
| =research= | search, knowledgebase, refs/citation                      | yes (=SPC w s=)     |

*Note (2026-04-28):* feed moved from =research= to =work= so the "main
content surface" cycles feed → reader → composer in one slot. Research
is now auxiliary tools (search / kb / refs) only.

- One slot per class per frame.
- Cross-class splits forbidden.
- Slot positions are layout-defined.
- Each slot: =open=, =rail=, or =hidden=.
- Class transition: selecting from a research buffer opens a reader in
  the work slot — adds to work's buffer list, doesn't duplicate the
  source.

** Layouts

*Updated 2026-04-28:* the original five-layout list (=read=, =write=,
=triage=, =chat=, =zen=) collapsed to **one base layout** (chat /
work / research, all collapsible) plus =chat= as a preset. Reasoning:
home/read/write are not different *arrangements*, they're different
*content* in the same arrangement — =SPC b b= already swaps reader↔
composer without needing a layout switch. Named-layout recall (=SPC l
<key>=) becomes a user-savable-perspectives feature, deferred.

Layout-scoped buffer state still applies *if/when* user-saved perspectives
ship: switching back to a named perspective restores its last buffers
(Emacs perspectives style, not VS Code view-mode style).

** Frames

- *One URL.* The WM shell owns state; URL is just the entry point.
- *Multiple browser tabs of that URL = multiple frames* sharing the
  same Tendrl engine.
- *New tab loads default state* (default layout, no specific buffer
  pre-opened beyond what the layout defines).
- *Buffer list shared* across frames via =BroadcastChannel=.
- *Layout/window config per-frame* — each tab can be in a different
  layout simultaneously.
- *Engine never tracks "what's open."* Open-list is client-side.

** Buffer model

- *Stable IDs* — Nostr address (=kind:pubkey:d_tag=) or event id for
  events; draft id for drafts; synthetic id from query state for
  search/refs; fixed id for singletons.
- *Multi-instance for most* (reader, composer, profile, ignored, feed,
  knowledgebase, search, refs). *Chat is singleton.*
- *Concurrent edit* of the same draft in two frames: last-write-wins.
  =BroadcastChannel= notifies other frames; they refresh from engine.
- *Kill semantics:* drafts prompt-to-save, others are ephemeral. Chat
  persists to a local file independent of buffer lifetime.

** Modal editing

Two modes; mode shown in mode-line at left.

*Normal mode:*

| Key             | Action                                                |
|-----------------+-------------------------------------------------------|
| =h= / =←=       | Focus previous slot (rails included; no auto-expand)  |
| =l= / =→=       | Focus next slot (rails included; no auto-expand)      |
| =j= / =↓=       | Cycle buffer down within slot's class                 |
| =k= / =↑=       | Cycle buffer up within slot's class                   |
| =Enter=         | Expand focused slot if it's a rail                    |
| =i=             | Standard insert — focus the buffer's entry field      |
| =o=             | Open-and-insert (buffer-kind-specific; see below)     |
| =:=             | Open M-x                                              |
| =SPC=           | Leader prefix                                         |
| =SPC b b=       | Switch buffer (class-scoped)                          |
| =SPC b B=       | Switch buffer (global, with flash)                    |
| =SPC b r=       | Recently closed                                       |
| =SPC b k=       | Kill buffer                                           |
| =SPC w c=       | Toggle focused slot collapse/expand                   |

*=i= vs =o=:*

- =i= is the canonical "enter insert mode" — focuses the focused
  buffer's entry field (chat input, search input, composer cursor).
  Pure vim semantics.
- =o= is "open then insert" — context-dependent on buffer kind:
  - *composer*: appends a new editable section block to the document
    and enters insert in it (analog of vim's "open new line below").
  - *chat* (singleton): same as =i= — chat can't split.
  - *other splittable* (reader, search, refs, feed, knowledgebase,
    profile, ignored): would create a same-class window split, then
    insert. Falls back to =i= until split-create lands (deferred).

*Insert mode:* typing flows into the focused buffer's entry field
(chat input, search input, composer). Only escape keys are intercepted.

*Escape keys* (any returns to normal / closes minibuffer): =Esc=,
=C-[=, =C-g=. =C-h= avoided (Emacs help prefix; Chrome history clash).

** M-x

- Always-on command palette via =:= in normal mode or M-x button in
  header.
- Same minibuffer UI as buffer switchers; different completion source.
- Production reads commands from registry (see "Engine vs client
  command split" below).

** Engine vs client command split

Tendrl is the engine; commands divide by where the work happens.

- *Engine commands* — content/operation work that goes through the HTTP
  API. Examples: =tendrl-find-event=, =tendrl-find-draft=,
  =tendrl-save-draft=, =tendrl-publish-draft=, =tendrl-fork-section=,
  =tendrl-show-relays=, =tendrl-toggle-network-mode=,
  =tendrl-show-event-json=, =tendrl-login=, =tendrl-logout=,
  =tendrl-refresh=.
- *Client commands* — UI operations that don't touch the engine.
  Examples: =tendrl-switch-buffer*=, =tendrl-recent-buffer=,
  =tendrl-kill-buffer=, =tendrl-toggle-rail=, =tendrl-split-window=,
  =tendrl-switch-layout=, =tendrl-save-layout=,
  =tendrl-cycle-editor-view=, =tendrl-quit=.

The web client mirrors engine command names from the registry but adds
its own UI commands. The Emacs port (eventually) will do the same:
Emacs Lisp adds =tendrl-= commands wrapping the HTTP API for engine
ones, plus its own UI ones (=tendrl-find-event= → opens an Emacs
buffer; =kill-buffer= is just Emacs's built-in).

* Current artboards

- =/design= — visual system showcase (typography, color, components).
- =/design/layouts= — A/B comparison page that resolved layout-scoping
  to Option A.
- =/design/shell= — *removed in Phase 2* (was a duplicate of root; the
  WM shell now mounts at root =/+page.svelte=). Kept here for the
  historical record of what the artboard proved out: class-typed slots,
  layouts, splits, buffer switcher (class/global/recent), M-x, modal
  navigation, =SPC w c= toggle, focused-rail visual.

* Active pass: SPC leader

The prefix-popup mechanism. Press =SPC= in normal mode → popup of
next-key bindings (which-key style); press a key → either invoke a
leaf command or descend into a sub-prefix popup.

Sub-prefix tree (initial set):

#+begin_example
SPC
├── b — buffer
│   ├── b — switch (class-scoped)
│   ├── B — switch (global)
│   ├── r — recently closed
│   ├── k — kill
│   └── n / p — next / previous in class
├── w — window
│   ├── c — collapse/expand focused slot
│   ├── h / j / k / l — focus by direction
│   └── s — split (defer)
├── f — find (engine commands)
│   ├── e — find-event
│   ├── d — find-draft
│   └── p — find-publication
├── l — layout
│   ├── (named layouts retired — one base layout; slots toggle/collapse)
│   └── s — save current as named layout
├── t — toggle
│   └── n — network mode (Auto/Confirm)
├── q — quit
│   └── q — quit frame
└── : — immediate M-x
#+end_example

*To build:*

1. Prefix state machine (=null → SPC → SPC b → SPC b b=).
2. Popup component above mode-line, columns of =key → description=
   pairs, color-coded by category.
3. Key handler: SPC in normal opens popup; subsequent keys descend or
   execute; =Esc=/=C-g=/=C-[= cancel.
4. Wire each leaf to its existing command.
5. Update legend in =/design/shell=.

*Defer:*

- Timeout-based "no popup if released quickly" Doom behavior.
- Sub-prefix discoverability hover/help.
- Customization (user-defined prefixes).

* Deferred (within this project)

See the [[*Roadmap][Roadmap]] section above for the canonical list with
checkboxes. Items are tracked there as Phase 1.5 / Phase 2 / Polish /
Deferred-indefinitely.

* Out of scope (broader product roadmap)

These were flagged in =docs/archive/design-brief.org= but are not part of the WM
shell work — they're broader product concerns:

- First-run / login UX redesign (encrypted keys, ncryptsec).
- Loading-state pattern across all buffers (skeletons, optimistic
  placeholders).
- Heterogeneous search results display (publications, sections,
  profiles, document pages mixed).
- Visual language polish beyond iceberg tokens (typography hierarchy
  in reader/composer, density tuning).
- Compose flow internal modes (button/plain/wysiwyg/preview) — see
  =docs/archive/design-brief-compose.org=. Orthogonal to the WM shell; the composer
  buffer hosts these modes inside it.
- Mobile / narrow viewport.
