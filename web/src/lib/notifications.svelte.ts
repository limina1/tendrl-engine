// Notifications — what is for the user, chosen by kind in Settings ›
// Notifications, seen on the desktop modeline's 🔔 pill and the phone
// drawer's status row (docs/zettel/feat-notifications.org). One module-level
// store: rows + counts from `GET /api/v1/notifications`, refreshed on demand,
// when the buffer opens, and — debounced — when the store stream announces
// an ingested event of an enabled kind. Never a push: the count sits there
// until the list is opened. Leaf module (no wm imports) so any renderer or
// the modeline can read it without a cycle.
import { browser } from '$app/environment';
import * as api from '$lib/api';
import { onStoreEvent } from '$lib/store-events.svelte';

export type NotificationsState = {
	acts: api.NotificationRow[];
	activity: api.NotificationRow[];
	mine: api.NotificationRow[];
	/** Settled asks for me (merged / declined) — the engine's `done` group. */
	done: api.NotificationRow[];
	counts: api.NotificationCounts;
	rules: api.NotificationRule[];
	loading: boolean;
	error: string | null;
	/** Set once the engine answered at least once (so a zero is a real zero). */
	loaded: boolean;
};

const EMPTY_COUNTS: api.NotificationCounts = { acts: 0, activity_count: 0, activity_list: 0 };

let state = $state<NotificationsState>({
	acts: [],
	activity: [],
	mine: [],
	done: [],
	counts: EMPTY_COUNTS,
	rules: [],
	loading: false,
	error: null,
	loaded: false
});

// Plain bookkeeping, never rendered — keep it out of $state (see the
// $effect/async loop memory: reactive reads inside async fns re-trigger).
let inflight: Promise<void> | null = null;
let timer: ReturnType<typeof setTimeout> | null = null;
let seq = 0;

/** The kinds the enabled rules watch — what a store-stream handle must name
 *  to trigger a refresh. Falls back to the act kinds until the rules load. */
function watchedKinds(): Set<number> {
	const on = state.rules.filter((r) => r.on || r.fixed).map((r) => r.kind);
	return new Set(on.length ? on : [818, 7, 30041]);
}

/** Read the list (and, the first time, the rules). Coalesces concurrent calls. */
export function refresh(status: 'open' | 'all' = 'open'): Promise<void> {
	if (!browser) return Promise.resolve();
	if (inflight) return inflight;
	const mySeq = ++seq;
	state.loading = true;
	inflight = (async () => {
		try {
			const [list, prefs] = await Promise.all([
				api.getNotifications(status),
				state.rules.length ? Promise.resolve(null) : api.getNotificationPrefs()
			]);
			if (mySeq !== seq) return;
			state.acts = list.acts ?? [];
			state.activity = list.activity ?? [];
			state.mine = list.mine ?? [];
			state.done = list.done ?? [];
			state.counts = list.counts ?? EMPTY_COUNTS;
			if (prefs) state.rules = prefs.rules ?? [];
			state.error = null;
			state.loaded = true;
		} catch (e) {
			if (mySeq !== seq) return;
			// No identity yet (or the engine is not up): a quiet zero, not a toast.
			state.error = e instanceof Error ? e.message : String(e);
			state.loaded = true;
		} finally {
			state.loading = false;
			inflight = null;
		}
	})();
	return inflight;
}

/** Debounced refresh — the store stream and the settings pane call this. */
export function refreshSoon(delayMs = 300): void {
	if (!browser) return;
	if (timer) clearTimeout(timer);
	timer = setTimeout(() => {
		timer = null;
		void refresh();
	}, delayMs);
}

/** Re-read the rules (after a settings change) and the list. */
export async function reloadPrefs(): Promise<api.NotificationRule[]> {
	const p = await api.getNotificationPrefs();
	state.rules = p.rules ?? [];
	refreshSoon(0);
	return state.rules;
}

/** Save preference entries; the engine answers with the effective rules. */
export async function savePrefs(entries: Record<string, { on: boolean; how: 'count' | 'list' }>): Promise<api.NotificationRule[]> {
	const p = await api.putNotificationPrefs(entries);
	state.rules = p.rules ?? [];
	refreshSoon(0);
	return state.rules;
}

/** Pull from relays (Confirm-gated in Confirm mode — the FetchConfirmModal
 *  handles the intent; the request blocks until resolved). */
export async function fetchFromRelays(policy: 'local_first' | 'fetch_always' = 'fetch_always'): Promise<void> {
	if (!browser) return;
	state.loading = true;
	try {
		const list = await api.fetchNotifications({ policy, mode_confirm: true });
		state.acts = list.acts ?? [];
		state.activity = list.activity ?? [];
		state.mine = list.mine ?? [];
		state.counts = list.counts ?? EMPTY_COUNTS;
		state.error = null;
		state.loaded = true;
	} finally {
		state.loading = false;
	}
}

export const notifications = {
	get state() {
		return state;
	},
	refresh,
	refreshSoon,
	reloadPrefs,
	savePrefs,
	fetchFromRelays
};

// Live: an ingested event of a watched kind (or a request-record change)
// re-reads the list after a short debounce. Registered once at module
// scope, as a callback — not an $effect over the state.
if (browser) {
	onStoreEvent((ev) => {
		if (ev.type === 'request') {
			refreshSoon();
			return;
		}
		if (watchedKinds().has(ev.kind)) refreshSoon();
	});
	// First read shortly after boot (identity may still be unlocking —
	// a failure is a quiet zero and the next store event or open retries).
	setTimeout(() => void refresh(), 1500);
}
