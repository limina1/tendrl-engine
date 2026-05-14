// Reusable relay-fetch pattern.
//
// Two modes, decided by network state:
//   - online   → auto-fetch from the engine's configured fetch relays
//   - offline  → pop a modal listing the configured relays as toggles
//                + an "append a relay" input. Resolves when the user
//                confirms; rejects to null on cancel.
//
// Either mode can be forced with `forcePrompt` so a UI affordance
// (kebab / shift-click) can always invoke the modal, even online, for
// per-session relay overrides.

import * as api from '$lib/api';

export type RelayFetchOpts = {
	/** Short human-readable purpose, surfaced in the modal header. */
	title: string;
	kinds: number[];
	authors: string[];
	limit?: number;
};

export type RelayFetchResult = {
	relays: string[];
	per_relay: { relay: string; fetched: number; error?: string }[];
	total_fetched: number;
};

type Pending = {
	opts: RelayFetchOpts;
	configRelays: string[];
	resolve: (relays: string[] | null) => void;
};

// Module-level reactive state. Exported as an object whose properties
// are read in the consuming template (`fetchModal.pending`) — Svelte 5
// requires the read to happen through property access, not a function
// call, for cross-module reactivity to propagate.
//
// `sessionRelays` carries relays the user appended in a previous modal
// invocation. They persist across multiple fetches in the same browser
// session (until reload) so a user who added `wss://relay.foo` once
// doesn't have to re-type it for every subsequent fetch — they remain
// pre-checked in the modal and are merged into auto-fetches when online.
export const fetchModal = $state<{
	pending: Pending | null;
	sessionRelays: string[];
}>({ pending: null, sessionRelays: [] });

export function addSessionRelay(url: string) {
	if (!url) return;
	if (fetchModal.sessionRelays.includes(url)) return;
	fetchModal.sessionRelays = [...fetchModal.sessionRelays, url];
}

export function removeSessionRelay(url: string) {
	fetchModal.sessionRelays = fetchModal.sessionRelays.filter((u) => u !== url);
}

export function confirmFetchModal(relays: string[]) {
	const p = fetchModal.pending;
	if (!p) return;
	fetchModal.pending = null;
	p.resolve(relays);
}

export function cancelFetchModal() {
	const p = fetchModal.pending;
	if (!p) return;
	fetchModal.pending = null;
	p.resolve(null);
}

/**
 * Run a fetch against relays, prompting only when necessary.
 *
 * `forcePrompt` makes the modal appear regardless of network state.
 * Returns null when the user cancels or no relays were selected.
 */
export async function fetchFromRelaysWithPrompt(
	opts: RelayFetchOpts,
	flags: { isOnline: boolean; forcePrompt?: boolean }
): Promise<RelayFetchResult | null> {
	const cfg = await api.getRelayConfig();
	const configRelays = cfg.fetch.urls;

	let chosen: string[] | null;
	if (flags.isOnline && !flags.forcePrompt) {
		// Online auto-fetch: configured fetch relays + any session
		// relays the user has previously added. Sessions augment config
		// for the duration of the page, not replace it.
		const union = new Set<string>([...configRelays, ...fetchModal.sessionRelays]);
		chosen = [...union];
	} else {
		if (fetchModal.pending) {
			// A previous prompt is still on screen. Don't pile another one
			// on top — let the caller try again after the user closes it.
			return null;
		}
		chosen = await new Promise<string[] | null>((resolve) => {
			fetchModal.pending = { opts, configRelays, resolve };
		});
	}

	if (!chosen || chosen.length === 0) return null;

	const per_relay: RelayFetchResult['per_relay'] = [];
	let total = 0;
	const limit = opts.limit ?? 500;
	// This is an explicit user-initiated action. Bypass the engine's
	// offline-mode short-circuit so the relay round-trip actually
	// happens; otherwise tracked_fetch returns an empty Vec when the
	// engine is in offline mode and the user sees "nothing fetched".
	for (const relay of chosen) {
		try {
			const r = await api.fetchFromRelay(relay, opts.kinds, opts.authors, limit, {
				bypassOffline: true
			});
			per_relay.push({ relay, fetched: r.fetched });
			total += r.fetched;
		} catch (e) {
			per_relay.push({
				relay,
				fetched: 0,
				error: e instanceof Error ? e.message : String(e)
			});
		}
	}
	return { relays: chosen, per_relay, total_fetched: total };
}
