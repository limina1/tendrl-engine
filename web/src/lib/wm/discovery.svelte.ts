// Contextual walkthrough — small "coachmark" tips that point at real UI as the
// user first discovers it. Not a forced linear tour: each tip fires the first
// time its surface is encountered (a buffer gains focus, a feature first
// appears), is descriptive up front, and is always dismissable (an X marks it
// seen so it never nags again). Deeper tips may carry a "try it" action.
//
// Two ways tips surface, both driven by `trigger(key)`:
//   - the intro sequence (`INTRO_SEQUENCE`) is enqueued on first load after the
//     network-mode choice, and by the "Run walkthrough" / "W" affordances;
//   - every other tip fires from its own discovery point in the UI (e.g. the
//     search-history pill the first time a search result appears).
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
	body: string;
	/** Where the card sits relative to its anchor. Default 'top'. */
	placement?: 'top' | 'bottom' | 'left' | 'right';
	/** Optional "try it" affordance — present only on deeper tips. */
	action?: TourAction;
};

// Registry. Copy + the full surface list are mapped out collaboratively
// (step 4); seeded here with the first-load surfaces so the click-through and
// the on-discovery flow are both live and testable end-to-end.
export const TIPS: Record<string, TourTip> = {
	modeline: {
		key: 'modeline',
		anchor: 'modeline',
		title: 'The mode-line',
		body: 'Your status bar: the current layout (L:), the focused slot (@class), the active buffer, the network mode, and the relay & search pills all live here.',
		placement: 'top'
	},
	feed: {
		key: 'feed',
		anchor: 'feed',
		title: 'The feed',
		body: 'Your stream of publications. Move with j / k, open with Enter. This is home base — everything else opens from here.',
		placement: 'bottom'
	},
	'search-history': {
		key: 'search-history',
		anchor: 'search-history',
		title: 'Search history',
		body: 'Every search you run is kept here. Click the pill to reopen and replay any past query.',
		placement: 'top'
	}
};

/** Ordered intro sequence — fired on first load and by "Run walkthrough".
 *  Other tips fire from their own discovery triggers, not from this list. */
export const INTRO_SEQUENCE: string[] = ['modeline', 'feed'];

type DiscoveryState = {
	/** Master switch. False = never show tips (user opted out / not chosen). */
	enabled: boolean;
	/** Tip keys already shown — never re-shown until a reset clears this. */
	seen: string[];
	/** Pending tip keys; `queue[0]` is the one currently on screen. */
	queue: string[];
};

/** Live walkthrough state. `enabled` defaults true so a fresh load with the
 *  modal's toggle left checked runs the intro; `loadDiscovery()` reconciles it
 *  with any persisted preference. */
export const discovery = $state<DiscoveryState>({ enabled: true, seen: [], queue: [] });

/** The tip currently on screen, or null when the queue is empty. */
export function activeTip(): TourTip | null {
	const key = discovery.queue[0];
	return key ? (TIPS[key] ?? null) : null;
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
export function trigger(key: string) {
	if (!discovery.enabled) return;
	if (discovery.seen.includes(key)) return;
	if (!TIPS[key]) return;
	if (discovery.queue.includes(key)) return;
	discovery.queue.push(key);
}

/** Dismiss the active tip (the X, Esc, or a "try it" action): mark it seen and
 *  advance to the next queued tip. */
export function dismissActive() {
	const key = discovery.queue[0];
	if (!key) return;
	if (!discovery.seen.includes(key)) discovery.seen.push(key);
	discovery.queue.shift();
	persist();
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

/** Re-arm and run from the top: enable, clear seen + queue, then enqueue the
 *  intro sequence. Backs the first-run modal (when the toggle is on), the
 *  mode-line "W" button, and the Settings "Run walkthrough" control. */
export function startWalkthrough() {
	discovery.enabled = true;
	discovery.seen = [];
	discovery.queue = [];
	persist();
	for (const key of INTRO_SEQUENCE) trigger(key);
}

/** Clear the seen set without starting the intro — discovery tips can fire
 *  fresh again as their surfaces are re-encountered. */
export function rearmDiscovery() {
	discovery.enabled = true;
	discovery.seen = [];
	persist();
}
