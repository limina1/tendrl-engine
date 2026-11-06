// Which base renders a section? Pure precedence over the engine's declared
// format and the user's preferences. No heuristics anywhere: a section with
// no declared format and no user choice renders as source.
//
//   1. forced       — a `?highlight=` deep link: the anchor must land on the
//                     byte-identical text, so source wins over everything.
//   2. pick         — the per-buffer chip the user clicked (null = "source").
//                     Explicit beats remembered.
//   3. publication  — "always for this book" binding (by 30040 coordinate).
//   4. mimetype     — "always for <mimetype>" binding, keyed by the section's
//                     effective format (own m tag, else the kind's spec, else
//                     inherited from the nearest enclosing index that
//                     declares one).
//   5. format       — no binding, but a base registered for the effective
//                     format is enabled → use the first such base.
//   6. untagged     — the section declares nothing, but the user set a
//                     fallback markup for untagged sections: steps 4–5 run
//                     with that format instead (reported as `untagged`).
//   7. none         — source.
//
// Disabled layers never resolve: a stale binding to a layer the user has
// since switched off falls through, so "enabled" stays the single switch.

import { basesFor, isLayerId, SOURCE_ID } from './registry';
import type { LayerPrefs } from './store.svelte';

export type LayerSource =
	| 'forced'
	| 'pick'
	| 'publication'
	| 'mimetype'
	| 'format'
	| 'untagged'
	| 'none';
export type LayerDecision = { layerId: string | null; source: LayerSource };

/** `undefined` = no pick for this buffer; `null` = the user picked "source". */
export type LayerPick = string | null | undefined;

export function resolveLayer(
	input: {
		effectiveFormat: string | null | undefined;
		pubCoord: string | null | undefined;
		pick: LayerPick;
		forceSource?: boolean;
	},
	prefs: LayerPrefs
): LayerDecision {
	const enabled = (id: string | undefined | null) => !!id && prefs.enabled.includes(id);
	if (input.forceSource) return { layerId: null, source: 'forced' };
	if (input.pick !== undefined) {
		if (input.pick === null || input.pick === SOURCE_ID) return { layerId: null, source: 'pick' };
		if (isLayerId(input.pick) && enabled(input.pick)) return { layerId: input.pick, source: 'pick' };
		// A pick for a layer that has since been disabled / removed: fall through.
	}
	const byPub = input.pubCoord ? prefs.byPublication[input.pubCoord] : undefined;
	if (byPub === SOURCE_ID) return { layerId: null, source: 'publication' };
	if (enabled(byPub)) return { layerId: byPub!, source: 'publication' };
	const declared = input.effectiveFormat ?? null;
	const fmt = declared ?? prefs.untagged ?? null;
	if (fmt) {
		// With the untagged fallback the user's answer *is* the decision —
		// a per-mimetype binding still applies (it is their answer too).
		const via = (s: LayerSource): LayerSource => (declared ? s : 'untagged');
		const byMime = prefs.byMimetype[fmt];
		if (byMime === SOURCE_ID) return { layerId: null, source: via('mimetype') };
		if (enabled(byMime)) return { layerId: byMime!, source: via('mimetype') };
		const first = basesFor(fmt).find((l) => enabled(l.id));
		if (first) return { layerId: first.id, source: via('format') };
	}
	return { layerId: null, source: 'none' };
}
