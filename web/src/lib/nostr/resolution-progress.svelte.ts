// Per-reader nostrdown reference-resolution progress.
//
// Threaded as a PROP, deliberately NOT Svelte context: this WM's buffer
// rendering doesn't provide a reliable component-init context, so getContext /
// setContext / onDestroy throw `lifecycle_outside_component` when a reader mounts
// (the codebase uses `$effect` teardowns instead). RichContent reports into a
// tracker it receives as a prop, via an `$effect` whose return is the teardown.
//
// Structurally loop-proof: the counts live in a plain (non-reactive) Map, and a
// single `$state` tick is bumped on mutation. Writers (`report`/`remove`) read
// nothing reactive, so calling them from RichContent's `$effect` can't make that
// effect depend on the state it writes — which would be an infinite update loop
// (`effect_update_depth_exceeded`). Only the getters read the tick, so only the
// reader's toolbar (which renders them) reacts.

export class ResolutionTracker {
	#counts = new Map<string, { total: number; resolved: number }>();
	#tick = $state(0);

	/** Report a section's counts. `total` = parsed reference tokens; `resolved` =
	 *  references the engine has returned a resolution entry for. */
	report(id: string, total: number, resolved: number) {
		const cur = this.#counts.get(id);
		if (cur && cur.total === total && cur.resolved === resolved) return;
		this.#counts.set(id, { total, resolved });
		this.#tick++;
	}

	remove(id: string) {
		if (this.#counts.delete(id)) this.#tick++;
	}

	#sum(select: (v: { total: number; resolved: number }) => number): number {
		void this.#tick; // reactive dependency
		let s = 0;
		for (const v of this.#counts.values()) s += select(v);
		return s;
	}

	get total(): number {
		return this.#sum((v) => v.total);
	}
	get resolved(): number {
		return this.#sum((v) => v.resolved);
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
