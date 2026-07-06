import type {
	Buffer,
	ClassName,
	LayoutConfig,
	OpenBuf,
	Position,
	Slot,
	SlotState,
	SplitNode
} from './types';

const POSITION_ORDER: Position[] = ['left', 'center', 'right'];

export type NavAction =
	| 'up'
	| 'down'
	| 'left'
	| 'right'
	| 'select'
	| 'back'
	| 'top'
	| 'bottom'
	| 'insert'
	/** Open the cursored item in the universal event menu. Cursor-based
	 *  buffers (feed, search, reader, profile) translate this to the
	 *  appropriate `app.openAddressableInModal` / `getEventForModal` call. */
	| 'menu';
export type NavHandler = (action: NavAction) => boolean;

function cloneTree(t: SplitNode): SplitNode {
	if (t.type === 'leaf') return { type: 'leaf', buffer: t.buffer };
	return { type: 'split', orient: t.orient, children: t.children.map(cloneTree) };
}

function nodeAt(tree: SplitNode, path: number[]): SplitNode | null {
	let n: SplitNode = tree;
	for (const idx of path) {
		if (n.type !== 'split') return null;
		const child = n.children[idx];
		if (!child) return null;
		n = child;
	}
	return n;
}

function replaceAt(tree: SplitNode, path: number[], replacer: (n: SplitNode) => SplitNode): SplitNode {
	if (path.length === 0) return replacer(tree);
	if (tree.type !== 'split') return tree;
	const [head, ...rest] = path;
	const newChildren = tree.children.slice();
	newChildren[head] = replaceAt(tree.children[head], rest, replacer);
	return { ...tree, children: newChildren };
}

function firstLeafPath(tree: SplitNode, prefix: number[] = []): number[] {
	if (tree.type === 'leaf') return prefix;
	return firstLeafPath(tree.children[0], [...prefix, 0]);
}

function leafPaths(tree: SplitNode, prefix: number[] = []): number[][] {
	if (tree.type === 'leaf') return [prefix];
	return tree.children.flatMap((c, i) => leafPaths(c, [...prefix, i]));
}

export class BufferStore {
	openBuffers = $state<OpenBuf[]>([]);
	recentlyClosed = $state<OpenBuf[]>([]);
	currentLayoutName = $state<string>('base');

	// Live slot state — initialized from layout config on setLayout, mutates
	// over time as the user collapses, expands, splits, swaps buffers.
	slotStates = $state<Partial<Record<Position, SlotState>>>({});
	slotTrees = $state<Partial<Record<Position, SplitNode>>>({});
	// Path of indices to the focused leaf within each slot's tree.
	// For a leaf-only slot, the path is []. For a 2-way split, [0] or [1].
	focusedLeafPath = $state<Partial<Record<Position, number[]>>>({});

	focusedSlot = $state<Position>('center');
	flashSlot = $state<Position | null>(null);
	bufferState = new Map<string, unknown>();

	// Buffer ids in most-recently-viewed-first order — the "go back" stack
	// for kill-buffer. Touched whenever a buffer becomes the visible/focused
	// one; killed buffers pop out. Non-reactive: only consulted on kill.
	private visitOrder: string[] = [];

	private touchVisit(id: string) {
		this.visitOrder = [id, ...this.visitOrder.filter((v) => v !== id)].slice(0, 100);
	}

	private dropVisit(id: string) {
		this.visitOrder = this.visitOrder.filter((v) => v !== id);
	}

	// Per-buffer modeline status line — small transient text (loading
	// indicators etc.) shown ONLY in the WM modeline. Deliberately separate
	// from `Buffer.kicker`, which also renders in pane headers and the buffer
	// switcher; this channel is the modeline alone.
	private _modelineStatus = $state<Record<string, string>>({});

	// Per-buffer keyboard nav handlers — populated by renderers on mount,
	// dispatched by the global keydown handler when normal mode + non-editable
	// focus + no leader. The Map itself is non-reactive: an `$effect` that
	// calls `registerNavHandler` would otherwise read it (in `new Map(...)`),
	// track it as a dep, then re-trigger when the method writes — infinite
	// loop. We expose `navHandlerKeys` separately as a reactive snapshot
	// for the modeline diagnostic.
	private _navHandlers = new Map<string, NavHandler>();
	navHandlerKeys = $state<string[]>([]);

