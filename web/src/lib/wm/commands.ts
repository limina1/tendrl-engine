// The single source of truth for the SPC : command palette. Every entry
// here is what M-x completion offers; `deferred` marks entries that are
// declared but not yet wired to a handler (the palette toasts instead of
// silently no-oping). Keybinding strings list only bindings that actually
// fire — aspirational chords (C-x b, C-/, g r …) were removed when this
// registry was extracted from +page.svelte.
//
// Leaf module on purpose: imports types only, so buffer renderers
// (SettingsBuffer) can import it without a wm/registry cycle.
import type { Command, CommandScope } from './types.js';

export const SCOPE_ORDER: CommandScope[] = ['action', 'contextual', 'opener', 'nav'];

export const SCOPE_META: Record<CommandScope, { label: string; blurb: string }> = {
	action: {
		label: 'direct actions',
		blurb: 'Act immediately, from anywhere — no target or setup needed.'
	},
	contextual: {
		label: 'contextual actions',
		blurb: 'Real actions, but only meaningful in the right situation — inert elsewhere.'
	},
	opener: {
		label: 'openers',
		blurb: 'Open the surface where the real action happens (a buffer or form).'
	},
	nav: {
		label: 'navigation',
		blurb: 'Window and buffer plumbing — moving around, not acting on content.'
	}
};

export const commands: Command[] = [
	// Buffer
	{ id: 'tendrl-switch-buffer', name: 'tendrl-switch-buffer', description: 'Switch buffer in focused slot (class-scoped)', category: 'Buffer', scope: 'nav', keybinding: 'SPC b b' },
	{ id: 'tendrl-switch-buffer-global', name: 'tendrl-switch-buffer-global', description: 'Switch to any open buffer across all classes', category: 'Buffer', scope: 'nav', keybinding: 'SPC b B' },
	{ id: 'tendrl-recent-buffer', name: 'tendrl-recent-buffer', description: 'Re-open a recently closed buffer', category: 'Buffer', scope: 'nav', keybinding: 'SPC b r' },
	{ id: 'tendrl-kill-buffer', name: 'tendrl-kill-buffer', description: 'Kill the focused buffer', category: 'Buffer', scope: 'nav', keybinding: 'SPC b k' },
	{ id: 'tendrl-find-event', name: 'tendrl-find-event', description: 'Open a Nostr event by id or address into a reader', category: 'Buffer', scope: 'opener', deferred: true, keybinding: 'SPC f e' },
	{ id: 'tendrl-find-draft', name: 'tendrl-find-draft', description: 'Open a draft into a composer', category: 'Buffer', scope: 'opener', deferred: true, keybinding: 'SPC f d' },
	// Window
	{ id: 'tendrl-toggle-rail', name: 'tendrl-toggle-rail', description: 'Collapse focused slot to rail (or expand if rail)', category: 'Window', scope: 'nav', keybinding: 'SPC w c', shells: ['desktop'] },
	{ id: 'tendrl-split-window', name: 'tendrl-split-window', description: 'Split focused slot horizontally with another same-class buffer', category: 'Window', scope: 'nav', keybinding: 'SPC w s', shells: ['desktop'] },
	// Layout
	{ id: 'tendrl-switch-layout', name: 'tendrl-switch-layout', description: 'Switch the active layout', category: 'Layout', scope: 'nav', deferred: true, keybinding: 'SPC l b', shells: ['desktop'] },
	{ id: 'tendrl-save-layout', name: 'tendrl-save-layout', hiddenByDefault: true, description: 'Save the current frame configuration as a named layout', category: 'Layout', scope: 'nav', deferred: true, shells: ['desktop'] },
	// Compose
	{ id: 'tendrl-save-draft', name: 'tendrl-save-draft', hiddenByDefault: true, description: 'Save the current draft to the engine', category: 'Compose', scope: 'contextual', context: 'a composer buffer', deferred: true },
	{ id: 'tendrl-publish-draft', name: 'tendrl-publish-draft', hiddenByDefault: true, description: 'Sign and broadcast the current draft', category: 'Compose', scope: 'contextual', context: 'a composer buffer', deferred: true },
	{ id: 'tendrl-fork-section', name: 'tendrl-fork-section', hiddenByDefault: true, description: 'Fork an imported section to make it editable', category: 'Compose', scope: 'contextual', context: 'a composer with an imported section', deferred: true },
	{ id: 'tendrl-cycle-editor-view', name: 'tendrl-cycle-editor-view', description: 'Cycle through composer modes (button/plain/wysiwyg/preview)', category: 'Compose', scope: 'contextual', context: 'a composer buffer', deferred: true, keybinding: 'SPC e v' },
	{ id: 'tendrl-highlight', name: 'tendrl-highlight', description: 'General highlighter — paste text, cite any source (nostr / URL / ISBN / DOI), annotate, publish', category: 'Compose', scope: 'action', keybinding: 'SPC h' },
	// Configuration
	{ id: 'tendrl-toggle-network-mode', name: 'tendrl-toggle-network-mode', description: 'Toggle between auto and confirm network mode', category: 'Configuration', scope: 'action', keybinding: 'SPC t n' },
	{ id: 'tendrl-show-relays', name: 'tendrl-show-relays', description: 'Open the relay-config buffer', category: 'Configuration', scope: 'opener', keybinding: 'SPC r r' },
	{ id: 'tendrl-open-settings', name: 'tendrl-open-settings', description: 'Open the settings buffer', category: 'Configuration', scope: 'opener', keybinding: 'SPC s s' },
	{ id: 'tendrl-demo-publish-progress', name: 'tendrl-demo-publish-progress', hiddenByDefault: true, description: 'Open the publish-progress buffer with mock data (design demo)', category: 'Configuration', scope: 'action' },
	{ id: 'tendrl-login', name: 'tendrl-login', description: 'Open settings at the identity login form (ncryptsec or NIP-07)', category: 'Configuration', scope: 'opener', keybinding: 'SPC s i' },
	{ id: 'tendrl-logout', name: 'tendrl-logout', description: 'Logout active identity', category: 'Configuration', scope: 'action' },
	{ id: 'tendrl-switch-source', name: 'tendrl-switch-source', hiddenByDefault: true, description: 'Open settings to switch signing source (engine / nip07 / signer app)', category: 'Configuration', scope: 'opener' },
	{ id: 'tendrl-edit-profile', name: 'tendrl-edit-profile', description: 'Edit your kind 0 profile metadata and broadcast', category: 'Configuration', scope: 'opener', keybinding: 'SPC s p' },
	{ id: 'tendrl-embed-missing', name: 'tendrl-embed-missing', description: 'Embed knowledge-base events not yet in the semantic index', category: 'Configuration', scope: 'action', keybinding: 'SPC e m' },
	{ id: 'tendrl-reembed-all', name: 'tendrl-reembed-all', description: 'Clear the semantic index and re-embed every eligible event', category: 'Configuration', scope: 'action', keybinding: 'SPC e A' },
	// View
	{ id: 'tendrl-show-event-json', name: 'tendrl-show-event-json', hiddenByDefault: true, description: 'Show the raw JSON of the focused event', category: 'View', scope: 'contextual', context: 'a focused buffer carrying an event' },
	{ id: 'tendrl-highlight-mode', name: 'tendrl-highlight-mode', hiddenByDefault: true, description: 'Toggle highlight mode — select text in a reader/doc to publish a NIP-84 highlight', category: 'View', scope: 'contextual', context: 'a reader or doc buffer, then a text selection' },
	// Versioning
	{ id: 'tendrl-undo', name: 'tendrl-undo', hiddenByDefault: true, description: 'Undo the last action', category: 'Versioning', scope: 'contextual', context: 'an editable buffer', deferred: true },
	{ id: 'tendrl-redo', name: 'tendrl-redo', hiddenByDefault: true, description: 'Redo', category: 'Versioning', scope: 'contextual', context: 'an editable buffer', deferred: true },
	// Application
	{ id: 'tendrl-quit', name: 'tendrl-quit', hiddenByDefault: true, description: 'Close this frame', category: 'Application', scope: 'nav', deferred: true },
	{ id: 'tendrl-refresh', name: 'tendrl-refresh', description: 'Reload the focused buffer', category: 'Application', scope: 'contextual', context: 'a focused buffer', deferred: true, keybinding: 'SPC b R' },
	{ id: 'tendrl-cycle-shell', name: 'tendrl-cycle-shell', description: 'Cycle the shell: auto → desktop (WM) → mobile (bottom bar)', category: 'Application', scope: 'action' },
	// The desktop entry point for the walk is the logo's W dropdown — chrome the
	// mobile shell doesn't render, so the palette carries an entry in both shells.
	{ id: 'tendrl-run-walkthrough', name: 'tendrl-run-walkthrough', description: 'Replay the first-run guided walkthrough from the top', category: 'Application', scope: 'action' }
];

