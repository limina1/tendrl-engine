// The annotations table's pure model — no Svelte, no IO. Types and
// signatures are the Track 0 contract every parallel track builds against
// (docs/annotations-table-contract.org); the layout bodies are Track C's.
//
// What is engine-side and therefore NOT here: which 30041s anchor to a
// root, their anchors, roles and request pointers (`insertions/load`), the
// author/parent histograms (`counts`), and the spell text the engine
// returns as `query`. This module only arranges what came back.

import type { LoadedInsertion, InsertionCounts, AnnotationRequest } from '$lib/api';

// ---------------------------------------------------------------- ids

/** `annotations:<root coordinate>` — the root is `30040:<pk>:<d>`, and the
 *  d-tag may itself contain ':', so it is the trailing segment. */
export function annotationsBufferId(root: string): string {
	return `annotations:${root}`;
}

export function rootFromAnnotationsId(id: string): string | null {
	const m = id.match(/^annotations:(30040:[0-9a-f]{64}:.+)$/);
	return m ? m[1] : null;
}

/** The reader's id is `reader:30040:<pk>:<d>`; other reader ids (an event
 *  reader, a draft) have no publication root. */
export function rootFromReaderId(id: string): string | null {
	const m = id.match(/^reader:(30040:[0-9a-f]{64}:.+)$/);
	return m ? m[1] : null;
}

// ---------------------------------------------------------------- inputs

export type LayoutMode = 'section' | 'author' | 'timeline' | 'free' | 'collective';

/** One TOC row from the reader's flattened publication tree, in reading
 *  order. `coordinate` is the section's `30041:<pk>:<d>` (or the nested
 *  index's `30040:…`); `n` is its 1-based ordinal for the `§n` chip. */
export interface Section {
	coordinate: string;
	title: string;
	depth: number;
	n: number;
}

/** A card on the table: a loaded note, or an annotation request (from
 *  the merged request list). Everything the layouts and the card need,
 *  flattened once by `buildCards`. */
export interface TableCard {
	id: string;
	kind: 'note' | 'request';
	/** Hex pubkey of the signer; `''` for an unsigned local request. */
	author: string;
	/** Parent coordinate (section or root) the card anchors to. */
	parent: string;
	placement: 'annotation' | 'before' | 'after';
	title: string;
	/** Plain-text excerpt for the card body (RichContent renders the full
	 *  note when expanded). */
	excerpt: string;
	createdAt: number;
	style?: string;
	/** Notes: the request coordinate they answer. */
	request?: string;
	/** Requests: derived status. */
	status?: 'open' | 'answered';
	/** Back-references so the card can open the event / request. */
	note?: LoadedInsertion;
	req?: AnnotationRequest;
}

// ---------------------------------------------------------------- layout

export interface Lane {
	key: string;
	label: string;
	y: number;
	h: number;
	/** For the section layout: the section this lane is (hover lights it). */
	parent?: string;
}

export interface Placed {
	card: TableCard;
	x: number;
	y: number;
	w: number;
	h: number;
	lane: string;
	pinned?: boolean;
}

export interface Tick {
	x: number;
	label: string;
}

export interface LayoutResult {
	mode: LayoutMode;
	lanes: Lane[];
	placed: Placed[];
	/** Timeline / collective axis marks. */
	ticks: Tick[];
	width: number;
	height: number;
}

/** Card metrics (CSS px). The reader-side DOM contract names the
 *  matching classes: `.acard`, `.acard.m` (small), `.acard.x` (expanded). */
export const SIZES = {
	W: 200,
	WS: 150,
	WX: 330,
	H: 96,
	HS: 64,
	HX: 220,
	GAP: 12,
	PAD: 16,
	LANE_HEAD: 26,
	AXIS: 120
} as const;

/** Per-buffer view state, parked in `store.bufferState` on teardown and
 *  read once on mount. Pins are local view state — never published. */
export interface AnnotationsViewState {
	mode: LayoutMode;
	/** Selected `by:` pubkeys; empty = all. */
	by: string[];
	/** Days back, or null for all time. */
	sinceDays: number | null;
	cursor: number;
	expanded: string | null;
	pins: Record<string, { x: number; y: number }>;
	scroll: { x: number; y: number };
}

