// `tendrl.layers` — the user's rendering-layer preferences.
//
// The three markup bases are on by default: a fresh install (no stored
// prefs) renders a tagged section with its base straight away. Once the user
// has touched the settings the stored object is honoured as-is — a base
// switched off stays off. Bindings remember a pick the user promoted from
// the reader's tools bar ("always for text/markdown", "always for this
// book"); `SOURCE_ID` as a binding value means "never decorate this one".
// `untagged` is the user's own answer for sections that declare no format
// (Alexandria publishes 30041s without an `m` tag): null = source.
//
// Per-device display preference, same as reading typography: localStorage
// only, never engine config, never a Nostr event. A per-buffer pick that was
// NOT promoted lives in the reader's buffer view state, not here.

import { browser } from '$app/environment';
import {
	ALL_BASE_IDS,
	isCanonicalMimetype,
	isLayerId,
	SOURCE_ID,
	type CanonicalMimetype
} from './registry';

const STORAGE_KEY = 'tendrl.layers';

export type LayerPrefs = {
	/** Layer ids switched on (Settings → Layers). All bases by default. */
	enabled: string[];
	/** canonical mimetype → layer id | SOURCE_ID. */
	byMimetype: Record<string, string>;
	/** "30040:<pubkey>:<d_tag>" → layer id | SOURCE_ID. */
	byPublication: Record<string, string>;
	/** Markup to assume for a section that declares none; null = source. */
	untagged: CanonicalMimetype | null;
	/** Hide markup sigils (`**`, `## `, `[[ ]]`, …) as the composer's
	 *  Rendered view does; false = show them dimmed. */
	hideMarkers: boolean;
};

export function layerDefaults(): LayerPrefs {
	return {
		enabled: [...ALL_BASE_IDS],
		byMimetype: {},
		byPublication: {},
		untagged: null,
		hideMarkers: false
	};
}
export const LAYER_DEFAULTS: LayerPrefs = layerDefaults();

const PUB_COORD = /^30040:[0-9a-f]{64}:/;

function bindingValue(v: unknown): string | null {
	return v === SOURCE_ID || isLayerId(v) ? (v as string) : null;
}

/** Validate a stored object. `null` (nothing stored) → the defaults; any
 *  stored object → exactly what it says, unknown ids dropped. */
function normalize(p: Partial<LayerPrefs> | null | undefined): LayerPrefs {
	if (!p || typeof p !== 'object') return layerDefaults();
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
	const untagged = isCanonicalMimetype(p.untagged) ? p.untagged : null;
	const hideMarkers = p.hideMarkers === true;
	return { enabled: [...new Set(enabled)], byMimetype, byPublication, untagged, hideMarkers };
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

/** The markup to assume for sections that declare none (`null` = source). */
export function setUntaggedFormat(mimetype: string | null) {
	const next = isCanonicalMimetype(mimetype) ? mimetype : null;
	if (layers.untagged === next) return;
	layers.untagged = next;
	persist();
}

/** Show or hide markup sigils in the reader. */
export function setHideMarkers(on: boolean) {
	if (layers.hideMarkers === on) return;
	layers.hideMarkers = on;
	persist();
}

export function resetLayers() {
	Object.assign(layers, layerDefaults());
	persist();
}
