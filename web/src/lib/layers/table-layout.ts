// Table grid — turn a base's table spans into rows × cells over the source.
//
// A literal pipe table only reads when its author padded the columns; in a
// proportional face, or once a cell holds a ref chip, it never lines up.
// So when a base is active the reader draws tables as a real grid
// (RichContent): each cell renders the SOURCE runs of its range, with their
// `data-src-*` stamps, so highlight capture and nostrdown refs keep their
// offsets. Only the separators (pipes, the `|---|` rule, padding) are not
// drawn.
//
// Input, from the bases: `table` (the block; Org emits it per line, so
// adjacent ones merge), `tr` / `th` (a row; `th` = header row) and `mark`
// spans of exactly `|` (the cell separators). Pure; leaf module.

import type { StyleSpan } from './registry';
import type { ContentSegment } from '$lib/nostr/nostrdown';

export type TableCell = { start: number; end: number };
export type TableRow = { start: number; end: number; header: boolean; cells: TableCell[] };
export type TableBlock = {
	/** Source range the grid replaces — includes the newline after the last
	 *  row, so no blank line is left under the grid. */
	start: number;
	end: number;
	cols: number;
	rows: TableRow[];
};

const isWs = (c: string | undefined) => c === ' ' || c === '\t' || c === '\n' || c === '\r';

function trimRange(text: string, start: number, end: number): TableCell {
	while (start < end && isWs(text[start])) start++;
	while (end > start && isWs(text[end - 1])) end--;
	return { start, end };
}

export function tableLayout(text: string, spans: StyleSpan[]): TableBlock[] {
	const sorted = [...spans].sort((a, b) => a.start - b.start || b.end - a.end);
	// Table blocks, adjacent / overlapping ones merged (Org: one per line).
	const blocks: { start: number; end: number }[] = [];
	for (const s of sorted) {
		if (s.cls !== 'table' || s.end <= s.start) continue;
		const last = blocks[blocks.length - 1];
		if (last && s.start <= last.end + 1) last.end = Math.max(last.end, s.end);
		else blocks.push({ start: s.start, end: s.end });
	}
	if (blocks.length === 0) return [];
	const pipes = sorted
		.filter((s) => s.cls === 'mark' && s.end - s.start === 1 && text[s.start] === '|')
		.map((s) => s.start);
	const heads = sorted.filter((s) => s.cls === 'th');

	const out: TableBlock[] = [];
	for (const b of blocks) {
		const rowSpans = new Map<number, { start: number; end: number }>();
		for (const s of sorted) {
			if ((s.cls !== 'tr' && s.cls !== 'th') || s.start < b.start || s.end > b.end) continue;
			const prev = rowSpans.get(s.start);
			if (!prev || s.end > prev.end) rowSpans.set(s.start, { start: s.start, end: s.end });
		}
		const rows: TableRow[] = [];
		for (const r of [...rowSpans.values()].sort((a, b) => a.start - b.start)) {
			const cuts = pipes.filter((p) => p >= r.start && p < r.end);
			const pieces: TableCell[] = [];
			let at = r.start;
			for (const p of cuts) {
				pieces.push({ start: at, end: p });
				at = p + 1;
			}
			pieces.push({ start: at, end: r.end });
			// A leading / trailing pipe leaves an empty piece outside the row.
			if (cuts.length && text.slice(pieces[0].start, pieces[0].end).trim() === '') pieces.shift();
			const tail = pieces[pieces.length - 1];
			if (cuts.length && tail && text.slice(tail.start, tail.end).trim() === '') pieces.pop();
			if (pieces.length === 0) continue;
			rows.push({
				start: r.start,
				end: r.end,
				header: heads.some((h) => h.start <= r.start && r.start < h.end),
				cells: pieces.map((c) => trimRange(text, c.start, c.end))
			});
		}
		if (rows.length === 0) continue;
		const end = text[b.end] === '\n' ? b.end + 1 : b.end;
		out.push({
			start: b.start,
			end,
			cols: Math.max(...rows.map((r) => r.cells.length)),
			rows
		});
	}
	return out;
}

// --- segments → flow + grids -------------------------------------------------

/** A code fence drawn as its own block: lines kept whole (no wrap), the
 *  block scrolls sideways — box-drawing diagrams and wide code survive a
 *  narrow measure. Range includes the newline after the closing line. */
export type FenceBlock = { start: number; end: number };

