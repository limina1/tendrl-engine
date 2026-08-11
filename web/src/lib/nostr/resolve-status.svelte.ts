// Aggregated wiki-link resolution status for the reader views currently on
// screen. Renderers register on mount, push {total, found, busy, refetch}
// as refs land, and unregister on teardown — and since buffer renderers
// unmount on every switch, the registry always reflects exactly what's
// visible. The modeline renders the aggregate as a progress pill that is
// also the "resolve everything here" button (one Confirm-mode intent for
// the whole screen; a manual re-fetch in Auto). Leaf module — no wm/state
// imports (WM import-cycle constraint).

export type ResolveViewStatus = {
	/** Wiki refs in the view (token count until resolution lands). */
	total: number;
	/** Wiki refs resolved to a local event. */
	found: number;
	/** A resolve/backfill/forced pass is in flight for this view. */
	busy: boolean;
	/** Force a relay pass for this view; resolves to newly-found count. */
	refetch: () => Promise<number>;
};

let seq = 0;
const views = $state<Record<number, ResolveViewStatus>>({});

export function registerResolveView(): {
	update: (s: ResolveViewStatus) => void;
	unregister: () => void;
} {
	const id = ++seq;
	return {
		update(s: ResolveViewStatus) {
			views[id] = s;
		},
		unregister() {
			delete views[id];
		}
	};
}

export const resolveStatus = {
	get total(): number {
		return Object.values(views).reduce((n, v) => n + v.total, 0);
	},
	get found(): number {
		return Object.values(views).reduce((n, v) => n + v.found, 0);
	},
	get busy(): boolean {
		return Object.values(views).some((v) => v.busy);
	},
	/** Run every visible view's forced relay pass; total newly-found refs. */
	async refetchAll(): Promise<number> {
		const counts = await Promise.all(
			Object.values(views).map((v) => v.refetch().catch(() => 0))
		);
		return counts.reduce((a, b) => a + b, 0);
	}
};
