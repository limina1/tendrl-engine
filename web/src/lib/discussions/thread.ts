// NIP-22 thread view helpers. Pure tree-walks over a `ThreadNode` forest —
// no engine, no DOM, no Svelte.
//
// The forest itself is BUILT engine-side now (`src/discussions.rs::build_thread`,
// returned by `POST /discussions/list` with `threaded: true`) per the
// frontend/backend boundary — the parent-resolution algorithm used to live here
// as `buildThread` and was moved to Rust. What remains is the depth→indent
// rendering support and a couple of interaction helpers (does the forest contain
// id X, how many nodes total) that operate on the already-built tree. The
// `ThreadNode` / `NostrEvent` types describe the JSON the engine returns.

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
