// AsciiDoc decorated-source base — a hand-written subset → style spans.
//
// Scope is what NKBIP-01 publications actually use (modelled on what
// reference/gc-alexandria renders): section titles, block titles, attribute
// entries and block-attribute lines, delimited blocks (listing / literal /
// passthrough / example / sidebar / quote / open / comment / table),
// lists (bulleted, numbered, description, checklist, callout), admonition
// paragraphs, constrained + unconstrained emphasis, link / xref / image /
// include macros, bare URLs, hard line breaks and the `'''` rule. Nothing is
// rewritten; delimiters and sigils get `mark`.
//
// The EPUB-import classes (docs/epub-import-contract.org): `fn` / `fnbody` /
// `fnname`, `xref` / `xref-id` / `xref-text`, `anchor`, `sup` / `sub`,
// `role` / `smallcaps`, and the block classes `sidebar` / `example` /
// `verse`. Table lines also emit `cellspec` (with `mark`) over a cell spec
// (`2+`, `.2+a`…) so `tableLayout` can read spans and keep it out of cells.
//
// `[[anchor]]` has the same bytes as the Nostr wikilink nostrdown claims
// (asciidoctor would swallow it as an inline anchor — the case Alexandria
// has to reconstruct afterwards). Here the nostrdown atom always wins:
// `applyStyles` keeps it whole and this base only styles around it.

import type { BaseImpl, StyleSpan } from '../registry';
import {
	findClose,
	isSpace,
	isWordChar,
	Mask,
	maskNostrdown,
	push,
	scanBareUrls,
	splitLines,
	type Line
} from './lines';
import { parseCellSpec, type CellSpec } from '../table-layout';

type BlockKind = 'fence' | 'comment' | 'quote' | 'table' | 'container';

const TITLE = /^(={1,6}|#{1,6})\s+\S/;
const DELIM = /^(-{4,}|\.{4,}|\+{4,}|={4,}|\*{4,}|_{4,}|\/{4,}|`{3,}|-{2}|\|={3,})\s*$/;
const BLOCK_TITLE = /^\.(?![\s.])\S.*$/;
const ATTR_ENTRY = /^:[\w!][\w.-]*!?:(\s|$)/;
const ATTR_LINE = /^\[([^\]]*)\]\s*$/;
const LINE_COMMENT = /^\/\/(?!\/)/;
const ADMON_PARA = /^(NOTE|TIP|IMPORTANT|WARNING|CAUTION):\s/;
const ADMONITIONS = new Set(['NOTE', 'TIP', 'IMPORTANT', 'WARNING', 'CAUTION']);
const LIST = /^(\s*)(\*{1,5}|-|\.{1,5}|\d+\.|[a-zA-Z]\.|[ivxlcIVXLC]+\))\s+/;
const CHECKBOX = /^\[([ x*])\]\s+/;
const CALLOUT_ITEM = /^<\d+>\s+/;
const DESC = /^(\S.*?)(::{1,3})(\s+|$)/;
const HR = /^'{3,}\s*$/;
const PAGE_BREAK = /^<{3}\s*$/;
const HARD_BREAK = /\s\+$/;
const STEM = new Set(['stem', 'latexmath', 'asciimath']);

function delimKind(d: string): BlockKind {
	switch (d[0]) {
		case '-':
			return d.length === 2 ? 'container' : 'fence';
		case '.':
		case '+':
		case '`':
			return 'fence';
		case '/':
			return 'comment';
		case '_':
			return 'quote';
		case '|':
			return 'table';
		default:
			return 'container'; // ==== example, **** sidebar
	}
}

/** Macro targets: `link:`, `image:`/`image::`, `include::`, `footnoteref:`,
 *  `mailto:`, `kbd:`, `btn:`, `menu:` and `scheme://…[text]`. Whole →
 *  `link`; target → `url`; brackets → `mark`. (`footnote:`, `xref:` and
 *  `anchor:` have their own classes — see `scanRefs`.) */
