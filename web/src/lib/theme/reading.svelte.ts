// Reading typography — the presentation layer for plain text.
//
// tendrl stores section bodies as plain text (no markup library, by design), so
// everything that makes a document *readable* has to come from typography
// rather than from parsed structure: a measured column, a reading face, open
// leading, and a size the reader picks. These knobs are emitted as CSS custom
// properties (`--rd-*`) onto the reading containers (continuous view, pager,
// doc buffer); `RichContent` consumes them with fallbacks, so every non-reading
// surface that renders a body (compose preview, outline peek) is untouched.
//
// Deliberately NOT a content transform: the reader body stays one `pre-wrap`
// run whose DOM offsets map 1:1 onto the event's UTF-16 content — that mapping
// is what NIP-84 highlight capture and nostrdown span overlays ride on. Any
// prettification that rewrites text (reflowing hard wraps, splitting paragraphs
// into <p>) would break it, so this layer is CSS only.
//
// Like the UI text scale, this is a per-device *display* preference: it lives
// in localStorage, never in the engine config, and never touches Rust.

import { browser } from '$app/environment';
import { textScale, ROOT_PX } from './text-scale.svelte';

const STORAGE_KEY = 'tendrl.reading';

export type ReadingFontId = 'sans' | 'serif' | 'mono' | 'custom';

/** Reading faces. Stacks only — no webfont is fetched (the app runs offline,
 *  and a local-first reader shouldn't phone home for type). `custom` carries a
 *  user-typed family list for whatever is installed on the machine. */
export const READING_FONTS: { id: ReadingFontId; label: string; stack: string }[] = [
	{ id: 'serif', label: 'Serif', stack: 'var(--font-serif)' },
	{ id: 'sans', label: 'Sans', stack: 'var(--font-sans)' },
	{ id: 'mono', label: 'Mono', stack: 'var(--font-mono)' },
	{ id: 'custom', label: 'Custom', stack: '' }
];

// Size is a step count, not a preset: each step is +7.5% of the body size, so
// the same control lands sensibly on a phone, a laptop, and a 4K monitor —
// which is exactly what a fixed four-preset ladder can't do.
export const SIZE_STEP = 0.075;
export const SIZE_MIN = -3;
export const SIZE_MAX = 12;

// Measure in `ch` of the reading face. 0 = full width (no column).
export const MEASURE_MIN = 40;
export const MEASURE_MAX = 120;
export const MEASURE_STEP = 4;

export const LEADING_MIN = 1.2;
export const LEADING_MAX = 2.2;
export const LEADING_STEP = 0.05;

export type ReadingPrefs = {
	font: ReadingFontId;
	/** Family list used when `font === 'custom'`. */
	custom: string;
	/** Step offset from the body size; see SIZE_STEP. */
	size: number;
	/** Column width in `ch`; 0 = full width. */
	measure: number;
	leading: number;
	justify: boolean;
};

export const READING_DEFAULTS: ReadingPrefs = {
	font: 'serif',
	custom: '',
	size: 0,
	measure: 68,
	leading: 1.6,
	justify: false
};

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

/** A custom family list goes straight into a `style` attribute, so keep it to
 *  the characters a font-family value can legitimately contain. */
function sanitizeFamily(raw: string): string {
	return raw
		.replace(/[^\w\s,'"-]/g, '')
		.trim()
		.slice(0, 120);
}

function normalize(p: Partial<ReadingPrefs> | null | undefined): ReadingPrefs {
	const d = READING_DEFAULTS;
	if (!p || typeof p !== 'object') return { ...d };
	const font = READING_FONTS.some((f) => f.id === p.font) ? (p.font as ReadingFontId) : d.font;
	const measure = typeof p.measure === 'number' && p.measure > 0
		? clamp(Math.round(p.measure), MEASURE_MIN, MEASURE_MAX)
		: p.measure === 0
			? 0
			: d.measure;
	return {
		font,
		custom: typeof p.custom === 'string' ? sanitizeFamily(p.custom) : d.custom,
		size: typeof p.size === 'number' ? clamp(Math.round(p.size), SIZE_MIN, SIZE_MAX) : d.size,
		measure,
		leading:
			typeof p.leading === 'number' ? clamp(p.leading, LEADING_MIN, LEADING_MAX) : d.leading,
		justify: !!p.justify
	};
}

function stored(): ReadingPrefs {
	if (!browser) return { ...READING_DEFAULTS };
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		return normalize(raw ? JSON.parse(raw) : null);
	} catch {
		return { ...READING_DEFAULTS };
	}
}

/** Live reading preferences. Read by `readingVars()` (and the controls UI);
 *  written by the setters below. Seeded synchronously at module load. */
export const reading = $state<ReadingPrefs>(stored());

function persist() {
	if (!browser) return;
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify($state.snapshot(reading)));
	} catch {
		// Storage full / disabled — the in-memory prefs still hold this session.
	}
}

