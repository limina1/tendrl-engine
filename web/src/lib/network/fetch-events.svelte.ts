// Subscribes to the engine's /api/v1/network/fetch-events SSE stream.
//
// The engine owns the confirm/auto decision and the gating — this
// module is a pure renderer. An `intent` that needs confirmation
// becomes a modal; one that doesn't (Auto mode) becomes a toast.
// `progress`/`completed`/`failed` update that toast.
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

// operation_id → toast id, so progress/completed update the right toast.
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

function toastFor(app: AppState, operationId: string, label: string): number {
	let id = opToasts.get(operationId);
	if (id === undefined) {
		// Long TTL — the operation drives the lifecycle, not the timer.
		id = app.pushToast(label, 'pending', 120_000);
		opToasts.set(operationId, id);
	}
	return id;
}

function handleEvent(ev: FetchEvent) {
	// `intent` confirmation only touches confirmState, so it works even
	// before AppState exists. Toast updates need AppState — skip if not.
	if (ev.type === 'intent' && ev.needs_confirmation) {
		confirmState.intent = ev;
		return;
	}

	const app = appOrNull();
	if (!app) return;

	switch (ev.type) {
		case 'intent':
			// Auto mode — an informational progress toast, no modal.
			toastFor(app, ev.operation_id, ev.label);
			break;
		case 'progress': {
			const id = toastFor(app, ev.operation_id, ev.label);
			const suffix = ev.total != null ? ` ${ev.done}/${ev.total}` : '';
			app.updateToast(id, { message: ev.label + suffix }, 120_000);
			break;
		}
		case 'completed': {
			const id = opToasts.get(ev.operation_id);
			if (id !== undefined) {
				const n = ev.event_count;
				app.updateToast(
					id,
					{ message: `Fetched ${n} event${n === 1 ? '' : 's'}`, kind: 'success' },
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
					app.updateToast(id, { message: `Fetch failed: ${ev.error}`, kind: 'error' }, 4500);
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
