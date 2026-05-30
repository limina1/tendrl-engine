// Publish-progress store: shape + mock data for the demo buffer.
//
// Real publish flow (to be wired later, gated on a signed identity):
//   1. Send event to tendrl's local relay first — durable local copy.
//   2. Broadcast in parallel to external relays in the publish set.
//   3. Track each (event_id × relay_url) cell as it transitions
//      pending → sending → accepted / rejected / timeout.
//   4. For 30040 publications: publish all 30041 sections first; only
//      publish the 30040 index once every section has at least one
//      accept across the relay set (so readers don't see broken `a`
//      tags). See docs/alexandria-publishing-documentation.org for the
//      gaps in Alexandria's flow we're explicitly trying to fix.
//
// Today, this module exists for UI design iteration. `mockProgress()`
// produces a representative cross-section of relay outcomes so the
// renderer can be shaped against real-looking data without a key.

export type RelayResult = 'pending' | 'sending' | 'accepted' | 'rejected' | 'timeout';

export interface PublishRelayStatus {
	url: string;
	isLocal: boolean;
	state: RelayResult;
	message?: string;
	durationMs?: number;
}

export interface PublishEventStatus {
	eventId: string;
	/** `kind:pubkey:dtag` form. Encode to `naddr` via `api.encode` for display. */
	aTag?: string;
	kind: number;
	title: string | null;
	author: string;
	relays: PublishRelayStatus[];
	/** Short preview of the content being published — so the buffer shows
	 *  *what* is going out, not just titles + relay status. */
	contentPreview?: string;
	/** Full signed event JSON, for the inline JSON modal. */
	rawEvent?: unknown;
}

export interface PublishProgressState {
	/** `kind:pubkey:dtag` for the publication index. Encode via `api.encode`. */
	aTag?: string;
	title?: string;
	authorPubkey?: string;
	events: PublishEventStatus[];
	startedAt: number;
	completed: boolean;
}

// Treat any localhost / loopback URL as the tendrl-relay. The
// "must-succeed for at least the local copy" guarantee is anchored on
// this identification.
export function isLocalRelay(url: string): boolean {
	const lower = url.toLowerCase();
	return (
		lower.includes('localhost') ||
		lower.includes('127.0.0.1') ||
		lower.includes('[::1]') ||
		lower.includes('://0.0.0.0')
	);
}

// Aggregate ratio: total accepted relay-cells / total relay-cells.
// Drives the red-yellow-green spectrum on the top-level bar.
export function aggregateAcceptRatio(state: PublishProgressState): {
	accepted: number;
	total: number;
	ratio: number;
} {
	let accepted = 0;
	let total = 0;
	for (const ev of state.events) {
		for (const r of ev.relays) {
			total++;
			if (r.state === 'accepted') accepted++;
		}
	}
	const ratio = total === 0 ? 0 : accepted / total;
	return { accepted, total, ratio };
}

// Per-event ratio for the inline sub-bar.
export function eventAcceptRatio(ev: PublishEventStatus): {
	accepted: number;
	total: number;
	ratio: number;
} {
	let accepted = 0;
	for (const r of ev.relays) if (r.state === 'accepted') accepted++;
	const ratio = ev.relays.length === 0 ? 0 : accepted / ev.relays.length;
	return { accepted, total: ev.relays.length, ratio };
}

// Map a 0..1 ratio to the red→yellow→green token spectrum.
// Thresholds match the modeline pill conventions: <0.5 is "warn",
// <0.85 is "ok-ish", ≥0.85 is "fully landed."
export function ratioColor(ratio: number): { fg: string; bg: string } {
	if (ratio >= 0.85) {
		return {
			fg: 'var(--state-online)',
			bg: 'color-mix(in srgb, var(--state-online) 22%, transparent)'
		};
	}
	if (ratio >= 0.5) {
		return {
			fg: 'var(--id-yours)',
			bg: 'color-mix(in srgb, var(--id-yours) 22%, transparent)'
		};
	}
	return {
		fg: 'var(--id-draft)',
		bg: 'color-mix(in srgb, var(--id-draft) 22%, transparent)'
	};
}

