// Overlapping highlights: the engine returns overlapping spans; the web
// splits them into runs, each carrying every covering highlight.
import { describe, expect, it } from 'vitest';
import { buildSegments, flattenHighlightSpans } from '$lib/nostr/nostrdown';

const span = (id: string, start: number, end: number, color?: string) => ({
	id,
	pubkey: 'p',
	start,
	end,
	highlighter: color ? { label: id, color } : undefined
});

describe('flattenHighlightSpans', () => {
	it('splits an overlap into runs with stacked layers', () => {
		const runs = flattenHighlightSpans([span('outer', 0, 10), span('inner', 4, 7)]);
		expect(runs.map((r) => [r.start, r.end, r.layers.map((l) => l.id)])).toEqual([
			[0, 4, ['outer']],
			[4, 7, ['outer', 'inner']],
			[7, 10, ['outer']]
		]);
	});

	it('partial overlap and a gap', () => {
		const runs = flattenHighlightSpans([span('a', 0, 5), span('b', 3, 8), span('c', 10, 12)]);
		expect(runs.map((r) => [r.start, r.end, r.layers.map((l) => l.id).join('+')])).toEqual([
			[0, 3, 'a'],
			[3, 5, 'a+b'],
			[5, 8, 'b'],
			[10, 12, 'c']
		]);
	});

	it('identical spans share one run', () => {
		const runs = flattenHighlightSpans([span('x', 2, 6), span('y', 2, 6)]);
		expect(runs).toHaveLength(1);
		expect(runs[0].layers.map((l) => l.id)).toEqual(['x', 'y']);
	});
});

describe('buildSegments with overlapping highlights', () => {
	it('keeps the text intact and carries highlighter + layers', () => {
		const content = 'both owned and findable.';
		const segs = buildSegments(
			content,
			[span('claim', 0, 23, '#f7a35c'), span('for', 5, 23, '#8fd694')],
			[]
		);
		const text = segs.map((s) => (s.type === 'highlight' ? s.parts.map((p) => (p.type === 'text' ? p.text : '')).join('') : s.type === 'text' ? s.text : '')).join('');
		expect(text).toBe(content);
		const marks = segs.filter((s) => s.type === 'highlight');
		expect(marks).toHaveLength(2);
		const second = marks[1];
		if (second.type !== 'highlight') throw new Error('unreachable');
		expect(second.highlight.layers.map((l) => l.id)).toEqual(['claim', 'for']);
		expect(second.highlight.highlighter?.color).toBe('#8fd694');
	});

	it('focus applies when any layer is focused', () => {
		const segs = buildSegments('abcdef', [span('o', 0, 6), span('i', 2, 4)], [], [], 'o');
		const marks = segs.filter((s) => s.type === 'highlight');
		expect(marks.every((m) => m.type === 'highlight' && m.highlight.focused)).toBe(true);
	});
});
