import { describe, expect, it } from 'vitest';
import { tableLayout } from './table-layout';
import { base as md } from './bases/markdown';
import { base as org } from './bases/org';
import { base as adoc } from './bases/asciidoc';
import type { BaseImpl } from './registry';

const grid = (t: string, b: BaseImpl) =>
	tableLayout(t, b.spans(t)).map((tb) => ({
		cols: tb.cols,
		rows: tb.rows.map((r) => ({ h: r.header, c: r.cells.map((c) => t.slice(c.start, c.end)) }))
	}));

describe('table layout', () => {
	it('markdown: header, rule skipped, unpadded cells, empty cell kept', () => {
		const t = 'before\n\n| a | bee |\n|---|---|\n| long cell | |\n| x | y |\n\nafter';
		expect(grid(t, md)).toEqual([
			{
				cols: 2,
				rows: [
					{ h: true, c: ['a', 'bee'] },
					{ h: false, c: ['long cell', ''] },
					{ h: false, c: ['x', 'y'] }
				]
			}
		]);
		const [tb] = tableLayout(t, md.spans(t));
		expect(t.slice(tb.end)).toBe('\nafter');
	});

	it('org: per-line table spans merge, rule row skipped', () => {
		const t = '| Name | Qty |\n|------+-----|\n| apple | 3 |\n';
		expect(grid(t, org)).toEqual([
			{ cols: 2, rows: [{ h: true, c: ['Name', 'Qty'] }, { h: false, c: ['apple', '3'] }] }
		]);
	});

	it('asciidoc: one-line rows and one-cell-per-line rows', () => {
		const t = '|===\n|A |B\n\n|1 |2\n|3\n|4\n|===\n';
		expect(grid(t, adoc)).toEqual([
			{
				cols: 2,
				rows: [
					{ h: true, c: ['A', 'B'] },
					{ h: false, c: ['1', '2'] },
					{ h: false, c: ['3', '4'] }
				]
			}
		]);
	});

	it('asciidoc: cols attribute sets the width', () => {
		const t = '[cols="1,1,1"]\n|===\n|a\n|b\n|c\n|d\n|e\n|f\n|===';
		expect(grid(t, adoc)[0].rows.map((r) => r.c)).toEqual([
			['a', 'b', 'c'],
			['d', 'e', 'f']
		]);
	});
});

describe('layoutSegments', () => {
	it('splits a highlight across cells and keeps every cell run offset-exact', async () => {
		const { layoutSegments } = await import('./table-layout');
		const t = 'x\n\n| ab | cd |\n|--|--|\n| ef | gh |\n\ny';
		const tables = tableLayout(t, md.spans(t));
		const hl = {
			type: 'highlight' as const,
			text: t.slice(5, 11),
			srcStart: 5,
			srcEnd: 11,
			highlight: { id: 'h', pubkey: 'p', focused: false, layers: [] },
			parts: [{ type: 'text' as const, text: t.slice(5, 11), srcStart: 5, srcEnd: 11 }]
		};
		const segs = [
			{ type: 'text' as const, text: t.slice(0, 5), srcStart: 0, srcEnd: 5 },
			hl,
			{ type: 'text' as const, text: t.slice(11), srcStart: 11, srcEnd: t.length }
		];
		const blocks = layoutSegments(segs, tables);
		expect(blocks.map((b) => b.kind)).toEqual(['flow', 'table', 'flow']);
		const tb = blocks[1] as Extract<(typeof blocks)[number], { kind: 'table' }>;
		const cellText = tb.cells.map((r) =>
			r.map((c) => c.map((s) => ('text' in s ? s.text : '')).join(''))
		);
		expect(cellText).toEqual([
			['ab', 'cd'],
			['ef', 'gh']
		]);
		for (const r of tb.cells)
			for (const c of r)
				for (const s of c) if ('text' in s) expect(t.slice(s.srcStart, s.srcEnd)).toBe(s.text);
		expect(tb.cells[0][0].map((s) => s.type)).toEqual(['highlight']);
		expect(tb.cells[0][1][0].type).toBe('highlight');
	});
});
