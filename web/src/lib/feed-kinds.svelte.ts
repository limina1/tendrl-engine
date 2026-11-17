// The feed's kind registry, read from the engine (`GET /api/v1/feed/kinds`,
// `src/feed_kinds.rs` owns the table) — which kinds the feed lists, how a
// row's kind reads, and the kind-0 presence option. Leaf module: no wm or
// state imports, so the feed, the reader and the modeline can all read it.
import { browser } from '$app/environment';
import * as api from '$lib/api';

let view = $state<api.FeedKindsView>({ kinds: [], require_profile: false });
let loaded = $state(false);
let inflight: Promise<void> | null = null;

export function loadFeedKinds(): Promise<void> {
	if (!browser) return Promise.resolve();
	if (inflight) return inflight;
	inflight = (async () => {
		try {
			view = await api.getFeedKinds();
			loaded = true;
		} catch {
			// engine not up yet — the feed still lists (the engine applies
			// the same prefs); the toggles appear on the next read
		} finally {
			inflight = null;
		}
	})();
	return inflight;
}

export const feedKinds = {
	get view() {
		return view;
	},
	get loaded() {
		return loaded;
	},
	/** The registry entry for `kind`, if it is a feed kind. */
	entry(kind: number): api.FeedKind | undefined {
		return view.kinds.find((k) => k.kind === kind);
	},
	/** Kinds the feed currently lists (what an ingest must name to re-read). */
	enabledKinds(): number[] {
		const on = view.kinds.filter((k) => k.on).map((k) => k.kind);
		return on.length ? on : [30040];
	},
	async setKind(kind: number, on: boolean) {
		view = await api.putFeedKinds({ kinds: { [kind]: on } });
	},
	async setRequireProfile(on: boolean) {
		view = await api.putFeedKinds({ require_profile: on });
	}
};