export function setReadingFont(id: ReadingFontId) {
	if (!READING_FONTS.some((f) => f.id === id)) return;
	reading.font = id;
	persist();
}

/** Set (and switch to) a custom family list, e.g. `"ETBembo", Georgia, serif`.
 *  An empty string falls back to the serif stack. */
export function setReadingCustomFont(family: string) {
	const clean = sanitizeFamily(family);
	reading.custom = clean;
	reading.font = clean ? 'custom' : 'serif';
	persist();
}

export function stepReadingSize(delta: number) {
	reading.size = clamp(reading.size + delta, SIZE_MIN, SIZE_MAX);
	persist();
}

/** Widen (+1) / narrow (−1) the column. One step past the widest setting is
 *  full width; stepping back down re-enters at the widest column. */
export function stepReadingMeasure(delta: number) {
	if (reading.measure === 0) {
		reading.measure = delta < 0 ? MEASURE_MAX : 0;
	} else {
		const next = reading.measure + delta * MEASURE_STEP;
		reading.measure = next > MEASURE_MAX ? 0 : clamp(next, MEASURE_MIN, MEASURE_MAX);
	}
	persist();
}

export function stepReadingLeading(delta: number) {
	// Round through hundredths — 0.05 steps accumulate float dust otherwise.
	const next = Math.round((reading.leading + delta * LEADING_STEP) * 100) / 100;
	reading.leading = clamp(next, LEADING_MIN, LEADING_MAX);
	persist();
}

export function setReadingJustify(on: boolean) {
	reading.justify = on;
	persist();
}

export function resetReading() {
	Object.assign(reading, READING_DEFAULTS);
	persist();
}

/** The resolved family list for the active face. */
export function readingFamily(): string {
	if (reading.font === 'custom' && reading.custom) return reading.custom;
	return READING_FONTS.find((f) => f.id === reading.font)?.stack ?? 'var(--font-sans)';
}

export function readingSizeFactor(): number {
	return 1 + reading.size * SIZE_STEP;
}

/** Approximate rendered body size in px — the UI scale's rem anchor times the
 *  reading step. Shown on the size stepper so the number means something. */
export function readingSizePx(): number {
	return Math.round(ROOT_PX * textScale.factor * readingSizeFactor());
}

/** The `--rd-*` custom properties for a reading container's `style` attribute.
 *  Call it inside a template so the vars track the prefs reactively. */
export function readingVars(): string {
	return [
		`--rd-font:${readingFamily()}`,
		`--rd-size:calc(var(--t-base) * ${readingSizeFactor().toFixed(3)})`,
		// Headings track the body size (a heading smaller than its own body text
		// reads as a caption) — set here so surfaces outside a reading container
		// keep their UI-chrome heading size via the fallback.
		`--rd-title:calc(var(--t-base) * ${(readingSizeFactor() * 1.12).toFixed(3)})`,
		`--rd-measure:${reading.measure ? `${reading.measure}ch` : 'none'}`,
		`--rd-leading:${reading.leading.toFixed(2)}`,
		`--rd-align:${reading.justify ? 'justify' : 'start'}`,
		`--rd-hyphens:${reading.justify ? 'auto' : 'manual'}`
	].join(';');
}