	layouts: Record<string, LayoutConfig>;

	constructor(layouts: Record<string, LayoutConfig>, initialLayout = 'base') {
		this.layouts = layouts;
		this.currentLayoutName = initialLayout;
		this.hydrateFromLayout(initialLayout);
	}

	private hydrateFromLayout(name: string) {
		const layout = this.layouts[name];
		if (!layout) return;
		const states: Partial<Record<Position, SlotState>> = {};
		const trees: Partial<Record<Position, SplitNode>> = {};
		const focused: Partial<Record<Position, number[]>> = {};
		for (const pos of POSITION_ORDER) {
			const slot = layout.slots[pos];
			if (!slot) continue;
			states[pos] = slot.state;
			trees[pos] = cloneTree(slot.tree);
			focused[pos] = firstLeafPath(slot.tree);
		}
		this.slotStates = states;
		this.slotTrees = trees;
		this.focusedLeafPath = focused;
	}

	get currentLayout(): LayoutConfig {
		return this.layouts[this.currentLayoutName];
	}

	// Build a Slot view that combines layout (className) + live state.
	slotFor(pos: Position): Slot | null {
		const layoutSlot = this.currentLayout.slots[pos];
		const tree = this.slotTrees[pos];
		const state = this.slotStates[pos];
		if (!layoutSlot || !tree || !state) return null;
		return { className: layoutSlot.className, state, tree };
	}

	effectiveState(pos: Position): SlotState | null {
		return this.slotStates[pos] ?? null;
	}

	treeFor(pos: Position): SplitNode | null {
		return this.slotTrees[pos] ?? null;
	}

	classFor(pos: Position): ClassName | null {
		return this.currentLayout.slots[pos]?.className ?? null;
	}

	focusedLeaf(pos: Position): { buffer: Buffer; path: number[] } | null {
		const tree = this.slotTrees[pos];
		if (!tree) return null;
		let path = this.focusedLeafPath[pos] ?? [];
		let n = nodeAt(tree, path);
		if (!n) {
			path = firstLeafPath(tree);
			n = nodeAt(tree, path);
		}
		if (n?.type === 'leaf') return { buffer: n.buffer, path };
		// Path is at a split — descend to first leaf.
		const fullPath = path.concat(firstLeafPath(n!).slice(0));
		const leaf = nodeAt(tree, fullPath);
		if (leaf?.type === 'leaf') return { buffer: leaf.buffer, path: fullPath };
		return null;
	}

	focusedBuffer(): Buffer | null {
		return this.focusedLeaf(this.focusedSlot)?.buffer ?? null;
	}

	focusedSlotClass(): ClassName | null {
		return this.classFor(this.focusedSlot);
	}

	setLayout(name: string) {
		if (!this.layouts[name]) return;
		this.currentLayoutName = name;
		this.hydrateFromLayout(name);
	}

	toggleSlot(pos: Position) {
		const cur = this.slotStates[pos];
		if (cur === 'open') this.slotStates = { ...this.slotStates, [pos]: 'rail' };
		else if (cur === 'rail') this.slotStates = { ...this.slotStates, [pos]: 'open' };
	}

	toggleFocusedSlot() {
		this.toggleSlot(this.focusedSlot);
	}

	focusSlot(pos: Position) {
		this.focusedSlot = pos;
		const buf = this.focusedLeaf(pos)?.buffer;
		if (buf) this.touchVisit(buf.id);
	}

	// Replace the buffer at the focused leaf within `pos`.
	setLeaf(pos: Position, buf: Buffer) {
		const tree = this.slotTrees[pos];
		if (!tree) return;
		const path = this.focusedLeafPath[pos] ?? firstLeafPath(tree);
		this.slotTrees = {
			...this.slotTrees,
			[pos]: replaceAt(tree, path, () => ({ type: 'leaf', buffer: buf }))
		};
		this.focusedLeafPath = { ...this.focusedLeafPath, [pos]: path };
		this.touchVisit(buf.id);
	}

