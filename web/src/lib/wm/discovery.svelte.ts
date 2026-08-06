// Contextual walkthrough — small "coachmark" tips that point at real UI as the
// user first discovers it. Not a forced linear tour: each tip fires the first
// time its surface is encountered (a modal opens, a sign-in succeeds, a feature
// first appears), is descriptive up front, and is always dismissable (an X marks
// it seen so it never nags again). Deeper tips may carry a "try it" action.
//
// The first-run flow is the "feed sync" login walk: the Confirm-mode fetch modal
// explains the broad pull → points at Settings to sign in → (once Settings opens)
// shows the two ways in at the source controls → notes the name-less pubkey once
// signed in → sends you home → shows that "General feed" is now an optional,
// narrowable pull. It is *event-gated*, not a pre-queued stepper: the
// open-Settings / sign-in / reopen-modal beats depend on the user actually doing
// them, so each fires from its real point. Where the next surface is reliably present (modal→Settings,
// me-chip→home) a tip carries `next` and chains on dismiss; auto-skip (anchor
// never mounts) runs the same dismiss path, so the chain self-heals.
//
// This is pure frontend view/interaction state (per the engine/web boundary):
// the engine owns no part of it. "Seen" + "enabled" persist to localStorage so
// the experience doesn't repeat across reloads; the Settings reset clears them.

import { browser } from '$app/environment';
import { runSearchExample } from '$lib/search/search-tour';

const ENABLED_KEY = 'tendrl.walkthrough.enabled';
const SEEN_KEY = 'tendrl.walkthrough.seen';

export type TourAction = {
	label: string;
	/** Runs when the user clicks the "try it" affordance. Advances the tip. */
	run: () => void;
};

/** World-state the walkthrough reads to decide whether a beat is still
 *  relevant. Injected by the app shell via `setWalkthroughWorld` so this
 *  module stays decoupled from the app state store (no circular import). */
export type WalkthroughWorld = {
	/** A signer is connected — the user has an identity. */
	hasIdentity: boolean;
	/** The local pool already holds events (the feed is non-empty). */
	dbHasEvents: boolean;
};

let worldFn: (() => WalkthroughWorld) | null = null;

/** Register the world accessor. Call once from the app shell on mount. */
export function setWalkthroughWorld(fn: () => WalkthroughWorld) {
	worldFn = fn;
}

/** Current world-state, or a pessimistic default (nothing done yet) before the
 *  accessor is registered — so a beat is never wrongly suppressed at boot. */
function world(): WalkthroughWorld {
	return worldFn?.() ?? { hasIdentity: false, dbHasEvents: false };
}

export type TourTip = {
	key: string;
	/** `data-tour` value of the element this tip points at. */
	anchor: string;
	title: string;
	/** Description. May contain `{token}` placeholders filled at trigger time
	 *  from the tip's runtime vars (see `trigger` / `setTipVars`) — used for
	 *  data-driven tips like "this publication contains {sections}". */
	body: string;
	/** Where the card sits relative to its anchor. Default 'top'. */
	placement?: 'top' | 'bottom' | 'left' | 'right';
	/** Next tip to surface when this one is dismissed — used to chain a guided
	 *  segment where the next surface is reliably on screen (modal→Settings,
	 *  me-chip→home). Omit when the next step is event-gated (sign-in, modal
	 *  reopen) and fires from its own trigger instead. */
	next?: string;
	/** Optional "try it" affordance — present only on deeper tips. */
	action?: TourAction;
	/** Precondition: return `false` when the goal this beat teaches is already
	 *  accomplished (e.g. already signed in, db already populated). A beat whose
	 *  `relevantWhen` is false is *silently* marked seen and skipped — never
	 *  shown — and its `next` chains on, so the auto walk only ever teaches what
	 *  the user hasn't already done. Omit for beats that are always relevant
	 *  (and for on-demand `W` tours, which the user asked for explicitly). */
	relevantWhen?: (w: WalkthroughWorld) => boolean;
	/** Per-shell overrides, applied by the overlay when the mobile shell is
	 *  active. A beat anchored to desktop chrome (header, mode-line) would
	 *  otherwise auto-skip on mobile — silently marked seen, burning the walk
	 *  without teaching anything. Override the anchor to the mobile chrome
	 *  equivalent (see MobileShell's data-tour attributes) and, where the body
	 *  narrates desktop chrome ("click the tendrl logo"), the body too. */
	mobile?: { anchor?: string; body?: string; placement?: 'top' | 'bottom' | 'left' | 'right' };
};