const MACRO =
	/\b(?:(https?|ftp|irc|mailto):([^\s\[]+)|(link|image|include|footnoteref|kbd|btn|menu|icon|pass):{1,2}([^\s\[]*))\[([^\]]*)\]/g;
/** `footnote:[`, `footnote:name[` — the body runs to the first `]` that is
 *  neither escaped (`\]`) nor inside a nostrdown token (`closeBracket`). */
const FOOTNOTE_RE = /(?<!\\)footnote:([\w-]*)\[/g;
/** `xref:id[` — the text closes like a footnote body. */
const XREF_MACRO_RE = /(?<![\w\\])xref:([^\s\[]+)\[/g;
/** `<<id>>` / `<<id,text>>` — the id has no whitespace; the text runs to `>>`. */
const XREF_RE = /(?<!\\)<<([\w#/.:{}-][^\s,>]*)(?:,([^>]*?))?>>/g;
/** `anchor:id[]` / `anchor:id[xreflabel]`. */
const ANCHOR_MACRO_RE = /(?<![\w\\])anchor:([\w:.-]+)\[([^\]]*)\]/g;
/** `[.role]#text#`, `[.a.b]##text##`, `[#id]#text#`, `[#id.role]#text#`. */
const ROLE_OPEN_RE = /\[(#[\w:.-]+?)?((?:\.[\w-]+)*)\](#{1,2})(?=\S)/g;
/** `^sup^` / `~sub~`: no whitespace inside (AsciiDoc's own rule). */
const SUPSUB: [RegExp, string][] = [
	[/(?<!\\)\^([^\s^]+)\^/g, 'sup'],
	[/(?<!\\)~([^\s~]+)~/g, 'sub']
];
const INLINE_ANCHOR = /\[\[([^\]]+)\]\]/g;
const ATTR_REF = /(?<!\{)\{([a-zA-Z0-9_][\w-]*)\}(?!\})/g;
const SMALLCAPS = new Set(['smallcaps', 'small-caps', 'sc']);

// marker, class, unconstrained (doubled) form?
const INLINE: [string, string][] = [
	['`', 'code'],
	['*', 'strong'],
	['_', 'em'],
	['#', 'hl']
];

/** Footnotes, cross references, inline anchors and roles. Their markers
 *  must be free (never inside a nostrdown token); their bodies are scanned
 *  on their own (`scanInline` on the slice), so emphasis, `^n^` or a `{{ }}`
 *  token inside a footnote / xref text / role works and can never pair with
 *  a marker outside. */
/** The `]` closing a macro body opened at `open` (the `[`): not escaped,
 *  not inside a nostrdown token. -1 when the line has none. */
function closeBracket(line: string, open: number, mask: Mask): number {
	for (let j = open + 1; j < line.length; j++) {
		if (line[j] === '\\') j++;
		else if (line[j] === ']' && !mask.taken(j)) return j;
	}
	return -1;
}

function scanRefs(line: string, base: number, mask: Mask, out: StyleSpan[]): void {
	// Local copies: the bodies recurse into `scanInline`, which would reset a
	// shared global regex's `lastIndex` under the outer loop.
	const FOOTNOTE = new RegExp(FOOTNOTE_RE);
	const XREF_MACRO = new RegExp(XREF_MACRO_RE);
	const XREF = new RegExp(XREF_RE);
	const ANCHOR_MACRO = new RegExp(ANCHOR_MACRO_RE);
	const ROLE_OPEN = new RegExp(ROLE_OPEN_RE);
	let m: RegExpExecArray | null;
	FOOTNOTE.lastIndex = 0;
	while ((m = FOOTNOTE.exec(line))) {
		const s = m.index;
		const open = s + m[0].length - 1; // the `[`
		if (!mask.free(s, open + 1)) continue;
		const close = closeBracket(line, open, mask);
		if (close < 0) continue;
		const e = close + 1;
		const body = line.slice(open + 1, close);
		push(out, base + s, base + e, 'fn');
		push(out, base + s, base + open + 1, 'mark'); // `footnote:name[`
		if (m[1]) push(out, base + s + 9, base + s + 9 + m[1].length, 'fnname');
		push(out, base + open + 1, base + e - 1, 'fnbody');
		push(out, base + e - 1, base + e, 'mark');
		mask.take(s, e);
		if (body) scanInline(body, base + open + 1, out, 0, true);
		FOOTNOTE.lastIndex = e;
	}
	XREF_MACRO.lastIndex = 0;
	while ((m = XREF_MACRO.exec(line))) {
		const s = m.index;
		const open = s + m[0].length - 1; // the `[`
		if (!mask.free(s, open + 1)) continue;
		const close = closeBracket(line, open, mask);
		if (close < 0) continue;
		const e = close + 1;
		const body = line.slice(open + 1, close);
		const idAt = s + 5;
		push(out, base + s, base + e, 'xref');
		push(out, base + idAt, base + idAt + m[1].length, 'xref-id');
		if (body) {
			// Labelled: `xref:id[` hides behind the text.
			push(out, base + s, base + open + 1, 'mark');
			push(out, base + open + 1, base + e - 1, 'xref-text');
		} else {
			push(out, base + s, base + idAt, 'mark');
			push(out, base + open, base + open + 1, 'mark');
		}
		push(out, base + e - 1, base + e, 'mark');
		mask.take(s, e);
		if (body) scanInline(body, base + open + 1, out, 0, true);
		XREF_MACRO.lastIndex = e;
	}
	XREF.lastIndex = 0;
	while ((m = XREF.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, s + 2 + m[1].length) || !mask.free(e - 2, e)) continue;
		push(out, base + s, base + e, 'xref');
		push(out, base + s + 2, base + s + 2 + m[1].length, 'xref-id');
		const text = m[2];
		if (text !== undefined && text.trim() !== '') {
			// `<<id,` hides behind the text; the text keeps its own spacing.
			const lead = text.length - text.trimStart().length;
			const tAt = s + 2 + m[1].length + 1 + lead;
			push(out, base + s, base + tAt, 'mark');
			push(out, base + tAt, base + e - 2, 'xref-text');
			scanInline(line.slice(tAt, e - 2), base + tAt, out, 0, true);
		} else {
			push(out, base + s, base + s + 2, 'mark');
			if (text !== undefined) push(out, base + s + 2 + m[1].length, base + e - 2, 'mark');
		}
		push(out, base + e - 2, base + e, 'mark');
		mask.take(s, e);
	}
	ANCHOR_MACRO.lastIndex = 0;
	while ((m = ANCHOR_MACRO.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		// Renders as nothing (an empty target): the whole macro is a sigil.
		push(out, base + s, base + e, 'anchor');
		push(out, base + s, base + e, 'mark');
		mask.take(s, e);
	}
	ROLE_OPEN.lastIndex = 0;
	while ((m = ROLE_OPEN.exec(line))) {
		const s = m.index;
		const id = m[1];
		const roles = m[2];
		if (!id && !roles) continue;
		const marker = m[3];
		const bodyAt = s + m[0].length;
		if (!mask.free(s, bodyAt)) continue;
		const j = findClose(
			line,
			bodyAt - marker.length,
			marker,
			mask,
			(j) => !isSpace(line[j - 1]) && (marker.length === 2 || !isWordChar(line[j + 1]))
		);
		if (j < 0) continue;
		const e = j + marker.length;
		if (roles) push(out, base + s, base + e, 'role');
		if (roles.split('.').some((r) => SMALLCAPS.has(r))) push(out, base + s, base + e, 'smallcaps');
		if (id) push(out, base + s, base + e, 'anchor');
		push(out, base + s, base + bodyAt, 'mark');
		push(out, base + j, base + e, 'mark');
		mask.take(s, e);
		scanInline(line.slice(bodyAt, j), base + bodyAt, out, 0, true);
		ROLE_OPEN.lastIndex = e;
	}
}

function scanSupSub(line: string, base: number, mask: Mask, out: StyleSpan[]): void {
	for (const [re, cls] of SUPSUB) {
		re.lastIndex = 0;
		let m: RegExpExecArray | null;
		while ((m = re.exec(line))) {
			const s = m.index;
			const e = s + m[0].length;
			if (!mask.free(s, e)) {
				re.lastIndex = s + 1;
				continue;
			}
			push(out, base + s + 1, base + e - 1, cls);
			push(out, base + s, base + s + 1, 'mark');
			push(out, base + e - 1, base + e, 'mark');
			mask.take(s, e);
		}
	}
}

function scanInline(line: string, base: number, out: StyleSpan[], from = 0, nested = false): void {
	const mask = new Mask(line.length);
	mask.take(0, from);
	maskNostrdown(line, mask);

	// References and macros first — their bodies are opaque to emphasis.
	scanRefs(line, base, mask, out);
	let m: RegExpExecArray | null;
	MACRO.lastIndex = 0;
	while ((m = MACRO.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		const target = m[2] ?? m[4] ?? '';
		const targetAt = s + m[0].indexOf(target, m[1] ? m[1].length : (m[3]?.length ?? 0));
		push(out, base + s, base + e, 'link');
		if (target) push(out, base + targetAt, base + targetAt + target.length, 'url');
		// A labelled link reads as its label: the scheme / macro name in front
		// of the target (`https:`, `link:`, `xref:`) is a marker too, so the
		// rendered view hides it along with the target.
		if (target && m[5].trim() !== '' && targetAt > s) push(out, base + s, base + targetAt, 'mark');
		const br = e - m[5].length - 2;
		push(out, base + br, base + br + 1, 'mark');
		push(out, base + e - 1, base + e, 'mark');
		mask.take(s, e);
	}
	// `{empty}` renders as nothing (converters put it in front of text that
	// would otherwise open a block) — a marker.
	const EMPTY_ATTR = /\{empty\}/g;
	while ((m = EMPTY_ATTR.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		push(out, base + s, base + e, 'mark');
		mask.take(s, e);
	}
	// `[[anchor]]`: styled as a link; nostrdown's atom wins if it is a wikilink.
	INLINE_ANCHOR.lastIndex = 0;
	while ((m = INLINE_ANCHOR.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		push(out, base + s, base + e, 'link');
		push(out, base + s, base + s + 2, 'mark');
		push(out, base + e - 2, base + e, 'mark');
		mask.take(s, e);
	}
	scanBareUrls(line, base, mask, out);
	ATTR_REF.lastIndex = 0;
	while ((m = ATTR_REF.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		push(out, base + s, base + e, 'kw');
		mask.take(s, e);
	}
	// Monospace before sup/sub, so `` `a^b^` `` stays code.
	scanMarker(line, base, mask, out, '``', 'code', false);
	scanMarker(line, base, mask, out, '`', 'code', true);
	scanSupSub(line, base, mask, out);

	for (const [ch, cls] of INLINE) {
		if (cls === 'code') continue;
		// Unconstrained (`**x**`) first, then constrained (`*x*`).
		scanMarker(line, base, mask, out, ch + ch, cls, false);
		scanMarker(line, base, mask, out, ch, cls, true);
	}

	if (nested) return;
	const hb = HARD_BREAK.exec(line);
	if (hb && mask.free(line.length - 1, line.length)) {
		push(out, base + line.length - 1, base + line.length, 'mark');
	}
}

function scanMarker(
	line: string,
	base: number,
	mask: Mask,
	out: StyleSpan[],
	marker: string,
	cls: string,
	constrained: boolean
): void {
	const mlen = marker.length;
	let i = 0;
	for (;;) {
		i = line.indexOf(marker, i);
		if (i < 0) return;
		const openOk =
			mask.free(i, i + mlen) &&
			!isSpace(line[i + mlen]) &&
			line[i + mlen] !== marker[0] &&
			(!constrained || !isWordChar(line[i - 1]));
		if (!openOk) {
			i += 1;
			continue;
		}
		const j = findClose(
			line,
			i,
			marker,
			mask,
			(j) => !isSpace(line[j - 1]) && (!constrained || !isWordChar(line[j + mlen]))
		);
		if (j < 0) {
			i += 1;
			continue;
		}
		push(out, base + i, base + j + mlen, cls);
		push(out, base + i, base + i + mlen, 'mark');
		push(out, base + j, base + j + mlen, 'mark');
		if (cls === 'code') mask.take(i, j + mlen);
		else {
			mask.take(i, i + mlen);
			mask.take(j, j + mlen);
		}
		i = j + mlen;
	}
}

/** `[#id]`, `[#id.role]`, `[quote#id,…]`, `[id=…]` — the line names an anchor. */
const ATTR_ID = /^[\w-]*(?:[.%][\w-]+)*#[\w:-]|(?:^|,)\s*id\s*=/;

function attrLine(line: Line, inner: string, out: StyleSpan[]): string {
	push(out, line.start, line.end, 'kw');
	if (ATTR_ID.test(inner)) push(out, line.start, line.end, 'anchor');
	const fields = inner.split(',').map((f) => f.trim());
	const name = fields[0] ?? '';
	// `[source,python]` → the language; `[NOTE]` → admonition.
	if (name === 'source' && fields[1]) {
		const at = line.text.indexOf(fields[1], 1 + name.length);
		if (at > 0) push(out, line.start + at, line.start + at + fields[1].length, 'lang');
	} else if (ADMONITIONS.has(name)) {
		push(out, line.start + 1, line.start + 1 + name.length, 'admon');
	} else if (STEM.has(name) || name === 'plantuml' || name === 'graphviz' || name === 'mermaid') {
		push(out, line.start + 1, line.start + 1 + name.length, 'lang');
	}
	return name;
}

/** Column count from a `[cols=…]` attribute: `3*`, `1,2,1`, `2*,3a` (0 =
 *  none given). */
function colsCount(attr: string): number {
	const c = /cols="?([^"\]]+)/.exec(attr)?.[1]?.trim();
	if (!c) return 0;
	let n = 0;
	for (const part of c.split(/[,;]/)) {
		const mult = /^\s*(\d+)\*/.exec(part);
		n += mult ? Number(mult[1]) : 1;
	}
	return n;
}

type Sep = { pipe: number; specStart: number; spec: CellSpec };

/** The cell separators on one table line: every `|` that is not escaped
 *  and not inside a nostrdown token (`{{…}}`, `[[…]]`), a `+…+` passthrough
 *  or a backtick run — plus the cell spec right before it (`2+`, `.2+`,
 *  `2.3+`, `a`, `^.>`, `3*`…), which belongs to the separator: at the start
 *  of the line, or after whitespace (Asciidoctor's rule). */
function tableSeparators(t: string): Sep[] {
	const mask = new Mask(t.length);
	maskNostrdown(t, mask);
	for (const m of t.matchAll(/\[\[[^\]]*\]\]/g)) mask.take(m.index, m.index + m[0].length);
	for (const m of t.matchAll(/\+\+\+.+?\+\+\+|\+\+.+?\+\+/g)) mask.take(m.index, m.index + m[0].length);
	// Constrained `+…+` / `` `…` ``: opener not after a word character, closer
	// not before one (so the `+` of a `2+|` spec never opens a passthrough).
	for (const q of ['+', '`']) {
		for (let i = t.indexOf(q); i >= 0; i = t.indexOf(q, i + 1)) {
			if (mask.taken(i) || isWordChar(t[i - 1]) || isSpace(t[i + 1]) || t[i + 1] === q) continue;
			let j = i + 1;
			for (; (j = t.indexOf(q, j + 1)) >= 0; ) {
				if (!isSpace(t[j - 1]) && !isWordChar(t[j + 1])) break;
			}
			if (j < 0) continue;
			mask.take(i, j + 1);
			i = j;
		}
	}
	const out: Sep[] = [];
	let prevEnd = 0;
	for (let i = 0; i < t.length; i++) {
		if (t[i] !== '|' || mask.taken(i) || t[i - 1] === '\\') continue;
		const before = t.slice(prevEnd, i);
		let specStart = i;
		let spec: CellSpec = { colspan: 1, rowspan: 1 };
		const lead = prevEnd === 0 ? /^\s*(\S+)$/.exec(before) : null;
		const tail = /\s(\S+)$/.exec(before);
		const cand = lead?.[1] ?? tail?.[1];
		const parsed = cand !== undefined ? parseCellSpec(cand) : null;
		if (parsed && cand !== undefined) {
			spec = parsed;
			specStart = i - cand.length;
		}
		out.push({ pipe: i, specStart, spec });
		prevEnd = i + 1;
	}
	return out;
}

/** Group a table's cells into rows of `cols` columns, honouring colspan and
 *  rowspan (a rowspan keeps its columns taken in the rows below). Yields
 *  [row start offset, index of the row's first cell after it]. */
function groupRows(cells: { at: number; cs: number; rs: number }[], cols: number): [number, number][] {
	const rows: [number, number][] = [];
	let carry = new Array<number>(cols).fill(0);
	let k = 0;
	let guard = 0;
	while (k < cells.length && guard++ < cells.length * 4 + cols * 4) {
		const taken = carry.map((c) => c > 0);
		carry = carry.map((c) => (c > 0 ? c - 1 : 0));
		const first = k;
		let col = 0;
		while (k < cells.length) {
			while (col < cols && taken[col]) col++;
			if (col >= cols) break;
			const { cs, rs } = cells[k];
			for (let j = col; j < Math.min(cols, col + cs); j++) {
				taken[j] = true;
				carry[j] = Math.max(carry[j], rs - 1);
			}
			col += cs;
			k++;
		}
		if (k > first) rows.push([cells[first].at, k]);
	}
	return rows;
}

/** Paragraph styles that become a block class on the next paragraph. */
const PARA_STYLES = new Set(['verse', 'sidebar', 'example']);

/** The block class a delimited block adds: `****` sidebar, `====` example
 *  (unless an admonition style makes it an admonition block), `____` /
 *  `--` after `[verse]` → verse. */
function blockClass(delim: string, kind: BlockKind, style: string): string {
	if (style === 'verse' && (delim[0] === '_' || delim === '--')) return 'verse';
	if (kind !== 'container') return '';
	if (delim[0] === '*') return 'sidebar';
	if (delim[0] === '=') return ADMONITIONS.has(style) ? '' : 'example';
	if (delim === '--' && (style === 'sidebar' || style === 'example')) return style;
	return '';
}

function spans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	const lines = splitLines(text);
	// Delimited-block stack: containers (example / sidebar / open / quote)
	// may nest; verbatim ones (listing / literal / passthrough / comment /
	// table) never scan their body.
	const stack: { delim: string; kind: BlockKind; start: number; cls: string }[] = [];
	// A styled paragraph (`[verse]`, `[sidebar]`, `[example]` + a paragraph).
	let para: { cls: string; start: number; end: number } | null = null;
	const flushPara = () => {
		if (para) push(out, para.start, para.end, para.cls);
		para = null;
	};
	// Inside a verse (block or paragraph) a newline is a line break.
	const inVerse = () => para?.cls === 'verse' || stack.some((b) => b.cls === 'verse');
	let pendingAttr = ''; // name from the last `[…]` line, consumed by the next block
	let pendingAttrRaw = ''; // its full text (for `[options="header"]` / `[%header]`)
	let tableHeaderDone = false;
	let tableHeaderByAttr = false;
	// Rows for the table grid: AsciiDoc cells may sit one per line, so a
	// row is `tableCols` cells — from `[cols=…]`, else the first row line.
	let tableCols = 0;
	// Each cell: where it starts (its spec, else its `|`) and its spans.
	let tableCells: { at: number; cs: number; rs: number }[] = [];
	const emitRows = (closeAt: number) => {
		if (tableCols <= 0) return;
		for (const [from, next] of groupRows(tableCells, tableCols)) {
			let to = next < tableCells.length ? tableCells[next].at : closeAt;
			while (to > from && /\s/.test(text[to - 1])) to--;
			if (to > from) push(out, from, to, 'tr');
		}
	};
	// Soft line breaks: a newline inside a paragraph (or after a wrapped
	// list item / admonition) is a space in AsciiDoc, not a line break —
	// unless the line ends with the ` +` hard break. The newline before a
	// continuing line gets `soft` (rendered collapsible; the character and
	// every offset stay).
	let prevJoinable = false;
	// The span covers the newline AND the continuation line's indentation,
	// so a wrapped list item collapses to a single space too.
	const soft = (line: Line) => {
		if (line.start === 0 || inVerse()) return;
		const indent = /^[ \t]*/.exec(line.text)![0].length;
		push(out, line.start - 1, line.start + indent, 'soft');
	};

	for (let n = 0; n < lines.length; n++) {
		const line = lines[n];
		const t = line.text;
		const top = stack[stack.length - 1];
		const joinPrev = prevJoinable;
		prevJoinable = false;

		if (para && (t.trim() === '' || DELIM.test(t) || ATTR_LINE.test(t) || TITLE.test(t))) flushPara();

		const d = DELIM.exec(t);
		if (d) {
			const delim = d[1];
			if (top && top.delim === delim) {
				push(out, line.start, line.end, 'mark');
				if (top.kind !== 'container') push(out, top.start, line.end, top.kind);
				if (top.cls) push(out, top.start, line.end, top.cls);
				if (top.kind === 'table') emitRows(line.start);
				stack.pop();
				pendingAttr = '';
				continue;
			}
			if (!top || top.kind === 'container' || top.kind === 'quote') {
				let kind = delimKind(delim);
				// `[stem]` / `[latexmath]` + `++++` or `--` → maths block = fence.
				if (STEM.has(pendingAttr) && (kind === 'container' || kind === 'fence')) kind = 'fence';
				const cls = blockClass(delim, kind, pendingAttr);
				push(out, line.start, line.end, 'mark');
				if (
					!lines.slice(n + 1).some((l) => DELIM.exec(l.text)?.[1] === delim)
				) {
					// Unterminated: the block runs to the end of the text.
					if (kind !== 'container') push(out, line.start, text.length, kind);
					if (cls) push(out, line.start, text.length, cls);
					if (kind === 'container' || kind === 'quote') {
						for (let k = n + 1; k < lines.length; k++)
							scanInline(lines[k].text, lines[k].start, out);
					}
					break;
				}
				stack.push({ delim, kind, start: line.start, cls });
				tableHeaderDone = false;
				tableHeaderByAttr = /header/.test(pendingAttrRaw);
				tableCells = [];
				tableCols = colsCount(pendingAttrRaw);
				pendingAttr = '';
				pendingAttrRaw = '';
				continue;
			}
			// A different delimiter inside a verbatim block is just text.
		}

		if (top && (top.kind === 'fence' || top.kind === 'comment')) continue;
		if (top && top.kind === 'table') {
			// Rows: `|` separators marked; the first row (before a blank line)
			// is the header.
			if (t.trim() === '') {
				tableHeaderDone = true;
				continue;
			}
			if (!tableHeaderDone && n > 0 && lines[n - 1].text.trim().startsWith('|===')) {
				const blankNext = n + 1 < lines.length && lines[n + 1].text.trim() === '';
				if (blankNext || tableHeaderByAttr) push(out, line.start, line.end, 'th');
				tableHeaderDone = true;
			}
			const seps = tableSeparators(t);
			let rowWidth = 0;
			let textAt = 0; // where the current cell's text starts on this line
			for (const sep of seps) {
				const at = line.start + sep.specStart;
				if (sep.specStart > textAt) scanInline(t.slice(textAt, sep.specStart), line.start + textAt, out, 0, true);
				if (sep.specStart < sep.pipe) {
					push(out, at, line.start + sep.pipe, 'mark');
					push(out, at, line.start + sep.pipe, 'cellspec');
				}
				push(out, line.start + sep.pipe, line.start + sep.pipe + 1, 'mark');
				tableCells.push({ at, cs: sep.spec.colspan, rs: sep.spec.rowspan });
				rowWidth += sep.spec.colspan;
				textAt = sep.pipe + 1;
			}
			if (textAt < t.length) scanInline(t.slice(textAt), line.start + textAt, out, 0, true);
			if (tableCols === 0 && rowWidth > 0) tableCols = rowWidth;
			continue;
		}

		const h = TITLE.exec(t);
		if (h) {
			const level = Math.min(h[1].length, 6);
			push(out, line.start, line.end, `h${level}`);
			push(out, line.start, line.start + h[1].length, 'mark');
			scanInline(t, line.start, out, h[1].length);
			pendingAttr = '';
			continue;
		}
		const a = ATTR_LINE.exec(t);
		if (a) {
			pendingAttr = attrLine(line, a[1], out);
			pendingAttrRaw = a[1];
			continue;
		}
		if (ATTR_ENTRY.test(t)) {
			push(out, line.start, line.end, 'kw');
			continue;
		}
		if (LINE_COMMENT.test(t)) {
			push(out, line.start, line.end, 'comment');
			continue;
		}
		if (BLOCK_TITLE.test(t)) {
			push(out, line.start, line.start + 1, 'mark');
			push(out, line.start, line.end, 'strong');
			continue;
		}
		if (HR.test(t) || PAGE_BREAK.test(t)) {
			push(out, line.start, line.end, HR.test(t) ? 'hr' : 'mark');
			continue;
		}
		if (t.trim() === '') continue;
		if (para) para.end = line.end;
		else if (PARA_STYLES.has(pendingAttr)) {
			para = { cls: pendingAttr, start: line.start, end: line.end };
			pendingAttr = '';
		}
		const hardBreak = HARD_BREAK.test(t);
		const ad = ADMON_PARA.exec(t);
		if (ad) {
			push(out, line.start, line.start + ad[1].length + 1, 'admon');
			scanInline(t, line.start, out, ad[0].length);
			prevJoinable = !hardBreak;
			continue;
		}
		const co = CALLOUT_ITEM.exec(t);
		if (co) {
			push(out, line.start, line.start + co[0].trimEnd().length, 'mark');
			scanInline(t, line.start, out, co[0].length);
			prevJoinable = !hardBreak;
			continue;
		}
		const li = LIST.exec(t);
		if (li) {
			let i = li[0].length;
			push(out, line.start + li[1].length, line.start + li[1].length + li[2].length, 'mark');
			const cb = CHECKBOX.exec(t.slice(i));
			if (cb) {
				push(out, line.start + i, line.start + i + 3, 'mark');
				i += cb[0].length;
			}
			scanInline(t, line.start, out, i);
			prevJoinable = !hardBreak;
			continue;
		}
		const de = DESC.exec(t);
		if (de) {
			push(out, line.start, line.start + de[1].length, 'strong');
			push(out, line.start + de[1].length, line.start + de[1].length + de[2].length, 'mark');
			scanInline(t, line.start, out, de[0].length);
			prevJoinable = !hardBreak;
			continue;
		}
		if (t.trim() === '+') {
			push(out, line.start, line.end, 'mark'); // list continuation
			continue;
		}
		if (joinPrev) soft(line);
		prevJoinable = !hardBreak;
		scanInline(t, line.start, out);
	}
	flushPara();
	return out;
}

export const base: BaseImpl = { spans };
