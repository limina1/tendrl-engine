// Per-reader nostrdown reference-resolution progress.
//
// Threaded as a PROP, deliberately NOT Svelte context: this WM's buffer
// rendering doesn't provide a reliable component-init context, so getContext /
// setContext / onDestroy throw `lifecycle_outside_component` when a reader mounts
// (the codebase uses `$effect` teardowns instead — see ReaderBuffer). RichContent
// reports into a tracker it receives as a prop, via an `$effect` whose return is
// the teardown. A reader (published or draft) creates one tracker and renders the
// aggregate; anywhere without a tracker prop this is simply inert.

export class ResolutionTracker {
	byId = $state<Record<string, { total: number; resolved: number }>>({});

	/** Report a section's counts. `total` = parsed reference tokens; `resolved` =
	 *  references the engine has returned a resolution entry for. */
	report(id: string, total: number, resolved: number) {
		this.byId = { ...this.byId, [id]: { total, resolved } };
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

let counter = 0;
/** A stable per-instance id for a reporting RichContent. */
export function nextResolutionId(): string {
	return `nd-${++counter}`;
}
