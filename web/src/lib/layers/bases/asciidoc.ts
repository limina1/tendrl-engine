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

/** Macro targets: `link:`, `xref:`, `image:`/`image::`, `include::`,
 *  `anchor:`, `footnote:`, `mailto:`, `kbd:`, `btn:`, `menu:` and
 *  `scheme://…[text]`. Whole → `link`; target → `url`; brackets → `mark`. */
const MACRO =
	/\b(?:(https?|ftp|irc|mailto):([^\s\[]+)|(link|xref|image|include|anchor|footnote|footnoteref|kbd|btn|menu|icon|pass):{1,2}([^\s\[]*))\[([^\]]*)\]/g;
const XREF = /<<([^>]+)>>/g;
const INLINE_ANCHOR = /\[\[([^\]]+)\]\]/g;
const ATTR_REF = /(?<!\{)\{([a-zA-Z0-9_][\w-]*)\}(?!\})/g;

// marker, class, unconstrained (doubled) form?
const INLINE: [string, string][] = [
	['`', 'code'],
	['*', 'strong'],
	['_', 'em'],
	['#', 'hl']
];

function scanInline(line: string, base: number, out: StyleSpan[], from = 0): void {
	const mask = new Mask(line.length);
	mask.take(0, from);
	maskNostrdown(line, mask);

	// Macros and references first — their bodies are opaque to emphasis.
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
		const br = e - m[5].length - 2;
		push(out, base + br, base + br + 1, 'mark');
		push(out, base + e - 1, base + e, 'mark');
		mask.take(s, e);
	}
	XREF.lastIndex = 0;
	while ((m = XREF.exec(line))) {
		const s = m.index;
		const e = s + m[0].length;
		if (!mask.free(s, e)) continue;
		push(out, base + s, base + e, 'link');
		push(out, base + s, base + s + 2, 'mark');
		push(out, base + e - 2, base + e, 'mark');
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

	for (const [ch, cls] of INLINE) {
		// Unconstrained (`**x**`) first, then constrained (`*x*`).
		scanMarker(line, base, mask, out, ch + ch, cls, false);
		scanMarker(line, base, mask, out, ch, cls, true);
	}

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

function attrLine(line: Line, inner: string, out: StyleSpan[]): string {
	push(out, line.start, line.end, 'kw');
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

function spans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	const lines = splitLines(text);
	// Delimited-block stack: containers (example / sidebar / open / quote)
	// may nest; verbatim ones (listing / literal / passthrough / comment /
	// table) never scan their body.
	const stack: { delim: string; kind: BlockKind; start: number }[] = [];
	let pendingAttr = ''; // name from the last `[…]` line, consumed by the next block
	let pendingAttrRaw = ''; // its full text (for `[options="header"]` / `[%header]`)
	let tableHeaderDone = false;
	let tableHeaderByAttr = false;
	// Rows for the table grid: AsciiDoc cells may sit one per line, so a
	// row is `tableCols` cells — from `[cols=…]`, else the first row line.
	let tableCols = 0;
	let tablePipes: number[] = [];
	const emitRows = (closeAt: number) => {
		if (tableCols <= 0) return;
		for (let k = 0; k < tablePipes.length; k += tableCols) {
			const from = tablePipes[k];
			let to = k + tableCols < tablePipes.length ? tablePipes[k + tableCols] : closeAt;
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
		if (line.start === 0) return;
		const indent = /^[ \t]*/.exec(line.text)![0].length;
		push(out, line.start - 1, line.start + indent, 'soft');
	};

	for (let n = 0; n < lines.length; n++) {
		const line = lines[n];
		const t = line.text;
		const top = stack[stack.length - 1];
		const joinPrev = prevJoinable;
		prevJoinable = false;

		const d = DELIM.exec(t);
		if (d) {
			const delim = d[1];
			if (top && top.delim === delim) {
				push(out, line.start, line.end, 'mark');
				if (top.kind !== 'container') push(out, top.start, line.end, top.kind);
				if (top.kind === 'table') emitRows(line.start);
				stack.pop();
				pendingAttr = '';
				continue;
			}
			if (!top || top.kind === 'container' || top.kind === 'quote') {
				let kind = delimKind(delim);
				// `[stem]` / `[latexmath]` + `++++` or `--` → maths block = fence.
				if (STEM.has(pendingAttr) && (kind === 'container' || kind === 'fence')) kind = 'fence';
				push(out, line.start, line.end, 'mark');
				if (
					!lines.slice(n + 1).some((l) => DELIM.exec(l.text)?.[1] === delim)
				) {
					// Unterminated: the block runs to the end of the text.
					if (kind !== 'container') push(out, line.start, text.length, kind);
					if (kind === 'container' || kind === 'quote') {
						for (let k = n + 1; k < lines.length; k++)
							scanInline(lines[k].text, lines[k].start, out);
					}
					break;
				}
				stack.push({ delim, kind, start: line.start });
				tableHeaderDone = false;
				tableHeaderByAttr = /header/.test(pendingAttrRaw);
				tablePipes = [];
				{
					const c = /cols="?([^"\]]+)/.exec(pendingAttrRaw)?.[1]?.trim();
					const mult = c && /^(\d+)\*/.exec(c);
					tableCols = mult ? Number(mult[1]) : c ? c.split(/[,;]/).length : 0;
				}
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
			let rowPipes = 0;
			for (let i = 0; i < t.length; i++)
				if (t[i] === '|') {
					push(out, line.start + i, line.start + i + 1, 'mark');
					tablePipes.push(line.start + i);
					rowPipes++;
				}
			if (tableCols === 0 && rowPipes > 0) tableCols = rowPipes;
			scanInline(t, line.start, out);
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
	return out;
}

export const base: BaseImpl = { spans };