	// Set / clear a buffer's modeline status line (see `_modelineStatus`).
	// A renderer pushes loading progress here; `clear` removes it. Modeline
	// only — never touches `Buffer.kicker`, the slot trees, or `openBuffers`,
	// so pane headers and the buffer switcher never show this text.
	setModelineStatus(id: string, status: string) {
		if ((this._modelineStatus[id] ?? '') === status) return;
		this._modelineStatus = { ...this._modelineStatus, [id]: status };
	}

	clearModelineStatus(id: string) {
		if (!(id in this._modelineStatus)) return;
		const next = { ...this._modelineStatus };
		delete next[id];
		this._modelineStatus = next;
	}

	modelineStatus(id: string): string {
		return this._modelineStatus[id] ?? '';
	}

	// Split the focused leaf with a new buffer. The new leaf becomes focused.
	splitFocused(buf: Buffer, orient: 'h' | 'v' = 'h') {
		const pos = this.focusedSlot;
		const tree = this.slotTrees[pos];
		if (!tree) return;
		const path = this.focusedLeafPath[pos] ?? firstLeafPath(tree);
		this.slotTrees = {
			...this.slotTrees,
			[pos]: replaceAt(tree, path, (oldNode) => ({
				type: 'split',
				orient,
				children: [oldNode, { type: 'leaf', buffer: buf }]
			}))
		};
		// Focus the new leaf at index 1 of the new split.
		this.focusedLeafPath = { ...this.focusedLeafPath, [pos]: [...path, 1] };
		this.flash(pos);
	}

	// Cycle which leaf is focused within the focused slot's tree.
	cycleLeafInFocusedSlot(dir: 1 | -1) {
		const pos = this.focusedSlot;
		const tree = this.slotTrees[pos];
		if (!tree) return;
		const paths = leafPaths(tree);
		if (paths.length < 2) return;
		const curPath = this.focusedLeafPath[pos] ?? firstLeafPath(tree);
		const curIdx = paths.findIndex((p) => p.length === curPath.length && p.every((v, i) => v === curPath[i]));
		const nextIdx = (curIdx + dir + paths.length) % paths.length;
		this.focusedLeafPath = { ...this.focusedLeafPath, [pos]: paths[nextIdx] };
		const buf = this.focusedLeaf(pos)?.buffer;
		if (buf) this.touchVisit(buf.id);
	}

	findSlotForClass(cls: ClassName): Position | null {
		for (const pos of POSITION_ORDER) {
			if (this.classFor(pos) === cls) return pos;
		}
		return null;
	}

	flash(pos: Position) {
		this.flashSlot = pos;
		setTimeout(() => {
			if (this.flashSlot === pos) this.flashSlot = null;
		}, 700);
	}

	selectBuffer(entry: OpenBuf): boolean {
		const target = this.findSlotForClass(entry.className);
		if (!target) return false;
		const cur = this.slotStates[target];
		if (cur === 'rail' || cur === 'hidden') {
			this.slotStates = { ...this.slotStates, [target]: 'open' };
		}
		this.focusedSlot = target;
		this.setLeaf(target, entry.buffer);
		this.flash(target);
		return true;
	}

	// Class fallback when killing the last buffer of a class. These are
	// always-conceptually-open singletons; reopening them is cheap.
	private classDefault(cls: ClassName): Buffer | null {
		if (cls === 'chat') return { id: 'chat', kind: 'chat', label: 'chat', kicker: '' };
		if (cls === 'work') return { id: 'feed', kind: 'feed', label: 'feed', kicker: '' };
		if (cls === 'research') return { id: 'search', kind: 'search', label: 'search', kicker: '' };
		return null;
	}

	// Kill-replacement policy: go back to the most recently *viewed* open
	// buffer of the class (the visit stack), else the most recently *opened*
	// one. Class-scoped because slots are class-typed. Callers fall back to
	// classDefault when this returns null.
	private mruReplacement(cls: ClassName): Buffer | null {
		const byId = new Map(this.openBuffers.map((b) => [b.buffer.id, b]));
		for (const id of this.visitOrder) {
			const cand = byId.get(id);
			if (cand && cand.className === cls) return cand.buffer;
		}
		return [...this.openBuffers].reverse().find((b) => b.className === cls)?.buffer ?? null;
	}

