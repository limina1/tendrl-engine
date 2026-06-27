// UI text-size control. Every font size in the app is a rem anchored to the
// single `:root { font-size }` in tokens.css; that anchor is multiplied by the
// `--text-scale` custom property, so writing one number here rescales the whole
// UI as a unit (type, spacing, and chrome alike). This is a per-device *display*
// preference — what's comfortable on a laptop differs from an external monitor —
// so it lives in localStorage on the client, NOT in the engine's config.toml
// (it intentionally does not sync across machines, and never touches Rust).
//
// No-flash: an inline script in app.html applies the saved factor to
// documentElement before first paint; this module is the reactive source of
// truth the Settings UI binds to and re-applies on change.

import { browser } from '$app/environment';

const STORAGE_KEY = 'tendrl.textScale';

export type TextScaleId = 'compact' | 'default' | 'large' | 'xlarge';

/** Preset steps. `factor` multiplies the 13px root grid, so the px column is
 *  the resolved body size (`--t-base`). Order = display order in Settings.
 *  `default`'s factor must match the --text-scale fallback in tokens.css. */
export const TEXT_SCALE_PRESETS: {
	id: TextScaleId;
	label: string;
	factor: number;
	px: number;
}[] = [
	{ id: 'compact', label: 'Compact', factor: 1.0, px: 13 },
	{ id: 'default', label: 'Default', factor: 1.08, px: 14 },
	{ id: 'large', label: 'Large', factor: 1.23, px: 16 },
	{ id: 'xlarge', label: 'Extra large', factor: 1.38, px: 18 }
];

const DEFAULT_ID: TextScaleId = 'default';

function presetFor(id: TextScaleId) {
	return TEXT_SCALE_PRESETS.find((p) => p.id === id) ?? TEXT_SCALE_PRESETS[1];
}

/** Read the persisted preset id, falling back to the shipped default. Shared
 *  with the inline no-flash script's logic (kept in sync by hand — see
 *  app.html). */
function storedId(): TextScaleId {
	if (!browser) return DEFAULT_ID;
	const raw = localStorage.getItem(STORAGE_KEY);
	return TEXT_SCALE_PRESETS.some((p) => p.id === raw) ? (raw as TextScaleId) : DEFAULT_ID;
}

/** Live, app-wide text scale. Read by the Settings control; written by
 *  `setTextScale`. Seeded synchronously from localStorage at module load so the
 *  Settings UI shows the active preset without a flash of the default. */
export const textScale = $state<{ id: TextScaleId }>({ id: storedId() });

/** Push the current factor onto documentElement so the CSS `calc()` picks it
 *  up. The inline app.html script does this first for no-flash; this keeps it
 *  current on every change. */
function applyTextScale(id: TextScaleId) {
	if (!browser) return;
	document.documentElement.style.setProperty('--text-scale', String(presetFor(id).factor));
}

/** Change the active preset: update state, persist, and re-apply live. */
export function setTextScale(id: TextScaleId) {
	if (!TEXT_SCALE_PRESETS.some((p) => p.id === id)) return;
	textScale.id = id;
	if (browser) {
		try {
			localStorage.setItem(STORAGE_KEY, id);
		} catch {
			// Storage full / disabled — the in-memory + DOM apply still hold for
			// this session.
		}
	}
	applyTextScale(id);
}

// Self-apply at module load. The inline script already set the var pre-paint,
// but this re-asserts it (and covers a dev/HMR reload where the inline script
// may not have run) and keeps documentElement in lockstep with `textScale`.
applyTextScale(textScale.id);
