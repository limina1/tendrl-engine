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
import { validateLeaderChord, type LeaderBindingOverride } from './leader.js';

export type CommandPref = {
	/** Hidden from the SPC : palette (bindings, if any, keep working). */
	hidden?: boolean;
	/** Custom binding, space-joined tokens: 'SPC o s' (leader chord) or 'u'
	 *  (single normal-mode key). Replaces the default binding. */
	keys?: string;
};

const STORAGE_KEY = 'tendrl.command-prefs.v1';

function load(): Record<string, CommandPref> {
	if (typeof localStorage === 'undefined') return {};
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		return raw ? JSON.parse(raw) : {};
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

export function setHidden(id: string, hidden: boolean) {
	const p = { ...prefFor(id) };
	if (hidden) p.hidden = true;
	else delete p.hidden;
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
	return commands.flatMap((cmd) => {
		const keys = commandPrefs.byId[cmd.id]?.keys;
		if (!keys) return [];
		// Tokens are the chord AFTER the SPC prefix. A single-key binding
		// contributes an empty-token override: it still removes the
		// command's default leader leaf (the custom binding replaces the
		// default), but nothing is grafted — dispatch happens via
		// singleKeyBindings() in normal-mode keydown.
		const tokens = keys.startsWith('SPC ') ? keys.split(' ').slice(1) : [];
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
	// Collision with another command's custom binding.
	const joined = tokens.join(' ');
	for (const [id, p] of Object.entries(commandPrefs.byId)) {
		if (id !== forId && p.keys === joined) {
			return `already bound to ${id.replace(/^tendrl-/, '')}`;
		}
	}
	if (tokens[0] === 'SPC') {
		if (tokens.length === 1) return 'SPC alone is the leader';
		return validateLeaderChord(tokens.slice(1), forId);
	}
	if (tokens.length > 1) return 'sequences must start with SPC';
	if (RESERVED_SINGLE.has(tokens[0])) return `${tokens[0]} is a reserved normal-mode key`;
	return null;
}
