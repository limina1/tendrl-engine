// Per-author colors for discussion rendering.
//
// Highlight overlays, drawer swatches, and comment border-rails all
// take their hue from the event's pubkey so the same author is
// visually consistent everywhere they appear. Inline marks, drawer
// swatches, and stripe indicators must all compute identical hues —
// that's why this is a pure deterministic function, named verbatim
// from the discussions plan (§6).

/**
 * Map a hex pubkey to a stable hue in [0, 360).
 *
 * First 6 hex chars (24 bits) modulo 360. Matches Alexandria's
 * `pubkeyToHue` so a user moving between clients sees the same author
 * in the same color.
 */
export function pubkeyToHue(pubkey: string): number {
	if (!pubkey || pubkey.length < 6) return 0;
	const n = parseInt(pubkey.slice(0, 6), 16);
	if (Number.isNaN(n)) return 0;
	return n % 360;
}

/**
 * General-purpose color for an author. Used for comment border rails
 * and small author dots.
 */
export function pubkeyToColor(pubkey: string, alpha = 1): string {
	return `hsla(${pubkeyToHue(pubkey)}, 65%, 55%, ${alpha})`;
}

/**
 * Soft background tint suitable for a highlight `<mark>` over body
 * text. Recipe from the plan doc: 70% sat / 60% light / 0.22 alpha
 * keeps the underlying content legible while the hue stays readable
 * across the dark and light themes.
 */
export function pubkeyToHighlightFill(pubkey: string): string {
	return `hsla(${pubkeyToHue(pubkey)}, 70%, 60%, 0.22)`;
}

/**
 * Stronger color for the inset stripe inside a highlight `<mark>` and
 * for the matching stripe in the drawer. Same hue, higher contrast.
 */
export function pubkeyToHighlightStroke(pubkey: string): string {
	return `hsla(${pubkeyToHue(pubkey)}, 70%, 50%, 0.9)`;
}

/**
 * Drawer swatch color — a slightly lighter take used for the small
 * square next to the author row.
 */
export function pubkeyToSwatch(pubkey: string): string {
	return `hsla(${pubkeyToHue(pubkey)}, 70%, 60%, 0.85)`;
}
