// Shared scaffolding for the hand-written decorated-source tokenizers (Org,
// AsciiDoc). Both are line-driven: a tiny state machine over lines for block
// structure, then an inline pass per line for emphasis / links. Everything
// here is pure and offset-exact (UTF-16 code units, the basis RichContent,
// highlight capture and nostrdown all share).
//
// The tokenizers never rewrite text: they only emit `StyleSpan`s, and
// sigils get the `mark` class (dimmed by CSS, never hidden).

import type { StyleSpan } from '../registry';

export type Line = {
	/** Offset of the first character of the line. */
	start: number;
	/** Offset one past the last character, excluding the `\n`. */
	end: number;
	text: string;
};

/** Split `text` into lines with exact offsets. A trailing `\n` yields no
 *  empty extra line; `\r\n` keeps the `\r` on the line (never styled). */
export function splitLines(text: string): Line[] {
	const out: Line[] = [];
	let start = 0;
	for (;;) {
		const nl = text.indexOf('\n', start);
		if (nl < 0) {
			if (start < text.length) out.push({ start, end: text.length, text: text.slice(start) });
			return out;
		}
		out.push({ start, end: nl, text: text.slice(start, nl) });
		start = nl + 1;
	}
}

/** Push a span, dropping empties. */
export function push(out: StyleSpan[], start: number, end: number, cls: string): void {
	if (end > start) out.push({ start, end, cls });
}

/** Per-line occupancy mask for the inline pass: once a construct (a nostrdown
 *  token, an inline code run, a link macro) has claimed a range, emphasis
 *  scanning neither starts nor ends inside it — so a `*` inside `{{…}}`
 *  can never pair with one outside. */
export class Mask {
	private readonly bits: Uint8Array;
	constructor(len: number) {
		this.bits = new Uint8Array(len);
	}
	take(a: number, b: number): void {
		for (let i = Math.max(0, a); i < Math.min(b, this.bits.length); i++) this.bits[i] = 1;
	}
	taken(i: number): boolean {
		return i >= 0 && i < this.bits.length && this.bits[i] === 1;
	}
	/** True when no position in [a, b) is taken. */
	free(a: number, b: number): boolean {
		for (let i = Math.max(0, a); i < Math.min(b, this.bits.length); i++) {
			if (this.bits[i] === 1) return false;
		}
		return true;
	}
}

/** Claim every `{{ … }}` nostrdown token on the line so no markup construct
 *  starts or ends inside one. The token itself gets no class — it becomes an
 *  atom in RichContent regardless (nostrdown resolves it engine-side; the
 *  layer never decides what it is). */
export function maskNostrdown(line: string, mask: Mask): void {
	let i = 0;
	for (;;) {
		const open = line.indexOf('{{', i);
		if (open < 0) return;
		const close = line.indexOf('}}', open + 2);
		if (close < 0) return;
		mask.take(open, close + 2);
		i = close + 2;
	}
}

/** Is `ch` a "word" character for emphasis boundary rules? */
export function isWordChar(ch: string | undefined): boolean {
	return ch !== undefined && /[\p{L}\p{N}_]/u.test(ch);
}

export function isSpace(ch: string | undefined): boolean {
	return ch === undefined || ch === ' ' || ch === '\t' || ch === '\r';
}

/** Scan one line for bare URLs (outside already-taken ranges) and style them
 *  `url`. Trailing punctuation that is almost never part of a URL is left
 *  out of the span. */
export function scanBareUrls(line: string, base: number, mask: Mask, out: StyleSpan[]): void {
	const re = /\b(?:https?|ftp|wss?):\/\/[^\s<>\]\)}"']+/g;
	let m: RegExpExecArray | null;
	while ((m = re.exec(line))) {
		let s = m.index;
		let e = s + m[0].length;
		while (e > s && /[.,;:!?]/.test(line[e - 1])) e--;
		if (!mask.free(s, e)) continue;
		mask.take(s, e);
		push(out, base + s, base + e, 'url');
	}
}

/** Marker-delimited inline run (`*bold*`, `_em_`, `=verbatim=`…) with
 *  per-format boundary rules. Returns the closing index or -1. */
export function findClose(
	line: string,
	open: number,
	marker: string,
	mask: Mask,
	closeOk: (j: number) => boolean
): number {
	const mlen = marker.length;
	let j = open + mlen;
	for (;;) {
		j = line.indexOf(marker, j);
		if (j < 0) return -1;
		// At least one character between the markers, closer not already claimed.
		if (j >= open + mlen + 1 && mask.free(j, j + mlen) && closeOk(j)) return j;
		j += 1;
	}
}
