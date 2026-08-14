import type { CommandCat, MinibufferMode } from './types.js';

export type LeafKind = 'engine' | 'client';

export type LeafCmd = {
	type: 'leaf';
	desc: string;
	category: CommandCat;
	kind: LeafKind;
	deferred?: boolean;
	/** Palette command this leaf mirrors — lets a user's custom binding
	 *  replace the default leaf (see applyBindingOverrides). */
	commandId?: string;
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
	openSplitPicker: (orient: 'h' | 'v') => void;
	closeFocusedWindow: () => void;
	openSettings: () => void;
	openProfileEdit: () => void;
	openCompose: () => void;
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
					b: { type: 'leaf', desc: 'switch (class)', commandId: 'tendrl-switch-buffer', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('class') },
					B: { type: 'leaf', desc: 'switch (global)', commandId: 'tendrl-switch-buffer-global', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('global') },
					r: { type: 'leaf', desc: 'recently closed', commandId: 'tendrl-recent-buffer', category: 'Buffer', kind: 'client', run: () => ctx.openMinibuffer('recent') },
					k: { type: 'leaf', desc: 'kill buffer', commandId: 'tendrl-kill-buffer', category: 'Buffer', kind: 'client', run: () => ctx.killFocusedBuffer() },
					n: { type: 'leaf', desc: 'next in class', category: 'Buffer', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					p: { type: 'leaf', desc: 'previous in class', category: 'Buffer', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) },
					o: {
						type: 'prefix',
						desc: 'open',
						children: {
							c: { type: 'leaf', desc: 'compose', commandId: 'tendrl-open-compose', category: 'Compose', kind: 'client', run: () => ctx.openCompose() },
							// Same commandId as SPC s s — the two chords are aliases;
							// desc/category match so listLeaderBindings merges the rows.
							s: { type: 'leaf', desc: 'open settings', commandId: 'tendrl-open-settings', category: 'Configuration', kind: 'client', run: () => ctx.openSettings() }
						}
					}
				}
			},
			w: {
				type: 'prefix',
				desc: 'window',
				children: {
					c: { type: 'leaf', desc: 'collapse / expand', commandId: 'tendrl-toggle-rail', category: 'Window', kind: 'client', run: () => ctx.toggleFocusedSlot() },
					// h/l move focus across slots (chat ↔ work ↔ research).
					// j/k move focus within the focused slot — between its split
					// windows, or through its class's buffers when it has only one.
					h: { type: 'leaf', desc: 'focus left slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(-1) },
					l: { type: 'leaf', desc: 'focus right slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(1) },
					ArrowLeft: { type: 'leaf', desc: 'focus left slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(-1) },
					ArrowRight: { type: 'leaf', desc: 'focus right slot', category: 'Window', kind: 'client', run: () => ctx.navigateSlot(1) },
					j: { type: 'leaf', desc: 'next buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					k: { type: 'leaf', desc: 'prev buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) },
					ArrowDown: { type: 'leaf', desc: 'next buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(1) },
					ArrowUp: { type: 'leaf', desc: 'prev buffer in slot', category: 'Window', kind: 'client', run: () => ctx.cycleBufferInSlot(-1) },
					s: { type: 'leaf', desc: 'split below (same-class)', commandId: 'tendrl-split-window', category: 'Window', kind: 'client', run: () => ctx.openSplitPicker('h') },
					v: { type: 'leaf', desc: 'split right (same-class)', commandId: 'tendrl-vsplit-window', category: 'Window', kind: 'client', run: () => ctx.openSplitPicker('v') },
					x: { type: 'leaf', desc: 'kill window', commandId: 'tendrl-close-window', category: 'Window', kind: 'client', run: () => ctx.closeFocusedWindow() }
				}
			},
			f: {
				type: 'prefix',
				desc: 'find',
				children: {
					e: { type: 'leaf', desc: 'find-event', commandId: 'tendrl-find-event', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-event') },
					d: { type: 'leaf', desc: 'find-draft', commandId: 'tendrl-find-draft', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-draft') },
					p: { type: 'leaf', desc: 'find-publication', category: 'Buffer', kind: 'engine', run: () => ctx.prefilterMx('find-publication') }
				}
			},
			l: {
				type: 'prefix',
				desc: 'layout',
				children: {
					b: { type: 'leaf', desc: 'base', commandId: 'tendrl-switch-layout', category: 'Layout', kind: 'client', run: () => ctx.setLayout('base') },
					s: { type: 'leaf', desc: 'save current (deferred)', commandId: 'tendrl-save-layout', category: 'Layout', kind: 'client', deferred: true, run: () => {} }
				}
			},
			t: {
				type: 'prefix',
				desc: 'toggle',
				children: {
					n: { type: 'leaf', desc: 'network mode', commandId: 'tendrl-toggle-network-mode', category: 'Configuration', kind: 'engine', run: () => ctx.toggleNetworkMode() }
				}
			},
			s: {
				type: 'prefix',
				desc: 'settings',
				children: {
					s: { type: 'leaf', desc: 'open settings', commandId: 'tendrl-open-settings', category: 'Configuration', kind: 'client', run: () => ctx.openSettings() },
					i: { type: 'leaf', desc: 'identity (login / source)', commandId: 'tendrl-login', category: 'Configuration', kind: 'client', run: () => ctx.openSettings() },
					p: { type: 'leaf', desc: 'profile (kind 0)', commandId: 'tendrl-edit-profile', category: 'Configuration', kind: 'engine', run: () => ctx.openProfileEdit() }
				}
			},
			q: {
				type: 'prefix',
				desc: 'quit',
				children: {
					q: { type: 'leaf', desc: 'quit frame', commandId: 'tendrl-quit', category: 'Application', kind: 'client', run: () => {} }
				}
			},
			':': { type: 'leaf', desc: 'commands', category: 'Application', kind: 'client', run: () => ctx.openMinibuffer('mx') }
		}
	};
}

export type LeaderBinding = {
	keys: string;
	desc: string;
	category: CommandCat;
	kind: LeafKind;
	deferred?: boolean;
};

/** A user's custom binding for a palette command (from command-prefs).
 *  tokens are the chord AFTER the SPC prefix ('SPC o s' → ['o','s']);
 *  single-key bindings never reach the leader tree. */
export type LeaderBindingOverride = {
	commandId: string;
	tokens: string[];
	desc: string;
	category: CommandCat;
	run: () => void;
};

/** Apply custom bindings onto a freshly built root, in place: every
 *  overridden command's default tagged leaves are removed (a custom
 *  binding REPLACES the default, it doesn't alias it), then SPC-chord
 *  overrides are grafted, creating prefix nodes as needed. Emptied
 *  default prefixes are pruned so the which-key popup stays clean. */
export function applyBindingOverrides(root: SubPrefix, overrides: LeaderBindingOverride[]): SubPrefix {
	if (overrides.length === 0) return root;
	const overridden = new Set(overrides.map((o) => o.commandId));
	const prune = (node: SubPrefix) => {
		for (const [key, child] of Object.entries(node.children)) {
			if (child.type === 'prefix') {
				prune(child);
				if (Object.keys(child.children).length === 0) delete node.children[key];
			} else if (child.commandId && overridden.has(child.commandId)) {
				delete node.children[key];
			}
		}
	};
	prune(root);
	for (const o of overrides) {
		if (o.tokens.length === 0) continue; // single-key override: removal only
		let node = root;
		for (const key of o.tokens.slice(0, -1)) {
			const next = node.children[key];
			if (next?.type === 'prefix') {
				node = next;
			} else {
				const created: SubPrefix = { type: 'prefix', desc: GRAFT_PREFIX_DESC[key] ?? key, children: {} };
				node.children[key] = created;
				node = created;
			}
		}
		const last = o.tokens[o.tokens.length - 1];
		node.children[last] = {
			type: 'leaf',
			desc: o.desc,
			category: o.category,
			kind: 'client',
			commandId: o.commandId,
			run: o.run
		};
	}
	return root;
}

// Which-key labels for prefixes that only exist via grafted bindings
// (default or custom) — without an entry the raw key is the label.
const GRAFT_PREFIX_DESC: Record<string, string> = {
	e: 'engine / editor',
	r: 'relays'
};

/** Command ids the DEFAULT tree carries a tagged leaf for. Commands whose
 *  default keybinding is a SPC chord but who are NOT in this set (e.g.
 *  tendrl-highlight → SPC h) get their default grafted at runtime by the
 *  same override path custom bindings use. */
export function defaultTreeCommandIds(): Set<string> {
	const ids = new Set<string>();
	const walk = (node: SubPrefix) => {
		for (const child of Object.values(node.children)) {
			if (child.type === 'prefix') walk(child);
			else if (child.commandId) ids.add(child.commandId);
		}
	};
	walk(buildNoopRoot());
	return ids;
}

/** Can `tokens` (chord after SPC) be grafted for `forCommandId` without
 *  colliding with the default tree? Descending through an existing prefix
 *  is fine; landing on one, or crossing/landing on a live leaf that
 *  belongs to a different command, is not. */
export function validateLeaderChord(tokens: string[], forCommandId: string): string | null {
	if (tokens.length === 0) return 'empty chord';
	let node: LeaderNode = buildNoopRoot();
	for (let i = 0; i < tokens.length; i++) {
		if (node.type !== 'prefix') return `SPC ${tokens.slice(0, i).join(' ')} is already a command`;
		const child: LeaderNode | undefined = node.children[tokens[i]];
		if (!child) return null; // free slot from here down
		node = child;
	}
	if (node.type === 'prefix') return `SPC ${tokens.join(' ')} is a prefix (would shadow its subtree)`;
	if (node.commandId === forCommandId) return null; // rebinding to its own default
	return `SPC ${tokens.join(' ')} is taken by "${node.desc}"`;
}

const KEY_LABEL: Record<string, string> = {
	ArrowLeft: '←',
	ArrowRight: '→',
	ArrowUp: '↑',
	ArrowDown: '↓'
};

function buildNoopRoot(): SubPrefix {
	const noop = () => {};
	return buildLeaderRoot({
		openMinibuffer: noop,
		prefilterMx: noop,
		killFocusedBuffer: noop,
		cycleBufferInSlot: noop,
		toggleFocusedSlot: noop,
		navigateSlot: noop,
		setLayout: noop,
		toggleNetworkMode: noop,
		openSplitPicker: noop,
		closeFocusedWindow: noop,
		openSettings: noop,
		openProfileEdit: noop,
		openCompose: noop
	});
}

/** Flatten the leader tree into displayable binding rows for the settings
 *  registry. Built against a noop context — nothing is ever run — so the
 *  listing can't drift from the real tree. Custom-binding overrides are
 *  applied first, so the listing shows the EFFECTIVE tree. Alias keys
 *  (arrows mirroring h/j/k/l) merge into the row they duplicate. */
export function listLeaderBindings(overrides: LeaderBindingOverride[] = []): LeaderBinding[] {
	const root = applyBindingOverrides(buildNoopRoot(), overrides);
	const out: LeaderBinding[] = [];
	const walk = (node: SubPrefix, path: string[]) => {
		for (const [key, child] of Object.entries(node.children)) {
			if (child.type === 'prefix') {
				walk(child, [...path, key]);
				continue;
			}
			const keys = ['SPC', ...path, KEY_LABEL[key] ?? key].join(' ');
			const twin = out.find((b) => b.desc === child.desc && b.category === child.category);
			if (twin) twin.keys += ` · ${keys}`;
			else out.push({ keys, desc: child.desc, category: child.category, kind: child.kind, deferred: child.deferred });
		}
	};
	walk(root, []);
	return out;
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
