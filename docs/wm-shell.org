#+TITLE: WM Shell — Active Spec
#+SUBTITLE: Scoped working spec for the tiling-WM web UI redesign
#+DATE: 2026-04-28
#+STATUS: WORKING — currently building SPC leader

* Scope

This is the *active spec* for the web UI redesign as an Emacs/i3-flavored
tiling window manager. It captures resolved decisions, what we're
building right now, and what's deferred.

For high-level context (what tendrl is, why a redesign, the engine/
interface separation, the data model), read =docs/design-brief.org=.
This doc is the working spec; the brief is the orientation packet.

* Resolved decisions

** Class-typed slots

| Class      | Buffer types                                              | Splits?             |
|------------+-----------------------------------------------------------+---------------------|
| =chat=     | chat                                                      | no — singleton      |
| =work=     | reader, composer, profile, ignored                        | yes (internal only) |
| =research= | feed, knowledgebase, search, refs/citation                | yes (internal only) |

- One slot per class per frame.
- Cross-class splits forbidden.
- Slot positions are layout-defined.
- Each slot: =open=, =rail=, or =hidden=.
- Class transition: selecting from a research buffer opens a reader in
  the work slot — adds to work's buffer list, doesn't duplicate the
  source.

** Layouts

Five named: =read=, =write=, =triage=, =chat=, =zen=. *Layout-scoped*
buffer state — switching from =write= to =read= and back restores the
write layout's last buffers (Emacs perspectives style, not VS Code
view-mode style).

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
| =o=             | Enter insert mode (expands rail first if needed)      |
| =:=             | Open M-x                                              |
| =SPC=           | Leader prefix                                         |
| =SPC b b=       | Switch buffer (class-scoped)                          |
| =SPC b B=       | Switch buffer (global, with flash)                    |
| =SPC b r=       | Recently closed                                       |
| =SPC b k=       | Kill buffer                                           |
| =SPC w c=       | Toggle focused slot collapse/expand                   |

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

These are part of the WM shell work but punted to a later pass:

- *Real keybindings.* Artboard uses click-driven button stand-ins.
  Production needs a real keybinding dispatcher with proper precedence.
- *Drag-split / dynamic split create.* Splits are layout-defined for
  now; user can't create a new same-class split interactively.
- *Buffer-list buffer.* Emacs has both transient =C-x b= switch and
  persistent =*Buffer List*=. We have the first; the second is deferred.
- *Layout persistence backend.* localStorage vs per-identity Nostr
  replaceable event. localStorage is fine to start.
- *Deep-link routing.* =?event=<naddr>= query params for sharing
  links that auto-open a buffer.
- *Migration plan* from current =+layout.svelte= to the WM shell
  incrementally — currently the shell only lives at =/design/shell=.
- *=Rail.svelte= naming clash.* Existing component is a VS-Code-style
  activity bar; the WM rail is the closed-state strip. Rename or pick a
  new name when extracting components from the artboard.

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
