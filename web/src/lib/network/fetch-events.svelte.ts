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

/** The pending confirm intent the FetchConfirmModal renders, if any. */
export const confirmState = $state<{ intent: IntentEvent | null }>({ intent: null });

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
	if ((ev.type === 'intent' || ev.type === 'publish_intent') && ev.needs_confirmation) {
		// FetchConfirmModal only handles fetch intents today; publish
		// confirmations fall through to a basic toast for now (the
		// existing modal doesn't yet render publish summaries).
		if (ev.type === 'intent') {
			confirmState.intent = ev;
			return;
		}
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
			// If the failed/cancelled op was awaiting the modal, close it.
			if (confirmState.intent?.operation_id === ev.operation_id) {
				confirmState.intent = null;
			}
			break;
		}
	}
}

/** Open the SSE subscription. */
function startFetchEvents() {
	const es = new EventSource('/api/v1/network/fetch-events');
	es.onmessage = (msg) => {
		try {
			handleEvent(JSON.parse(msg.data) as FetchEvent);
		} catch (e) {
			console.error('[fetch-events] bad SSE message', e);
		}
	};
	es.onerror = (e) => console.error('[fetch-events] SSE connection error', e);
}

/** The FetchConfirmModal's confirm/cancel reply. `relays` overrides the
 *  engine's proposed relay set when the user adjusted it. */
export function resolveConfirm(approved: boolean, relays?: string[]) {
	const intent = confirmState.intent;
	if (!intent) return;
	confirmState.intent = null;
	api.confirmFetch(intent.operation_id, approved, relays).catch((e: unknown) => {
		console.error('[fetch-events] confirm POST failed', e);
	});
}

// Self-start in the browser. Module scope is guaranteed to run on
// import — it does not depend on any component mounting.
if (browser) {
	startFetchEvents();
}
