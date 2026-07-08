// Relay-fetch helper.
//
// The engine owns the confirm/auto gating now — this just fires one
// multi-relay fetch and lets the engine emit the confirm Intent
// (Confirm mode) or fetch straight away (Auto). The SSE subscriber
// (lib/network/fetch-events) renders the modal and the progress toast.

import * as api from '$lib/api';

export type RelayFetchOpts = {
	/** Short human-readable purpose (legacy — the engine now builds the
	 *  label shown in the confirm modal). */
	title: string;
	query?: string;
	kinds: number[];
	authors: string[];
	limit?: number;
	/** NIP-50 free-text search string. */
	search?: string;
	/** NIP-01 `until` bound — backfill cursor for paging older events. */
	until?: number;
};

export type RelayFetchResult = {
	relays: string[];
	total_fetched: number;
};

/**
 * Fetch from the engine's configured fetch relays in one operation.
 *
 * In Confirm mode the engine gates this behind the confirm modal; the
 * `flags` argument is accepted only for call-site compatibility and is
 * ignored — the engine, not the UI, decides whether to prompt.
 */
export async function fetchFromRelaysWithPrompt(
	opts: RelayFetchOpts,
	_flags?: { isOnline?: boolean; forcePrompt?: boolean }
): Promise<RelayFetchResult | null> {
	const cfg = await api.getRelayConfig();
	const relays = cfg.fetch.urls;
	if (relays.length === 0) return null;
	try {
		const r = await api.fetchFromRelay(relays, opts.kinds, opts.authors, opts.limit ?? 500, {
			modeConfirm: true,
			search: opts.search,
			until: opts.until
		});
		return { relays: r.relays ?? relays, total_fetched: r.fetched };
	} catch (e) {
		console.warn('[relay-fetch] fetch failed', e);
		return null;
	}
}
