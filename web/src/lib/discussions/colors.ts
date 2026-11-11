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

/** `#rrggbb` → `rgba(…, alpha)`. */
export function hexToRgba(hex: string, alpha: number): string {
	const h = hex.replace('#', '');
	const n = (i: number) => parseInt(h.slice(i, i + 2), 16) || 0;
	return `rgba(${n(0)}, ${n(2)}, ${n(4)}, ${alpha})`;
}

/** The standard highlighter colours offered first (contract: ColorPicker). */
export const STANDARD_HIGHLIGHT_COLORS: { name: string; value: string }[] = [
	{ name: 'Yellow', value: '#f7d94c' },
	{ name: 'Green', value: '#8fd694' },
	{ name: 'Blue', value: '#7fb7f0' },
	{ name: 'Pink', value: '#f49ac2' },
	{ name: 'Orange', value: '#f7a35c' },
	{ name: 'Purple', value: '#b79cf2' }
];

/** A stable colour for a label nobody coloured yet: a hash into the
 *  standard set, so the same label lands on the same colour everywhere. */
export function labelToColor(label: string): string {
	let x = 0;
	for (const c of label.trim().toLowerCase()) x = (Math.imul(x, 31) + c.charCodeAt(0)) >>> 0;
	return STANDARD_HIGHLIGHT_COLORS[x % STANDARD_HIGHLIGHT_COLORS.length].value;
}

/** A highlight layer's fill at `alpha` (0..1): its highlighter colour, else
 *  the author hue. */
export function highlightFill(
	layer: { pubkey: string; highlighter?: { color: string } },
	alpha: number
): string {
	return layer.highlighter
		? hexToRgba(layer.highlighter.color, alpha)
		: `hsla(${pubkeyToHue(layer.pubkey)}, 70%, 60%, ${alpha})`;
}

/** The matching stripe colour (opaque). */
export function highlightStroke(layer: { pubkey: string; highlighter?: { color: string } }): string {
	return layer.highlighter ? layer.highlighter.color : pubkeyToHighlightStroke(layer.pubkey);
}

/** Inline CSS showing a highlighter's marks on a short sample (chips,
 *  drawer labels): fill tint, underline, bold, italic — all in its ink. */
export function markSampleStyle(h: { color: string; marks?: string[] }): string {
	const m = h.marks?.length ? h.marks : ['fill'];
	let css = '';
	if (m.includes('fill')) css += `background: ${hexToRgba(h.color, 0.45)};`;
	if (m.includes('underline'))
		css += ` text-decoration: underline 2px ${h.color}; text-underline-offset: 2px;`;
	if (m.includes('bold')) css += ' font-weight: 700;';
	if (m.includes('italic')) css += ' font-style: italic;';
	if (!m.includes('fill')) css += ` color: ${h.color};`;
	return css;
}