	killFocused() {
		const cur = this.focusedLeaf(this.focusedSlot);
		if (!cur) return;
		const buf = cur.buffer;

		if (this.openBuffers.some((b) => b.buffer.id === buf.id)) {
			this.killBuffer(buf.id);
			return;
		}
		this.dropVisit(buf.id);

		// Buffer not registered in openBuffers (transient leaf) — just swap
		// the focused leaf to another buffer of the slot's class.
		const cls = this.classFor(this.focusedSlot);
		if (!cls) {
			this.flash(this.focusedSlot);
			return;
		}
		const replacement = this.mruReplacement(cls);
		if (replacement) {
			this.setLeaf(this.focusedSlot, replacement);
			this.flash(this.focusedSlot);
			return;
		}
		const fallback = this.classDefault(cls);
		if (fallback) {
			this.openBuffers = [...this.openBuffers, { className: cls, buffer: fallback }];
			this.setLeaf(this.focusedSlot, fallback);
		}
		this.flash(this.focusedSlot);
	}

	// Kill a specific open buffer by id (the tab ×, and the registered-buffer
	// path of killFocused). Same replacement policy as killFocused — the most
	// recently viewed buffer of the class, then the most recently opened,
	// else restore the class default — but it doesn't require focus: every
	// leaf currently showing the killed buffer (any slot, any split) is
	// swapped to the replacement.
	killBuffer(id: string) {
		const entry = this.openBuffers.find((b) => b.buffer.id === id);
		if (!entry) return;
		this.openBuffers = this.openBuffers.filter((b) => b.buffer.id !== id);
		this.recentlyClosed = [entry, ...this.recentlyClosed].slice(0, 20);
		this.dropVisit(id);

		const cls = entry.className;
		let replacement = this.mruReplacement(cls);
		if (!replacement) {
			const fallback = this.classDefault(cls);
			if (fallback) {
				this.openBuffers = [...this.openBuffers, { className: cls, buffer: fallback }];
				replacement = fallback;
			}
		}
		if (!replacement) return;

		let swapped = false;
		for (const pos of POSITION_ORDER) {
			const tree = this.slotTrees[pos];
			if (!tree) continue;
			let next = tree;
			let changed = false;
			for (const path of leafPaths(tree)) {
				const n = nodeAt(next, path);
				if (n?.type === 'leaf' && n.buffer.id === id) {
					const buf = replacement;
					next = replaceAt(next, path, () => ({ type: 'leaf', buffer: buf }));
					changed = true;
				}
			}
			if (changed) {
				this.slotTrees = { ...this.slotTrees, [pos]: next };
				this.flash(pos);
				swapped = true;
			}
		}
		// The replacement is now the visible buffer where the kill happened.
		if (swapped) this.touchVisit(replacement.id);
	}

	registerNavHandler(bufferId: string, handler: NavHandler) {
		this._navHandlers.set(bufferId, handler);
		this.navHandlerKeys = Array.from(this._navHandlers.keys());
	}

	unregisterNavHandler(bufferId: string) {
		this._navHandlers.delete(bufferId);
		this.navHandlerKeys = Array.from(this._navHandlers.keys());
	}

	dispatchNav(action: NavAction): boolean {
		const buf = this.focusedLeaf(this.focusedSlot)?.buffer;
		if (!buf) return false;
		const handler = this._navHandlers.get(buf.id);
		if (!handler) return false;
		return handler(action);
	}