// Mock data used by the demo buffer. Designed to surface every
// state combination so the UI gets exercised:
//   - section 1: all four relays accept → green bar
//   - section 2: local accepts, two externals reject (rate-limited,
//                auth-required), one timeout → mixed
//   - section 3: local accepts, three externals still pending
//   - section 4: local accepts, one external rejected, others fine
//   - section 5: local rejects (!) — worst case; broken at the source
//   - index    : all four accept (we publish index last, after every
//                section has at least one accept; mock assumes this
//                guard already passed)
export function mockProgress(): PublishProgressState {
	const relays = [
		'ws://localhost:3334',
		'wss://relay.damus.io',
		'wss://nos.lol',
		'wss://theforest.nostr1.com'
	];
	const author = 'a01b2c3d4e5f6071829384a5b6c7d8e9f0a1b2c3d4e5f6071829384a5b6c7d8';
	const dTag = 'demo-publication-001';

	const mkRelays = (
		states: [RelayResult, string?][]
	): PublishRelayStatus[] => {
		return relays.map((url, i) => {
			const [state, message] = states[i] ?? ['pending'];
			return {
				url,
				isLocal: isLocalRelay(url),
				state,
				message,
				durationMs: state === 'accepted' ? 120 + i * 40 : undefined
			};
		});
	};

	const mkSection = (
		idSuffix: string,
		dSuffix: string,
		title: string,
		content: string,
		states: [RelayResult, string?][]
	): PublishEventStatus => {
		const sectionDTag = `${dTag}-${dSuffix}`;
		const eventId = `fa1c1c1c${idSuffix}${'0'.repeat(64 - 8 - idSuffix.length)}`;
		const aTag = `30041:${author}:${sectionDTag}`;
		return {
			eventId,
			aTag,
			kind: 30041,
			title,
			author,
			relays: mkRelays(states),
			contentPreview: content,
			rawEvent: {
				id: eventId,
				kind: 30041,
				pubkey: author,
				created_at: Math.floor((Date.now() - 1500) / 1000),
				tags: [
					['d', sectionDTag],
					['title', title]
				],
				content,
				sig: 'd34db33f'.repeat(16)
			}
		};
	};

	const indexEvent: PublishEventStatus = {
		eventId: `fa1c1c1c00${'0'.repeat(54)}`,
		aTag: `30040:${author}:${dTag}`,
		kind: 30040,
		title: 'Demo Publication — overall index',
		author,
		relays: mkRelays([['accepted'], ['accepted'], ['accepted'], ['accepted']]),
		contentPreview: 'Index — references 5 sections',
		rawEvent: {
			id: `fa1c1c1c00${'0'.repeat(54)}`,
			kind: 30040,
			pubkey: author,
			created_at: Math.floor(Date.now() / 1000),
			tags: [
				['d', dTag],
				['title', 'Demo Publication'],
				['a', `30041:${author}:${dTag}-introduction`, ''],
				['a', `30041:${author}:${dTag}-method`, ''],
				['a', `30041:${author}:${dTag}-results`, ''],
				['a', `30041:${author}:${dTag}-discussion`, ''],
				['a', `30041:${author}:${dTag}-conclusion`, '']
			],
			content: '',
			sig: 'd34db33f'.repeat(16)
		}
	};

	const sections: PublishEventStatus[] = [
		mkSection('01', 'introduction', 'Introduction', 'Lorem ipsum…', [
			['accepted'],
			['accepted'],
			['accepted'],
			['accepted']
		]),
		mkSection('02', 'method', 'Method', 'We did the thing.', [
			['accepted'],
			['rejected', 'rate-limited: max 1 event per 2 minutes'],
			['rejected', 'auth-required: relay requires NIP-42'],
			['timeout', 'no OK after 10s']
		]),
		mkSection('03', 'results', 'Results', 'p < 0.05.', [
			['accepted'],
			['sending'],
			['pending'],
			['pending']
		]),
		mkSection('04', 'discussion', 'Discussion', 'Discussion of the thing.', [
			['accepted'],
			['accepted'],
			['rejected', 'invalid: bad signature'],
			['accepted']
		]),
		mkSection('05', 'conclusion', 'Conclusion', 'In conclusion.', [
			['rejected', 'pow: requires min 16 leading zero bits'],
			['accepted'],
			['accepted'],
			['accepted']
		]),
		indexEvent
	];

	return {
		aTag: `30040:${author}:${dTag}`,
		title: 'Demo Publication',
		authorPubkey: author,
		events: sections,
		startedAt: Date.now() - 1500,
		completed: true
	};
}

