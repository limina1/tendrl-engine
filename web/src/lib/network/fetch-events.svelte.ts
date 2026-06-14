// Subscribes to the engine's /api/v1/network/fetch-events SSE stream.
//
// The engine owns the confirm/auto decision and the gating — this
// module is a pure renderer. An `intent` that needs confirmation
// becomes a modal; one that doesn't (Auto mode) becomes an
// **activity toast** (pushActivityToast). `progress`/`relay_status`
// update that toast. `completed`/`failed` flip its kind + (re)set the
// auto-dismiss timer. The user can click → pin, then Expand into the
// FetchActivityModal for the full structured view (filters / phases /
// per-relay).
//
// The subscription self-starts at module scope (browser only) rather
// than from a component lifecycle hook — the stream is app-global and
// lives for the page's lifetime, so it doesn't belong to any one
// component's mount.

import { browser } from '$app/environment';
import { getAppState } from '$lib/state.svelte';
import type { FetchEvent } from '$lib/types';
import * as api from '$lib/api';

type AppState = ReturnType<typeof getAppState>;
type IntentEvent = Extract<FetchEvent, { type: 'intent' }>;
type PublishIntentEvent = Extract<FetchEvent, { type: 'publish_intent' }>;

/** The pending confirm intent a modal renders, if any. A fetch `intent`
 *  drives FetchConfirmModal; a `publish_intent` drives
 *  PublishConfirmModal — `+layout` branches on `.type`. Both carry an
 *  `operation_id` and `relays`, which is all `resolveConfirm` needs.
 *
 *  `queue` holds intents that arrived while one was already showing.
 *  With a single slot, a second intent overwrote the first, orphaning
 *  the engine's blocked oneshot until its 60s timeout — the user never
 *  saw (or got to decide on) the overwritten operation. Intents are
 *  presented strictly one at a time, FIFO. */
export const confirmState = $state<{
	intent: IntentEvent | PublishIntentEvent | null;
	queue: (IntentEvent | PublishIntentEvent)[];
}>({
	intent: null,
	queue: []
});

/** Show the next queued intent, if any. */
function advanceConfirmQueue() {
	confirmState.intent = confirmState.queue.shift() ?? null;
}

/** When a `reissueConfirm` is in flight, the next arriving intent REPLACES the
 *  current modal slot in place (instead of queueing or being preceded by a
 *  null), so re-composing the query — e.g. flipping the General-feed toggle —
 *  updates the open modal without closing + reopening it. */
let pendingReplace = false;

// operation_id → toast id, so progress/relay_status/completed update
// the right toast.
const opToasts = new Map<string, number>();

/** Resolve the AppState lazily — it's created by +layout, which may run
 *  after this module is first evaluated. Returns null if not ready. */
function appOrNull(): AppState | null {
	try {
		return getAppState();
	} catch {
		return null;
	}
}

function openActivityToast(
	app: AppState,
	operationId: string,
	label: string,
	mode: 'fetch' | 'publish',
	intent: Extract<FetchEvent, { type: 'intent' | 'publish_intent' }>
): number {
	let id = opToasts.get(operationId);
	if (id !== undefined) return id;
	// Long TTL — the engine's `completed`/`failed` drives the lifecycle.
	id = app.pushActivityToast(label, 120_000, {
		operation_id: operationId,
		summary: intent.summary,
		mode,
		// Seed relays from the intent so the modal can show pending rows
		// before any RelayStatus events arrive.
		relays: Object.fromEntries(intent.relays.map((r) => [r, { kind: 'connecting' as const }]))
	});
	opToasts.set(operationId, id);
	return id;
}