// Registry. The first-run "feed sync" login walk (feed-sync → sign-in →
// sign-in-methods → signed-in → home → general-feed → feed-first-pub →
// feed-first-badges) plus standalone discovery tips. `ENTRY_TIP`
// is what the "Run walkthrough" / "W" affordances kick off; everything else
// fires from its own discovery point (see the per-surface triggers in +page /
// +layout).
//
// Component tours hang off the feed's "read or work with the event" fork: the
// reader tour (reader-open → reader-menu → reader-edit) is event-gated — it
// fires once when a publication first opens. The rest are on-demand only,
// surfaced by their component's own W chip via `runTour` (never auto-thrown):
//   • composer — six discrete tours in its W *dropdown* (registry.ts
//     `composer.tours`): output, views, plain (→ shape → nest → tags),
//     detected, sections (→ lock → select → trash), publish (→ diff). The
//     `mode`-tagged ones switch the editor view on select.
//   • mode-line (modeline-overview → focus → status).
//   • search — a hands-on drill whose steps carry "Try it" actions that run a
//     real example query through the live box.
//   • event menu (menu-overview → copy → actions → pool → found).
export const TIPS: Record<string, TourTip> = {
	// ── First-run login walk ─────────────────────────────────────────────
	'feed-sync': {
		key: 'feed-sync',
		anchor: 'feed-sync',
		title: 'Feed sync',
		body: "Nothing is fetched until you approve it — this panel shows exactly what tendrl will pull, and from which relays. You're logged out, so this is the broad public feed: recent publications from these relays. Open *Details* to see the precise query and how many. When you're ready, close this (`Esc`) — let's sign in.",
		placement: 'right',
		next: 'sign-in',
		// A populated pool means the user has fetched before — skip the
		// "here's your first sync" beat entirely.
		relevantWhen: (w) => !w.dbHasEvents
	},
	'sign-in': {
		key: 'sign-in',
		anchor: 'settings',
		title: 'Sign in',
		body: 'Open *Settings* here and sign in — a `NIP-07` browser extension, or paste an `ncryptsec` key with its password. Heads up: the moment you sign in you\'ll see your pubkey but no name yet.',
		placement: 'bottom',
		// Already signed in (e.g. a key auto-loaded from the keyring)? Don't
		// teach signing in. Skips this and the source-controls beat below.
		relevantWhen: (w) => !w.hasIdentity,
		mobile: {
			anchor: 'mobile-cmds',
			body: 'Open *cmds* and run `tendrl-open-settings` to sign in — a `NIP-07` browser extension, or paste an `ncryptsec` key with its password. Heads up: the moment you sign in you\'ll see your pubkey but no name yet.',
			placement: 'top'
		}
	},
	'sign-in-methods': {
		key: 'sign-in-methods',
		anchor: 'identity-source',
		title: 'Two ways in',
		body: '*Engine* — paste an `ncryptsec` below and unlock it with its password; held for this session only, never written to disk. *NIP-07* — a browser extension holds the key and the engine never sees it. To connect one: (1) make sure your extension is activated/unlocked, (2) pick `nip07` and press `Reconnect`, (3) your signer pops up asking to read your public key — `Authorize`, or `Authorize forever`.',
		placement: 'bottom',
		// Same gate: no point explaining the sign-in methods to someone already
		// signed in. (Reachable on demand via Settings' walkthrough regardless.)
		relevantWhen: (w) => !w.hasIdentity
	},
	'signed-in-noname': {
		key: 'signed-in-noname',
		anchor: 'me-chip',
		title: 'Signed in — no name yet',
		body: "There you are: a pubkey, but no display name or avatar. That's expected — your profile (a `kind 0` event) lives on a relay you haven't pulled from yet. Fetching the feed will bring it in.",
		placement: 'bottom',
		next: 'go-home',
		mobile: {
			anchor: 'mobile-menu',
			body: "You're signed in. The `☰` drawer's *identity* row shows a pubkey, but no display name or avatar yet. That's expected — your profile (a `kind 0` event) lives on a relay you haven't pulled from yet. Fetching the feed will bring it in.",
			placement: 'bottom'
		}
	},
	'go-home': {
		key: 'go-home',
		anchor: 'home',
		title: 'Back to the feed',
		body: 'Head home — click the *tendrl* logo (or the feed tab) to return to your feed, then sync it again.',
		placement: 'bottom',
		mobile: {
			anchor: 'mobile-menu',
			body: 'Head home — open the `☰` drawer and pick *feed* to return to your feed, then sync it again.',
			placement: 'bottom'
		}
	},
	'general-feed': {
		key: 'general-feed',
		anchor: 'general-feed',
		title: 'General feed — now optional',
		body: "Now that you're signed in, the query is scoped to you. *General feed* adds the broad public pull on top — toggle it off to fetch only the relays and authors you choose. Open *Details* to watch the query change (it now carries `by:‹you›`).",
		placement: 'left'
	},
	// ── After the first fetch: the feed has events ────────────────────────
	'feed-first-pub': {
		key: 'feed-first-pub',
		anchor: 'feed-first-pub',
		title: 'A publication is a book',
		body: 'A publication is like a book — a `kind-30040` index that orders a set of sections (`kind 30041`) into a whole. This first one, *{title}*, gathers {sections}.',
		placement: 'bottom',
		next: 'feed-first-badges'
	},
	'feed-first-badges': {
		key: 'feed-first-badges',
		anchor: 'feed-first-badges',
		title: 'Provenance & actions',
		body: 'These pills show provenance — where the event lives in the network. The top one says it’s on *{relays}* right now. Click the row body to *read* the publication, or the `menu` pill to work with the raw event in depth.',
		placement: 'bottom',
		next: 'walk-done'
	},
	// Closing card of the one auto walk — hands off to the opt-in affordances so
	// the user knows where every other tour lives. The last thing they see.
	'walk-done': {
		key: 'walk-done',
		anchor: 'modeline',
		title: "You're all set",
		body: "That's the guided tour. From here, every panel has its own walkthrough — tap `W` (and `?` for the full reference) on the mode-line, reader, composer, search, and event menus to go deeper whenever you want. They never interrupt; they wait for you to ask.",
		placement: 'top',
		mobile: {
			anchor: 'mobile-bar',
			body: "That's the guided tour. From here, panels keep their own walkthroughs — tap `W` (and `?` for the full reference) in the reader, composer, search, and event menus to go deeper whenever you want. They never interrupt; they wait for you to ask.",
			placement: 'top'
		}
	},

	// ── Reader tour (event-gated: fires when a publication first opens) ───
	// The "read the publication" branch from the feed — clicking a row body
	// spawns the reader. This chain fires once, the first time any reader
	// mounts (seen-gated), then never nags again.
	'reader-open': {
		key: 'reader-open',
		anchor: 'reader-toolbar',
		title: 'Reading a publication',
		body: 'You opened the reader. The same publication renders three ways: `Outline` (the table of contents — sections in order), `Paginated` (one section at a time), and `Continuous` (the whole thing as one scroll). `h`/`l` cycles between them.',
		placement: 'bottom',
		next: 'reader-menu'
	},
	'reader-menu': {
		key: 'reader-menu',
		anchor: 'reader-menu',
		title: 'The raw event',
		body: '`menu` opens the event tools for this publication — inspect the raw `30040` JSON, copy its `naddr`, or find everything that references it. The reader shows the document; this is the event underneath.',
		placement: 'bottom',
		next: 'reader-edit'
	},
	'reader-edit': {
		key: 'reader-edit',
		anchor: 'reader-edit',
		title: 'Edit in the composer',
		body: '`Edit` pulls the whole publication into the composer to revise it. Imported sections arrive *locked* (yellow once you claim one) — unlock just the ones you want to change, then sign a new snapshot. The composer has its own `W` tour.',
		placement: 'bottom'
	},

	// ── Search history (closing beat of the search tour, opt-in only) ─────
	// Formerly auto-fired the first time any search landed; now it's the tail
	// of the on-demand search tour (search-tour-relays → here), so it surfaces
	// only when the user runs that tour — not unbidden on a stray search.
	'search-history': {
		key: 'search-history',
		anchor: 'search-history',
		title: 'Search history',
		body: 'And every search you run is kept here. Click the `🔍` pill on the mode-line to reopen and replay any past query.',
		placement: 'top'
	},

	// ── Mode-line tour (on demand: the mode-line's own W chip) ────────────
	// Not part of the login walk and never auto-fired — `runTour` surfaces it
	// only when the user taps W on the mode-line, then it chains through itself.
	'modeline-overview': {
		key: 'modeline-overview',
		anchor: 'modeline',
		title: 'The mode-line',
		body: 'This bottom strip is the mode-line — a live status bar (Emacs-style). Click any empty part of it to open the `menu` (the `SPC` leader). It never interrupts you: tap `W` here for this quick tour, or `?` for the full reference. Left half tells you *where you are*; right half is *live status* with quick toggles.',
		placement: 'top',
		next: 'modeline-focus'
	},
	'modeline-focus': {
		key: 'modeline-focus',
		anchor: 'ml-mode',
		title: 'Where you are',
		body: 'The focused slot-class (`@work` / `@chat` / `@research`) and the focused buffer. Switch buffers with `SPC b b`.',
		placement: 'top',
		next: 'modeline-status'
	},
	'modeline-status': {
		key: 'modeline-status',
		anchor: 'ml-pills',
		title: 'Status & toggles',
		body: 'Live engine state, much of it clickable: relay config, fetch mode (click to flip `auto` / `confirm`), embedding health, and your identity. The `🔍` pill replays past searches.',
		placement: 'top'
	},

	// ── Composer tours (on demand: the composer's own W dropdown) ─────────
	// Six discrete tutorials, never auto-fired — the in-chrome W lists them all
	// and `runTour(key)` plays one. `mode`-tagged tours (registry.ts) switch the
	// editor to that view on select. The shared `?` modal is the flat reference.
	//
	// A · Output — the kind selector (Publication vs atomic blog/wiki/custom).
	'compose-output': {
		key: 'compose-output',
		anchor: 'compose-kind',
		title: 'Output — what you publish',
		body: 'The `kind` selector sets your output. *Publication* parses the editor into a `30040` index over `30041` sections (the default). *Blog* (`30023`), *Wiki* (`30818`), or a *Custom* kind instead publish the whole body as a single atomic event — no section graph, delimiter, or nesting.',
		placement: 'bottom'
	},
	// B · Views — the Full / Plain / Read switch (view-agnostic; no mode tag).
	'compose-views': {
		key: 'compose-views',
		anchor: 'compose-view',
		title: 'Views — Full, Plain, Read',
		body: '`Full` shows each section as an editable card. `Plain` is one text buffer whose headings parse live into a *Detected* outline. `Read` previews the rendered result in its own buffer.',
		placement: 'bottom'
	},
	// C · Plain chain — the markup mechanics: write → shape → delim/nest → tags.
	'compose-plain': {
		key: 'compose-plain',
		anchor: 'compose-plain',
		title: 'Plain — one buffer, live sections',
		body: 'Write freely on the left; your headings parse live into the *Detected* panel — each heading becomes a `30041` section. Nothing is split until you sign.',
		placement: 'top',
		next: 'compose-shape'
	},
	'compose-shape': {
		key: 'compose-shape',
		anchor: 'compose-shape',
		title: 'Publication or Notes — your title decides',
		body: 'The shape pill reads your top-level `= Title`. *With* a title → `Publication`: the sections bind under one `30040` index. *Without* one → `Notes` (tinted): each section publishes as a standalone `30041`, no index — scattered notes. It is read-only — add or remove the document title to flip between them.',
		placement: 'bottom',
		next: 'compose-nest'
	},
	'compose-nest': {
		key: 'compose-nest',
		anchor: 'compose-nest',
		title: 'Delimiter & nesting',
		body: "`delim` is your markup's heading character — `=` by default, `#` for Markdown. Level 1 (`= Title`) is the publication, level 2 (`== Heading`) a section. `nest` sets how deep that folds: `flat` keeps one index over a flat list of sections; each tier turns one more heading level into nested `30040` sub-indices — books → chapters → sections.",
		placement: 'bottom',
		next: 'compose-tags'
	},
	'compose-tags': {
		key: 'compose-tags',
		anchor: 'compose-plain',
		title: 'Tags inline',
		body: 'Under any heading, `:name: value` adds a `["name","value"]` tag and `:tags: a, b, c` expands to three `t` tags. This works in every mode — including the atomic body, where a leading `:tag:` block is parsed, stripped, and shown as *tags from body* chips.',
		placement: 'top'
	},
	// D · Detected — the live outline rail (Plain only).
	'compose-detected': {
		key: 'compose-detected',
		anchor: 'compose-detected',
		title: 'Detected — the live outline',
		body: "The right rail mirrors the parse in real time: the document title, a tag count, and every section indented by its nesting depth. Reorder with `↑`/`↓`, send a section to chat, or read its provenance badge. Sections you haven't saved yet show a `new` pill.",
		placement: 'left'
	},
	// Nostrdown chain — reference other events from your prose (builder → forms).
	'compose-nostrdown': {
		key: 'compose-nostrdown',
		anchor: 'compose-ref',
		title: 'Nostrdown — link & quote',
		body: "Reference other Nostr events right in your prose. This `{{ }}` button opens the reference builder — pick a sibling section, wiki topic, embed, slot, quote, or profile mention and the token lands at your cursor. The `auto` checkbox keeps inline autocomplete on: type `{{` and pick a kind, then `Tab`/`Enter` to accept. In the reader (and here), plain-click a reference to *preview* it, `⌘/Ctrl-click` to *follow*.",
		placement: 'bottom',
		next: 'compose-nostrdown-syntax'
	},
	'compose-nostrdown-syntax': {
		key: 'compose-nostrdown-syntax',
		anchor: 'compose-plain',
		title: 'The token forms',
		body: "`{{ref:Section Title}}` links a sibling section · `{{wiki:topic}}` a wiki/article · `{{embed:naddr…}}` transcludes *any* event — a note, an article, even an `npub` profile — as a card · `{{quote:naddr…|the text}}` quotes a passage inline, attributed (markup-agnostic, no headings) · `{{slot:naddr…}}` (own line) slots a whole 30040/30041 in as a child section · `{{@npub…}}` mentions a profile. Each reference also lands as a resolution tag (`a`/`e`/`p`/`wikilink`) on the event — check *Preview events* before you sign.",
		placement: 'top'
	},
	// E · Sections chain — the Full-view cards: cards → lock/claim → select → trash.
	'compose-sections': {
		key: 'compose-sections',
		anchor: 'compose-sections',
		title: 'Sections — each card is an event',
		body: 'Every card becomes a `30041` — its own title, content, and tags. *Drag* to reorder; *collapse* to titles for a quick outline.',
		placement: 'top',
		next: 'compose-lock'
	},
	'compose-lock': {
		key: 'compose-lock',
		anchor: 'compose-sections',
		title: 'Locked until you claim',
		body: "Imported and new sections arrive *locked*. Claim one (it turns yellow) to edit it; with a source publication, `Unlock all` / `Lock all` do it in bulk. An unlocked-but-*untouched* section still publishes as a transclusion of the original — you'll get a confirm before it does.",
		placement: 'top',
		next: 'compose-select'
	},
	'compose-select': {
		key: 'compose-select',
		anchor: 'compose-toolbar',
		title: 'Act on a selection',
		body: '`All` / `Inv` build a selection; `◂` sends it to chat, `▸` publishes just those sections, and `▸ all` collapses every card to reorder the outline quickly.',
		placement: 'bottom',
		next: 'compose-trash'
	},
	'compose-trash': {
		key: 'compose-trash',
		anchor: 'compose-toolbar',
		title: 'Two-stage delete',
		body: '`🗑` first removes the selection from the composer only. Arm it again and it becomes *delete everywhere* with a 10-second countdown — a deliberate second step.',
		placement: 'bottom'
	},
	// F · Publish chain — sign → diff/republish (view-agnostic).
	'compose-publish': {
		key: 'compose-publish',
		anchor: 'compose-actions',
		title: 'Sign → snapshot → broadcast',
		body: '`Sign` turns the draft into a signed snapshot — the only way an event enters the db. `Sign (N)` signs just the checked sections. *Broadcasting* to relays is a separate, later step. `Preview events` shows the exact JSON first.',
		placement: 'top',
		next: 'compose-diff'
	},
	'compose-diff': {
		key: 'compose-diff',
		anchor: 'compose-actions',
		title: 'Replace or fork',
		body: "`Diff vs published` compares the draft to the last published version of this article. If a publication of yours with this title already exists, signing *reuses its identifiers and replaces it* (republish); otherwise it's a new publication. Title-less *Notes* always take the flat, non-fork path (and hide `Diff vs published`).",
		placement: 'top'
	},

	// ── Search tour (on demand: the search panel's own W chip) ────────────
	// A hands-on drill: each step points at the search box and its "Try it"
	// runs a real example query (via `runSearchExample` → app.searchFor, which
	// fills the box and executes), then chains to the next. Surfaced only by
	// the search panel's W chip; the ? chip is the full syntax reference.
	'search-tour-intro': {
		key: 'search-tour-intro',
		anchor: 'search-input',
		title: 'Searching the pool',
		body: 'This box queries your local pool first; you can then extend a search out to relays. Filters compose with spaces. Tap `?` any time for the full syntax — here we’ll run a few live. Each step’s *Try it* fills the box and runs it.',
		placement: 'bottom',
		next: 'search-tour-text'
	},
	'search-tour-text': {
		key: 'search-tour-text',
		anchor: 'search-input',
		title: 'Exact text',
		body: 'A quoted `"phrase"` matches that text exactly inside event content. Bare words are looser. Try the exact-phrase form:',
		placement: 'bottom',
		next: 'search-tour-kind',
		action: { label: 'Try "nostr"', run: () => runSearchExample('"nostr"') }
	},
	'search-tour-kind': {
		key: 'search-tour-kind',
		anchor: 'search-input',
		title: 'By kind — incl. 30023',
		body: '`k:N` filters by event kind. `k:30023` is NIP-23 long-form articles; `k:1` short notes; `k:0` profiles; `k:30040` publication indexes. Pull the long-form articles in your pool:',
		placement: 'bottom',
		next: 'search-tour-author',
		action: { label: 'Try k:30023', run: () => runSearchExample('k:30023') }
	},
	'search-tour-author': {
		key: 'search-tour-author',
		anchor: 'search-input',
		title: 'By author / npub',
		body: '`by:` filters on the publishing key: `by:npub1…` for a specific person, `by:name:alice` for a profile-name partial, or `by:me` for yourself. Try your own events (swap in any `by:npub1…` for someone else):',
		placement: 'bottom',
		next: 'search-tour-nip19',
		action: { label: 'Try by:me', run: () => runSearchExample('by:me') }
	},
	'search-tour-nip19': {
		key: 'search-tour-nip19',
		anchor: 'search-input',
		title: 'Paste an entity',
		body: 'Paste any NIP-19 entity and it decodes to a precise filter: `note1…`/`nevent1…` jumps to one event, `npub1…`/`nprofile1…` to a person, `naddr1…` retrieves a specific publication. `id:<64-hex>` does the same as a raw event id. (No example to auto-run — paste your own.)',
		placement: 'bottom',
		next: 'search-tour-publication'
	},
	'search-tour-publication': {
		key: 'search-tour-publication',
		anchor: 'search-input',
		title: 'Retrieve a publication',
		body: '`k:30040` lists publication indexes — the “books” from the feed. Open one to read it, or address an exact one with its `naddr1…`. List the publications in your pool:',
		placement: 'bottom',
		next: 'search-tour-compose',
		action: { label: 'Try k:30040', run: () => runSearchExample('k:30040') }
	},
	'search-tour-compose': {
		key: 'search-tour-compose',
		anchor: 'search-input',
		title: 'Compose filters',
		body: 'Tokens *AND* together with spaces, so you narrow by stacking them: kind + text, author + kind, kind + time bound (`since:`/`until:`). Combine a kind with an exact phrase:',
		placement: 'bottom',
		next: 'search-tour-semantic',
		action: { label: 'Try k:30023 "nostr"', run: () => runSearchExample('k:30023 "nostr"') }
	},
	'search-tour-semantic': {
		key: 'search-tour-semantic',
		anchor: 'search-input',
		title: 'Semantic search',
		body: '`~:concept` finds events by *meaning*, not keywords — `~:"a longer phrase":15` caps the result count. It needs the embedding index (the `⚙` enables it; the mode-line pill shows its health). Try a concept:',
		placement: 'bottom',
		next: 'search-tour-tags',
		action: { label: 'Try ~:nostr', run: () => runSearchExample('~:nostr') }
	},
	'search-tour-tags': {
		key: 'search-tour-tags',
		anchor: 'search-input',
		title: 'Tag operators',
		body: '`has:NAME` matches any event carrying a NAME tag; `NAME:value` (a bare key — *no* `#`) filters on a tag’s value, e.g. `t:nostr`. Find everything with a title tag:',
		placement: 'bottom',
		next: 'search-tour-relays',
		action: { label: 'Try has:title', run: () => runSearchExample('has:title') }
	},
	'search-tour-relays': {
		key: 'search-tour-relays',
		anchor: 'search-input',
		title: 'Local first, then relays',
		body: 'Every search hits your *local pool* first — instant, offline. When the results look thin, the panel offers to *extend to relays* and pull what’s missing into the pool. That’s the whole loop: query local, reach out when needed, read or work with what you find.',
		placement: 'bottom',
		next: 'search-history'
	},

	// ── Event-menu tour ───────────────────────────────────────────────────
	// The "work with the event" branch from the feed/search. The menu is a
	// keyboard-chord surface (c/a/p prefixes); this walk names each section.
	// Surfaced two ways: the modal's own W chip (modal already open → straight
	// to `menu-overview`), and the global logo W (modal closed → `menu-open`
	// points at a feed row's `menu` pill and the chain resumes once the modal
	// mounts, via the armed-surface-tour flag — see `armSurfaceTour`).
	'menu-open': {
		key: 'menu-open',
		anchor: 'menu-pill',
		title: 'Open an event menu',
		body: 'This walkthrough lives *inside* an event’s menu. Click the `menu` pill on any article to open it — the tour picks up there automatically.',
		placement: 'right'
	},
	'menu-overview': {
		key: 'menu-overview',
		anchor: 'menu-header',
		title: 'The event menu',
		body: 'Everything you can do with one event, in one place. It’s keyboard-driven: press a section’s letter (`c`, `a`, `p`) then the inner key — e.g. `c` then `i` copies the id. Or just click. `Esc` closes.',
		placement: 'bottom',
		next: 'menu-copy'
	},
	'menu-copy': {
		key: 'menu-copy',
		anchor: 'menu-copy',
		title: 'Copy as',
		body: 'Grab the event’s identifiers: `i` the hex id, `e` an `nevent1…`, `a` an `naddr1…` (for replaceables like publications), `n` the author’s `npub1…`. These are exactly what the search box and others decode.',
		placement: 'right',
		next: 'menu-actions'
	},
	'menu-actions': {
		key: 'menu-actions',
		anchor: 'menu-actions',
		title: 'Actions',
		body: '`r` reads it (opens the reader), `f` finds the publications that contain this section, `i` inserts it into your draft, and `b` *broadcasts* it to your configured relays — a deliberate per-event push, never automatic.',
		placement: 'right',
		next: 'menu-pool'
	},
	'menu-pool': {
		key: 'menu-pool',
		anchor: 'menu-pool',
		title: 'The working pool',
		body: 'Route the event into your pool: `context` (chat), `compose` (a draft), or `refs` (held, no routing). The *lock* marks an import claimed vs. locked-to-source; `drop` removes it from every pool.',
		placement: 'right',
		next: 'menu-found'
	},
	'menu-found': {
		key: 'menu-found',
		anchor: 'menu-found',
		title: 'Found on',
		body: '*Provenance*: which relays this event has actually been seen on (or broadcast to), plus the always-present `local cache`. Click a relay chip for its NIP-11 info.',
		placement: 'top'
	}
};

