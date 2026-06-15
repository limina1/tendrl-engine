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

const ENABLED_KEY = 'tendrl.walkthrough.enabled';
const SEEN_KEY = 'tendrl.walkthrough.seen';

export type TourAction = {
	label: string;
	/** Runs when the user clicks the "try it" affordance. Advances the tip. */
	run: () => void;
};

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
};

// Registry. The first-run "feed sync" login walk (feed-sync → sign-in →
// sign-in-methods → signed-in → home → general-feed → feed-first-pub →
// feed-first-badges) plus standalone discovery tips. `ENTRY_TIP`
// is what the "Run walkthrough" / "W" affordances kick off; everything else
// fires from its own discovery point (see the per-surface triggers in +page /
// +layout).
export const TIPS: Record<string, TourTip> = {
	// ── First-run login walk ─────────────────────────────────────────────
	'feed-sync': {
		key: 'feed-sync',
		anchor: 'feed-sync',
		title: 'Feed sync',
		body: "Nothing is fetched until you approve it — this panel shows exactly what tendrl will pull, and from which relays. You're logged out, so this is the broad public feed: recent publications from these relays. Open Details to see the precise query and how many. When you're ready, close this (Esc) — let's sign in.",
		placement: 'right',
		next: 'sign-in'
	},
	'sign-in': {
		key: 'sign-in',
		anchor: 'settings',
		title: 'Sign in',
		body: 'Open Settings here and sign in — a NIP-07 browser extension, or paste an ncryptsec key with its password. Heads up: the moment you sign in you\'ll see your pubkey but no name yet.',
		placement: 'bottom'
	},
	'sign-in-methods': {
		key: 'sign-in-methods',
		anchor: 'identity-source',
		title: 'Two ways in',
		body: 'Engine — paste an ncryptsec below and unlock it with its password; held for this session only, never written to disk. NIP-07 — a browser extension holds the key and the engine never sees it. To connect one: (1) make sure your extension is activated/unlocked, (2) pick nip07 and press Reconnect, (3) your signer pops up asking to read your public key — Authorize, or Authorize forever.',
		placement: 'bottom'
	},
	'signed-in-noname': {
		key: 'signed-in-noname',
		anchor: 'me-chip',
		title: 'Signed in — no name yet',
		body: "There you are: a pubkey, but no display name or avatar. That's expected — your profile (a kind 0 event) lives on a relay you haven't pulled from yet. Fetching the feed will bring it in.",
		placement: 'bottom',
		next: 'go-home'
	},
	'go-home': {
		key: 'go-home',
		anchor: 'home',
		title: 'Back to the feed',
		body: 'Head home — click the tendrl logo (or the feed tab) to return to your feed, then sync it again.',
		placement: 'bottom'
	},
	'general-feed': {
		key: 'general-feed',
		anchor: 'general-feed',
		title: 'General feed — now optional',
		body: "Now that you're signed in, the query is scoped to you. “General feed” adds the broad public pull on top — toggle it off to fetch only the relays and authors you choose. Open Details to watch the query change (it now carries by:‹you›).",
		placement: 'left'
	},
	// ── After the first fetch: the feed has events ────────────────────────
	'feed-first-pub': {
		key: 'feed-first-pub',
		anchor: 'feed-first-pub',
		title: 'A publication is a book',
		body: 'A publication is like a book — a kind-30040 index that orders a set of sections (kind 30041) into a whole. This first one, “{title}”, gathers {sections}.',
		placement: 'bottom',
		next: 'feed-first-badges'
	},
	'feed-first-badges': {
		key: 'feed-first-badges',
		anchor: 'feed-first-badges',
		title: 'Provenance & actions',
		body: 'These pills show provenance — where the event lives in the network. The top one says it’s on {relays} right now. Click the row body to read the publication, or the “menu” pill to work with the raw event in depth.',
		placement: 'bottom'
	},

	// ── Standalone discovery tips (fire from their own surface) ───────────
	'search-history': {
		key: 'search-history',
		anchor: 'search-history',
		title: 'Search history',
		body: 'Every search you run is kept here. Click the pill to reopen and replay any past query.',
		placement: 'top'
	},

	// ── Mode-line tour (on demand: the mode-line's own W chip) ────────────
	// Not part of the login walk and never auto-fired — `runTour` surfaces it
	// only when the user taps W on the mode-line, then it chains through itself.
	'modeline-overview': {
		key: 'modeline-overview',
		anchor: 'modeline',
		title: 'The mode-line',
		body: 'This bottom strip is the mode-line — a live status bar (Emacs-style). It never interrupts you: tap W here for this quick tour, or ? for the full reference. Left half tells you where you are; right half is live status with quick toggles.',
		placement: 'top',
		next: 'modeline-focus'
	},
	'modeline-focus': {
		key: 'modeline-focus',
		anchor: 'ml-mode',
		title: 'Where you are',
		body: 'Your current mode and active layout (L:name), then the focused slot-class (@work / @chat / @research) and buffer. Switch buffers with SPC b b; pick layouts from the SPC leader.',
		placement: 'top',
		next: 'modeline-status'
	},
	'modeline-status': {
		key: 'modeline-status',
		anchor: 'ml-pills',
		title: 'Status & toggles',
		body: 'Live engine state, much of it clickable: relay config, fetch mode (click to flip auto / confirm), embedding health, and your identity. The 🔍 pill replays past searches.',
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
};

/** Live walkthrough state. `enabled` defaults true so a fresh load with the
 *  modal's toggle left checked runs the intro; `loadDiscovery()` reconciles it
 *  with any persisted preference. */
export const discovery = $state<DiscoveryState>({ enabled: true, seen: [], queue: [], vars: {} });

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
export function trigger(key: string, vars?: TipVars) {
	if (vars) discovery.vars[key] = vars;
	if (!discovery.enabled) return;
	if (discovery.seen.includes(key)) return;
	if (!TIPS[key]) return;
	if (discovery.queue.includes(key)) return;
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
 *  mode-line "W" button) where no surface is about to mount on its own. With the
 *  fetch modal open `feed-sync` resolves; with it closed the entry auto-skips and
 *  chains straight to `sign-in` (the always-present Settings button). */
export function replayWalkthrough() {
	startWalkthrough();
	trigger(ENTRY_TIP);
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
	trigger(entryKey);
}

/** Clear the seen set without starting the intro — discovery tips can fire
 *  fresh again as their surfaces are re-encountered. */
export function rearmDiscovery() {
	discovery.enabled = true;
	discovery.seen = [];
	persist();
}
