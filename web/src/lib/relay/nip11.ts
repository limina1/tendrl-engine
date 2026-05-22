// Thin client for the engine's NIP-11 cache.
//
// The canonical fetcher + cache lives in Rust at `src/nip11.rs` and is
// served by `GET /api/v1/relay/info?url=...`. This module is just an
// adapter so Svelte components can read the engine's status without
// each one re-implementing a poll loop.
//
// Design notes:
//   - Engine returns the four-state machine (Pending | Loading |
//     Loaded | Failed). We retransmit it verbatim — UI sections per
//     `docs/relay-classes-and-info-port.md` §5 are pure functions of
//     the doc.
//   - When the engine returns Loading, we poll once after a short
//     delay so the row settles without the caller wiring its own
//     timer. Engine TTL (1h) handles longer-term caching, so a
//     completed fetch sticks across navigations within the tab.

export type Nip11Doc = {
	name?: string;
	description?: string;
	pubkey?: string;
	contact?: string;
	software?: string;
	version?: string;
	icon?: string;
	banner?: string;
	supported_nips?: number[];
	privacy_policy?: string;
	terms_of_service?: string;
	posting_policy?: string;
	limitation?: {
		auth_required?: boolean;
		payment_required?: boolean;
		restricted_writes?: boolean;
		max_message_length?: number;
		max_event_tags?: number;
		max_content_length?: number;
		max_subscriptions?: number;
		max_limit?: number;
		min_pow_difficulty?: number;
		created_at_lower_limit?: number;
		created_at_upper_limit?: number;
	};
	retention?: { kinds?: (number | [number, number])[]; time?: number; count?: number }[];
	relay_countries?: string[];
	language_tags?: string[];
	tags?: string[];
	fees?: {
		admission?: { amount: number; unit: string }[];
		subscription?: { amount: number; unit: string; period?: number }[];
		publication?: { amount: number; unit: string; kinds?: number[] }[];
	};
};

export type Nip11Status =
	| { state: 'pending' }
	| { state: 'loading' }
	| { state: 'loaded'; doc: Nip11Doc; fetched_at: number }
	| { state: 'failed'; error: string; fetched_at: number };

type Envelope = { url: string; status: Nip11Status };

export function normalizeRelayUrl(url: string): string {
	return url.trim().toLowerCase().replace(/\/+$/, '');
}

async function fetchOnce(url: string, force = false): Promise<Nip11Status> {
	const q = `url=${encodeURIComponent(url)}${force ? '&refresh=true' : ''}`;
	const resp = await fetch(`/api/v1/relay/info?${q}`);
	if (!resp.ok) {
		return { state: 'failed', error: `engine HTTP ${resp.status}`, fetched_at: 0 };
	}
	const env = (await resp.json()) as Envelope;
	return env.status;
}

// Per-tab dedup so two components asking about the same relay during
// the same render frame don't both poll the engine.
const inflight = new Map<string, Promise<Nip11Status>>();

/**
 * Read the engine's current NIP-11 status for a relay. If the engine
 * reports `Loading`, schedules a single follow-up poll so the caller's
 * `onUpdate` fires once the fetch lands without needing its own timer.
 *
 * Pass `{ force: true }` to bypass both the per-tab dedup and the
 * engine's cache (a retry after a transient failure).
 */
export function getRelayInfo(
	url: string,
	onUpdate?: (s: Nip11Status) => void,
	opts?: { force?: boolean }
): Nip11Status {
	const key = normalizeRelayUrl(url);
	const force = opts?.force ?? false;

	let promise = force ? undefined : inflight.get(key);
	if (!promise) {
		promise = fetchOnce(url, force).finally(() => inflight.delete(key));
		inflight.set(key, promise);
	}

	promise.then((status) => {
		if (onUpdate) onUpdate(status);
		// Engine returned Loading — fetch was kicked off, poll once when
		// the typical NIP-11 round trip should be done.
		if (status.state === 'loading') {
			setTimeout(() => {
				fetchOnce(url).then((s) => {
					if (onUpdate) onUpdate(s);
				});
			}, 1500);
		}
	});

	return { state: 'pending' };
}
