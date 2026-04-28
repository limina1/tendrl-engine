import type {
	Buffer,
	ClassName,
	LayoutConfig,
	OpenBuf,
	Position,
	Slot,
	SlotState
} from './types';

const POSITION_ORDER: Position[] = ['left', 'center', 'right'];

export class BufferStore {
	openBuffers = $state<OpenBuf[]>([]);
	recentlyClosed = $state<OpenBuf[]>([]);
	currentLayoutName = $state<string>('write');
	slotOverrides = $state<Partial<Record<Position, SlotState>>>({});
	leafOverrides = $state<Partial<Record<Position, Buffer>>>({});
	focusedSlot = $state<Position>('center');
	flashSlot = $state<Position | null>(null);
	bufferState = new Map<string, unknown>();

	layouts: Record<string, LayoutConfig>;

	constructor(layouts: Record<string, LayoutConfig>, initialLayout = 'write') {
		this.layouts = layouts;
		this.currentLayoutName = initialLayout;
	}

	get currentLayout(): LayoutConfig {
		return this.layouts[this.currentLayoutName];
	}

	effectiveState(pos: Position): SlotState | null {
		const slot = this.currentLayout.slots[pos];
		if (!slot) return null;
		return this.slotOverrides[pos] ?? slot.state;
	}

	effectiveLeaf(pos: Position, slot: Slot): Buffer | null {
		const override = this.leafOverrides[pos];
		if (override) return override;
		const t = slot.tree;
		if (t.type === 'leaf') return t.buffer;
		return t.children[0].type === 'leaf' ? t.children[0].buffer : null;
	}

	focusedBuffer(): Buffer | null {
		const slot = this.currentLayout.slots[this.focusedSlot];
		if (!slot) return null;
		return this.effectiveLeaf(this.focusedSlot, slot);
	}

	focusedSlotClass(): ClassName | null {
		const s = this.currentLayout.slots[this.focusedSlot];
		return s ? s.className : null;
	}

	setLayout(name: string) {
		if (!this.layouts[name]) return;
		this.currentLayoutName = name;
		this.slotOverrides = {};
		this.leafOverrides = {};
	}

	toggleSlot(pos: Position) {
		const cur = this.effectiveState(pos);
		if (cur === 'open') this.slotOverrides = { ...this.slotOverrides, [pos]: 'rail' };
		else if (cur === 'rail') this.slotOverrides = { ...this.slotOverrides, [pos]: 'open' };
	}

	toggleFocusedSlot() {
		this.toggleSlot(this.focusedSlot);
	}

	focusSlot(pos: Position) {
		this.focusedSlot = pos;
	}

	setLeaf(pos: Position, buf: Buffer) {
		this.leafOverrides = { ...this.leafOverrides, [pos]: buf };
	}

	findSlotForClass(cls: ClassName): Position | null {
		for (const pos of POSITION_ORDER) {
			const s = this.currentLayout.slots[pos];
			if (s && s.className === cls) return pos;
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
		const cur = this.effectiveState(target);
		if (cur === 'rail' || cur === 'hidden') {
			this.slotOverrides = { ...this.slotOverrides, [target]: 'open' };
		}
		this.setLeaf(target, entry.buffer);
		this.focusedSlot = target;
		this.flash(target);
		return true;
	}

	killFocused() {
		// Phase 1 stub — flash for visual feedback. Production removes from
		// openBuffers, prompts-to-save if dirty draft, propagates via BroadcastChannel.
		this.flash(this.focusedSlot);
	}

	cycleBufferInSlot(dir: 1 | -1) {
		const slot = this.currentLayout.slots[this.focusedSlot];
		if (!slot) return;
		if (this.effectiveState(this.focusedSlot) === 'rail') return;
		const classBuffers = this.openBuffers.filter((b) => b.className === slot.className);
		if (classBuffers.length < 2) return;
		const cur = this.effectiveLeaf(this.focusedSlot, slot);
		const idx = cur ? classBuffers.findIndex((b) => b.buffer.id === cur.id) : -1;
		const next = classBuffers[(idx + dir + classBuffers.length) % classBuffers.length];
		this.setLeaf(this.focusedSlot, next.buffer);
	}

	navigateSlot(dir: 1 | -1) {
		const visible = POSITION_ORDER.filter((p) => {
			const s = this.currentLayout.slots[p];
			return s && (this.effectiveState(p) === 'open' || this.effectiveState(p) === 'rail');
		});
		const idx = visible.indexOf(this.focusedSlot);
		if (idx === -1 && visible.length) {
			this.focusedSlot = visible[0];
			this.flash(visible[0]);
			return;
		}
		const next = visible[(idx + dir + visible.length) % visible.length];
		this.focusedSlot = next;
		this.flash(next);
	}

	expandFocusedIfRail(): boolean {
		if (this.effectiveState(this.focusedSlot) === 'rail') {
			this.slotOverrides = { ...this.slotOverrides, [this.focusedSlot]: 'open' };
			this.flash(this.focusedSlot);
			return true;
		}
		return false;
	}

	seed(open: OpenBuf[]) {
		this.openBuffers = open;
	}

	openBuffer(entry: OpenBuf): boolean {
		const idx = this.openBuffers.findIndex((b) => b.buffer.id === entry.buffer.id);
		if (idx === -1) {
			this.openBuffers = [...this.openBuffers, entry];
		} else {
			// Replace in place — buffer metadata may have changed (label, kicker, modified).
			const next = this.openBuffers.slice();
			next[idx] = entry;
			this.openBuffers = next;
		}

		// If the current layout doesn't host this class, switch to one that does.
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

// Phase 1 uses a module-level singleton — one shell instance per page.
// Multi-frame (multiple browser tabs sharing buffer state) will swap this
// for a per-frame store synced via BroadcastChannel.
let _activeStore: BufferStore | null = null;

export function setActiveStore(store: BufferStore): void {
	_activeStore = store;
}

export function getActiveStore(): BufferStore {
	if (!_activeStore) {
		throw new Error('BufferStore not active — call setActiveStore() in the shell root');
	}
	return _activeStore;
}
