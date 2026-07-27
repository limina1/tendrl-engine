// Per-user command preferences: which commands the SPC : palette shows,
// and custom keybindings. Pure frontend interaction state, so it persists
// client-side (localStorage, this device only) — same policy as text
// scale and theme, never engine config.
//
// A custom binding REPLACES the command's default: the default leader
// leaf (tagged with commandId in leader.ts) is removed and the custom
// chord grafted in its place, so the registry can never show a key that
// doesn't fire.
//
// Leaf module: imports only sibling leaf modules (types/commands/leader),
// safe for buffer renderers — no wm/registry cycle.
import type { Command } from './types.js';
import { commands } from './commands.js';
import { defaultTreeCommandIds, validateLeaderChord, type LeaderBindingOverride } from './leader.js';

export type CommandPref = {
	/** Palette visibility override (in either direction — a command can
	 *  ship hiddenByDefault and be opted back in). Bindings keep working
	 *  regardless. Absent = the command's default. */
	hidden?: boolean;
	/** Custom binding, space-joined tokens: 'SPC o s' (leader chord) or 'u'
	 *  (single normal-mode key). Replaces the default binding. */
	keys?: string;
};

const STORAGE_KEY = 'tendrl.command-prefs.v1';

const byId = new Map(commands.map((c) => [c.id, c]));

function load(): Record<string, CommandPref> {
	if (typeof localStorage === 'undefined') return {};
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		const stored: Record<string, CommandPref> = raw ? JSON.parse(raw) : {};
		// Normalize: prefs that merely restate the shipped defaults (e.g.
		// saved before those defaults existed) collapse away, so the UI
		// shows them as defaults, not customizations.
		for (const [id, p] of Object.entries(stored)) {
			const cmd = byId.get(id);
			if (!cmd) continue;
			if (p.keys !== undefined && p.keys === cmd.keybinding) delete p.keys;
			if (p.hidden !== undefined && p.hidden === (cmd.hiddenByDefault ?? false)) delete p.hidden;
			if (Object.keys(p).length === 0) delete stored[id];
		}
		return stored;
	} catch {
		return {};
	}
}

export const commandPrefs = $state<{ byId: Record<string, CommandPref> }>({ byId: load() });

function persist() {
	if (typeof localStorage === 'undefined') return;
	localStorage.setItem(STORAGE_KEY, JSON.stringify(commandPrefs.byId));
}

export function prefFor(id: string): CommandPref {
	return commandPrefs.byId[id] ?? {};
}

/** Effective palette visibility: pref override, else the shipped default. */
export function isCommandHidden(cmd: Command): boolean {
	return prefFor(cmd.id).hidden ?? cmd.hiddenByDefault ?? false;
}

export function setHidden(id: string, hidden: boolean) {
	const p = { ...prefFor(id) };
	if (hidden === (byId.get(id)?.hiddenByDefault ?? false)) delete p.hidden;
	else p.hidden = hidden;
	updatePref(id, p);
}

export function setBinding(id: string, keys: string) {
	updatePref(id, { ...prefFor(id), keys });
}

export function clearBinding(id: string) {
	const p = { ...prefFor(id) };
	delete p.keys;
	updatePref(id, p);
}

function updatePref(id: string, p: CommandPref) {
	if (Object.keys(p).length === 0) delete commandPrefs.byId[id];
	else commandPrefs.byId[id] = p;
	persist();
}

/** The binding the registry/palette should display — custom wins. */
export function effectiveKeybinding(cmd: Command): string | undefined {
	return prefFor(cmd.id).keys ?? cmd.keybinding;
}

/** Overrides to apply onto the built leader tree: every custom-bound
 *  command drops its default tagged leaves; SPC chords graft new leaves
 *  that run the command. `run` is injected by the caller (+page wires it
 *  to executeCommand; listings pass a noop). */
export function leaderOverrides(run: (cmd: Command) => void): LeaderBindingOverride[] {
	const treeIds = defaultTreeCommandIds();
	return commands.flatMap((cmd) => {
		// Custom binding wins; otherwise a default SPC chord that the
		// structural tree doesn't carry (e.g. tendrl-highlight → SPC h)
		// grafts itself, so shipped defaults fire out of the box through
		// the same path customs do.
		const custom = commandPrefs.byId[cmd.id]?.keys;
		const keys = custom ?? (treeIds.has(cmd.id) ? undefined : cmd.keybinding);
		if (!keys) return [];
		// Tokens are the chord AFTER the SPC prefix. A single-key binding
		// contributes an empty-token override: it still removes the
		// command's default leader leaf (the custom binding replaces the
		// default), but nothing is grafted — dispatch happens via
		// singleKeyBindings() in normal-mode keydown.
		const tokens = keys.startsWith('SPC ') ? keys.split(' ').slice(1) : [];
		if (tokens.length === 0 && !custom) return []; // default non-chord: nothing to graft
		return [
			{
				commandId: cmd.id,
				tokens,
				desc: cmd.name.replace(/^tendrl-/, ''),
				category: cmd.category,
				run: () => run(cmd)
			}
		];
	});
}

/** Single-key (non-leader) custom bindings, key → command. */
export function singleKeyBindings(): Record<string, Command> {
	const map: Record<string, Command> = {};
	for (const cmd of commands) {
		const keys = commandPrefs.byId[cmd.id]?.keys;
		if (keys && !keys.includes(' ') && keys !== 'SPC') map[keys] = cmd;
	}
	return map;
}

// Keys the normal-mode layer already owns — a custom single-key binding
// may not shadow them (see onGlobalKeydown in +page.svelte).
const RESERVED_SINGLE = new Set([
	'h', 'j', 'k', 'l', 'm', 'g', 'G', 'i', 'o', ':', ' ',
	'Enter', 'Escape', 'Backspace',
	'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'
]);

/** Validate a candidate binding for `forId`. Returns an error message or
 *  null when the binding is acceptable. */
export function validateBinding(tokens: string[], forId: string): string | null {
	if (tokens.length === 0) return 'empty binding';
	const joined = tokens.join(' ');
	// Collisions with every other command's EFFECTIVE binding (custom or
	// shipped default) — equal, shadowed-by, or shadowing.
	for (const cmd of commands) {
		if (cmd.id === forId) continue;
		const eff = commandPrefs.byId[cmd.id]?.keys ?? cmd.keybinding;
		if (!eff) continue;
		const short = cmd.name.replace(/^tendrl-/, '');
		if (eff === joined) return `taken by ${short}`;
		if (eff.startsWith(joined + ' ')) return `would shadow ${short} (${eff})`;
		if (joined.startsWith(eff + ' ')) return `${eff} (${short}) blocks this chord`;
	}
	if (tokens[0] === 'SPC') {
		if (tokens.length === 1) return 'SPC alone is the leader';
		return validateLeaderChord(tokens.slice(1), forId);
	}
	if (tokens.length > 1) return 'sequences must start with SPC';
	if (RESERVED_SINGLE.has(tokens[0])) return `${tokens[0]} is a reserved normal-mode key`;
	return null;
}
