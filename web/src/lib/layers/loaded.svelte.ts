// Runtime cache of loaded base implementations.
//
// A base's chunk is fetched lazily (first time a section needs it) and
// cached for the session. Bookkeeping is kept in plain objects; the ONE
// `$state` signal is a version counter that flips after an import resolves,
// so a `$derived` that called `baseImpl()` re-runs and finds the impl.
// The async continuation only *writes* that signal — it never reads
// `$state`, which is the pattern that looped before
// (project_effect_async_state_loop).

import { layerById, type BaseImpl, type StyleSpan } from './registry';

const impls: Record<string, BaseImpl> = {};
const inflight = new Set<string>();
let ver = 0;

/** Bumps once per completed load. Read it (via `baseImpl`) inside a
 *  `$derived` to subscribe. */
export const loadedVersion = $state({ n: 0 });

/** Kick off loading a layer's implementation (idempotent, fire-and-forget). */
export function ensureLoaded(id: string | null | undefined): void {
	if (!id || impls[id] || inflight.has(id)) return;
	const layer = layerById(id);
	if (!layer) return;
	inflight.add(id);
	layer
		.load()
		.then((impl) => {
			impls[id] = impl;
		})
		.catch((e) => {
			console.warn(`[layers] failed to load ${id}`, e);
		})
		.finally(() => {
			inflight.delete(id);
			ver += 1;
			loadedVersion.n = ver; // write-only; subscribers re-derive
		});
}

/** The loaded implementation, or `null` until its chunk has landed.
 *  Reactive: reads `loadedVersion`. */
export function baseImpl(id: string | null | undefined): BaseImpl | null {
	void loadedVersion.n;
	return id ? (impls[id] ?? null) : null;
}

// Tiny memo for the sync parse: a continuous view re-derives on every pref
// change and the same (layer, text) pair recurs constantly. Keyed on the
// text string itself — JS strings share the backing store, no copy.
const spanCache = new Map<string, StyleSpan[]>();
const SPAN_CACHE_MAX = 256;

/** Style spans for `text` under `layerId`; `[]` when the base is not
 *  loaded yet (the section renders as source until it is). */
export function spansFor(layerId: string | null | undefined, text: string | null | undefined): StyleSpan[] {
	if (!layerId || !text) return [];
	const impl = baseImpl(layerId);
	if (!impl) return [];
	const key = `${layerId}\u0000${text}`;
	const hit = spanCache.get(key);
	if (hit) return hit;
	let spans: StyleSpan[];
	try {
		spans = impl.spans(text);
	} catch (e) {
		console.warn(`[layers] ${layerId} failed to parse a section`, e);
		spans = [];
	}
	if (spanCache.size >= SPAN_CACHE_MAX) {
		const oldest = spanCache.keys().next().value;
		if (oldest !== undefined) spanCache.delete(oldest);
	}
	spanCache.set(key, spans);
	return spans;
}
