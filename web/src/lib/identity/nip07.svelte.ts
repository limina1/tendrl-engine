// Reactive window.nostr availability.
//
// A NIP-07 extension normally injects `window.nostr` at document_start, but a
// user can also *enable or unlock* an extension at any time after the page has
// loaded — the walkthrough literally tells them to "click your extension to
// activate it". A one-shot or time-boxed check then latches "no extension"
// forever (the radio stays disabled) until the surface re-mounts, which is why
// the only known workaround was to leave Settings and come back.
//
// This watcher keeps the flag live and reactive instead. It is event-driven
// (no perpetual polling): a short burst on start covers the inject race, and a
// fresh burst runs whenever the tab regains focus / becomes visible — the exact
// moment a user returns after clicking their extension. Consumers just read
// `nip07.available`; Svelte re-derives when it flips.

import { browser } from '$app/environment';
import { detectNip07 } from './signer';

/** Reactive availability of a `window.nostr` signer. */
export const nip07 = $state<{ available: boolean }>({ available: false });

let started = false;
let bursting = false;

/** A few quick checks with backoff — covers an extension that injects a beat
 *  after the page (boot race) or a beat after focus returns (just enabled). */
async function pollBurst() {
	if (nip07.available || bursting) return;
	bursting = true;
	try {
		for (const delay of [0, 100, 250, 500, 1000]) {
			if (delay) await new Promise((r) => setTimeout(r, delay));
			if (detectNip07()) {
				nip07.available = true;
				return;
			}
		}
	} finally {
		bursting = false;
	}
}

/** Begin watching for `window.nostr`. Idempotent — every surface that cares can
 *  call it; the first call wires the listeners, the rest no-op. Listeners are
 *  process-lived (the app is a single page); `pollBurst` self-skips once found. */
export function startNip07Watch() {
	if (!browser || started) return;
	started = true;
	void pollBurst();
	// Re-check on return-to-tab: clicking a browser extension blurs the page,
	// enabling one in browser settings means leaving and coming back — either
	// way `focus`/`visibilitychange` fires when the user is back, and the burst
	// catches the freshly-injected signer without a manual re-pick.
	window.addEventListener('focus', () => void pollBurst());
	document.addEventListener('visibilitychange', () => {
		if (document.visibilityState === 'visible') void pollBurst();
	});
}