	cycleBufferInSlot(dir: 1 | -1) {
		const pos = this.focusedSlot;
		if (this.slotStates[pos] === 'rail') return;
		const tree = this.slotTrees[pos];
		if (!tree) return;
		// If the slot has multiple leaves, cycle which leaf is focused (intra-slot nav).
		if (leafPaths(tree).length > 1) {
			this.cycleLeafInFocusedSlot(dir);
			return;
		}
		// Single-leaf slot: cycle the buffer in that leaf to a different same-class buffer.
		const cls = this.classFor(pos);
		if (!cls) return;
		const classBuffers = this.openBuffers.filter((b) => b.className === cls);
		if (classBuffers.length < 2) return;
		const cur = this.focusedLeaf(pos)?.buffer;
		const idx = cur ? classBuffers.findIndex((b) => b.buffer.id === cur.id) : -1;
		const next = classBuffers[(idx + dir + classBuffers.length) % classBuffers.length];
		this.setLeaf(pos, next.buffer);
	}

	navigateSlot(dir: 1 | -1) {
		const visible = POSITION_ORDER.filter((p) => {
			const s = this.slotStates[p];
			return s === 'open' || s === 'rail';
		});
		const idx = visible.indexOf(this.focusedSlot);
		if (idx === -1 && visible.length) {
			this.focusedSlot = visible[0];
			this.flash(visible[0]);
			return;
		}
		const next = visible[(idx + dir + visible.length) % visible.length];
		this.focusedSlot = next;
		const buf = this.focusedLeaf(next)?.buffer;
		if (buf) this.touchVisit(buf.id);
		this.flash(next);
	}

	expandFocusedIfRail(): boolean {
		if (this.slotStates[this.focusedSlot] === 'rail') {
			this.slotStates = { ...this.slotStates, [this.focusedSlot]: 'open' };
			this.flash(this.focusedSlot);
			return true;
		}
		return false;
	}

	seed(open: OpenBuf[]) {
		this.openBuffers = open;
	}

	openBuffer(entry: OpenBuf): boolean {
		// Idempotent fast-path: if this buffer is already the focused leaf of an
		// open slot, opening it again is a pure no-op — NO state writes. Without
		// this, a reactive caller (an `$effect`/derived that opens a buffer) drives
		// an infinite loop: openBuffer → selectBuffer → setLeaf rebuilds the leaf →
		// the component re-mounts → the effect re-runs → openBuffer again. Compare
		// by id (for singletons like settings/feed that's identity), focus, and
		// open-state so a genuine switch/restore still falls through.
		const cur = this.findSlotForClass(entry.className);
		if (
			cur &&
			this.focusedSlot === cur &&
			this.slotStates[cur] === 'open' &&
			this.focusedLeaf(cur)?.buffer.id === entry.buffer.id &&
			this.openBuffers.some((b) => b.buffer.id === entry.buffer.id)
		) {
			return true;
		}
		const idx = this.openBuffers.findIndex((b) => b.buffer.id === entry.buffer.id);
		if (idx === -1) {
			this.openBuffers = [...this.openBuffers, entry];
		} else {
			const next = this.openBuffers.slice();
			next[idx] = entry;
			this.openBuffers = next;
		}

		if (!this.findSlotForClass(entry.className)) {
			const fallback = Object.keys(this.layouts).find((name) =>
				Object.values(this.layouts[name].slots).some((s) => s?.className === entry.className)
			);
			if (fallback) this.setLayout(fallback);
		}

		return this.selectBuffer(entry);
	}

	get positionOrder(): readonly Position[] {
		return POSITION_ORDER;
	}
}

// Phase 1 uses a singleton — one shell instance per page. Stash it on
// globalThis so HMR module-duplication can't fork the active store
// (otherwise +page.svelte's copy of this module and a renderer's copy
// each end up with their own `_activeStore`, and registrations in one
// are invisible to the other — which manifests as `nav:✗` in the
// modeline).
const ACTIVE_STORE_KEY = '__tendrl_active_buffer_store__';

type GlobalWithStore = typeof globalThis & { [ACTIVE_STORE_KEY]?: BufferStore };

export function setActiveStore(store: BufferStore): void {
	(globalThis as GlobalWithStore)[ACTIVE_STORE_KEY] = store;
}

export function getActiveStore(): BufferStore {
	const store = (globalThis as GlobalWithStore)[ACTIVE_STORE_KEY];
	if (!store) {
		throw new Error('BufferStore not active — call setActiveStore() in the shell root');
	}
	return store;
}
