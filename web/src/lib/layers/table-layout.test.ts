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

describe('asciidoc spans and specs', () => {
	const cells = (t: string) =>
		tableLayout(t, adoc.spans(t)).map((tb) => ({
			cols: tb.cols,
			rows: tb.rows.map((r) =>
				r.cells.map((c) => {
					const o: Record<string, unknown> = { t: t.slice(c.start, c.end) };
					if (c.colspan) o.cs = c.colspan;
					if (c.rowspan) o.rs = c.rowspan;
					if (c.style) o.st = c.style;
					if (c.align) o.al = c.align;
					if (c.valign) o.va = c.valign;
					return o;
				})
			)
		}));

	it('parseCellSpec', async () => {
		const { parseCellSpec } = await import('./table-layout');
		expect(parseCellSpec('2+')).toEqual({ colspan: 2, rowspan: 1 });
		expect(parseCellSpec('.3+')).toEqual({ colspan: 1, rowspan: 3 });
		expect(parseCellSpec('2.3+a')).toEqual({ colspan: 2, rowspan: 3, style: 'a' });
		expect(parseCellSpec('^.>s')).toEqual({ colspan: 1, rowspan: 1, align: 'center', valign: 'bottom', style: 's' });
		expect(parseCellSpec('3*')).toEqual({ colspan: 3, rowspan: 1 });
		for (const bad of ['', '+', '.+', 'x', 'ab', '2', '*', 'data']) expect(parseCellSpec(bad)).toBeNull();
	});

	it('colspan: the spec stays out of the previous cell; later rows do not shift', () => {
		const t = '[cols="3*"]\n|===\n|a |b |c\n\n2+|wide |c2\n|x |y |z\n|===';
		expect(cells(t)).toEqual([
			{
				cols: 3,
				rows: [
					[{ t: 'a' }, { t: 'b' }, { t: 'c' }],
					[{ t: 'wide', cs: 2 }, { t: 'c2' }],
					[{ t: 'x' }, { t: 'y' }, { t: 'z' }]
				]
			}
		]);
	});

	it('colspan on the first row counts toward the width (no cols attribute)', () => {
		const t = '|===\n2+|title |r\n|a |b |c\n|===';
		const [tb] = cells(t);
		expect(tb.cols).toBe(3);
		expect(tb.rows).toEqual([
			[{ t: 'title', cs: 2 }, { t: 'r' }],
			[{ t: 'a' }, { t: 'b' }, { t: 'c' }]
		]);
	});

	it('rowspan: the rows below hold fewer cells, grouping stays aligned', () => {
		const t = '[cols="1,1,1"]\n|===\n.2+|tall |a |b\n|c |d\n|e |f |g\n|===';
		expect(cells(t)).toEqual([
			{
				cols: 3,
				rows: [
					[{ t: 'tall', rs: 2 }, { t: 'a' }, { t: 'b' }],
					[{ t: 'c' }, { t: 'd' }],
					[{ t: 'e' }, { t: 'f' }, { t: 'g' }]
				]
			}
		]);
	});

	it('both spans, styles and alignment; one cell per line', () => {
		const t = '[cols="3*"]\n|===\n2.2+a|block\n|r1\n|r2\n^.^s|centre\n|p\n|q\n|===';
		expect(cells(t)).toEqual([
			{
				cols: 3,
				rows: [
					[{ t: 'block', cs: 2, rs: 2, st: 'a' }, { t: 'r1' }],
					[{ t: 'r2' }],
					[{ t: 'centre', al: 'center', va: 'middle', st: 's' }, { t: 'p' }, { t: 'q' }]
				]
			}
		]);
	});

	it('a| cell keeps its source text, styled by the base', () => {
		const t = '|===\na|*bold* and footnote:[n]\n|===';
		const [tb] = tableLayout(t, adoc.spans(t));
		const c = tb.rows[0].cells[0];
		expect(t.slice(c.start, c.end)).toBe('*bold* and footnote:[n]');
		expect(c.style).toBe('a');
		const s = adoc.spans(t);
		expect(s.some((x) => x.cls === 'strong' && t.slice(x.start, x.end) === '*bold*')).toBe(true);
		expect(s.some((x) => x.cls === 'fn')).toBe(true);
	});

	it('masked pipes stay inside their cell', () => {
		const t = '|===\n|{{ref:a|b}} |[[t|alias]]\n|+x|y+ |`p|q`\n|===';
		expect(cells(t)[0].rows).toEqual([
			[{ t: '{{ref:a|b}}' }, { t: '[[t|alias]]' }],
			[{ t: '+x|y+' }, { t: '`p|q`' }]
		]);
	});

	it('emphasis never pairs across cells', () => {
		const t = '|===\n|*a |b*\n|===';
		expect(adoc.spans(t).filter((x) => x.cls === 'strong')).toEqual([]);
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

describe('fence blocks', () => {
	it('a fence is one block; a pipe table inside it stays code', async () => {
		const { fenceLayout, layoutSegments } = await import('./table-layout');
		const t = 'a\n\n```text\n| x | y |\n|---|---|\n║ box ║\n```\nb';
		const spans = md.spans(t);
		const fences = fenceLayout(t, spans);
		expect(fences.map((f) => t.slice(f.start, f.end))).toEqual(['```text\n| x | y |\n|---|---|\n║ box ║\n```\n']);
		const blocks = layoutSegments(
			[{ type: 'text', text: t, srcStart: 0, srcEnd: t.length }],
			tableLayout(t, spans),
			fences
		);
		expect(blocks.map((b) => b.kind)).toEqual(['flow', 'fence', 'flow']);
		const fence = blocks[1] as Extract<(typeof blocks)[number], { kind: 'fence' }>;
		expect(fence.segs.map((s) => ('text' in s ? s.text : '')).join('')).toBe(t.slice(fences[0].start, fences[0].end));
	});
});