/** The tip the "Run walkthrough" / "W" affordances kick off. On first run the
 *  Confirm-mode fetch modal is already open so this resolves immediately; with
 *  the modal closed it auto-skips and chains straight to `sign-in`. */
export const ENTRY_TIP = 'feed-sync';

type TipVars = Record<string, string | number>;

type DiscoveryState = {
	/** Master switch. False = never show tips (user opted out / not chosen). */
	enabled: boolean;
	/** Tip keys already shown — never re-shown until a reset clears this. */
	seen: string[];
	/** Pending tip keys; `queue[0]` is the one currently on screen. */
	queue: string[];
	/** Per-tip `{token}` substitutions, set at trigger time. Ephemeral — not
	 *  persisted; a data-driven tip restocks them each time its surface fires. */
	vars: Record<string, TipVars>;
	/** A tour armed to start when its surface next mounts. Set when a tour is
	 *  launched from afar (the global W) but its anchors live in a surface the
	 *  user must open first — e.g. the event menu. The surface clears it via
	 *  `consumeArmedTour` on mount and runs the tour then. Ephemeral. */
	armed: string | null;
};

/** Live walkthrough state. `enabled` defaults true so a fresh load with the
 *  modal's toggle left checked runs the intro; `loadDiscovery()` reconciles it
 *  with any persisted preference. */
