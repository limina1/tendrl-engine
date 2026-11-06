// Per-section convenience over resolve + the loaded cache, for the two
// reading views (ContinuousView, SectionCard). Reactive on the `layers`
// store and on the loaded-version signal.

import { layers } from './store.svelte';
import { resolveLayer, type LayerDecision, type LayerPick } from './resolve';
import { ensureLoaded, spansFor } from './loaded.svelte';
import type { StyleSpan } from './registry';

export type { LayerPick, LayerDecision };
export { ensureLoaded, spansFor };

/** Which base (if any) renders `section` in the given reading context. */
export function decideFor(
	section: { effective_format?: string | null } | null | undefined,
	pubCoord: string | null | undefined,
	pick: LayerPick,
	forceSource = false
): LayerDecision {
	return resolveLayer(
		{
			effectiveFormat: section?.effective_format ?? null,
			pubCoord: pubCoord && pubCoord.startsWith('30040:') ? pubCoord : null,
			pick,
			forceSource
		},
		layers
	);
}

/** Decide + produce spans in one go (spans are `[]` until the chunk lands). */
export function stylesFor(
	section: { effective_format?: string | null; content?: string | null } | null | undefined,
	pubCoord: string | null | undefined,
	pick: LayerPick,
	forceSource = false
): StyleSpan[] {
	const d = decideFor(section, pubCoord, pick, forceSource);
	return spansFor(d.layerId, section?.content);
}
