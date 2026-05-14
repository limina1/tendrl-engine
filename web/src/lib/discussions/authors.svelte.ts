// Reactive helpers for resolving a pubkey to a display name.
//
// The profile cache lives in `$lib/api` and notifies via
// `onProfileUpdate`. To make Svelte components re-render when that
// cache changes, we mirror the notification into a `$state` counter at
// module scope. Reading `getAuthorDisplayName` inside a `$derived` (or
// `$effect`) inside any component registers a dependency on `version`;
// when a new profile lands, `version` ticks and every dependent
// re-runs. Same pattern Svelte's docs recommend for bridging
// non-reactive caches into the runes world.

import * as api from '$lib/api';

let version = $state(0);
let subscribed = false;

function ensureSubscribed() {
	if (subscribed) return;
	subscribed = true;
	api.onProfileUpdate(() => {
		version += 1;
	});
}

function truncatePk(pubkey: string): string {
	if (!pubkey) return '';
	if (pubkey.length <= 12) return pubkey;
	return pubkey.slice(0, 8) + '…';
}

/**
 * Best display string for a pubkey. Resolution order:
 *   1. `display_name` from kind 0
 *   2. `name` from kind 0
 *   3. Truncated pubkey (first 8 chars + ellipsis)
 *
 * This is reactive: components that call it inside a `$derived` will
 * re-render when the underlying profile lands.
 */
export function getAuthorDisplayName(pubkey: string): string {
	ensureSubscribed();
	// Touch the reactive version so dependents subscribe.
	void version;
	const p = api.getCachedProfile(pubkey);
	if (p) {
		if (p.display_name && p.display_name.trim()) return p.display_name.trim();
		if (p.name && p.name.trim()) return p.name.trim();
	}
	return truncatePk(pubkey);
}

/**
 * True when we have a known name (not just a truncated pubkey) for the
 * author. Useful for components that want to render a name + a
 * monospaced pubkey side-by-side only when both pieces of info exist.
 */
export function hasAuthorName(pubkey: string): boolean {
	ensureSubscribed();
	void version;
	const p = api.getCachedProfile(pubkey);
	return !!(p?.display_name?.trim() || p?.name?.trim());
}

/**
 * Reactive read of the full cached profile (or null). Use this when a
 * caller needs both name and avatar in one place.
 */
export function getAuthorProfile(pubkey: string): api.Profile | null {
	ensureSubscribed();
	void version;
	return api.getCachedProfile(pubkey);
}

/** Trigger a debounced batch fetch for any pubkeys we haven't seen yet. */
export function prefetchAuthors(pubkeys: string[]) {
	api.prefetchProfiles(pubkeys);
}

/** Force a relay re-fetch of kind 0 for the given pubkeys + drop the
 *  web cache so renamed authors show up. Resolves once new profiles
 *  are in cache; subscribed components re-render automatically. */
export async function refreshAuthors(pubkeys: string[]): Promise<void> {
	await api.refreshProfiles(pubkeys);
}
