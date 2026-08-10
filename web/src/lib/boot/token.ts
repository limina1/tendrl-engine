/**
 * Loopback auth-token capture — must run before any API call.
 *
 * The Tauri Android host boots the engine with a per-boot secret and opens
 * the WebView at `/?shell=mobile&auth_token=<t>` (on Android the loopback
 * port is reachable by every app on the device, so the engine 401s `/api/`
 * requests without the token). This module moves the token out of the URL
 * and into a same-origin cookie, which then rides on every `fetch` AND every
 * `EventSource` automatically — no per-call-site attach code anywhere.
 *
 * Imported for its side effect from `+layout.ts` (SSR is off, so this only
 * ever runs in the browser). On desktop, where no token is issued, the param
 * is absent and this is a no-op.
 *
 * Leaf module by design — imports nothing (same TDZ rule as wm/shell).
 */

const PARAM = 'auth_token';
const COOKIE = 'tendrl_token';

export function captureLoopbackToken(): void {
	if (typeof window === 'undefined') return;
	const url = new URL(window.location.href);
	const token = url.searchParams.get(PARAM);
	if (!token) return;

	// Path-wide, strict same-site, session-lifetime: the host re-issues the
	// token (and re-opens this URL) every boot, so nothing needs to persist.
	document.cookie = `${COOKIE}=${encodeURIComponent(token)}; path=/; SameSite=Strict`;

	// Scrub the secret from the address bar / history; keep every other
	// param (`shell=` in particular) intact.
	url.searchParams.delete(PARAM);
	window.history.replaceState(window.history.state, '', url.toString());
}

captureLoopbackToken();
