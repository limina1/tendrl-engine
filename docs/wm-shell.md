#+TITLE: WM Shell — Active Spec
#+SUBTITLE: Scoped working spec for the tiling-WM web UI redesign
#+DATE: 2026-04-28
#+STATUS: WORKING — Phase 1.5 (layout simplification + split-create)

* Scope

This is the *active spec* for the web UI redesign as an Emacs/i3-flavored
tiling window manager. It captures resolved decisions, what we're
building right now, and what's deferred.

For high-level context (what tendrl is, why a redesign, the engine/
interface separation, the data model), read =docs/design-brief.org=.
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

** Phase 1.5 — Layout simplification + split-create [ ] (current)

- [X] Trim named layouts to one base (=chat= / =work= / =research=, all collapsible) + =chat= preset; defer the named-layout list to a user-savable-perspectives feature
- [X] Wire =SPC w s= same-class split-create (horizontal split with class-scoped buffer picker; new leaf takes focus)
- [X] Intra-slot leaf navigation (=j= / =k= cycles between leaves when slot has multiple; focuses one buffer at a time)
- [X] Class taxonomy revision: =feed= moves from =research= to =work= so feed/reader/composer cycle the same center slot (the "main content surface"). Research class is now auxiliary tools only (search/refs/kb).

** Phase 2 — Promote to root [ ]

- [ ] Replace =+layout.svelte= three-column chrome with the WM shell
- [ ] Existing routes (=/=, =/p/=, =/compose=, =/profile/=, =/ignored=) deleted or made redirects
- [ ] AppState shrink — remove dead per-route fields once renderers fully own state

** Polish (parallel) [ ]

- [ ] Free-text find prompt for =SPC f e/d/p=
- [ ] Statusbar pill chrome (segmented mode-line with =pill--online= / =dot--fetching=)
- [ ] Rail keybinding glyphs (=rail-key= chips)
- [ ] Scroll position retention across layout switches
- [ ] Cross-tab =BroadcastChannel= sync for buffer list

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
- =/design/shell= — interactive WM shell with class-typed slots, five
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
│   ├── r / w / t / c / z — switch to read/write/triage/chat/zen
│   └── s — save current as named layout
├── t — toggle
│   └── n — network mode (online/offline)
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

These were flagged in =design-brief.org= but are not part of the WM
shell work — they're broader product concerns:

- First-run / login UX redesign (encrypted keys, ncryptsec).
- Loading-state pattern across all buffers (skeletons, optimistic
  placeholders).
- Heterogeneous search results display (publications, sections,
  profiles, document pages mixed).
- Visual language polish beyond iceberg tokens (typography hierarchy
  in reader/composer, density tuning).
- Compose flow internal modes (button/plain/wysiwyg/preview) — see
  =design-brief-compose.org=. Orthogonal to the WM shell; the composer
  buffer hosts these modes inside it.
- Mobile / narrow viewport.