export const discovery = $state<DiscoveryState>({
	enabled: true,
	seen: [],
	queue: [],
	vars: {},
	armed: null
});

/** Arm a tour to fire when its surface next mounts (see `armed`). Used by the
 *  global W's "Event menu" entry: the menu modal isn't open, so we point the
 *  user at the `menu` pill (`menu-open`) and arm `menu-overview` to run once the
 *  modal appears. */
export function armSurfaceTour(key: string) {
	discovery.armed = key;
}

/** If `key` is the armed tour, clear the flag and return true so the caller
 *  (the mounting surface) runs it. A no-op otherwise. */
export function consumeArmedTour(key: string): boolean {
	if (discovery.armed !== key) return false;
	discovery.armed = null;
	return true;
}

/** The tip currently on screen, or null when the queue is empty. */
export function activeTip(): TourTip | null {
	const key = discovery.queue[0];
	return key ? (TIPS[key] ?? null) : null;
}

/** A tip's body with its `{token}` placeholders resolved from runtime vars.
 *  Unknown tokens are left intact so a missing var reads as a visible gap
 *  rather than silently vanishing. */
export function renderBody(tip: TourTip): string {
	const vars = discovery.vars[tip.key];
	if (!vars) return tip.body;
	return tip.body.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
}

/** Stash `{token}` values for a tip without queuing it — for a tip reached via
 *  a `next` chain, whose data is known now but which surfaces only later. */