export const DEFAULT_VIEW: AnnotationsViewState = {
	mode: 'section',
	by: [],
	sinceDays: null,
	cursor: 0,
	expanded: null,
	pins: {},
	scroll: { x: 0, y: 0 }
};

// ---------------------------------------------------------------- functions
// Track C implements these; the signatures are frozen here so Tracks B/D
// can code against them. Each is a pure function of its arguments.

/** Reading-order rows from the reader's addr-keyed publication tree. The
 *  input shape is whatever `ReaderBuffer` already accumulates from
 *  `stream_publication_tree`; Track C adapts it. */
export function flattenToc(
	tree: { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null; depth: number }[]
): Section[] {
	return tree.map((row, i) => ({
		coordinate: `${row.addr.kind}:${row.addr.pubkey}:${row.addr.d_tag}`,
		title: (row.title ?? '').trim() || row.addr.d_tag,
		depth: row.depth,
		n: i + 1
	}));
}

/** Flatten loaded insertions (notes only — `role === 'request'` are drawn
 *  from `requests`) and the merged request list into cards. */
export function buildCards(
	insertions: LoadedInsertion[],
	requests: AnnotationRequest[],
	opts: { excerptChars?: number } = {}
): TableCard[] {
	const chars = opts.excerptChars ?? 240;
	const out: TableCard[] = [];
	for (const ins of insertions) {
		if (ins.role === 'request') continue;
		const ev = ins.event;
		out.push({
			id: ins.coordinate,
			kind: 'note',
			author: ev.pubkey,
			parent: ins.anchor.parent,
			placement: ins.anchor.placement,
			title: tagValue(ev.tags, 'title') ?? firstLine(ev.content) ?? '(untitled)',
			excerpt: excerpt(ev.content, chars),
			createdAt: ev.created_at,
			style: tagValue(ev.tags, 'style'),
			request: ins.request,
			note: ins
		});
	}
	for (const r of requests) {
		out.push({
			id: r.coordinate ?? r.id,
			kind: 'request',
			author: r.coordinate ? (r.coordinate.split(':')[1] ?? '') : '',
			parent: r.parent,
			placement: 'annotation',
			title: r.title || firstLine(r.task) || '(request)',
			excerpt: excerpt(r.task, chars),
			createdAt: r.created_at,
			style: r.style,
			status: r.status,
			req: r
		});
	}
	return out;
}

