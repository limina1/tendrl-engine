// Client-side cache over the engine's NIP-54 slug normalizer
// (`POST /api/v1/nostrdown/normalize`). The grammar — including slug
// normalization — has one home (the engine); this is the composer's caching
// layer so interactive slug matching (sibling-title filtering, heading scroll,
// autocomplete) stays responsive without a local re-implementation. Batch-prefetch
// a known candidate set with `ensureSlugs`, then read synchronously via
// `cachedSlug`; use `slug` for a single on-demand value.

import { normalizeNostrdown } from '$lib/api';

const cache = new Map<string, string>();

/** The cached slug for `value`, or `''` if not yet fetched (a safe filter
 *  default — an empty query matches everything). Populate with `ensureSlugs`. */
export function cachedSlug(value: string): string {
	return cache.get(value) ?? '';
}

/** Batch-normalize any of `values` not already cached, in one round trip. */
export async function ensureSlugs(values: string[]): Promise<void> {
	const missing = [...new Set(values)].filter((v) => v && !cache.has(v));
	if (missing.length === 0) return;
	const slugs = await normalizeNostrdown(missing);
	missing.forEach((v, i) => cache.set(v, slugs[i] ?? ''));
}

/** Normalize (and cache) a single value. */
export async function slug(value: string): Promise<string> {
	if (!value) return '';
	if (!cache.has(value)) await ensureSlugs([value]);
	return cache.get(value) ?? '';
}
