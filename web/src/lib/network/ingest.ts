// Ingest announcements — the engine's "events landed in the local store"
// signal (`ingested` on the fetch-events stream), fanned out to whoever
// shows those kinds.
//
// This is what makes views local-first AND reactive: a view reads its
// local query, renders at once, and re-reads when a kind it shows is
// ingested — instead of holding a request open until relays answer.
//
// Leaf module on purpose (no imports): `api.ts`, `state.svelte.ts` and
// `fetch-events.svelte.ts` all use it, and it must not close a cycle.

export interface IngestNotice {
	/** Distinct kinds in the batch. */
	kinds: number[];
	count: number;
	/** Authors of the kind-0 events in the batch. */
	profiles: string[];
}

type Listener = (n: IngestNotice) => void;
const listeners = new Set<Listener>();

/** Subscribe to every ingest notice. Returns the unsubscribe. */
export function onIngested(fn: Listener): () => void {
	listeners.add(fn);
	return () => listeners.delete(fn);
}

/**
 * Subscribe to ingests of specific kinds, throttled: a relay fetch lands
 * as many small batches, and nostrdb finishes writing a moment after the
 * notice, so the callback runs `delayMs` after the first matching batch
 * and then at most once per `delayMs` while batches keep coming — a long
 * sync fills the view progressively instead of all at the end.
 */
export function onKindsIngested(kinds: number[], fn: () => void, delayMs = 600): () => void {
	let timer: ReturnType<typeof setTimeout> | null = null;
	const off = onIngested((n) => {
		if (!n.kinds.some((k) => kinds.includes(k))) return;
		if (timer) return;
		timer = setTimeout(() => {
			timer = null;
			fn();
		}, delayMs);
	});
	return () => {
		if (timer) clearTimeout(timer);
		off();
	};
}

/** Called by the fetch-events stream for each `ingested` event. */
export function emitIngested(n: IngestNotice) {
	for (const fn of listeners) {
		try {
			fn(n);
		} catch (e) {
			console.error('[ingest] listener failed', e);
		}
	}
}
