import type { CommandCat, MinibufferMode } from './types.js';

export type LeafKind = 'engine' | 'client';

export type LeafCmd = {
	type: 'leaf';
	desc: string;
	category: CommandCat;
	kind: LeafKind;
	deferred?: boolean;
	run: () => void;
};

export type SubPrefix = {
	type: 'prefix';
	desc: string;
	children: Record<string, LeaderNode>;
};

export type LeaderNode = LeafCmd | SubPrefix;

export type LeaderContext = {
	openMinibuffer: (mode: MinibufferMode) => void;
	prefilterMx: (name: string) => void;
	killFocusedBuffer: () => void;
	cycleBufferInSlot: (dir: 1 | -1) => void;
	toggleFocusedSlot: () => void;
	navigateSlot: (dir: 1 | -1) => void;
	setLayout: (name: string) => void;
	toggleNetworkMode: () => void;
	openSplitPicker: () => void;
	openSettings: () => void;
};

export function buildLeaderRoot(ctx: LeaderContext): SubPrefix {
	return {
		type: 'prefix',
		desc: 'leader',
		children: {
			b: {
				type: 'prefix',
				desc: 'buffer',
				children: {
					b: { type: 'leaf', desc: 'switch (class)', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('class') },
					B: { type: 'leaf', desc: 'switch (global)', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('global') },
					r: { type: 'leaf', desc: 'recently closed', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('recent') },
					k: { type: 'leaf', desc: 'kill buffer', category: 'Buffer', kind: 'client', run: () => ctx.killFocusedBuffer() },
					n: { type: 'leaf', desc: 'next in class', category: 'Buffer', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					p: { type: 'leaf', desc: 'previous in class', category: 'Buffer', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) }
				}
			},
			w: {
				type: 'prefix',
				desc: 'window',
				children: {
					c: { type: 'leaf', desc: 'collapse / expand', category: 'Window', kind: 'client', run: () => ctx.toggleFocusedSlot() },
					// h/l move focus across slots (chat ↔ work ↔ research).
					// j/k cycle through buffers within the focused slot.
					h: { type: 'leaf', desc: 'focus left slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(-1) },
					l: { type: 'leaf', desc: 'focus right slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(1) },
					ArrowLeft: { type: 'leaf', desc: 'focus left slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(-1) },
					ArrowRight: { type: 'leaf', desc: 'focus right slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(1) },
					j: { type: 'leaf', desc: 'next buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					k: { type: 'leaf', desc: 'prev buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) },
					ArrowDown: { type: 'leaf', desc: 'next buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					ArrowUp: { type: 'leaf', desc: 'prev buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) },
					s: { type: 'leaf', desc: 'split (same-class h)', category: 'Window', kind: 'client', run: () => ctx.openSplitPicker() }
				}
			},
			f: {
				type: 'prefix',
				desc: 'find',
				children: {
					e: { type: 'leaf', desc: 'find-event', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-event') },
					d: { type: 'leaf', desc: 'find-draft', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-draft') },
					p: { type: 'leaf', desc: 'find-publication', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-publication') }
				}
			},
			l: {
				type: 'prefix',
				desc: 'layout',
				children: {
					b: { type: 'leaf', desc: 'base', category: 'Layout', kind: 'client', run: () => ctx.setLayout('base') },
					s: { type: 'leaf', desc: 'save current (deferred)', category: 'Layout', kind: 'client', deferred: true, run: () => {} }
				}
			},
			t: {
				type: 'prefix',
				desc: 'toggle',
				children: {
					n: { type: 'leaf', desc: 'network mode', category: 'Configuration', kind: 'engine', run: () => ctx.toggleNetworkMode() }
				}
			},
			s: {
				type: 'prefix',
				desc: 'settings',
				children: {
					s: { type: 'leaf', desc: 'open settings', category: 'Configuration', kind: 'client', run: () => ctx.openSettings() }
				}
			},
			q: {
				type: 'prefix',
				desc: 'quit',
				children: {
					q: { type: 'leaf', desc: 'quit frame', category: 'Application', kind: 'client', run: () => {} }
				}
			},
			':': { type: 'leaf', desc: 'M-x', category: 'Application', kind: 'client', run: () => ctx.openMinibuffer('mx') }
		}
	};
}

export function resolveLeaderNode(root: SubPrefix, path: string[]): LeaderNode | null {
	let n: LeaderNode = root;
	for (const k of path) {
		if (n.type !== 'prefix') return null;
		const child: LeaderNode | undefined = n.children[k];
		if (!child) return null;
		n = child;
	}
	return n;
}
