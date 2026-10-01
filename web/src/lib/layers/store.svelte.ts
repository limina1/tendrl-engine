// `tendrl.layers` — the user's rendering-layer preferences.
//
// Opt-in by construction: an empty `enabled` set means every section renders
// as source, exactly as before this layer existed. Bindings remember a pick
// the user promoted from the reader's tools bar ("always for text/markdown",
// "always for this book"); `SOURCE_ID` as a binding value means "never
// decorate this one".
//
// Per-device display preference, same as reading typography: localStorage
// only, never engine config, never a Nostr event. A per-buffer pick that was
// NOT promoted lives in the reader's buffer view state, not here.

import { browser } from '$app/environment';
import { isCanonicalMimetype, isLayerId, SOURCE_ID } from './registry';

const STORAGE_KEY = 'tendrl.layers';

export type LayerPrefs = {
	/** Layer ids the user switched on (Settings → Layers). */
	enabled: string[];
	/** canonical mimetype → layer id | SOURCE_ID. */
	byMimetype: Record<string, string>;
	/** "30040:<pubkey>:<d_tag>" → layer id | SOURCE_ID. */
	byPublication: Record<string, string>;
};

export const LAYER_DEFAULTS: LayerPrefs = { enabled: [], byMimetype: {}, byPublication: {} };

const PUB_COORD = /^30040:[0-9a-f]{64}:/;

function bindingValue(v: unknown): string | null {
	return v === SOURCE_ID || isLayerId(v) ? (v as string) : null;
}

function normalize(p: Partial<LayerPrefs> | null | undefined): LayerPrefs {
	if (!p || typeof p !== 'object') return { enabled: [], byMimetype: {}, byPublication: {} };
	const enabled = Array.isArray(p.enabled) ? p.enabled.filter(isLayerId) : [];
	const byMimetype: Record<string, string> = {};
	for (const [k, v] of Object.entries(p.byMimetype ?? {})) {
		const b = bindingValue(v);
		if (isCanonicalMimetype(k) && b) byMimetype[k] = b;
	}
	const byPublication: Record<string, string> = {};
	for (const [k, v] of Object.entries(p.byPublication ?? {})) {
		const b = bindingValue(v);
		if (PUB_COORD.test(k) && b) byPublication[k] = b;
	}
	return { enabled: [...new Set(enabled)], byMimetype, byPublication };
}

function stored(): LayerPrefs {
	if (!browser) return normalize(null);
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		return normalize(raw ? JSON.parse(raw) : null);
	} catch {
		return normalize(null);
	}
}

/** Live layer preferences. Seeded synchronously at module load. */
export const layers = $state<LayerPrefs>(stored());

function persist() {
	if (!browser) return;
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify($state.snapshot(layers)));
	} catch {
		// Storage unavailable — the in-memory prefs still hold this session.
	}
}

export function isLayerEnabled(id: string | null | undefined): boolean {
	return !!id && layers.enabled.includes(id);
}

export function setLayerEnabled(id: string, on: boolean) {
	if (!isLayerId(id)) return;
	const has = layers.enabled.includes(id);
	if (on && !has) layers.enabled = [...layers.enabled, id];
	else if (!on && has) layers.enabled = layers.enabled.filter((x) => x !== id);
	else return;
	persist();
}

/** Bind a mimetype to a layer (or SOURCE_ID); `null` removes the binding. */
export function bindMimetype(mimetype: string, layerId: string | null) {
	if (!isCanonicalMimetype(mimetype)) return;
	const next = { ...layers.byMimetype };
	if (layerId === null) delete next[mimetype];
	else if (bindingValue(layerId)) next[mimetype] = layerId;
	else return;
	layers.byMimetype = next;
	persist();
}

/** Bind a publication coordinate to a layer (or SOURCE_ID); `null` removes. */
export function bindPublication(coord: string, layerId: string | null) {
	if (!PUB_COORD.test(coord)) return;
	const next = { ...layers.byPublication };
	if (layerId === null) delete next[coord];
	else if (bindingValue(layerId)) next[coord] = layerId;
	else return;
	layers.byPublication = next;
	persist();
}

export function resetLayers() {
	Object.assign(layers, { enabled: [], byMimetype: {}, byPublication: {} });
	persist();
}