// Reactive singleton. Module-level $state in .svelte.ts works under
// Svelte 5; the buffer renderer reads `store.current` and re-renders
// when `setProgress` writes.
type Store = { current: PublishProgressState | null };

const store: Store = $state({ current: null });

export function getStore(): Store {
	return store;
}

export function setProgress(state: PublishProgressState | null): void {
	store.current = state;
}

/**
 * Build a real `PublishProgressState` from a `/api/v1/publish` (or
 * `/publish/blocks`) response. Each `broadcast_results` entry is one
 * `(event_id × relay)` cell; we group them into one row per published
 * event — sections first, the 30040 index last, matching broadcast
 * order. Relays that returned no OK for an event show as `timeout`.
 *
 * Today's publish is one-shot synchronous, so this is a final snapshot
 * (`completed: true`), not a live pending→accepted animation — that
 * needs the SSE PublishSession from docs/publish-flow-engine-plan.md.
 */
export function progressFromPublish(
	resp: {
		publication_id: string;
		section_ids: string[];
		broadcast_results?: {
			relay: string;
			success: boolean;
			message: string | null;
			event_id: string;
		}[];
		events?: unknown[];
	},
	meta: { title: string; authorPubkey: string; sections: { title: string | null; content: string }[] }
): PublishProgressState {
	const results = resp.broadcast_results ?? [];
	// Column order = the relays actually attempted, first-seen order.
	const relayOrder = [...new Set(results.map((r) => r.relay))];

	// Map event id -> full event JSON so each row can be inspected.
	const rawById = new Map<string, unknown>();
	for (const e of resp.events ?? []) {
		const id = (e as { id?: string })?.id;
		if (id) rawById.set(id, e);
	}

	const cellsFor = (eventId: string): PublishRelayStatus[] =>
		relayOrder.map((url) => {
			const hit = results.find((r) => r.event_id === eventId && r.relay === url);
			if (!hit) {
				return { url, isLocal: isLocalRelay(url), state: 'timeout', message: 'no response' };
			}
			return {
				url,
				isLocal: isLocalRelay(url),
				state: hit.success ? 'accepted' : 'rejected',
				message: hit.message ?? undefined
			};
		});

	const preview = (text: string): string => {
		const t = text.trim().replace(/\s+/g, ' ');
		return t.length > 240 ? `${t.slice(0, 240)}…` : t;
	};

	const events: PublishEventStatus[] = [
		...resp.section_ids.map((id, i) => ({
			eventId: id,
			kind: 30041,
			title: meta.sections[i]?.title ?? null,
			author: meta.authorPubkey,
			relays: cellsFor(id),
			contentPreview: preview(meta.sections[i]?.content ?? ''),
			rawEvent: rawById.get(id)
		})),
		{
			eventId: resp.publication_id,
			kind: 30040,
			title: meta.title,
			author: meta.authorPubkey,
			relays: cellsFor(resp.publication_id),
			// 30040 index content MUST be empty (NKBIP-01) — describe its role.
			contentPreview: `Index — references ${resp.section_ids.length} section${resp.section_ids.length === 1 ? '' : 's'}`,
			rawEvent: rawById.get(resp.publication_id)
		}
	];

	return {
		title: meta.title,
		authorPubkey: meta.authorPubkey,
		events,
		startedAt: Date.now(),
		completed: true
	};
}