/** Fence blocks from a base's `fence` spans (adjacent ones merged). */
export function fenceLayout(text: string, spans: StyleSpan[]): FenceBlock[] {
	const out: FenceBlock[] = [];
	for (const s of [...spans].sort((a, b) => a.start - b.start)) {
		if (s.cls !== 'fence' || s.end <= s.start) continue;
		const last = out[out.length - 1];
		if (last && s.start <= last.end + 1) last.end = Math.max(last.end, s.end);
		else out.push({ start: s.start, end: s.end });
	}
	for (const f of out) if (text[f.end] === '\n') f.end += 1;
	return out;
}

/** What RichContent draws: runs of flowing text, grids whose cells hold
 *  the runs inside each cell's range, and fence blocks. */
export type RenderBlock =
	| { kind: 'flow'; segs: ContentSegment[] }
	| { kind: 'fence'; segs: ContentSegment[] }
	| { kind: 'table'; table: TableBlock; cells: ContentSegment[][][] };

/** Split text / highlight runs at `at` (atoms are never split). */
function splitAt(seg: ContentSegment, at: number): ContentSegment[] {
	if (at <= seg.srcStart || at >= seg.srcEnd) return [seg];
	if (seg.type === 'text') {
		const k = at - seg.srcStart;
		return [
			{ ...seg, text: seg.text.slice(0, k), srcEnd: at },
			{ ...seg, text: seg.text.slice(k), srcStart: at }
		];
	}
	if (seg.type === 'highlight') {
		const k = at - seg.srcStart;
		const left: ContentSegment[] = [];
		const right: ContentSegment[] = [];
		for (const p of seg.parts) {
			for (const q of splitAt(p, at)) (q.srcStart < at ? left : right).push(q);
		}
		const out: ContentSegment[] = [];
		if (left.length) out.push({ ...seg, text: seg.text.slice(0, k), srcEnd: at, parts: left });
		if (right.length) out.push({ ...seg, text: seg.text.slice(k), srcStart: at, parts: right });
		return out;
	}
	return [seg];
}

export function layoutSegments(
	segments: ContentSegment[],
	tablesIn: TableBlock[],
	fences: FenceBlock[] = []
): RenderBlock[] {
	if (tablesIn.length === 0 && fences.length === 0) return [{ kind: 'flow', segs: segments }];
	// A table inside a fence is code, not a table.
	const tables = tablesIn.filter((t) => !fences.some((f) => t.start >= f.start && t.start < f.end));
	const cuts = new Set<number>();
	for (const f of fences) {
		cuts.add(f.start);
		cuts.add(f.end);
	}
	for (const t of tables) {
		cuts.add(t.start);
		cuts.add(t.end);
		for (const r of t.rows)
			for (const c of r.cells) {
				cuts.add(c.start);
				cuts.add(c.end);
			}
	}
	const sortedCuts = [...cuts].sort((a, b) => a - b);
	let pieces = segments;
	for (const c of sortedCuts) {
		const next: ContentSegment[] = [];
		for (const seg of pieces) next.push(...splitAt(seg, c));
		pieces = next;
	}
	const out: RenderBlock[] = [];
	let flow: ContentSegment[] = [];
	let ti = 0;
	let fi = 0;
	let open: Extract<RenderBlock, { kind: 'table' }> | null = null;
	let openFence: { block: Extract<RenderBlock, { kind: 'fence' }>; f: FenceBlock } | null = null;
	for (const seg of pieces) {
		while (fi < fences.length && seg.srcStart >= fences[fi].end) fi++;
		const f = fences[fi];
		if (f && seg.srcStart >= f.start) {
			if (!openFence || openFence.f !== f) {
				if (flow.length) out.push({ kind: 'flow', segs: flow });
				flow = [];
				openFence = { block: { kind: 'fence', segs: [] }, f };
				out.push(openFence.block);
			}
			openFence.block.segs.push(seg);
			continue;
		}
		while (ti < tables.length && seg.srcStart >= tables[ti].end) ti++;
		const t = tables[ti];
		if (!t || seg.srcStart < t.start) {
			flow.push(seg);
			continue;
		}
		if (!open || open.table !== t) {
			if (flow.length) out.push({ kind: 'flow', segs: flow });
			flow = [];
			open = {
				kind: 'table',
				table: t,
				cells: t.rows.map((r) => r.cells.map(() => [] as ContentSegment[]))
			};
			out.push(open);
		}
		// Inside the grid: keep runs that start in a cell; separators drop.
		t.rows.forEach((r, ri) =>
			r.cells.forEach((c, ci) => {
				if (seg.srcStart >= c.start && seg.srcStart < c.end) open!.cells[ri][ci].push(seg);
			})
		);
	}
	if (flow.length) out.push({ kind: 'flow', segs: flow });
	return out;
}
