// NIP-22 thread builder. Pure logic — no engine, no DOM, no Svelte.
//
// Parent detection follows NIP-22:
//   - lowercase `e` tag      = immediate parent (the event being replied to)
//   - uppercase `E`/`A` tag  = root scope (the article / top of the tree)
//
// We only walk lowercase `e` tags for parent edges. An event whose
// lowercase `e` target is not in our event set is treated as a root
// (it might be a top-level comment, or its parent simply hasn't been
// ingested yet — either way, surfacing it at the top level is the
// least-lossy fallback).
//
// All ids are normalized to lowercase before matching because relays
// can return mixed-case ids and `id1.toLowerCase() === id2` is the
// most defensive comparison.

export interface NostrEvent {
	id: string;
	kind: number;
	pubkey: string;
	created_at: number;
	content: string;
	tags: string[][];
	sig?: string;
}

export interface ThreadNode {
	event: NostrEvent;
	children: ThreadNode[];
	/** 0 = top-level root, increments per nesting level. */
	depth: number;
}

/**
 * Build a NIP-22 thread tree from a flat list of events.
 *
 * Returns root-level nodes (top-level comments). Each node carries its
 * replies in `children`, sorted by `created_at` ascending. Roots
 * themselves are sorted ascending too.
 *
 * Mixed kinds are accepted but only events whose lowercase `e` tag
 * matches another event in the set will be threaded — typically you'll
 * pre-filter to `kind === 1111` before calling this.
 */
export function buildThread(events: NostrEvent[]): ThreadNode[] {
	if (events.length === 0) return [];

	// Dedup by lowercase id. Relays can echo the same event across
	// multiple sub IDs and we don't want it appearing twice in the tree.
	const byId = new Map<string, NostrEvent>();
	for (const ev of events) {
		const key = ev.id.toLowerCase();
		if (!byId.has(key)) byId.set(key, ev);
	}

	// Build the node table.
	const nodes = new Map<string, ThreadNode>();
	for (const ev of byId.values()) {
		nodes.set(ev.id.toLowerCase(), { event: ev, children: [], depth: 0 });
	}

	// Resolve each node's parent. First lowercase `e` tag pointing to a
	// known event in the set wins. We deliberately ignore `e` tags that
	// resolve outside the set — those reference siblings/parents we
	// haven't loaded yet, and pretending they're roots beats hiding them.
	const parentOf = new Map<string, string>();
	for (const node of nodes.values()) {
		const selfId = node.event.id.toLowerCase();
		for (const tag of node.event.tags) {
			if (!tag || tag.length < 2) continue;
			if (tag[0] !== 'e') continue;
			const candidate = String(tag[1] ?? '').toLowerCase();
			if (candidate === selfId) continue;
			if (nodes.has(candidate)) {
				parentOf.set(selfId, candidate);
				break;
			}
		}
	}

	// Stitch children under parents; collect orphans (no resolvable parent
	// edge) as roots.
	const roots: ThreadNode[] = [];
	for (const node of nodes.values()) {
		const parentId = parentOf.get(node.event.id.toLowerCase());
		if (parentId) {
			const parent = nodes.get(parentId);
			if (parent) {
				parent.children.push(node);
				continue;
			}
		}
		roots.push(node);
	}

	// Sort + assign depth. We walk the tree top-down so depth is correct
	// even when an entry is technically a "root" because its real parent
	// hasn't been ingested yet.
	function descend(node: ThreadNode, depth: number) {
		node.depth = depth;
		node.children.sort((a, b) => a.event.created_at - b.event.created_at);
		for (const child of node.children) descend(child, depth + 1);
	}
	roots.sort((a, b) => a.event.created_at - b.event.created_at);
	for (const root of roots) descend(root, 0);

	return roots;
}

/**
 * Pre-order traversal of a thread forest. Useful when rendering the
 * tree as a flat list with indentation derived from `node.depth`
 * (which a virtualized renderer can pair with `node.depth` for a
 * left-padding step).
 */
export function flattenThread(roots: ThreadNode[]): ThreadNode[] {
	const out: ThreadNode[] = [];
	function visit(node: ThreadNode) {
		out.push(node);
		for (const child of node.children) visit(child);
	}
	for (const root of roots) visit(root);
	return out;
}

/**
 * Whether any node in the forest matches the given event id (case-
 * insensitive). Used by the reader to auto-open a thread block when a
 * `?focus_comment=<id>` marker lands inside it.
 */
export function threadContainsId(roots: ThreadNode[], id: string | null): boolean {
	if (!id) return false;
	const target = id.toLowerCase();
	function visit(node: ThreadNode): boolean {
		if (node.event.id.toLowerCase() === target) return true;
		return node.children.some(visit);
	}
	return roots.some(visit);
}

/**
 * Count total events in a thread forest (roots + every descendant).
 */
export function countThread(roots: ThreadNode[]): number {
	let n = 0;
	function visit(node: ThreadNode) {
		n++;
		for (const child of node.children) visit(child);
	}
	for (const root of roots) visit(root);
	return n;
}
