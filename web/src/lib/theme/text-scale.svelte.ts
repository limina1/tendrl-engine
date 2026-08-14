// UI text-size control. Every font size in the app is a rem anchored to the
// single `:root { font-size }` in tokens.css; that anchor is multiplied by the
// `--text-scale` custom property, so writing one number here rescales the whole
// UI as a unit (type, spacing, and chrome alike). This is a per-device *display*
// preference — what's comfortable on a laptop differs from an external monitor —
// so it lives in localStorage on the client, NOT in the engine's config.toml
// (it intentionally does not sync across machines, and never touches Rust).
//
// The control is a STEPPER, not a preset ladder: four named sizes can't span a
// phone, a laptop and a 4K panel — the same "Large" is cramped on one and huge
// on the next. A ±5% step with a live px readout lets each device land where it
// wants. Legacy preset ids still in localStorage migrate to their factor once.
//
// No-flash: an inline script in app.html applies the saved factor to
// documentElement before first paint; this module is the reactive source of
// truth the Settings UI binds to and re-applies on change.

import { browser } from '$app/environment';

/** Numeric factor (current). */
const STORAGE_KEY = 'tendrl.textScaleFactor';
/** Preset id (pre-stepper). Read once for migration, then cleared. */
const LEGACY_KEY = 'tendrl.textScale';

/** The rem anchor in tokens.css: `font-size: calc(81.25% * var(--text-scale))`
 *  — 81.25% of the 16px browser default. Multiply by the factor for the
 *  resolved body size in px. */
export const ROOT_PX = 13;

export const SCALE_MIN = 0.8;
export const SCALE_MAX = 2.0;
export const SCALE_STEP = 0.05;
/** Must match the `--text-scale` fallback in tokens.css and app.html. */
export const SCALE_DEFAULT = 1.08;

/** Factors the retired presets resolved to, kept for one-way migration of
 *  settings saved before the stepper landed. */
const LEGACY_FACTORS: Record<string, number> = {
	compact: 1.0,
	default: 1.08,
	large: 1.23,
	xlarge: 1.38
};

const clamp = (n: number) => Math.min(SCALE_MAX, Math.max(SCALE_MIN, n));
/** Snap to the step grid so the readout and the ± buttons stay in lockstep. */
const snap = (n: number) => Math.round(clamp(n) / SCALE_STEP) * SCALE_STEP;

function storedFactor(): number {
	if (!browser) return SCALE_DEFAULT;
	try {
		const raw = parseFloat(localStorage.getItem(STORAGE_KEY) ?? '');
		if (raw > 0) return snap(raw);
		const legacy = LEGACY_FACTORS[localStorage.getItem(LEGACY_KEY) ?? ''];
		if (legacy) return snap(legacy);
	} catch {
		// no localStorage — stylesheet default applies
	}
	return SCALE_DEFAULT;
}

/** Live, app-wide text scale. Read by the Settings control and by the reading
 *  layer's px readout; written by `setTextScale` / `stepTextScale`. */
export const textScale = $state<{ factor: number }>({ factor: storedFactor() });

/** Resolved body size in px at the current factor (the number on the stepper). */
export function textScalePx(): number {
	return Math.round(ROOT_PX * textScale.factor);
}

/** Push the current factor onto documentElement so the CSS `calc()` picks it
 *  up. The inline app.html script does this first for no-flash; this keeps it
 *  current on every change. */
function applyTextScale(factor: number) {
	if (!browser) return;
	document.documentElement.style.setProperty('--text-scale', String(factor));
}

export function setTextScale(factor: number) {
	if (!(factor > 0)) return;
	textScale.factor = snap(factor);
	if (browser) {
		try {
			localStorage.setItem(STORAGE_KEY, String(textScale.factor));
			localStorage.removeItem(LEGACY_KEY);
		} catch {
			// Storage full / disabled — the in-memory + DOM apply still hold for
			// this session.
		}
	}
	applyTextScale(textScale.factor);
}

/** One step up (+1) or down (−1). */
export function stepTextScale(delta: number) {
	setTextScale(textScale.factor + delta * SCALE_STEP);
}

export function resetTextScale() {
	setTextScale(SCALE_DEFAULT);
}

export const atScaleMin = () => textScale.factor <= SCALE_MIN + 1e-6;
export const atScaleMax = () => textScale.factor >= SCALE_MAX - 1e-6;

// Self-apply at module load. The inline script already set the var pre-paint,
// but this re-asserts it (and covers a dev/HMR reload where the inline script
// may not have run) and keeps documentElement in lockstep with `textScale`.
applyTextScale(textScale.factor);
