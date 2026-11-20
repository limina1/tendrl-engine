// Reader-side footnotes / xrefs / anchors. Spans are hand-built to the
// AsciiDoc base's contract (docs/epub-import-contract.org).
import { describe, expect, it } from 'vitest';
import type { StyleSpan } from '$lib/layers/registry';
import {
	anchorIdsIn,
	anchorsFromStyles,
	anchorsToUtf16,
	cellStyle,
	findAnchor,
	footnoteBody,
	footnoteMarks,
	gridSlots,
	readerHiddenSpans,
	utf8ToUtf16,
	xrefAt
} from './xref';

/** Spans for every `footnote:name?[body]` in `text`, as the base emits them. */
function fnSpans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	for (const m of text.matchAll(/footnote:([\w-]*)\[([^\]]*)\]/g)) {
		const start = m.index!;
		const end = start + m[0].length;
		out.push({ start, end, cls: 'fn' });
		const nameStart = start + 'footnote:'.length;
		if (m[1]) out.push({ start: nameStart, end: nameStart + m[1].length, cls: 'fnname' });
		const bodyStart = nameStart + m[1].length + 1;
		if (m[2]) out.push({ start: bodyStart, end: bodyStart + m[2].length, cls: 'fnbody' });
		out.push({ start, end: bodyStart, cls: 'mark' }, { start: end - 1, end, cls: 'mark' });
	}
	return out;
}

/** Spans for every `<<id>>` / `<<id,text>>`. */
function xrefSpans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	for (const m of text.matchAll(/<<([^,>]+)(?:,([^>]*))?>>/g)) {
		const start = m.index!;
		const end = start + m[0].length;
		out.push({ start, end, cls: 'xref' });
		out.push({ start: start + 2, end: start + 2 + m[1].length, cls: 'xref-id' });
		if (m[2] !== undefined) {
			const ts = start + 2 + m[1].length + 1;
			out.push({ start: ts, end: ts + m[2].length, cls: 'xref-text' });
		}
		out.push({ start, end: start + 2, cls: 'mark' }, { start: end - 2, end, cls: 'mark' });
	}
	return out;
}

describe('utf8ToUtf16', () => {
	it('maps byte offsets across 2-, 3- and 4-byte characters', () => {
		const s = 'aé€𝄞b';
		// bytes: a=0, é=1..2, €=3..5, 𝄞=6..9, b=10
		expect(utf8ToUtf16(s, [0, 1, 3, 6, 10, 11])).toEqual([0, 1, 2, 3, 5, 6]);
	});
	it('keeps input order and clamps', () => {
		expect(utf8ToUtf16('héllo', [6, 0, 99])).toEqual([5, 0, 5]);
	});
	it('an offset inside a character lands on its start', () => {
		expect(utf8ToUtf16('é', [1])).toEqual([0]);
	});
	it('re-bases engine anchors', () => {
		const content = 'ñ [#x]\nbody';
		const byteStart = new TextEncoder().encode('ñ [#x]\n').length;
		expect(anchorsToUtf16(content, [{ id: 'x', start: byteStart }])).toEqual([
			{ id: 'x', start: content.indexOf('body') }
		]);
	});
});

describe('footnoteMarks', () => {
	it('numbers per section in source order and reuses named notes', () => {
		const text =
			'A footnote:[one] B footnote:dis[Named note] C footnote:[three] D footnote:dis[] E';
		const marks = footnoteMarks(text, fnSpans(text));
		expect(marks.map((m) => m.n)).toEqual([1, 2, 3, 2]);
		expect(marks[3].reuse).toBe(true);
		expect(marks[3].name).toBe('dis');
		const body = footnoteBody(text, fnSpans(text), marks[3]);
		expect(body.text).toBe('Named note');
	});
	it('a reuse with no earlier definition gets its own number', () => {
		const text = 'x footnote:ghost[] y footnote:[a]';
		expect(footnoteMarks(text, fnSpans(text)).map((m) => m.n)).toEqual([1, 2]);
	});
	it('ignores span order and nested fn spans', () => {
		const text = 'footnote:[outer] z';
		const spans = fnSpans(text).reverse();
		spans.push({ start: 10, end: 15, cls: 'fn' });
		expect(footnoteMarks(text, spans)).toHaveLength(1);
	});
	it('body styles are clipped and re-based', () => {
		const text = 'q footnote:[*bold* tail]';
		const spans = [...fnSpans(text), { start: 12, end: 18, cls: 'strong' }, { start: 0, end: 30, cls: 'p' }];
		const [fn] = footnoteMarks(text, spans);
		const body = footnoteBody(text, spans, fn);
		expect(body.text).toBe('*bold* tail');
		expect(body.styles).toContainEqual({ start: 0, end: 6, cls: 'strong' });
		expect(body.styles).toContainEqual({ start: 0, end: 11, cls: 'p' });
	});
});

