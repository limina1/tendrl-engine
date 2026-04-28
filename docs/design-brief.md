#+TITLE: Tendrl Engine — Web UI Design Brief
#+SUBTITLE: Orientation packet for the WM-flavored web UI redesign
#+DATE: 2026-04-28
#+STATUS: WORKING — orientation. Active spec lives in =wm-shell.org=

* Read this first

This brief is the *orientation packet* — the high-level context for the
web UI redesign: what tendrl is, why a WM model, the engine/interface
separation, the data model, and the broader open product questions.

The *active spec* — what we're actually building right now — lives in
=docs/wm-shell.org=. That's where decisions are recorded as we make
them. Read this brief once for context; refer to the shell spec for
day-to-day work.

* What this app is, in one paragraph

Tendrl is a *local-first knowledge workbench* built on the Nostr protocol. It
lets a writer (a) collect notes and long-form documents from a decentralized
network, (b) discuss them with an LLM, and (c) assemble new publications by
composing existing notes plus original writing. Think of it as
"Obsidian + Roam + Substack" if your notes were public, addressable Nostr
events instead of files in a vault. It runs locally as a Rust engine + a
SvelteKit web UI; there is no server-side account.

* Engine and interface are separate layers

Read this before the visual model — it's the constraint that shapes
everything else.

*Tendrl is an engine, not an app.* The Rust core (=src/=) is a Nostr
management layer: retrieval, publication assembly, relay sync, identity,
embedding/search. It exposes a stable HTTP API. Interfaces are *pluggable
consumers* of that API: today the web UI (=web/=) and the TUI (=src/bin/
nostr-tree.rs=); long-term, an Emacs integration where Emacs is the editor
and Tendrl is the Nostr backend.

*Why this matters for the web UI.* The web UI is being built deliberately
Emacs-flavored because the interaction patterns worked out here are
intended to transfer to a future Tendrl+Emacs setup. WM choices are not
just aesthetic — they're a portability constraint. A pattern that doesn't
make sense in Emacs probably doesn't belong here either.

*What's out of scope.* Mobile is not a target. Tablet-with-keyboard is
acceptable but not a priority. The web UI is laptop/desktop-first.

* The visual model: a tiling window manager in the browser

The web UI is committed to an Emacs/i3/zellij-flavored *window manager*
metaphor — not a fixed-panel app. The earlier three-column layout
(chat / document / search) was a degenerate WM: three windows in a fixed
h-split with no buffer-swap and no layouts. The shift isn't about adding
features — it's about exposing the data model that was always there.

References that match the target feel: Emacs (windows, buffers,
=C-x= prefixes), i3 (workspaces, splits), zellij (visible mode-line),
Doom Emacs (=SPC= leader), pingdotgg/t3code.

For the full spec — class-typed slots (=chat= / =work= / =research=),
layouts, frames sharing a buffer list, modal editing, buffer switcher,
M-x, =SPC= leader, escape-key choice, routing — see =docs/wm-shell.org=.
That doc is kept current as decisions are made; this brief just gives
you the high-level shape.

* The mental model: data terms

These three terms come from the data model, not the UI. They appear
everywhere. Internalize them and the rest follows.

** Section (kind 30041)
An atomic note. A paragraph, a chapter, a code listing, a quote. Has a stable
address (=kind:pubkey:d_tag=) so it can be linked, versioned, and reused.
*Sections are the unit of composition.*

** Publication (kind 30040)
An *index* — an ordered list of references to sections. A publication owns no
content of its own; it curates. The same section can appear in multiple
publications. Editing a publication is editing its table of contents.

** Zettel
The mental frame: every section is a small, addressable, independently-useful
piece of writing. The UI should make it natural to /find one/, /drag it into
a new publication/, and /cite it elsewhere/.

If those three concepts don't feel solid, the rest of the design will fight
the data model. Spend ten minutes on this.

* Read-this-first list

These docs already exist in =docs/=. Read in this order — they each take 5–15
minutes:

1. =project-summary.org= — the 30,000-ft view: what we're building and why
2. =workbench-architecture.org= — earlier three-panel vision. *Outdated in
   places* — the WM model in this brief supersedes the fixed three-panel
   chrome. Read it for the data flow and the compose surface, not the layout.
3. =features.org= — the menu of capabilities (feed, search, compose, ignore,
   profiles, embeddings)
