// Reader helpers for AsciiDoc footnotes, cross-references and anchors.
//
// The AsciiDoc base (layers/bases/asciidoc.ts) emits the style spans —
// `fn` / `fnbody` / `fnname`, `xref` / `xref-id` / `xref-text`, `anchor` —
// and the engine ships each section's explicit anchors (`[#id]`,
// `[id=…]`, `anchor:id[]`) on the publication stream with UTF-8 *byte*
// offsets (src/epub_import/anchors.rs). Everything here turns those into
// what RichContent / ReaderBuffer need: footnote numbers, the id under a
// click, UTF-16 anchor offsets, the grid slots of a merged-cell table, and
// (the one DOM part) revealing an anchor inside a rendered section.
//
// Leaf module: imports nothing from wm/, state, or components.

import type { StyleSpan } from '$lib/layers/registry';

/** An explicit anchor: `start` is a UTF-8 byte offset (engine) or a UTF-16
 *  index (after `anchorsToUtf16`) — the type says which by where it's used. */
export type AnchorEntry = { id: string; start: number };

/** A section as the reader holds it, plus the engine's anchor list (leaf
 *  stream events carry `anchors`; LazySection doesn't declare it yet). */
export type WithAnchors<T> = T & { anchors?: AnchorEntry[] };

// ── Offsets ──────────────────────────────────────────────────────────────

/** UTF-8 byte offsets into `s` → UTF-16 indices (same order as given). A
 *  byte offset inside a multi-byte character maps to that character's
 *  start; past the end maps to `s.length`. */
export function utf8ToUtf16(s: string, bytes: number[]): number[] {
	const order = bytes.map((b, i) => [b, i] as const).sort((a, b) => a[0] - b[0]);
	const out = new Array<number>(bytes.length);
	let byte = 0;
	let i = 0;
	let k = 0;
	while (k < order.length) {
		const [target, slot] = order[k];
		if (i >= s.length || byte >= target) {
			out[slot] = i >= s.length && byte < target ? s.length : i;
			k++;
			continue;
		}
		const cp = s.codePointAt(i)!;
		const len = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
		if (byte + len > target) {
			// Inside this character: its start.
			out[slot] = i;
			k++;
			continue;
		}
		byte += len;
		i += cp >= 0x10000 ? 2 : 1;
	}
	return out;
}

/** The engine's anchors (byte offsets) re-based to UTF-16 indices. */
export function anchorsToUtf16(content: string, anchors: AnchorEntry[]): AnchorEntry[] {
	const idx = utf8ToUtf16(
		content,
		anchors.map((a) => a.start)
	);
	return anchors.map((a, i) => ({ id: a.id, start: idx[i] }));
}

// ── Anchors from the base's spans (fallback) ─────────────────────────────

const INLINE_ANCHOR = /^anchor:([\p{L}_:][\p{L}\p{N}_:.-]*)\[/u;
const ATTR_LINE = /^\[([^\]\n]*)\]/;

/** Anchors read off the base's `anchor` spans — used when the engine's list
 *  isn't at hand (a section loaded outside the stream, a draft preview).
 *  UTF-16 offsets. A `[#id]` line anchors the start of the next line. */
export function anchorsFromStyles(content: string, styles: StyleSpan[]): AnchorEntry[] {
	const out: AnchorEntry[] = [];
	const seen = new Set<string>();
	for (const s of styles) {
		if (s.cls !== 'anchor') continue;
		const key = `${s.start}:${s.end}`;
		if (seen.has(key)) continue;
		seen.add(key);
		const text = content.slice(s.start, s.end);
		const inline = INLINE_ANCHOR.exec(text);
		if (inline) {
			out.push({ id: inline[1], start: s.start });
			continue;
		}
		const attr = ATTR_LINE.exec(text);
		if (!attr) continue;
		const nl = content.indexOf('\n', s.start);
		const next = nl < 0 ? content.length : nl + 1;
		for (const id of attrIds(attr[1])) out.push({ id, start: next });
	}
	return out.sort((a, b) => a.start - b.start);
}

