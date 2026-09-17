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
export declare function flattenToc(
	tree: { addr: { kind: number; pubkey: string; d_tag: string }; title: string; depth: number }[]
): Section[];

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
export declare function layoutCards(
	cards: TableCard[],
	sections: Section[],
	mode: LayoutMode,
	view: Pick<AnnotationsViewState, 'expanded' | 'pins'>,
	width: number,
	authorLabel?: (pubkey: string) => string
): LayoutResult;

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