describe('xrefs', () => {
	const text = 'See <<ch2-notes,the notes>> and <<plain>>.';
	const spans = xrefSpans(text);
	it('reads the id under an offset', () => {
		expect(xrefAt(text, spans, text.indexOf('the notes'))?.id).toBe('ch2-notes');
		expect(xrefAt(text, spans, text.indexOf('plain'))?.id).toBe('plain');
		expect(xrefAt(text, spans, 0)).toBeNull();
	});
	it('hides the id (and comma) of a labelled xref only', () => {
		const hidden = readerHiddenSpans(spans);
		const first = text.indexOf('<<ch2');
		expect(hidden).toContainEqual({ start: first, end: text.indexOf('the notes'), cls: 'hide' });
		expect(hidden.some((h) => text.slice(h.start, h.end).includes('plain'))).toBe(false);
	});
	it('hides a whole footnote macro', () => {
		const t = 'a footnote:[b] c';
		expect(readerHiddenSpans(fnSpans(t))).toContainEqual({ start: 2, end: 14, cls: 'hide' });
	});
	it('finds the section holding an anchor', () => {
		const secs = [{ anchors: [{ id: 'a', start: 0 }] }, {}, { anchors: [{ id: 'b', start: 7 }] }];
		expect(findAnchor(secs, 'b')).toEqual({ index: 2, start: 7 });
		expect(findAnchor(secs, 'zz')).toBeNull();
	});
});

describe('anchors from spans', () => {
	it('reads inline and attribute-line anchors', () => {
		const text = 'intro anchor:r12[]word\n[#ch1-top.lead]\n==== Heading\n[quote, id=q9]\nQ\n';
		const spans: StyleSpan[] = [
			{ start: 6, end: 18, cls: 'anchor' },
			{ start: 23, end: 38, cls: 'kw' },
			{ start: 23, end: 38, cls: 'anchor' },
			{ start: 52, end: 66, cls: 'anchor' }
		];
		expect(anchorsFromStyles(text, spans)).toEqual([
			{ id: 'r12', start: 6 },
			{ id: 'ch1-top', start: text.indexOf('==== Heading') },
			{ id: 'q9', start: text.indexOf('Q\n') }
		]);
	});
	it('lists ids inside a run', () => {
		const a = [
			{ id: 'x', start: 2 },
			{ id: 'y', start: 4 },
			{ id: 'z', start: 9 }
		];
		expect(anchorIdsIn(a, 0, 5)).toBe('x y');
		expect(anchorIdsIn(a, 5, 9)).toBeUndefined();
	});
});

describe('gridSlots', () => {
	it('plain rows pad to the column count', () => {
		const slots = gridSlots([{ cells: [{}, {}] }, { cells: [{}] }], 2);
		expect(slots[1]).toEqual([
			{ cell: 0, colspan: 1, rowspan: 1 },
			{ cell: -1, colspan: 1, rowspan: 1 }
		]);
	});
	it('colspan and rowspan take their columns', () => {
		// | 2+| a        |
		// | .2+| b | c | d |  (b spans two rows)
		// |      e | f     |
		const rows = [
			{ cells: [{ colspan: 2 }, {}] },
			{ cells: [{ rowspan: 2 }, {}, {}] },
			{ cells: [{}, {}] }
		];
		const slots = gridSlots(rows, 3);
		expect(slots[0].map((s) => s.cell)).toEqual([0, 1]);
		expect(slots[1].map((s) => s.cell)).toEqual([0, 1, 2]);
		expect(slots[2].map((s) => s.cell)).toEqual([0, 1]);
		expect(slots[1][0].rowspan).toBe(2);
	});
	it('clamps a rowspan past the last row', () => {
		expect(gridSlots([{ cells: [{ rowspan: 5 }] }], 1)[0][0].rowspan).toBe(1);
	});
});

describe('cellStyle', () => {
	it('maps align / valign / style letters', () => {
		expect(cellStyle({ start: 0, end: 1 })).toBeUndefined();
		expect(cellStyle({ align: 'center', valign: 'bottom', style: 'e' })).toBe(
			'text-align:center;vertical-align:bottom;font-style:italic;'
		);
		expect(cellStyle({ style: 'm' })).toBe('font-family:var(--font-mono);');
	});
	it('ignores unknown values', () => {
		expect(cellStyle({ align: 'justify; color:red' })).toBeUndefined();
	});
});
