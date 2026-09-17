// The local store's change stream (`GET /api/v1/store/events`, SSE): the
// engine announces every event it ingests — a note the agent wrote over
// MCP, a comment, a style — and every annotation-request change. Views
// that show a publication subscribe and re-read their own root when a
// handle names it, so annotations show up as they are created without a
// click. Same shape as fetch-events: one module-level EventSource, explicit
// reconnect with capped backoff. Local only; this never opens a relay.
import { browser } from '$app/environment';

export type StoreEvent =
	| {
			type: 'ingested';
			kind: number;
			id: string;
			pubkey: string;
			coordinate?: string;
			roots: string[];
			addrs: string[];
	  }
	| { type: 'request'; id: string; root: string; parent: string; status: string };

type Listener = (ev: StoreEvent) => void;
const listeners = new Set<Listener>();

/** Subscribe; returns the unsubscribe. Call from an `$effect` and return
 *  the unsubscribe as its teardown. */
export function onStoreEvent(fn: Listener): () => void {
	listeners.add(fn);
	return () => {
		listeners.delete(fn);
	};
}

let backoffMs = 500;
let active: EventSource | null = null;
function start() {
	if (!browser) return;
	if (active) {
		active.close();
		active = null;
	}
	const es = new EventSource('/api/v1/store/events');
	active = es;
	es.onopen = () => {
		backoffMs = 500;
	};
	es.onmessage = (msg) => {
		let ev: StoreEvent;
		try {
			ev = JSON.parse(msg.data) as StoreEvent;
		} catch {
			return;
		}
		for (const fn of listeners) {
			try {
				fn(ev);
			} catch (e) {
				console.warn('[store-events] listener failed', e);
			}
		}
	};
	es.onerror = () => {
		es.close();
		if (active === es) active = null;
		const delay = backoffMs;
		backoffMs = Math.min(backoffMs * 2, 30_000);
		setTimeout(start, delay);
	};
}

if (browser) start();

/** Does this handle concern `root` — a note or comment anchored to it, a
 *  request on it, or a new version of one of its own events? */
export function concernsRoot(ev: StoreEvent, root: string): boolean {
	if (ev.type === 'request') return ev.root === root;
	if (ev.roots.includes(root) || ev.addrs.includes(root)) return true;
	if (ev.coordinate === root) return true;
	return false;
}