4. =publication-architecture.org= — the data model (publications, sections,
   versioning, addressing). Skim unless you want depth.
5. =data-lifecycle-roadmap.org= — fetch policies, online/offline, sync
6. =nostrdown.org= — our markup-agnostic citation/transclusion syntax
   (relevant for the editor buffer)
7. =design-brief-compose.org= — mode hierarchy /inside/ the composer buffer
   (button / plain / WYSIWYG / preview). Compose-specific; orthogonal to WM.

* Where to find each surface in code

The route table below is current state. Most of these become *buffer types*
in the WM model — the URL maps to "open this buffer in the center window,"
not to a full-page route.

| Buffer (target)        | Current route                  | Current component                                |
|------------------------+--------------------------------+--------------------------------------------------|
| Feed                   | =/=                            | =web/src/routes/+page.svelte=                    |
| Reader                 | =/p/[pubkey]/[d_tag]=          | =web/src/routes/p/[pubkey]/[d_tag]/+page.svelte= |
| Section reader        | =/p/[pubkey]/[d_tag]/[index]=  | =.../[index]/+page.svelte=                       |
| Composer               | =/compose=                     | =web/src/routes/compose/+page.svelte=            |
| Profile                | =/profile/[pubkey]=            | =web/src/routes/profile/[pubkey]/+page.svelte=   |
| Ignored                | =/ignored=                     | =web/src/routes/ignored/+page.svelte=            |
| Chat                   | (left panel today)             | =web/src/lib/components/ChatPanel.svelte=        |
| Search                 | (right panel today)            | =web/src/lib/components/SearchPanel.svelte=      |
| Shell                  | (all routes)                   | =web/src/routes/+layout.svelte= (to be replaced) |
| Visual reference       | =/design=                      | =web/src/routes/design/+page.svelte=             |

The web UI is *Svelte 5* (using runes: =$state=, =$effect=, =$derived=).
Client state lives in a single =AppState= class with reactive properties.
Once the WM shell lands, =AppState= grows a window tree, buffer registry,
and layout list alongside the existing reactive properties — the existing
chat/search/feed/etc. components are wired in as buffers without changing
their internals.

Iceberg design tokens and chrome primitives have already been ported into
=web/src/lib/styles/tokens.css= and =web/src/lib/design/=. The =/design=
route is a static showcase. =Rail.svelte= and =PanelHeader.svelte= are
partial fits for the WM shell — =Rail= becomes the closed-side strip,
=PanelHeader= becomes the per-window header, =NetworkPill= goes in the
mode-line.

* The five flows to design for

Every design decision should be evaluated against these flows. Listed in
order of how often a real user does them. Each is described in WM terms
(buffer + layout) where helpful.

** 1. Browse and read (highest traffic)
User opens app → feed buffer in center → picks a publication → reader buffer
takes the center → maybe opens a section, maybe jumps to author profile.
Likely uses the =read= layout (chat as rail, search as rail, single wide
center), with the option to open a window for refs/outline.

  - Pain points: feed is undifferentiated cards; no sense of recency,
    importance, or progress; reading view doesn't feel like reading.

** 2. Search and discover
Search buffer is open in a side window → user enters a query → results
appear → user opens or saves them. Semantic vs. keyword distinction should
be visible in the buffer chrome.

  - Pain points: search competes with reading; results are a flat list; no
    faceting; semantic vs. keyword distinction is invisible.

** 3. Compose
User starts a new publication → composer buffer in center → drags in
existing sections, writes new ones, asks the LLM to help → previews →
publishes. Likely uses the =write= layout (chat narrow on left, refs/search
on right, composer in center, possibly with an outline buffer above the
composer via h-split).

  - This is the *highest-leverage surface* and the one we have the most
    specific direction on. Read =docs/design-brief-compose.org= for the
    compose-internal mode hierarchy (button / plain / WYSIWYG overlay /
    preview), the plain-text-as-canonical principle, AsciiDoc as base case,
    the block model (editable / imported / forked), and the Nostrdown
    citation picker. Those modes are orthogonal to the WM shell — they live
    /inside/ the composer buffer.

** 4. Manage identity, relays, and ignore list
User logs in with an encrypted key (=ncryptsec=) → manages which relays they
read from and publish to → ignores noisy authors. In WM terms: a settings
buffer, openable via M-x or the leader.

  - Currently surfaced as toolbar buttons + modals. Belongs in a real
    settings buffer with sections, not modals.