function attrIds(inner: string): string[] {
	const ids: string[] = [];
	inner.split(',').forEach((raw, i) => {
		const a = raw.trim();
		const eq = a.indexOf('=');
		if (eq >= 0) {
			if (a.slice(0, eq).trim() === 'id') ids.push(a.slice(eq + 1).trim().replace(/^["']|["']$/g, ''));
		} else if (i === 0) {
			const m = /#([^.%#]+)/.exec(a);
			if (m) ids.push(m[1]);
		}
	});
	return ids.filter(Boolean);
}

/** Ids anchored inside `[start, end)` (space-separated, for
 *  `data-anchor-id`), or undefined. `anchors` sorted by start. */
export function anchorIdsIn(anchors: AnchorEntry[], start: number, end: number): string | undefined {
	let out: string | undefined;
	for (const a of anchors) {
		if (a.start >= end) break;
		if (a.start >= start) out = out ? `${out} ${a.id}` : a.id;
	}
	return out;
}

// ── Footnotes ────────────────────────────────────────────────────────────

export type Footnote = {
	/** The whole macro, UTF-16. */
	start: number;
	end: number;
	/** Number shown — per section, in source order. */
	n: number;
	name?: string;
	/** Range of the note's text (for a reuse, the definition's). */
	bodyStart?: number;
	bodyEnd?: number;
	/** `footnote:name[]` pointing back at an earlier `footnote:name[text]`. */
	reuse: boolean;
};

function inside(spans: StyleSpan[], cls: string, start: number, end: number): StyleSpan | undefined {
	return spans.find((s) => s.cls === cls && s.start >= start && s.end <= end);
}

/** Number the section's footnotes: each `fn` span gets the next number,
 *  except a named reuse (`footnote:name[]`), which shows the number its
 *  earlier definition got. A footnote inside a footnote isn't counted. */
export function footnoteMarks(content: string, styles: StyleSpan[]): Footnote[] {
	const fns = styles
		.filter((s) => s.cls === 'fn' && s.end > s.start)
		.sort((a, b) => a.start - b.start || b.end - a.end);
	const out: Footnote[] = [];
	const byName = new Map<string, Footnote>();
	let n = 0;
	let coveredTo = -1;
	for (const f of fns) {
		if (f.start < coveredTo) continue; // nested or duplicate
		coveredTo = f.end;
		const body = inside(styles, 'fnbody', f.start, f.end);
		const nameSpan = inside(styles, 'fnname', f.start, f.end);
		const name = nameSpan ? content.slice(nameSpan.start, nameSpan.end).trim() : undefined;
		const hasBody = !!body && body.end > body.start;
		const def = name ? byName.get(name) : undefined;
		if (name && !hasBody && def) {
			out.push({
				start: f.start,
				end: f.end,
				n: def.n,
				name,
				bodyStart: def.bodyStart,
				bodyEnd: def.bodyEnd,
				reuse: true
			});
			continue;
		}
		const note: Footnote = {
			start: f.start,
			end: f.end,
			n: ++n,
			name,
			bodyStart: hasBody ? body!.start : undefined,
			bodyEnd: hasBody ? body!.end : undefined,
			reuse: false
		};
		if (name && !byName.has(name)) byName.set(name, note);
		out.push(note);
	}
	return out;
}

/** The note's text and its style spans re-based to it (for a popover that
 *  renders the body on its own). Spans crossing the edge are clipped. */
export function footnoteBody(
	content: string,
	styles: StyleSpan[],
	fn: Footnote
): { text: string; styles: StyleSpan[] } {
	if (fn.bodyStart === undefined || fn.bodyEnd === undefined) return { text: '', styles: [] };
	const a = fn.bodyStart;
	const b = fn.bodyEnd;
	const sub: StyleSpan[] = [];
	for (const s of styles) {
		const start = Math.max(s.start, a);
		const end = Math.min(s.end, b);
		if (end > start) sub.push({ start: start - a, end: end - a, cls: s.cls });
	}
	return { text: content.slice(a, b), styles: sub };
}

/** Rendered-view extra `hide` spans: a footnote's whole macro (its number
 *  stands in for it) and an xref's id when it has link text. The text stays
 *  in the DOM with its offsets; only drawing changes. */
export function readerHiddenSpans(styles: StyleSpan[]): StyleSpan[] {
	const out: StyleSpan[] = [];
	for (const s of styles) {
		if (s.cls === 'fn') out.push({ start: s.start, end: s.end, cls: 'hide' });
		else if (s.cls === 'xref') {
			const text = inside(styles, 'xref-text', s.start, s.end);
			if (!text) continue;
			if (text.start > s.start) out.push({ start: s.start, end: text.start, cls: 'hide' });
			if (s.end > text.end) out.push({ start: text.end, end: s.end, cls: 'hide' });
		}
	}
	return out;
}

// ── Cross-references ─────────────────────────────────────────────────────

/** The cross-reference covering `offset`: its id (trimmed) and range. */
export function xrefAt(
	content: string,
	styles: StyleSpan[],
	offset: number
): { id: string; start: number; end: number } | null {
	let best: StyleSpan | undefined;
	for (const s of styles) {
		if (s.cls !== 'xref' || offset < s.start || offset >= s.end) continue;
		if (!best || s.end - s.start < best.end - best.start) best = s;
	}
	if (!best) return null;
	const idSpan = inside(styles, 'xref-id', best.start, best.end);
	const id = idSpan ? content.slice(idSpan.start, idSpan.end).trim() : '';
	return id ? { id, start: best.start, end: best.end } : null;
}

/** The section of the publication holding anchor `id` (engine anchors). */
export function findAnchor<T>(
	sections: WithAnchors<T>[],
	id: string
): { index: number; start: number } | null {
	for (let i = 0; i < sections.length; i++) {
		const hit = sections[i].anchors?.find((a) => a.id === id);
		if (hit) return { index: i, start: hit.start };
	}
	return null;
}

// ── Merged table cells ───────────────────────────────────────────────────

/** `TableCell` from layers/table-layout, whose `colspan` / `rowspan` are
 *  optional (absent = 1). Typed loosely so a cell without them fits. */
type SpanCell = { colspan?: number; rowspan?: number };

/** Grid slot per rendered cell: the row's cells (with their spans) and the
 *  empty cells that pad the row to `cols` once columns taken by a rowspan
 *  from above are subtracted. `cell` is the index into `row.cells`, or -1
 *  for padding. */
export function gridSlots(
	rows: { cells: object[] }[],
	cols: number
): { cell: number; colspan: number; rowspan: number }[][] {
	// Columns still occupied in each upcoming row by a rowspan above.
	const carry: number[] = new Array(rows.length).fill(0);
	return rows.map((row, ri) => {
		const slots: { cell: number; colspan: number; rowspan: number }[] = [];
		let used = carry[ri];
		(row.cells as SpanCell[]).forEach((c, ci) => {
			const colspan = Math.max(1, Math.floor(c.colspan ?? 1));
			const rowspan = Math.max(1, Math.min(Math.floor(c.rowspan ?? 1), rows.length - ri));
			slots.push({ cell: ci, colspan, rowspan });
			used += colspan;
			for (let r = 1; r < rowspan; r++) carry[ri + r] += colspan;
		});
		for (; used < cols; used++) slots.push({ cell: -1, colspan: 1, rowspan: 1 });
		return slots;
	});
}

/** Inline style for a grid cell from the optional AsciiDoc cell fields
 *  (`align`, `valign`, `style` letter) — undefined when it has none. */
export function cellStyle(cell: object | undefined): string | undefined {
	if (!cell) return undefined;
	const c = cell as { align?: string; valign?: string; style?: string };
	let css = '';
	if (c.align === 'left' || c.align === 'center' || c.align === 'right') css += `text-align:${c.align};`;
	if (c.valign === 'top' || c.valign === 'middle' || c.valign === 'bottom') css += `vertical-align:${c.valign};`;
	switch (c.style) {
		case 'e':
			css += 'font-style:italic;';
			break;
		case 's':
		case 'h':
			css += 'font-weight:650;';
			break;
		case 'm':
		case 'l':
			css += 'font-family:var(--font-mono);';
			break;
	}
	return css || undefined;
}

// ── DOM: reveal an anchor ────────────────────────────────────────────────

const FLASH = 'rd-flash';

function visible(el: Element): boolean {
	return el.getClientRects().length > 0;
}

/** Find the element an anchor lands on inside a rendered section body
 *  (`root` = the RichContent `<pre>`): one stamped `data-anchor-id`, else
 *  the stamped run holding `offset` (UTF-16). A hidden run (a marker, an
 *  attribute line) yields the next drawn one. */
export function anchorElement(root: Element, id: string, offset?: number): HTMLElement | null {
	const stamped = Array.from(root.querySelectorAll<HTMLElement>('[data-src-start]'));
	let at = -1;
	const marked = stamped.findIndex((el) => (el.dataset.anchorId ?? '').split(' ').includes(id));
	if (marked >= 0) at = marked;
	else if (offset !== undefined) {
		for (let i = 0; i < stamped.length; i++) {
			const s = Number(stamped[i].dataset.srcStart);
			if (s <= offset) at = i;
			else break;
		}
	}
	if (at < 0) return null;
	for (let i = at; i < stamped.length; i++) if (visible(stamped[i])) return stamped[i];
	return null;
}

/** Scroll an anchor into view and flash it. False when not found. */
export function revealAnchor(root: Element, id: string, offset?: number): boolean {
	const el = anchorElement(root, id, offset);
	if (!el) return false;
	const reduce = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
	el.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'center' });
	el.classList.remove(FLASH);
	void el.offsetWidth; // restart the animation on a repeat visit
	el.classList.add(FLASH);
	setTimeout(() => el.classList.remove(FLASH), 1600);
	return true;
}