function handleEvent(ev: FetchEvent) {
	// Confirm intents bypass the toast — they need the modal directly.
	// `intent` → FetchConfirmModal, `publish_intent` → PublishConfirmModal
	// (both keyed off `confirmState.intent`; +layout branches on type).
	if ((ev.type === 'intent' || ev.type === 'publish_intent') && ev.needs_confirmation) {
		if (pendingReplace) {
			// A reissue is in flight — swap this re-composed intent into the
			// open modal in place. No close/reopen.
			pendingReplace = false;
			confirmState.intent = ev;
		} else if (confirmState.intent === null) {
			confirmState.intent = ev;
		} else {
			confirmState.queue.push(ev);
		}
		return;
	}

	const app = appOrNull();
	if (!app) return;

	switch (ev.type) {
		case 'intent':
			openActivityToast(app, ev.operation_id, ev.label, 'fetch', ev);
			break;
		case 'publish_intent':
			openActivityToast(app, ev.operation_id, ev.label, 'publish', ev);
			break;
		case 'progress': {
			const id = opToasts.get(ev.operation_id);
			if (id === undefined) break;
			const suffix = ev.total != null ? ` ${ev.done}/${ev.total}` : '';
			app.updateToast(id, { message: ev.label + suffix });
			break;
		}
		case 'relay_status': {
			// Update the per-relay row inside the activity toast.
			app.updateActivityRelay(ev.operation_id, ev.relay, ev.status);
			break;
		}
		case 'completed': {
			const id = opToasts.get(ev.operation_id);
			if (id !== undefined) {
				const n = ev.event_count;
				app.updateToast(
					id,
					{ message: `Done — ${n} relay${n === 1 ? '' : 's'} accepted`, kind: 'success' },
					2500
				);
				opToasts.delete(ev.operation_id);
			}
			break;
		}
		case 'failed': {
			const id = opToasts.get(ev.operation_id);
			if (id !== undefined) {
				if (ev.error === 'cancelled') {
					app.dismissToast(id);
				} else {
					app.updateToast(
						id,
						{ message: `Operation failed: ${ev.error}`, kind: 'error' },
						4500
					);
				}
				opToasts.delete(ev.operation_id);
			}
			// If the failed/cancelled op was awaiting the modal, close it
			// (and show the next queued intent, if any). A queued intent
			// whose op died (60s confirm timeout) is dropped too, so the
			// user isn't asked to approve an operation the engine has
			// already abandoned.
			confirmState.queue = confirmState.queue.filter(
				(i) => i.operation_id !== ev.operation_id
			);
			if (confirmState.intent?.operation_id === ev.operation_id) {
				// Mid-reissue: this is the op WE just cancelled to re-compose the
				// query. Keep the modal showing it until the replacement lands,
				// so the toggle updates in place rather than flickering closed.
				if (!pendingReplace) advanceConfirmQueue();
			}
			break;
		}
	}
}

/** Open the SSE subscription with explicit reconnect on error.
 *  EventSource has built-in retry but it gives up on hard errors
 *  (e.g. engine re-exec during a purge); without explicit reconnect
 *  the web loses its event channel and no intents / toasts arrive
 *  until a full reload. Exponential backoff caps at 30s. */
let reconnectBackoffMs = 500;
let activeEventSource: EventSource | null = null;
function startFetchEvents() {
	if (activeEventSource) {
		activeEventSource.close();
		activeEventSource = null;
	}
	const es = new EventSource('/api/v1/network/fetch-events');
	activeEventSource = es;
	es.onopen = () => {
		// Successful (re-)connect — reset the backoff so the next
		// drop doesn't start at 30s.
		reconnectBackoffMs = 500;
	};
	es.onmessage = (msg) => {
		try {
			handleEvent(JSON.parse(msg.data) as FetchEvent);
		} catch (e) {
			console.error('[fetch-events] bad SSE message', e);
		}
	};
	es.onerror = (e) => {
		console.warn(
			`[fetch-events] SSE connection error — reconnecting in ${reconnectBackoffMs}ms`,
			e
		);
		es.close();
		if (activeEventSource === es) {
			activeEventSource = null;
		}
		const delay = reconnectBackoffMs;
		reconnectBackoffMs = Math.min(reconnectBackoffMs * 2, 30_000);
		setTimeout(() => {
			if (browser) startFetchEvents();
		}, delay);
	};
}

/** The FetchConfirmModal's confirm/cancel reply. `relays` overrides the
 *  engine's proposed relay set when the user adjusted it. */
export function resolveConfirm(approved: boolean, relays?: string[]) {
	const intent = confirmState.intent;
	if (!intent) return;
	pendingReplace = false;
	advanceConfirmQueue();
	api.confirmFetch(intent.operation_id, approved, relays).catch((e: unknown) => {
		console.error('[fetch-events] confirm POST failed', e);
	});
}

/** Re-compose the current confirm intent in place: silently cancel the current
 *  operation but keep the modal open, so the caller can re-run the request
 *  (e.g. flipping a query option) and have the new intent REPLACE the open one
 *  without a close/reopen. Used by the General-feed toggle. */
export function reissueConfirm() {
	const intent = confirmState.intent;
	if (!intent) return;
	pendingReplace = true;
	api.confirmFetch(intent.operation_id, false).catch((e: unknown) => {
		console.error('[fetch-events] reissue cancel failed', e);
	});
	// Safety net: if no replacement arrives (e.g. mode flipped to auto so the
	// re-run doesn't need confirmation), don't leave the modal stuck on a
	// cancelled intent.
	setTimeout(() => {
		if (pendingReplace) {
			pendingReplace = false;
			if (confirmState.intent === intent) advanceConfirmQueue();
		}
	}, 5000);
}

// Self-start in the browser. Module scope is guaranteed to run on
// import — it does not depend on any component mounting.
if (browser) {
	startFetchEvents();
}