** 5. Network mode (offline / online)
User toggles the engine offline (e.g. on a plane) → all work continues
against the local cache.

  - Lives in the *mode-line*, not the toolbar. Always visible, never
    interrupting.

* Constraints the design must respect

These come from the data model and engine, not from preference.

- *Loading is a first-class state.* Almost everything is fetched async with a
  =Pending → Loading → Loaded | Failed= state machine. Every buffer must
  show all four states gracefully — not just spinners. See =LoadStatus<T>=
  in =src/publication.rs=.
- *Content has multiple markup modes.* Sections can be Markdown, Org,
  AsciiDoc, or PlainText. Rendering needs to be consistent across them. See
  =content.rs= in =src/tree/=.
- *Addresses, not IDs.* Things are identified by =kind:pubkey:d_tag=, not by
  a database ID. URLs reflect this. Never invent IDs.
- *Local-first.* Everything works offline against the local cache. Network
  fetches are /augmentation/, not the primary path.
- *No accounts, no server.* Identity is a Nostr keypair. There is no
  forgot-password flow, no email, no admin. Designs that imply server-side
  accounts won't work.
- *WM shell as the default chrome.* Buffers, windows, layouts, splits,
  rails, mode-line, M-x. Routes map to "open this buffer in the center
  window." See the visual model section above.
- *The center is always present.* Side slots scale around it; it never
  disappears. Single-buffer (zen) mode is the floor, not an error state.

* Broader product questions

These are flagged but not part of the WM shell work — they're broader
product concerns that will need design passes of their own.

1. *Compose flow.* What does a first-class zettelkasten composer look
   like inside a composer buffer? See =design-brief-compose.org=. Modes
   (button/plain/wysiwyg/preview) live /inside/ the composer buffer; the
   WM shell hosts the buffer.
2. *Search results display.* Search returns publications, sections,
   profiles, and document pages mixed together. How does the search
   buffer present heterogeneous results without overwhelming?
3. *Loading states.* =Pending → Loading → Loaded | Failed= is everywhere
   and currently feels jittery. Skeletons? Optimistic placeholders?
   Progress affordances for slow relays?
4. *Identity UX.* Encrypted keys (ncryptsec passwords) are technically
   scary. How does first-run + daily login feel safe and ordinary?
5. *Visual language polish.* Iceberg tokens are landed
   (=web/src/lib/styles/tokens.css=) and the =/design/shell= artboard uses
   them, but actual reader/composer surfaces don't yet have refined
   typography hierarchy or density tuning.
6. *Layout persistence backend.* localStorage per-device, or
   per-identity in a Nostr replaceable event so layouts follow the user
   across machines? The latter is more on-brand but adds a load-state to
   the shell itself.

For *WM shell deferred items* (real keybindings, drag-split, buffer-list
buffer, deep-link routing, migration plan, =Rail.svelte= naming clash),
see =wm-shell.org=.

* Glossary

WM terms (from the visual model):

- *Buffer* — a piece of content/state shown in a window.
- *Window* — a slot hosting one buffer.
- *Layout* — a named arrangement of windows + buffers.
- *Split* — a window divided h or v into more windows.
- *Rail* — the collapsed state of a side window: vertical strip of buffer
  tabs.
- *Mode-line* — bottom strip showing prefix state, active buffer, layout,
  network mode.
- *M-x* — command palette, always-on header strip.
- *Leader* — =SPC= prefix in normal mode (Doom Emacs idiom).

Nostr / data terms:

- *Nostr* — decentralized protocol where signed events are gossiped between
  relays. No central server.
- *Relay* — a server that stores and forwards events. Users connect to many.
- *NKBIP-01* — our convention for structured publications using kinds 30040
  and 30041. See =nips/nkbip01.md= or =/nips/nips/= for related NIPs.
- *npub / nsec / ncryptsec* — public key (npub), private key (nsec), and
  encrypted private key (ncryptsec) in bech32 encoding.
- *Kind* — a number identifying what an event is (1 = note, 30023 = article,
  30040 = publication index, 30041 = section, etc.).
- *=a= tag / =e= tag* — references between events. =a= references
  replaceable events by address; =e= references by event id.
- *Addressable / replaceable event* — events at kinds 30000–39999 that the
  author can update; identified by =kind:pubkey:d_tag=.
