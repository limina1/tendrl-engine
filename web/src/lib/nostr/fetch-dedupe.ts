// Session-level dedupe for mount-time relay refreshes. Buffer renderers
// unmount on every switch, so any "fetch fresh data on mount" effect
// otherwise re-hits the relays each alternation. `shouldNetworkFetch`
// grants one network pass per key per TTL window; within the window the
// caller should read local-only (an explicit user refresh bypasses this
// by not consulting it). Leaf module — no wm/state imports (see the WM
// import-cycle constraint).

const lastFetched = new Map<string, number>();
const MAX_KEYS = 500;

export function shouldNetworkFetch(key: string, ttlMs = 120_000): boolean {
	const now = Date.now();
	const t = lastFetched.get(key);
	if (t !== undefined && now - t < ttlMs) return false;
	if (!lastFetched.has(key) && lastFetched.size >= MAX_KEYS) {
		const oldest = lastFetched.keys().next().value;
		if (oldest !== undefined) lastFetched.delete(oldest);
	}
	lastFetched.set(key, now);
	return true;
}
