// The annotations ledger's pure model — no Svelte, no IO. Types and
// functions per docs/annotations-ledger-contract.org ("Web model"). What
// is engine-side and therefore NOT here: which 30041s anchor to a root,
// their anchors, roles, request pointers, reply pointers and reply
// counts (`insertions/load`), the author histogram (`counts`), and the
// spell text the engine returns as `query`. This module only arranges
// what came back into rows, groups and facet counts.
import type { LoadedInsertion, AnnotationRequest } from '$lib/api';

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
export type LedgerKind = 'note' | 'def' | 'request' | 'mr' | 'kasten';
export type GroupBy = 'section' | 'author' | 'kind' | 'time';
export const GROUPS: GroupBy[] = ['section', 'author', 'kind', 'time'];
export const KINDS: LedgerKind[] = ['note', 'def', 'request', 'mr', 'kasten'];
/** Days the "when" sparkline covers (newest on the right). */
export const SPARK_DAYS = 21;

/** One TOC row from the reader's flattened publication tree, in reading
 *  order. `coordinate` is the section's `30041:<pk>:<d>` (or the nested
 *  index's `30040:…`); `n` is its 1-based ordinal for the `§n` cell. */
export interface Section {
	coordinate: string;
	title: string;
	depth: number;
	n: number;
}

export interface LedgerRow {
	/** Coordinate, `818:<id>` (a merge request from `insertions/load`) or
	 *  a request id (an unsigned local record). */
	id: string;
	kind: LedgerKind;
	/** Hex pubkey of the signer; `''` for an unsigned local request. */
	author: string;
	/** Section coordinate, or the root. */
	parent: string;
	/** -1 = whole publication, else the 0-based TOC ordinal. */
	sec: number;
	placement: 'annotation' | 'before' | 'after';
	title: string;
	/** Plain text for the board excerpt. */
	body: string;
	createdAt: number;
	status?: 'open' | 'answered' | 'merged' | 'declined';
	output?: 'notes' | 'kasten' | 'vocabulary' | 'ask';
	term?: string;
	glossary?: boolean;
	style?: string;
	/** The request this answers → nests under it (section grouping). */
	request?: string;
	/** A note in reply → "↩ <title>" on the row. */
	replyTo?: string;
	/** kind-1111 count from the `replies` map (0 when unknown). */
	replies: number;
	note?: LoadedInsertion;
	req?: AnnotationRequest;
}

export interface LedgerView {
	view: 'rows' | 'board';
	group: GroupBy;
	sort: 'reading' | 'newest';
	detail: 'inline' | 'pane';
	by: string[];
	secs: number[];
	kinds: LedgerKind[];
	sinceDays: number | null;
	/** N days ago (a sparkline pick), or null. */
	day: number | null;
	cursor: number;
	expanded: string | null;
	collapsed: string[];
	scroll: number;
}

export const DEFAULT_VIEW: LedgerView = {
	view: 'rows',
	group: 'section',
	sort: 'reading',
	detail: 'inline',
	by: [],
	secs: [],
	kinds: [],
	sinceDays: null,
	day: null,
	cursor: 0,
	expanded: null,
	collapsed: [],
	scroll: 0
};

export interface LedgerGroup {
	key: string;
	label: string;
	/** Section groups: -1 = whole publication. */
	sec?: number;
	/** Author groups: the pubkey. */
	author?: string;
	rows: LedgerRow[];
}

export interface FacetCounts {
	by: Record<string, number>;
	sec: Record<number, number>;
	kind: Record<LedgerKind, number>;
	/** Index = days ago (0 = today), length SPARK_DAYS. */
	day: number[];
}

// ---------------------------------------------------------------- rows
/** Reading-order rows from the reader's addr-keyed publication tree. */
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