export function setTipVars(key: string, vars: TipVars) {
	discovery.vars[key] = vars;
}

function escapeHtml(s: string): string {
	return s.replace(
		/[&<>"]/g,
		(c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c] ?? c
	);
}

/** A tip body rendered to safe HTML for `{@html}`: vars are interpolated, the
 *  whole string is HTML-escaped (titles etc. are untrusted), then our own
 *  inline markup is applied last so its tags survive — `` `token` `` becomes a
 *  highlighted keyword/action chip (matching the search help panel's accent),
 *  `*word*` becomes emphasis. Order matters: escape before markup so a literal
 *  `<` in a title can't inject, and the backtick/asterisk delimiters (not HTML)
 *  pass through the escape untouched. */
export function renderBodyHtml(tip: TourTip): string {
	const esc = escapeHtml(renderBody(tip));
	return esc
		.replace(/`([^`]+)`/g, '<code class="dt-kw">$1</code>')
		.replace(/\*([^*]+)\*/g, '<em class="dt-em">$1</em>');
}

function persist() {
	if (!browser) return;
	try {
		localStorage.setItem(ENABLED_KEY, discovery.enabled ? '1' : '0');
		localStorage.setItem(SEEN_KEY, JSON.stringify(discovery.seen));
	} catch {
		// Storage full / disabled — in-memory state still drives the session.
	}
}

/** Hydrate enabled + seen from localStorage. Call once on app init. */
export function loadDiscovery() {
	if (!browser) return;
	try {
		const en = localStorage.getItem(ENABLED_KEY);
		// Default ON when unset so first-run (modal toggle checked) just works;
		// an explicit '0' (user opted out) is respected.
		discovery.enabled = en !== '0';
		const raw = localStorage.getItem(SEEN_KEY);
		if (raw) {
			const p = JSON.parse(raw);
			if (Array.isArray(p)) discovery.seen = p.filter((k): k is string => typeof k === 'string');
		}
	} catch {
		// Corrupt entry — fall back to in-memory defaults silently.
	}
}

/** Queue a tip if it's enabled, unseen, known, and not already queued. The
 *  overlay resolves the anchor and skips (auto-advances) if it's not mounted,
 *  so callers can fire triggers freely without checking the DOM. */
export function trigger(key: string, vars?: TipVars, opts?: { force?: boolean }) {
	if (vars) discovery.vars[key] = vars;
	if (!discovery.enabled) return;
	if (discovery.seen.includes(key)) return;
	const tip = TIPS[key];
	if (!tip) return;
	if (discovery.queue.includes(key)) return;
	// Precondition: if the goal this beat teaches is already accomplished, never
	// show it — skip straight to its `next`, so the auto walk collapses over
	// everything the user has already done (signed in, pool populated) and only
	// surfaces what's actually new. We do NOT mark it `seen`: suppression is not
	// completion, and `seen` drives the "done" checkmark on the opt-in W menus —
	// marking a never-shown tour seen would wrongly tick it off. The predicate
	// is re-evaluated on every trigger, so it stays suppressed while the
	// condition holds without needing the seen flag. Skipped for `force` (an
	// on-demand W tour the user asked for explicitly).
	if (!opts?.force && tip.relevantWhen && !tip.relevantWhen(world())) {
		if (tip.next) trigger(tip.next);
		return;
	}
	discovery.queue.push(key);
}

/** Dismiss the active tip (the X, Esc, Next, or a "try it" action): mark it seen
 *  and advance. Also covers the overlay's auto-skip when an anchor never mounts,
 *  so a guided segment self-heals over an absent surface. If the tip declares a
 *  `next` and the queue is now empty, chain to it. */
export function dismissActive() {
	const key = discovery.queue[0];
	if (!key) return;
	if (!discovery.seen.includes(key)) discovery.seen.push(key);
	discovery.queue.shift();
	persist();
	const nextKey = TIPS[key]?.next;
	if (nextKey && discovery.queue.length === 0) trigger(nextKey);
}

/** Close the walkthrough entirely — drop every queued tip but leave them
 *  unseen (a later reset/run can show them again). Used by "skip the rest". */
export function endWalkthrough() {
	discovery.queue = [];
}

/** Flip the master switch. Off clears the queue so nothing lingers. Backs the
 *  modal's "Run walkthrough" toggle (unchecked = never show tips). */
export function setWalkthroughEnabled(on: boolean) {
	discovery.enabled = on;
	if (!on) discovery.queue = [];
	persist();
}

/** Arm the walkthrough from the top: enable, clear seen + queue. Does NOT push a
 *  tip — the surfaces drive themselves (on first run the fetch modal mounts and
 *  triggers `feed-sync` exactly when it appears, with no auto-skip race). Backs
 *  the first-run modal's "Run walkthrough" toggle. */
export function startWalkthrough() {
	discovery.enabled = true;
	discovery.seen = [];
	discovery.queue = [];
	persist();
}

/** Arm + immediately surface the entry tip — for an on-demand replay (the
 *  header "Run walkthrough" button) where no surface is about to mount on its
 *  own. Forced past preconditions: this is an explicit "show me the walk again"
 *  request, so the entry appears even for an established user (signed in, pool
 *  populated) who would otherwise have every first-run beat self-suppress. The
 *  later beats remain event-gated (they fire as the user navigates). */
export function replayWalkthrough() {
	startWalkthrough();
	trigger(ENTRY_TIP, undefined, { force: true });
}

/** Run a specific tour now, on demand (the mode-line's W chip): enable tips,
 *  clear the queue, and un-see this tour's `next` chain so it replays in full,
 *  then surface its entry. Leaves every other tip's seen-state untouched, so a
 *  component tour doesn't disturb the first-run login walk. */
export function runTour(entryKey: string) {
	discovery.enabled = true;
	discovery.queue = [];
	const chain = new Set<string>();
	let k: string | undefined = entryKey;
	while (k && !chain.has(k)) {
		chain.add(k);
		k = TIPS[k]?.next;
	}
	discovery.seen = discovery.seen.filter((s) => !chain.has(s));
	persist();
	// Force past any precondition: an on-demand tour is explicit user intent, so
	// show it even when its goal is already met (e.g. the sign-in-methods tour
	// for someone already signed in).
	trigger(entryKey, undefined, { force: true });
}

/** Clear the seen set without starting the intro — discovery tips can fire
 *  fresh again as their surfaces are re-encountered. */
export function rearmDiscovery() {
	discovery.enabled = true;
	discovery.seen = [];
	persist();
}

/** The first-run onboarding walk: the fixed set of tips that auto-fire (in
 *  order, event-gated) the first time through, ending at `walk-done`. It is not
 *  a single `next` chain — most beats are surfaced by their surface mounting —
 *  so it's enumerated here. Everything else in `TIPS` is an on-demand "feature
 *  tour" replayed from a panel's `W` chip. Listed explicitly so the two groups
 *  can be re-armed independently (see `rearmFeatureTours`). Add a new onboarding
 *  beat here; new panel/feature tours need no change (they're the complement). */
const ONBOARDING_TIPS = new Set<string>([
	'feed-sync',
	'sign-in',
	'sign-in-methods',
	'signed-in-noname',
	'go-home',
	'general-feed',
	'feed-first-pub',
	'feed-first-badges',
	'walk-done'
]);

/** Re-arm only the on-demand feature tours — the panel `W`-chip walks (mode-line,
 *  reader, composer, search, event menus) — by un-seeing every tip that isn't
 *  part of the first-run onboarding walk. Each panel's `W` glows "new" again and
 *  replays in full; the onboarding walk's seen-state is left untouched. */
export function rearmFeatureTours() {
	discovery.enabled = true;
	discovery.seen = discovery.seen.filter((s) => ONBOARDING_TIPS.has(s));
	persist();
}