function tagValue(tags: string[][], name: string): string | undefined {
	const t = tags.find((t) => t[0] === name && t[1]);
	return t ? t[1] : undefined;
}
function firstLine(s: string): string | undefined {
	const l = s.split('\n').map((x) => x.replace(/^[#*>\s]+/, '').trim()).find((x) => x.length > 0);
	return l ? l.slice(0, 120) : undefined;
}
function excerpt(s: string, chars: number): string {
	const flat = s.replace(/\s+/g, ' ').trim();
	return flat.length > chars ? flat.slice(0, chars - 1) + '…' : flat;
}

/** Arrange cards for a mode. `pins` only apply in `free`; the drag
 *  transient is NOT an input — the buffer moves the DOM node during a drag
 *  and commits to `pins` on release, so the derived layout never re-runs
 *  mid-drag. `width` is the table viewport width for packing. */
export function layoutCards(
	cards: TableCard[],
	sections: Section[],
	mode: LayoutMode,
	view: Pick<AnnotationsViewState, 'expanded' | 'pins'>,
	width: number,
	authorLabel?: (pubkey: string) => string
): LayoutResult {
	const { W, WS, WX, H, HS, HX, GAP, PAD, LANE_HEAD, AXIS } = SIZES;
	const small = mode === 'timeline' || mode === 'collective';
	const cardW = (c: TableCard) => (view.expanded === c.id ? WX : small ? WS : W);
	const cardH = (c: TableCard) => (view.expanded === c.id ? HX : small ? HS : H);
	const label = (pk: string) => (pk ? (authorLabel?.(pk) ?? pk.slice(0, 8) + '…') : 'unsigned');
	const lanes: Lane[] = [];
	const placed: Placed[] = [];
	const ticks: Tick[] = [];
	let y = 0;
	let cvWidth = Math.max(0, width);

	const secIndex = new Map<string, number>();
	sections.forEach((s, i) => secIndex.set(s.coordinate, i));
	const secOf = (c: TableCard) => secIndex.get(c.parent) ?? -1;
	const byTime = (a: TableCard, b: TableCard) => a.createdAt - b.createdAt;
	const inLane = (a: TableCard, b: TableCard) => ord(a) - ord(b) || byTime(a, b);
	const bySection = (a: TableCard, b: TableCard) => secOf(a) - secOf(b) || byTime(a, b);

	// Lane packing: left-to-right in as many columns as fit, then down.
	const pack = (key: string, lab: string, list: TableCard[], parent?: string) => {
		const cols = Math.max(1, Math.floor((cvWidth - 2 * PAD + GAP) / (W + GAP)));
		const lane: Lane = { key, label: `${lab} · ${list.length}`, y, h: 0, parent };
		lanes.push(lane);
		let maxY = y + LANE_HEAD;
		list.forEach((c, k) => {
			const x = PAD + (k % cols) * (W + GAP);
			const cy = y + LANE_HEAD + Math.floor(k / cols) * (H + GAP);
			placed.push({ card: c, x, y: cy, w: cardW(c), h: cardH(c), lane: key });
			maxY = Math.max(maxY, cy + cardH(c));
		});
		lane.h = list.length ? maxY - y + GAP : LANE_HEAD + 6;
		y += lane.h;
	};

	// Authors by descending count among the cards (the bars' order), ties
	// by first appearance.
	const authorOrder = () => {
		const counts = new Map<string, number>();
		for (const c of cards) counts.set(c.author, (counts.get(c.author) ?? 0) + 1);
		return [...counts.entries()].sort((a, b) => b[1] - a[1]).map(([pk]) => pk);
	};

	if (mode === 'section' || mode === 'free') {
		const rootCards = cards.filter((c) => secOf(c) < 0).sort(inLane);
		if (rootCards.length) pack('root', '§ · whole publication', rootCards, rootCards[0].parent);
		sections.forEach((s, i) => {
			pack(`sec:${s.coordinate}`, `§${s.n} ${s.title}`, cards.filter((c) => secOf(c) === i).sort(inLane), s.coordinate);
		});
	} else if (mode === 'author') {
		for (const pk of authorOrder()) {
			pack(`by:${pk}`, label(pk), cards.filter((c) => c.author === pk).sort(bySection));
		}
	} else if (mode === 'timeline') {
		const sorted = [...cards].sort(byTime);
		if (sorted.length) {
			const tMin = sorted[0].createdAt - 3600;
			const tMax = sorted[sorted.length - 1].createdAt + 3600;
			cvWidth = Math.max(cvWidth, sorted.length * 70);
			const span = Math.max(1, tMax - tMin);
			const usable = Math.max(0, cvWidth - WS - 2 * PAD);
			const xOf = (t: number) => PAD + ((t - tMin) / span) * usable;
			const rows: number[] = []; // right edge per row
			for (const c of sorted) {
				const x = xOf(c.createdAt);
				let r = rows.findIndex((edge) => edge + GAP <= x);
				if (r < 0) {
					r = rows.length;
					rows.push(0);
				}
				rows[r] = x + cardW(c);
				placed.push({ card: c, x, y: PAD + 22 + r * (HS + GAP), w: cardW(c), h: cardH(c), lane: 'time' });
			}
			y = PAD + 22 + rows.length * (HS + GAP) + PAD;
			const days = Math.max(1, Math.round(span / 86400));
			const step = span / days;
			for (let d = 0; d <= days; d++) {
				ticks.push({ x: xOf(tMin + d * step), label: d === days ? 'now' : `-${days - d}d` });
			}
		}
	} else if (mode === 'collective') {
		// Columns = sections along the axis; a leading column holds cards
		// anchored to the root itself when there are any.
		const lead = cards.some((c) => secOf(c) < 0) ? 1 : 0;
		const colX = (i: number) => AXIS + (i + lead) * (WS + GAP);
		cvWidth = Math.max(cvWidth, AXIS + (sections.length + lead) * (WS + GAP) + PAD);
		y = 22;
		for (const pk of authorOrder()) {
			const mine = cards.filter((c) => c.author === pk);
			const lane: Lane = { key: `by:${pk}`, label: `${label(pk)} · ${mine.length}`, y, h: 0 };
			lanes.push(lane);
			let maxY = y + LANE_HEAD;
			const cell = (list: TableCard[], cx: number) => {
				list.sort(byTime).forEach((c, k) => {
					const cy = y + LANE_HEAD + k * 22;
					placed.push({ card: c, x: cx, y: cy, w: cardW(c), h: cardH(c), lane: lane.key });
					maxY = Math.max(maxY, cy + cardH(c));
				});
			};
			if (lead) cell(mine.filter((c) => secOf(c) < 0), AXIS);
			sections.forEach((_, i) => cell(mine.filter((c) => secOf(c) === i), colX(i)));
			lane.h = maxY - y + GAP;
			y += lane.h;
		}
		if (lead) ticks.push({ x: AXIS, label: '§' });
		sections.forEach((s, i) => ticks.push({ x: colX(i), label: `§${s.n}` }));
	}

	if (mode === 'free') {
		for (const p of placed) {
			const pin = view.pins[p.card.id];
			if (pin) {
				p.x = pin.x;
				p.y = pin.y;
				p.pinned = true;
			}
		}
	}
	// Expanded cards and pins may reach past the packed bounds.
	for (const p of placed) {
		y = Math.max(y, p.y + p.h + PAD);
		cvWidth = Math.max(cvWidth, p.x + p.w + PAD);
	}
	return { mode, lanes, placed, ticks, width: cvWidth, height: Math.max(y, 200) };
}

/** Sort key within a section lane: requests, before, attached, after. */
function ord(c: TableCard): number {
	if (c.kind === 'request') return -1;
	return c.placement === 'before' ? 0 : c.placement === 'annotation' ? 1 : 2;
}

/** The table's query as the search DSL: `A:<root> k:30041 [by:…] [since:…]
 *  count:by count:a` — what the wand saves as a spell. Mirrors the engine's
 *  `query` string plus the table's own filters. */
export function buildSpell(
	root: string,
	view: Pick<AnnotationsViewState, 'by' | 'sinceDays'>,
	now: number = Math.floor(Date.now() / 1000)
): string {
	const parts = [`A:${root}`, 'k:30041'];
	for (const pk of view.by) parts.push(`by:${pk}`);
	if (view.sinceDays != null) parts.push(`since:${now - view.sinceDays * 86400}`);
	parts.push('count:by', 'count:a');
	return parts.join(' ');
}

/** Inverse of `buildSpell` for a hand-edited spell input: the filters the
 *  table understands, or null when the text names another root. */
export function parseSpell(
	text: string,
	root: string,
	now: number = Math.floor(Date.now() / 1000)
): Pick<AnnotationsViewState, 'by' | 'sinceDays'> | null {
	const toks = text.trim().split(/\s+/).filter(Boolean);
	const a = toks.find((t) => t.startsWith('A:'));
	if (!a || a.slice(2) !== root) return null;
	const by = toks.filter((t) => t.startsWith('by:')).map((t) => t.slice(3)).filter(Boolean);
	const since = toks.find((t) => t.startsWith('since:'));
	let sinceDays: number | null = null;
	if (since) {
		const ts = Number(since.slice(6));
		if (Number.isFinite(ts) && ts > 0) sinceDays = Math.max(1, Math.round((now - ts) / 86400));
	}
	return { by, sinceDays };
}

/** Author bars from the engine's histogram: sorted by count, `unsigned`
 *  appended when local unsigned requests exist. */
export function authorBars(
	counts: InsertionCounts | undefined,
	unsignedRequests: number
): { pubkey: string; label: string; count: number }[] {
	const bars = (counts?.by_author ?? [])
		.map((b) => ({ pubkey: b.value, label: b.label ?? b.value.slice(0, 8) + '…', count: b.count }))
		.sort((x, y) => y.count - x.count);
	if (unsignedRequests > 0) bars.push({ pubkey: '', label: 'unsigned', count: unsignedRequests });
	return bars;
}