function tagValue(tags: string[][], name: string): string | undefined {
	const t = tags.find((t) => t[0] === name && t[1]);
	return t ? t[1] : undefined;
}
function firstLine(s: string): string | undefined {
	const l = s.split('\n').map((x) => x.replace(/^[#*>\s]+/, '').trim()).find((x) => x.length > 0);
	return l ? l.slice(0, 120) : undefined;
}
function plain(s: string, chars = 400): string {
	const flat = s.replace(/\s+/g, ' ').trim();
	return flat.length > chars ? flat.slice(0, chars - 1) + '…' : flat;
}

/** Flatten loaded insertions (notes, definitions, bundles, 818 asks —
 *  `role === 'request'` rows are drawn from `requests`) and the merged
 *  request list into rows. `replies` is the engine's per-coordinate
 *  kind-1111 count. */
export function buildRows(
	insertions: LoadedInsertion[],
	requests: AnnotationRequest[],
	sections: Section[],
	replies: Record<string, number> = {}
): LedgerRow[] {
	const secIndex = new Map<string, number>();
	sections.forEach((s, i) => secIndex.set(s.coordinate, i));
	const secOf = (parent: string) => secIndex.get(parent) ?? -1;
	const out: LedgerRow[] = [];
	for (const ins of insertions) {
		if (ins.role === 'request') continue;
		const ev = ins.event;
		const kind: LedgerKind =
			ins.role === 'definition' ? 'def' : ins.role === 'bundle' ? 'kasten' : ins.role === 'ask' ? 'mr' : 'note';
		const title =
			kind === 'mr'
				? plain(ins.merge_request?.content ?? ev.content, 140) || 'merge request'
				: (tagValue(ev.tags, 'title') ?? firstLine(ev.content) ?? '(untitled)');
		out.push({
			id: ins.coordinate,
			kind,
			author: kind === 'mr' ? (ins.merge_request?.pubkey ?? ev.pubkey) : ev.pubkey,
			parent: ins.anchor.parent,
			sec: secOf(ins.anchor.parent),
			placement: ins.anchor.placement,
			title,
			body: plain(ev.content),
			createdAt: ev.created_at,
			status: kind === 'mr' ? (ins.merge_request?.status ?? 'open') : undefined,
			term: ins.term,
			glossary: ins.promoted,
			style: tagValue(ev.tags, 'style'),
			request: ins.request,
			replyTo: ins.reply_to,
			replies: replies[ins.coordinate] ?? 0,
			note: ins
		});
	}
	for (const r of requests) {
		const id = r.coordinate ?? r.id;
		out.push({
			id,
			kind: 'request',
			author: r.coordinate ? (r.coordinate.split(':')[1] ?? '') : '',
			parent: r.parent,
			sec: secOf(r.parent),
			placement: 'annotation',
			title: r.title || firstLine(r.task) || '(request)',
			body: plain(r.task),
			createdAt: r.created_at,
			status: r.status,
			output: r.output,
			style: r.style,
			replies: replies[id] ?? 0,
			req: r
		});
	}
	return out;
}

// ---------------------------------------------------------------- filters
const DAY = 86400;

function inFacets(row: LedgerRow, view: LedgerView, now: number, skip?: 'by' | 'sec' | 'kind' | 'time'): boolean {
	if (skip !== 'by' && view.by.length && !view.by.includes(row.author)) return false;
	if (skip !== 'sec' && view.secs.length && !view.secs.includes(row.sec)) return false;
	if (skip !== 'kind' && view.kinds.length && !view.kinds.includes(row.kind)) return false;
	if (skip !== 'time') {
		if (view.sinceDays != null && row.createdAt < now - view.sinceDays * DAY) return false;
		if (view.day != null && daysAgo(row.createdAt, now) !== view.day) return false;
	}
	return true;
}

export function daysAgo(ts: number, now: number): number {
	return Math.max(0, Math.floor((now - ts) / DAY));
}

/** The rows every facet admits. */
export function filterRows(rows: LedgerRow[], view: LedgerView, now = Math.floor(Date.now() / 1000)): LedgerRow[] {
	return rows.filter((r) => inFacets(r, view, now));
}

/** Counts for each facet over the rows the *other* facets admit, so a
 *  selected author still shows the other authors' counts (the bars) and a
 *  narrowed day still shows the sparkline. */
export function facetCounts(rows: LedgerRow[], view: LedgerView, now = Math.floor(Date.now() / 1000)): FacetCounts {
	const c: FacetCounts = {
		by: {},
		sec: {},
		kind: { note: 0, def: 0, request: 0, mr: 0, kasten: 0 },
		day: Array.from({ length: SPARK_DAYS }, () => 0)
	};
	for (const r of rows) {
		if (inFacets(r, view, now, 'by')) c.by[r.author] = (c.by[r.author] ?? 0) + 1;
		if (inFacets(r, view, now, 'sec')) c.sec[r.sec] = (c.sec[r.sec] ?? 0) + 1;
		if (inFacets(r, view, now, 'kind')) c.kind[r.kind] += 1;
		if (inFacets(r, view, now, 'time')) {
			const d = daysAgo(r.createdAt, now);
			if (d < SPARK_DAYS) c.day[d] += 1;
		}
	}
	return c;
}

// ---------------------------------------------------------------- grouping
const rank = (r: LedgerRow): number =>
	r.kind === 'request' ? 0 : r.kind === 'mr' ? 1 : r.placement === 'before' ? 2 : r.placement === 'after' ? 4 : 3;

export const KIND_LABELS: Record<LedgerKind, string> = {
	request: 'requests',
	mr: 'merge requests',
	def: 'definitions',
	kasten: 'kasten',
	note: 'notes'
};
const KIND_ORDER: LedgerKind[] = ['request', 'mr', 'def', 'kasten', 'note'];
const TIME_LABELS = ['today', 'this week', 'this month', 'earlier'];

export function sectionLabel(sec: number, sections: Section[]): string {
	if (sec < 0) return 'whole publication';
	const s = sections[sec];
	return s ? `§${s.n} · ${s.title}` : `§${sec + 1}`;
}

/** Group the (already filtered) rows per the contract's rules. In section
 *  grouping a request's answers are not top-level rows (they nest under
 *  their request); everywhere else every row is top-level. Empty groups
 *  are dropped. */
export function groupRows(
	rows: LedgerRow[],
	sections: Section[],
	view: LedgerView,
	authorLabel: (pk: string) => string,
	now = Math.floor(Date.now() / 1000)
): LedgerGroup[] {
	const byTime = (a: LedgerRow, b: LedgerRow) => a.createdAt - b.createdAt;
	const reading = (a: LedgerRow, b: LedgerRow) => a.sec - b.sec || rank(a) - rank(b) || byTime(a, b);
	const sortFn = view.sort === 'newest' ? (a: LedgerRow, b: LedgerRow) => b.createdAt - a.createdAt : reading;
	let groups: LedgerGroup[] = [];
	if (view.group === 'section') {
		const top = rows.filter((r) => !r.request || !rows.some((q) => q.id === r.request));
		const lanes = [-1, ...sections.map((_, i) => i)];
		groups = lanes.map((sec) => ({
			key: `s${sec}`,
			label: sectionLabel(sec, sections),
			sec,
			rows: top.filter((r) => r.sec === sec).sort(sortFn)
		}));
	} else if (view.group === 'author') {
		const counts = new Map<string, number>();
		for (const r of rows) counts.set(r.author, (counts.get(r.author) ?? 0) + 1);
		groups = [...counts.entries()]
			.sort((a, b) => b[1] - a[1])
			.map(([pk]) => ({
				key: `a${pk}`,
				label: pk ? authorLabel(pk) : 'unsigned',
				author: pk,
				rows: rows.filter((r) => r.author === pk).sort(sortFn)
			}));
	} else if (view.group === 'kind') {
		groups = KIND_ORDER.map((k) => ({
			key: `k${k}`,
			label: KIND_LABELS[k],
			rows: rows.filter((r) => r.kind === k).sort(sortFn)
		}));
	} else {
		const bucket = (r: LedgerRow) => {
			const d = (now - r.createdAt) / DAY;
			return d < 1 ? 0 : d < 7 ? 1 : d < 30 ? 2 : 3;
		};
		groups = TIME_LABELS.map((label, b) => ({
			key: `t${b}`,
			label,
			rows: rows.filter((r) => bucket(r) === b).sort((a, c) => c.createdAt - a.createdAt)
		}));
	}
	return groups.filter((g) => g.rows.length > 0);
}

/** The answers nested under a request in section grouping, in time order. */
export function answersOf(rows: LedgerRow[], requestId: string): LedgerRow[] {
	return rows.filter((r) => r.request === requestId).sort((a, b) => a.createdAt - b.createdAt);
}

/** The cursor's order: group by group (collapsed groups skipped), a
 *  request's answers right after it in section grouping. */
export function flatOrder(groups: LedgerGroup[], rows: LedgerRow[], view: LedgerView): LedgerRow[] {
	const out: LedgerRow[] = [];
	const collapsed = new Set(view.collapsed);
	for (const g of groups) {
		if (collapsed.has(g.key)) continue;
		for (const r of g.rows) {
			out.push(r);
			if (view.group === 'section' && r.kind === 'request') out.push(...answersOf(rows, r.id));
		}
	}
	return out;
}

// ---------------------------------------------------------------- spell
/** The ledger's query as the search DSL — what the wand saves as a spell:
 *  `A:<root> k:30041 [by:…] [a:<section>] [t:definition | t:annotation-
 *  request | k:818 | k:30040 t:kasten | -t:definition -t:annotation-
 *  request] [since:…] count:by count:a`. */
export function buildSpell(
	root: string,
	view: Pick<LedgerView, 'by' | 'secs' | 'kinds' | 'sinceDays' | 'day'>,
	sections: Section[],
	now: number = Math.floor(Date.now() / 1000)
): string {
	const parts = [`A:${root}`, 'k:30041'];
	for (const pk of view.by) parts.push(`by:${pk}`);
	for (const s of view.secs) parts.push(s < 0 ? `a:${root}` : `a:${sections[s]?.coordinate ?? ''}`);
	for (const k of view.kinds) {
		if (k === 'def') parts.push('t:definition');
		else if (k === 'request') parts.push('t:annotation-request');
		else if (k === 'mr') parts.push('k:818');
		else if (k === 'kasten') parts.push('k:30040', 't:kasten');
		else parts.push('-t:definition', '-t:annotation-request');
	}
	if (view.sinceDays != null) parts.push(`since:${now - view.sinceDays * DAY}`);
	else if (view.day != null) parts.push(`since:${now - (view.day + 1) * DAY}`, `until:${now - view.day * DAY}`);
	parts.push('count:by', 'count:a');
	return parts.filter((p) => !p.endsWith(':')).join(' ');
}

/** Inverse of `buildSpell` for a hand-edited spell input: the facets the
 *  ledger understands, or null when the text names another root. */
export function parseSpell(
	text: string,
	root: string,
	sections: Section[],
	now: number = Math.floor(Date.now() / 1000)
): Pick<LedgerView, 'by' | 'secs' | 'kinds' | 'sinceDays' | 'day'> | null {
	const toks = text.trim().split(/\s+/).filter(Boolean);
	const a = toks.find((t) => t.startsWith('A:'));
	if (!a || a.slice(2) !== root) return null;
	const by = toks.filter((t) => t.startsWith('by:')).map((t) => t.slice(3)).filter(Boolean);
	const secs: number[] = [];
	for (const t of toks) {
		if (!t.startsWith('a:')) continue;
		const coord = t.slice(2);
		if (coord === root) secs.push(-1);
		else {
			const i = sections.findIndex((s) => s.coordinate === coord);
			if (i >= 0) secs.push(i);
		}
	}
	const kinds: LedgerKind[] = [];
	if (toks.includes('t:definition')) kinds.push('def');
	if (toks.includes('t:annotation-request')) kinds.push('request');
	if (toks.includes('k:818')) kinds.push('mr');
	if (toks.includes('t:kasten')) kinds.push('kasten');
	if (toks.includes('-t:definition') && toks.includes('-t:annotation-request')) kinds.push('note');
	const since = toks.find((t) => t.startsWith('since:'));
	const until = toks.find((t) => t.startsWith('until:'));
	let sinceDays: number | null = null;
	let day: number | null = null;
	if (since) {
		const ts = Number(since.slice(6));
		if (Number.isFinite(ts) && ts > 0) {
			if (until) day = Math.max(0, Math.round((now - Number(until.slice(6))) / DAY));
			else sinceDays = Math.max(1, Math.round((now - ts) / DAY));
		}
	}
	return { by, secs, kinds, sinceDays, day };
}

/** Relative age for a row: now · 5m · 3h · 2d · a date. */
export function relTime(ts: number, now = Math.floor(Date.now() / 1000)): string {
	const s = Math.max(0, now - ts);
	if (s < 60) return 'now';
	if (s < 3600) return `${Math.floor(s / 60)}m`;
	if (s < DAY) return `${Math.floor(s / 3600)}h`;
	if (s < DAY * 30) return `${Math.floor(s / DAY)}d`;
	return new Date(ts * 1000).toLocaleDateString();
}

/** A display name when no kind-0 is known: the pubkey's first 8 hex, or
 *  "unsigned" for a local record with no signer. */
export function authorFallback(pk: string): string {
	return pk ? pk.slice(0, 8) + '…' : 'unsigned';
}

/** One glyph per kind (the contract): ◂ note (▴ before · ▾ after) · def ·
 *  ?/✓ request · ⑂ merge · ⊞ kasten. `cls` is the colour class. */
export function glyphOf(row: LedgerRow): { g: string; cls: string } {
	if (row.kind === 'request') return row.status === 'open' ? { g: '?', cls: 'req' } : { g: '✓', cls: 'ok' };
	if (row.kind === 'def') return { g: 'def', cls: 'def' };
	if (row.kind === 'mr') return { g: '⑂', cls: row.status === 'open' ? 'mr' : row.status === 'merged' ? 'merged' : 'declined' };
	if (row.kind === 'kasten') return { g: '⊞', cls: 'kasten' };
	return { g: row.placement === 'before' ? '▴' : row.placement === 'after' ? '▾' : '◂', cls: 'note' };
}
