// Grouping for the mobile annotations list — pure, no Svelte, no IO.
//
// The phone has no canvas: `AnnotationsList` renders one flat scroll of
// group headers (`.grp`) and 44 px rows (`.acard.m`). The groups follow
// the same ordering rules the desktop `layoutCards` lanes use (contract:
// docs/annotations-table-contract.org, "Layout rules"), but are computed
// here from the cards + sections + author bars directly so the list never
// depends on the desktop layout's geometry.

import type { TableCard, Section, LayoutMode } from './table-model';

export interface ListGroup {
	key: string;
	label: string;
	cards: TableCard[];
	/** Section groups: the section coordinate (or the root when whole-publication). */
	parent?: string;
}

/** The three modes the phone offers. `free` (drag-to-pin) and
 *  `collective` (a 2-D grid) have no list rendering; a view arriving in
 *  either is shown as `section`. */
export type ListMode = Extract<LayoutMode, 'section' | 'author' | 'timeline'>;

export function listMode(mode: LayoutMode): ListMode {
	return mode === 'author' || mode === 'timeline' ? mode : 'section';
}

const PLACEMENT_RANK: Record<TableCard['placement'], number> = { before: 1, annotation: 2, after: 3 };

/** requests, before, attached, after, then by time — the section-lane order. */
function byPlacementThenTime(a: TableCard, b: TableCard): number {
	const ra = a.kind === 'request' ? 0 : PLACEMENT_RANK[a.placement];
	const rb = b.kind === 'request' ? 0 : PLACEMENT_RANK[b.placement];
	if (ra !== rb) return ra - rb;
	return a.createdAt - b.createdAt;
}

export function groupCards(
	cards: TableCard[],
	sections: Section[],
	bars: { pubkey: string; label: string; count: number }[],
	mode: LayoutMode
): ListGroup[] {
	switch (listMode(mode)) {
		case 'author':
			return groupByAuthor(cards, sections, bars);
		case 'timeline':
			return groupByDay(cards);
		default:
			return groupBySection(cards, sections);
	}
}

/** One group per TOC row in reading order, plus a leading "whole
 *  publication" group when any card anchors to something outside the
 *  TOC (the root). Empty groups are dropped — a list has no lanes to
 *  keep. */
export function groupBySection(cards: TableCard[], sections: Section[]): ListGroup[] {
	const index = new Map<string, number>();
	sections.forEach((s, i) => index.set(s.coordinate, i));
	const buckets: TableCard[][] = sections.map(() => []);
	const orphans: TableCard[] = [];
	for (const c of cards) {
		const i = index.get(c.parent);
		if (i === undefined) orphans.push(c);
		else buckets[i].push(c);
	}
	const out: ListGroup[] = [];
	if (orphans.length) {
		out.push({
			key: 'root',
			label: 'whole publication',
			cards: orphans.sort(byPlacementThenTime),
			parent: orphans[0].parent
		});
	}
	sections.forEach((s, i) => {
		if (!buckets[i].length) return;
		out.push({
			key: `sec:${s.coordinate}`,
			label: `§${s.n} · ${s.title || '(untitled)'}`,
			cards: buckets[i].sort(byPlacementThenTime),
			parent: s.coordinate
		});
	});
	return out;
}

/** One group per author in `authorBars` order (descending count); authors
 *  the bars don't know (a card loaded after the histogram) follow, by
 *  their own count. Cards by section, then time. */
export function groupByAuthor(
	cards: TableCard[],
	sections: Section[],
	bars: { pubkey: string; label: string; count: number }[]
): ListGroup[] {
	const secIndex = new Map<string, number>();
	sections.forEach((s, i) => secIndex.set(s.coordinate, i));
	const bySection = (a: TableCard, b: TableCard) => {
		const ia = secIndex.get(a.parent) ?? -1;
		const ib = secIndex.get(b.parent) ?? -1;
		if (ia !== ib) return ia - ib;
		return a.createdAt - b.createdAt;
	};
	const buckets = new Map<string, TableCard[]>();
	for (const c of cards) {
		const list = buckets.get(c.author);
		if (list) list.push(c);
		else buckets.set(c.author, [c]);
	}
	const out: ListGroup[] = [];
	const seen = new Set<string>();
	for (const b of bars) {
		const list = buckets.get(b.pubkey);
		seen.add(b.pubkey);
		if (!list?.length) continue;
		out.push({ key: `au:${b.pubkey}`, label: b.label, cards: list.sort(bySection) });
	}
	const rest = [...buckets.entries()]
		.filter(([pk]) => !seen.has(pk))
		.sort((x, y) => y[1].length - x[1].length);
	for (const [pk, list] of rest) {
		out.push({ key: `au:${pk}`, label: authorFallback(pk), cards: list.sort(bySection) });
	}
	return out;
}

/** One group per calendar day (local time), newest day first, newest
 *  card first within the day. */
export function groupByDay(cards: TableCard[], now: number = Date.now()): ListGroup[] {
	const buckets = new Map<string, TableCard[]>();
	for (const c of cards) {
		const k = dayKey(c.createdAt);
		const list = buckets.get(k);
		if (list) list.push(c);
		else buckets.set(k, [c]);
	}
	return [...buckets.entries()]
		.sort((x, y) => (x[0] < y[0] ? 1 : x[0] > y[0] ? -1 : 0))
		.map(([k, list]) => ({
			key: `day:${k}`,
			label: dayLabel(list[0].createdAt, now),
			cards: list.sort((a, b) => b.createdAt - a.createdAt)
		}));
}

function dayKey(ts: number): string {
	const d = new Date(ts * 1000);
	const m = String(d.getMonth() + 1).padStart(2, '0');
	const day = String(d.getDate()).padStart(2, '0');
	return `${d.getFullYear()}-${m}-${day}`;
}

function dayLabel(ts: number, now: number): string {
	const today = dayKey(Math.floor(now / 1000));
	const yesterday = dayKey(Math.floor(now / 1000) - 86400);
	const k = dayKey(ts);
	if (k === today) return 'today';
	if (k === yesterday) return 'yesterday';
	const d = new Date(ts * 1000);
	const sameYear = d.getFullYear() === new Date(now).getFullYear();
	return d.toLocaleDateString(undefined, sameYear
		? { month: 'short', day: 'numeric' }
		: { year: 'numeric', month: 'short', day: 'numeric' });
}

export function authorFallback(pubkey: string): string {
	return pubkey ? pubkey.slice(0, 8) + '…' : 'unsigned';
}

/** "§n" for a card's parent, or the root glyph when it anchors to the
 *  whole publication. */
export function anchorLabel(card: TableCard, sections: Section[]): string {
	const s = sections.find((x) => x.coordinate === card.parent);
	return s ? `§${s.n}` : '⌂';
}

export function relTime(ts: number, now: number = Date.now()): string {
	const s = Math.max(0, Math.floor(now / 1000 - ts));
	if (s < 60) return 'now';
	if (s < 3600) return `${Math.floor(s / 60)}m`;
	if (s < 86400) return `${Math.floor(s / 3600)}h`;
	if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
	return new Date(ts * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