/** Whether a command is offered in the given shell (absent `shells` = both).
 *  Takes the mode as a literal so this module stays a leaf — callers read
 *  it from shell.svelte.ts. */
export function commandInShell(c: Command, mode: 'desktop' | 'mobile'): boolean {
	return !c.shells || c.shells.includes(mode);
}

/** Normal-mode keys hardwired in +page.svelte's onGlobalKeydown — the
 *  layer beneath the leader tree. Listed here so the settings keybinding
 *  registry shows the whole keyboard surface, not just SPC chords. */
export type BaseKey = { keys: string; desc: string };
export const BASE_KEYS: BaseKey[] = [
	{ keys: 'SPC', desc: 'leader — opens the which-key menu (all SPC chords below)' },
	{ keys: 'h j k l · arrows', desc: 'move the cursor in the focused buffer' },
	{ keys: 'g g · G', desc: 'jump to top / bottom of the focused buffer' },
	{ keys: 'Enter', desc: 'select / open the cursored item' },
	{ keys: 'm', desc: 'open the event menu on the cursored item' },
	{ keys: 'i', desc: 'insert mode — focus the buffer’s entry field' },
	{ keys: 'o', desc: 'open new (e.g. composer block), then insert' },
	{ keys: ':', desc: 'command palette (same as SPC :)' },
	{ keys: 'Esc · C-[ · C-g', desc: 'leave insert mode / close minibuffer / cancel leader' },
	{ keys: 'C-n · C-p', desc: 'next / previous row inside the minibuffer' }
];
