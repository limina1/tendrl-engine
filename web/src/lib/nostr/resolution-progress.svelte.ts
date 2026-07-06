// Aggregate "how many nostrdown references have resolved" across a reader's
// visible sections. Resolution is per-section and async (parse → resolve →
// relay-fetch for not-local embeds), so a document with many references can spend
// a visible while resolving. Each RichContent reports its own (total, resolved)
// counts into a tracker provided by the enclosing reader; the reader renders one
// progress indicator from the sum.
//
// Wired by Svelte context so it works across all three reader views (outline,
// paginated, continuous) without prop-drilling, and is simply absent (a no-op)
// anywhere RichContent renders outside a reader.

import { getContext, setContext } from 'svelte';

const KEY = Symbol('nd-resolution-progress');

export class ResolutionTracker {
	byId = $state<Record<string, { total: number; resolved: number }>>({});

	/** Report a section's counts. `total` = parsed reference tokens; `resolved` =
	 *  references that have finished resolving (found or definitively not-found —
	 *  a still-fetching `pending` ref does not count until it settles). */
	report(id: string, total: number, resolved: number) {
		this.byId[id] = { total, resolved };
	}

	remove(id: string) {
		if (id in this.byId) {
			const next = { ...this.byId };
			delete next[id];
			this.byId = next;
		}
	}

	get total(): number {
		return Object.values(this.byId).reduce((a, x) => a + x.total, 0);
	}
	get resolved(): number {
		return Object.values(this.byId).reduce((a, x) => a + x.resolved, 0);
	}
	/** True while at least one reference is still resolving. */
	get resolving(): boolean {
		return this.total > 0 && this.resolved < this.total;
	}
	/** 0..1 for a progress bar. */
	get fraction(): number {
		return this.total === 0 ? 1 : this.resolved / this.total;
	}
}

/** Called by the reader (once) to expose a tracker to its RichContent subtree. */
export function provideResolutionTracker(): ResolutionTracker {
	const t = new ResolutionTracker();
	setContext(KEY, t);
	return t;
}

/** Called by RichContent to find its reader's tracker, if any. */
export function useResolutionTracker(): ResolutionTracker | undefined {
	return getContext(KEY);
}

let counter = 0;
/** A stable per-instance id for a reporting RichContent. */
export function nextResolutionId(): string {
	return `nd-${++counter}`;
}
