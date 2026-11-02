// Org decorated-source base — the Orgdown OD1 subset
// (https://gitlab.com/publicvoit/orgdown) → style spans.
//
// "Not replicating functionality, just rendering": this recognises the
// *syntax* of OD1 (headings, emphasis, lists, links, blocks, tables,
// keywords, comments, horizontal rules) and styles it in place. No agenda,
// no babel, no export — the text is never rewritten, every sigil stays at
// its offset with the `mark` class.
//
// Nostrdown: `[[topic]]` / `[[d-tag][display]]` is both an Org link and the
// Nostr wikilink nostrdown already claims engine-side (`is_markup_link_target`
// in src/nostrdown.rs decides which). This base styles every `[[…]]` as a
// link and never guesses which ones are Nostr: `applyStyles` keeps the
// nostrdown atom and only attaches classes, so the chip always wins.
// `{{ … }}` tokens are masked before the inline pass so no emphasis marker
// can pair across one.

import type { BaseImpl, StyleSpan } from '../registry';
import {
	findClose,
	isSpace,
	Mask,
	maskNostrdown,
	push,
	scanBareUrls,
	splitLines,
	type Line
} from './lines';

type BlockKind = 'fence' | 'quote' | 'comment' | 'plain';

const BEGIN = /^(\s*)#\+begin_([a-z_-]+)(?:\s+(\S+))?/i;
const END = /^\s*#\+end_([a-z_-]+)\s*$/i;
const HEADING = /^(\*+)\s+/;
const KEYWORD = /^\s*#\+[a-z_-]+:?/i;
const COMMENT = /^\s*#(?:\s|$)/;
const HR = /^\s*-{5,}\s*$/;
const LIST = /^(\s*)([-+]|\d+[.)])\s+/;
const CHECKBOX = /^\[[ Xx-]\]\s*/;
const DESC = /^(.+?)\s+::(?:\s+|$)/;
const TABLE = /^\s*\|/;
const TABLE_RULE = /^\s*\|-/;
const FIXED_WIDTH = /^\s*:(?:\s|$)/;
const TODO = /^(TODO|DONE|NEXT|WAITING|CANCELLED|CANCELED)\b/;
const PRIORITY = /^(\s*)(\[#[A-Z0-9]\])/;
const TAGS = /\s+(:[\p{L}\p{N}_@#%:]+:)\s*$/u;

function blockKind(name: string): BlockKind {
	switch (name.toLowerCase()) {
		case 'src':
		case 'example':
		case 'verse':
		case 'export':
			return 'fence';
		case 'quote':
		case 'center':
			return 'quote';
		case 'comment':
			return 'comment';
		default:
			return 'plain';
	}
}

// Org emphasis boundary rules (org-emphasis-regexp-components): the opening
// marker is preceded by line start or one of ` \t-('"{`; the closing marker is
// followed by line end or one of ` \t-.,:!?;'")}\[`; neither border is
// whitespace.
const PRE = new Set([' ', '\t', '-', '(', "'", '"', '{']);
const POST = new Set([' ', '\t', '-', '.', ',', ':', '!', '?', ';', "'", '"', ')', '}', '[', '\r']);

const EMPHASIS: [string, string, boolean][] = [
	// marker, class, verbatim (body not re-scanned)
	['=', 'code', true],
	['~', 'code', true],
	['*', 'strong', false],
	['/', 'em', false],
	['_', 'underline', false],
	['+', 'strike', false]
];

function scanInline(line: string, base: number, out: StyleSpan[], from = 0): void {
	const mask = new Mask(line.length);
	mask.take(0, from);
	maskNostrdown(line, mask);
	scanLinks(line, base, mask, out);
	scanBareUrls(line, base, mask, out);
	for (const [marker, cls, verbatim] of EMPHASIS) {
		let i = from;
		for (;;) {
			i = line.indexOf(marker, i);
			if (i < 0) break;
			const ok =
				!mask.taken(i) &&
				(i === 0 || PRE.has(line[i - 1])) &&
				!isSpace(line[i + 1]) &&
				line[i + 1] !== marker;
			if (!ok) {
				i += 1;
				continue;
			}
			const j = findClose(
				line,
				i,
				marker,
				mask,
				(j) => !isSpace(line[j - 1]) && (j + 1 >= line.length || POST.has(line[j + 1]))
			);
			if (j < 0) {
				i += 1;
				continue;
			}
			push(out, base + i, base + j + 1, cls);
			push(out, base + i, base + i + 1, 'mark');
			push(out, base + j, base + j + 1, 'mark');
			if (verbatim) mask.take(i, j + 1);
			else {
				mask.take(i, i + 1);
				mask.take(j, j + 1);
			}
			i = j + 1;
		}
	}
}

/** `[[target]]` / `[[target][description]]`. Whole link → `link`; brackets
 *  → `mark`; the target → `url` when a description follows it (then the
 *  description is what the reader sees). */
function scanLinks(line: string, base: number, mask: Mask, out: StyleSpan[]): void {
	let i = 0;
	for (;;) {
		const open = line.indexOf('[[', i);
		if (open < 0) return;
		if (mask.taken(open)) {
			i = open + 2;
			continue;
		}
		const close = line.indexOf(']]', open + 2);
		if (close < 0) return;
		const inner = line.slice(open + 2, close);
		if (inner.length === 0 || !mask.free(open, close + 2)) {
			i = close + 2;
			continue;
		}
		const end = close + 2;
		push(out, base + open, base + end, 'link');
		push(out, base + open, base + open + 2, 'mark');
		const sep = inner.indexOf('][');
		if (sep >= 0) {
			push(out, base + open + 2, base + open + 2 + sep, 'url');
			push(out, base + open + 2 + sep, base + open + 2 + sep + 2, 'mark');
		}
		push(out, base + close, base + end, 'mark');
		mask.take(open, end);
		i = end;
	}
}

function heading(line: Line, m: RegExpMatchArray, out: StyleSpan[]): void {
	const level = Math.min(m[1].length, 6);
	push(out, line.start, line.end, `h${level}`);
	push(out, line.start, line.start + m[1].length, 'mark');
	let i = m[0].length;
	const rest = line.text.slice(i);
	const todo = TODO.exec(rest);
	if (todo) {
		push(out, line.start + i, line.start + i + todo[0].length, 'todo');
		i += todo[0].length;
		const pr = PRIORITY.exec(line.text.slice(i));
		if (pr) {
			const at = i + pr[1].length;
			push(out, line.start + at, line.start + at + pr[2].length, 'mark');
			i += pr[0].length;
		}
	}
	const tags = TAGS.exec(line.text);
	let titleEnd = line.text.length;
	if (tags && tags.index >= i) {
		const tagStart = tags.index + tags[0].indexOf(tags[1]);
		push(out, line.start + tagStart, line.start + tagStart + tags[1].length, 'tag');
		titleEnd = tags.index;
	}
	scanInline(line.text.slice(0, titleEnd), line.start, out, i);
}

function spans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	const lines = splitLines(text);
	let block: { name: string; kind: BlockKind; start: number } | null = null;
	let tableStart = -1; // index of the first row of the current table

	for (let n = 0; n < lines.length; n++) {
		const line = lines[n];
		const t = line.text;

		if (block) {
			const end = END.exec(t);
			if (end && end[1].toLowerCase() === block.name) {
				push(out, line.start, line.end, 'mark');
				if (block.kind !== 'plain') push(out, block.start, line.end, block.kind);
				block = null;
			} else if (block.kind === 'quote' || block.kind === 'plain') {
				scanInline(t, line.start, out);
			}
			continue;
		}

		const begin = BEGIN.exec(t);
		if (begin) {
			const name = begin[2].toLowerCase();
			const kind = blockKind(name);
			block = { name, kind, start: line.start };
			push(out, line.start, line.end, 'mark');
			if (name === 'src' && begin[3]) {
				const langAt = begin[0].length - begin[3].length;
				push(out, line.start + langAt, line.start + begin[0].length, 'lang');
			}
			// An unterminated block runs to the end of the text.
			const hasEnd = lines
				.slice(n + 1)
				.some((l) => (END.exec(l.text)?.[1] ?? '').toLowerCase() === name);
			if (!hasEnd) {
				if (kind !== 'plain') push(out, line.start, text.length, kind);
				if (kind === 'plain' || kind === 'quote') {
					for (let k = n + 1; k < lines.length; k++) scanInline(lines[k].text, lines[k].start, out);
				}
				break;
			}
			continue;
		}

		// Tables: a run of `|` lines; the first row is a header when a rule
		// (`|---`) follows it.
		if (TABLE.test(t)) {
			push(out, line.start, line.end, 'table');
			if (TABLE_RULE.test(t)) {
				push(out, line.start, line.end, 'mark');
			} else {
				if (tableStart < 0) {
					tableStart = n;
					if (n + 1 < lines.length && TABLE_RULE.test(lines[n + 1].text)) {
						push(out, line.start, line.end, 'th');
					}
				}
				markPipes(t, line.start, out);
				scanInline(t, line.start, out);
			}
			continue;
		}
		tableStart = -1;

		const h = HEADING.exec(t);
		if (h) {
			heading(line, h, out);
			continue;
		}
		if (KEYWORD.test(t)) {
			push(out, line.start, line.end, 'kw');
			continue;
		}
		if (COMMENT.test(t)) {
			push(out, line.start, line.end, 'comment');
			continue;
		}
		if (HR.test(t)) {
			push(out, line.start, line.end, 'hr');
			continue;
		}
		if (FIXED_WIDTH.test(t)) {
			push(out, line.start, line.end, 'code');
			continue;
		}
		const li = LIST.exec(t);
		if (li) {
			const bulletStart = li[1].length;
			let i = li[0].length;
			push(out, line.start + bulletStart, line.start + i - 1, 'mark');
			const cb = CHECKBOX.exec(t.slice(i));
			if (cb) {
				push(out, line.start + i, line.start + i + cb[0].trimEnd().length, 'mark');
				i += cb[0].length;
			}
			const desc = DESC.exec(t.slice(i));
			if (desc) {
				push(out, line.start + i, line.start + i + desc[1].length, 'strong');
				const sepAt = i + desc[0].lastIndexOf('::');
				push(out, line.start + sepAt, line.start + sepAt + 2, 'mark');
			}
			scanInline(t, line.start, out, i);
			continue;
		}
		scanInline(t, line.start, out);
	}
	return out;
}

function markPipes(line: string, base: number, out: StyleSpan[]): void {
	for (let i = 0; i < line.length; i++) {
		if (line[i] === '|') push(out, base + i, base + i + 1, 'mark');
	}
}

export const base: BaseImpl = { spans };
