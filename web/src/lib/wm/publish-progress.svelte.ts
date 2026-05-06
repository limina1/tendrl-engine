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
	naddr?: string;
	kind: number;
	title: string | null;
	author: string;
	relays: PublishRelayStatus[];
}

export interface PublishProgressState {
	naddr?: string;
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

	const sections: PublishEventStatus[] = [
		{
			eventId: 'fa1c1c1c01000000000000000000000000000000000000000000000000000001',
			naddr: `30041:${author}:${dTag}-introduction`,
			kind: 30041,
			title: 'Introduction',
			author,
			relays: mkRelays([['accepted'], ['accepted'], ['accepted'], ['accepted']])
		},
		{
			eventId: 'fa1c1c1c02000000000000000000000000000000000000000000000000000002',
			naddr: `30041:${author}:${dTag}-method`,
			kind: 30041,
			title: 'Method',
			author,
			relays: mkRelays([
				['accepted'],
				['rejected', 'rate-limited: max 1 event per 2 minutes'],
				['rejected', 'auth-required: relay requires NIP-42'],
				['timeout', 'no OK after 10s']
			])
		},
		{
			eventId: 'fa1c1c1c03000000000000000000000000000000000000000000000000000003',
			naddr: `30041:${author}:${dTag}-results`,
			kind: 30041,
			title: 'Results',
			author,
			relays: mkRelays([['accepted'], ['sending'], ['pending'], ['pending']])
		},
		{
			eventId: 'fa1c1c1c04000000000000000000000000000000000000000000000000000004',
			naddr: `30041:${author}:${dTag}-discussion`,
			kind: 30041,
			title: 'Discussion',
			author,
			relays: mkRelays([
				['accepted'],
				['accepted'],
				['rejected', 'invalid: bad signature'],
				['accepted']
			])
		},
		{
			eventId: 'fa1c1c1c05000000000000000000000000000000000000000000000000000005',
			naddr: `30041:${author}:${dTag}-conclusion`,
			kind: 30041,
			title: 'Conclusion',
			author,
			relays: mkRelays([
				['rejected', 'pow: requires min 16 leading zero bits'],
				['accepted'],
				['accepted'],
				['accepted']
			])
		},
		{
			eventId: 'fa1c1c1c00000000000000000000000000000000000000000000000000000000',
			naddr: `30040:${author}:${dTag}`,
			kind: 30040,
			title: 'Demo Publication — overall index',
			author,
			relays: mkRelays([['accepted'], ['accepted'], ['accepted'], ['accepted']])
		}
	];

	return {
		naddr: `30040:${author}:${dTag}`,
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
