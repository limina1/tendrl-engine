<script lang="ts">
	import type {
		Buffer,
		ClassName,
		Command,
		CommandCat,
		LayoutConfig,
		MinibufferMode,
		OpenBuf,
		Position,
		Slot,
		SlotState,
		SplitNode
	} from '$lib/wm/types';
	import {
		buildLeaderRoot,
		resolveLeaderNode,
		type LeaderNode,
		type SubPrefix
	} from '$lib/wm/leader';
	import { BufferStore, setActiveStore } from '$lib/wm/buffer-store.svelte';
	import BufferRenderer from '$lib/wm/BufferRenderer.svelte';
	import { rendererFor } from '$lib/wm/registry';
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();

	// Singleton buffers seeded on every frame.
	const chatBuf: Buffer = { id: 'chat', kind: 'chat', label: 'chat' };
	const feedBuf: Buffer = { id: 'feed', kind: 'feed', label: 'feed' };
	const composerBuf: Buffer = { id: 'composer:current', kind: 'composer', label: 'composer', kicker: 'untitled draft' };
	const searchBuf: Buffer = { id: 'search:default', kind: 'search', label: 'search' };
	const refsBuf: Buffer = { id: 'refs', kind: 'refs', label: 'refs' };
	const kbBuf: Buffer = { id: 'kb', kind: 'knowledgebase', label: 'kb' };
	const ignoredBuf: Buffer = { id: 'ignored', kind: 'ignored', label: 'ignored' };

	const openBuffers: OpenBuf[] = [
		{ className: 'chat', buffer: chatBuf },
		// Work class — main content surface. Cycles via SPC b b: feed → reader
		// → composer → ... Click a publication in the feed to spawn a reader
		// that joins this cycle (replaces feed in the active leaf).
		{ className: 'work', buffer: feedBuf },
		{ className: 'work', buffer: composerBuf },
		{ className: 'work', buffer: ignoredBuf },
		// Research class — auxiliary tools (search, refs, kb).
		{ className: 'research', buffer: searchBuf },
		{ className: 'research', buffer: refsBuf },
		{ className: 'research', buffer: kbBuf }
	];

	// One base layout: chat rail-left / work center / research rail-right. Each
	// slot collapsible via SPC w c. Named "modes" are just slot-state combos the
	// user produces interactively — no need for predefined layouts. A `chat`
	// preset is kept since chat-wide-left is genuinely different geometry.
	// Future: user-savable perspectives via `SPC l s` recalled via `SPC l <key>`.
	const layouts: Record<string, LayoutConfig> = {
		base: {
			name: 'base',
			desc: 'Workbench — main content center (feed/reader/composer cycle), chat and research as rails.',
			slots: {
				left: { className: 'chat', state: 'rail', tree: { type: 'leaf', buffer: chatBuf } },
				center: { className: 'work', state: 'open', tree: { type: 'leaf', buffer: feedBuf } },
				right: { className: 'research', state: 'rail', tree: { type: 'leaf', buffer: searchBuf } }
			}
		},
		chat: {
			name: 'chat',
			desc: 'Chat wide-left preset. Work in center, research as rail.',
			slots: {
				left: { className: 'chat', state: 'open', tree: { type: 'leaf', buffer: chatBuf } },
				center: { className: 'work', state: 'open', tree: { type: 'leaf', buffer: feedBuf } },
				right: { className: 'research', state: 'rail', tree: { type: 'leaf', buffer: searchBuf } }
			}
		}
	};

	const store = new BufferStore(layouts, 'base');
	store.seed(openBuffers);
	setActiveStore(store);

	// Redirect AppState navigation calls to spawn buffers in the shell
	// instead of route-navigating away from /design/shell.
	app.setNavigationHandlers({
		onPublication: (pubkey, d_tag) => {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:30040:${pubkey}:${d_tag}`,
					kind: 'reader',
					label: 'reader',
					kicker: d_tag
				}
			});
		},
		onProfile: (pubkey) => {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `profile:${pubkey}`,
					kind: 'profile',
					label: 'profile',
					kicker: pubkey.slice(0, 8) + '…'
				}
			});
		},
		onCompose: () => {
			store.openBuffer({
				className: 'work',
				buffer: { id: 'composer:current', kind: 'composer', label: 'composer', kicker: 'draft' }
			});
		},
		onHome: () => {
			store.openBuffer({
				className: 'work',
				buffer: { id: 'feed', kind: 'feed', label: 'feed' }
			});
		}
	});

	let prefixPath = $state<string[]>([]);
	let mode = $state<'normal' | 'insert'>('normal');
	let composerExtraBlocks = $state(0);
	let slotBodyEls: Partial<Record<Position, HTMLElement>> = {};

	// Representative subset — production reads from src/tree/command.rs (61 commands).
	const commands: Command[] = [
		// Buffer
		{ id: 'tendrl-switch-buffer', name: 'tendrl-switch-buffer', description: 'Switch buffer in focused slot (class-scoped)', category: 'Buffer', keybinding: 'SPC b b · C-x b' },
		{ id: 'tendrl-switch-buffer-global', name: 'tendrl-switch-buffer-global', description: 'Switch to any open buffer across all classes', category: 'Buffer', keybinding: 'SPC b B' },
		{ id: 'tendrl-recent-buffer', name: 'tendrl-recent-buffer', description: 'Re-open a recently closed buffer', category: 'Buffer', keybinding: 'SPC b r' },
		{ id: 'tendrl-kill-buffer', name: 'tendrl-kill-buffer', description: 'Kill the focused buffer (prompts to save if dirty)', category: 'Buffer', keybinding: 'SPC b k · C-x k' },
		{ id: 'tendrl-find-event', name: 'tendrl-find-event', description: 'Open a Nostr event by id or address into a reader', category: 'Buffer', keybinding: 'SPC f e' },
		{ id: 'tendrl-find-draft', name: 'tendrl-find-draft', description: 'Open a draft into a composer', category: 'Buffer', keybinding: 'SPC f d' },
		// Window
		{ id: 'tendrl-toggle-rail', name: 'tendrl-toggle-rail', description: 'Collapse focused slot to rail (or expand if rail)', category: 'Window', keybinding: 'SPC w c' },
		{ id: 'tendrl-split-window', name: 'tendrl-split-window', description: 'Split focused slot horizontally with another same-class buffer', category: 'Window', keybinding: 'SPC w s' },
		// Layout
		{ id: 'tendrl-switch-layout', name: 'tendrl-switch-layout', description: 'Switch the active layout (read/write/triage/chat/zen)', category: 'Layout', keybinding: 'SPC L' },
		{ id: 'tendrl-save-layout', name: 'tendrl-save-layout', description: 'Save the current frame configuration as a named layout', category: 'Layout' },
		// Compose
		{ id: 'tendrl-save-draft', name: 'tendrl-save-draft', description: 'Save the current draft to the engine', category: 'Compose', keybinding: 'C-x C-s' },
		{ id: 'tendrl-publish-draft', name: 'tendrl-publish-draft', description: 'Sign and broadcast the current draft', category: 'Compose' },
		{ id: 'tendrl-fork-section', name: 'tendrl-fork-section', description: 'Fork an imported section to make it editable', category: 'Compose' },
		{ id: 'tendrl-cycle-editor-view', name: 'tendrl-cycle-editor-view', description: 'Cycle through composer modes (button/plain/wysiwyg/preview)', category: 'Compose' },
		// Configuration
		{ id: 'tendrl-toggle-network-mode', name: 'tendrl-toggle-network-mode', description: 'Toggle between online and offline mode', category: 'Configuration' },
		{ id: 'tendrl-show-relays', name: 'tendrl-show-relays', description: 'Show configured relays', category: 'Configuration' },
		{ id: 'tendrl-login', name: 'tendrl-login', description: 'Unlock identity (ncryptsec)', category: 'Configuration' },
		{ id: 'tendrl-logout', name: 'tendrl-logout', description: 'Lock identity', category: 'Configuration' },
		// View
		{ id: 'tendrl-show-event-json', name: 'tendrl-show-event-json', description: 'Show the raw JSON of the focused event', category: 'View' },
		// Versioning
		{ id: 'tendrl-undo', name: 'tendrl-undo', description: 'Undo the last action', category: 'Versioning', keybinding: 'C-/ · u' },
		{ id: 'tendrl-redo', name: 'tendrl-redo', description: 'Redo', category: 'Versioning', keybinding: 'C-S-/' },
		// Application
		{ id: 'tendrl-quit', name: 'tendrl-quit', description: 'Close this frame', category: 'Application', keybinding: 'C-x C-c' },
		{ id: 'tendrl-refresh', name: 'tendrl-refresh', description: 'Reload the focused buffer', category: 'Application', keybinding: 'g r' }
	];
	let mb = $state<{ mode: MinibufferMode; query: string; selectedIndex: number }>({
		mode: 'closed',
		query: '',
		selectedIndex: 0
	});

	store.recentlyClosed = [
		{ className: 'work', buffer: { id: 'r1', kind: 'reader', label: 'reader', kicker: 'NIP-23 long-form · §2' } },
		{ className: 'research', buffer: { id: 'r2', kind: 'search', label: 'search', kicker: 'by:fiatjaf since:30d' } }
	];

	function setLayout(name: string) {
		store.setLayout(name);
		composerExtraBlocks = 0;
		closeMinibuffer();
	}

	function selectBuffer(entry: OpenBuf) {
		if (mb.mode === 'split') {
			store.splitFocused(entry.buffer, 'h');
		} else {
			store.selectBuffer(entry);
		}
		closeMinibuffer();
	}

	function openMinibuffer(mode: MinibufferMode) {
		mb = { mode, query: '', selectedIndex: 0 };
		prefixPath = [];
	}

	function closeMinibuffer() {
		mb = { mode: 'closed', query: '', selectedIndex: 0 };
	}

	const minibufferEntries = $derived.by<OpenBuf[]>(() => {
		if (mb.mode === 'closed' || mb.mode === 'mx') return [];
		const source = mb.mode === 'recent' ? store.recentlyClosed : store.openBuffers;
		const cls = store.focusedSlotClass();
		const filtered =
			(mb.mode === 'class' || mb.mode === 'split') && cls
				? source.filter((e) => e.className === cls)
				: source;
		const q = mb.query.trim().toLowerCase();
		if (!q) return filtered;
		return filtered.filter(
			(e) =>
				e.buffer.label.toLowerCase().includes(q) ||
				(e.buffer.kicker?.toLowerCase().includes(q) ?? false) ||
				e.className.toLowerCase().includes(q)
		);
	});

	const mxEntries = $derived.by<Command[]>(() => {
		if (mb.mode !== 'mx') return [];
		const q = mb.query.trim().toLowerCase();
		if (!q) return commands;
		return commands.filter(
			(c) =>
				c.name.toLowerCase().includes(q) ||
				c.description.toLowerCase().includes(q) ||
				c.category.toLowerCase().includes(q)
		);
	});

	function executeCommand(cmd: Command) {
		if (cmd.id === 'tendrl-switch-buffer') {
			openMinibuffer('class');
			return;
		}
		if (cmd.id === 'tendrl-switch-buffer-global') {
			openMinibuffer('global');
			return;
		}
		if (cmd.id === 'tendrl-recent-buffer') {
			openMinibuffer('recent');
			return;
		}
		if (cmd.id === 'tendrl-toggle-rail') {
			store.toggleFocusedSlot();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-toggle-network-mode') {
			toggleNetworkMode();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-show-event-json') {
			const fb = focusedBuffer;
			app.jsonModalData = fb ? { buffer: fb } : null;
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-logout') {
			app.handleIdentityLogout();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-login') {
			app.handleIdentityLock();
			closeMinibuffer();
			return;
		}
		// Stubs for unrecognized commands.
		closeMinibuffer();
	}

	function focusEntryFieldIn(pos: Position) {
		const body = slotBodyEls[pos];
		if (!body) return false;
		const input = body.querySelector<HTMLElement>('[data-entry]');
		if (input) {
			input.focus();
			return true;
		}
		return false;
	}

	function enterInsertMode() {
		mode = 'insert';
		// Try to focus the entry field of the currently focused slot.
		setTimeout(() => focusEntryFieldIn(store.focusedSlot), 0);
	}

	function exitInsertMode() {
		mode = 'normal';
		(document.activeElement as HTMLElement | null)?.blur();
	}

	function openAndInsert() {
		// `o` semantics: open something new, then enter insert.
		store.expandFocusedIfRail();
		const buf = focusedBuffer;
		if (buf?.kind === 'composer') {
			composerExtraBlocks += 1;
		}
		// For splittable non-composer buffers (reader, search, etc.), a same-class
		// split-create would happen here — deferred for now, falls through to insert.
		enterInsertMode();
	}

	function onGlobalKeydown(e: KeyboardEvent) {
		// Minibuffer always wins.
		if (mb.mode !== 'closed') {
			const len = mb.mode === 'mx' ? mxEntries.length : minibufferEntries.length;
			if (e.key === 'Escape' || (e.key === '[' && e.ctrlKey) || (e.key === 'g' && e.ctrlKey)) {
				e.preventDefault();
				closeMinibuffer();
			} else if (e.key === 'ArrowDown' || (e.key === 'n' && e.ctrlKey)) {
				e.preventDefault();
				mb.selectedIndex = Math.min(len - 1, mb.selectedIndex + 1);
			} else if (e.key === 'ArrowUp' || (e.key === 'p' && e.ctrlKey)) {
				e.preventDefault();
				mb.selectedIndex = Math.max(0, mb.selectedIndex - 1);
			} else if (e.key === 'Enter') {
				e.preventDefault();
				if (mb.mode === 'mx') {
					const sel = mxEntries[mb.selectedIndex];
					if (sel) executeCommand(sel);
				} else {
					const sel = minibufferEntries[mb.selectedIndex];
					if (sel) selectBuffer(sel);
				}
			}
			return;
		}

		// Insert mode: only Esc / C-[ exits.
		if (mode === 'insert') {
			if (e.key === 'Escape' || (e.key === '[' && e.ctrlKey) || (e.key === 'g' && e.ctrlKey)) {
				e.preventDefault();
				exitInsertMode();
			}
			return;
		}

		// Leader is open: every keystroke either descends, executes, or cancels.
		if (leaderOpen) {
			if (e.key === 'Escape' || (e.key === '[' && e.ctrlKey) || (e.key === 'g' && e.ctrlKey)) {
				e.preventDefault();
				prefixPath = [];
				return;
			}
			if (e.key === 'Backspace') {
				e.preventDefault();
				leaderUp();
				return;
			}
			if (e.metaKey || e.altKey || e.ctrlKey) return;
			e.preventDefault();
			leaderDescend(e.key);
			return;
		}

		// Normal mode: vim-style navigation + leader-ish.
		// Don't trigger if a meta-key is held (let browser shortcuts work).
		if (e.metaKey || e.altKey) return;

		if (e.key === ' ') {
			e.preventDefault();
			openLeader();
			return;
		}

		if (e.key === 'h' || e.key === 'ArrowLeft') {
			e.preventDefault();
			store.navigateSlot(-1);
		} else if (e.key === 'l' || e.key === 'ArrowRight') {
			e.preventDefault();
			store.navigateSlot(1);
		} else if (e.key === 'j' || e.key === 'ArrowDown') {
			e.preventDefault();
			store.cycleBufferInSlot(1);
		} else if (e.key === 'k' || e.key === 'ArrowUp') {
			e.preventDefault();
			store.cycleBufferInSlot(-1);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			store.expandFocusedIfRail();
		} else if (e.key === 'i') {
			e.preventDefault();
			store.expandFocusedIfRail();
			enterInsertMode();
		} else if (e.key === 'o') {
			e.preventDefault();
			openAndInsert();
		} else if (e.key === ':') {
			e.preventDefault();
			openMinibuffer('mx');
		}
	}

	function prefilterMx(name: string) {
		openMinibuffer('mx');
		mb.query = name;
		mb.selectedIndex = 0;
	}

	function toggleNetworkMode() {
		const next = app.networkStatus?.mode === 'online' ? 'offline' : 'online';
		app.handleSetNetworkMode(next);
	}

	const leaderRoot: SubPrefix = buildLeaderRoot({
		openMinibuffer,
		prefilterMx,
		killFocusedBuffer: () => store.killFocused(),
		cycleBufferInSlot: (dir) => store.cycleBufferInSlot(dir),
		toggleFocusedSlot: () => store.toggleFocusedSlot(),
		navigateSlot: (dir) => store.navigateSlot(dir),
		setLayout,
		toggleNetworkMode,
		openSplitPicker: () => openMinibuffer('split')
	});

	function openLeader() {
		closeMinibuffer();
		prefixPath = ['SPC'];
	}

	function leaderDescend(key: string) {
		const node = currentLeaderNode;
		if (!node || node.type !== 'prefix') {
			prefixPath = [];
			return;
		}
		const child = node.children[key];
		if (!child) return; // unknown key — popup stays open, ignore
		if (child.type === 'prefix') {
			prefixPath = [...prefixPath, key];
		} else {
			if (child.deferred) return; // deferred leaves are inert
			const run = child.run;
			prefixPath = [];
			run();
		}
	}

	function leaderUp() {
		if (prefixPath.length > 1) prefixPath = prefixPath.slice(0, -1);
		else prefixPath = [];
	}

	const focusedBuffer = $derived(store.focusedBuffer());

	const leaderOpen = $derived(prefixPath[0] === 'SPC');

	const currentLeaderNode = $derived.by<LeaderNode | null>(() => {
		if (!leaderOpen) return null;
		return resolveLeaderNode(leaderRoot, prefixPath.slice(1));
	});

	const leaderPathLabel = $derived(prefixPath.length === 0 ? '' : prefixPath.join(' '));

	const modelineText = $derived.by(() => {
		const parts: string[] = [];
		parts.push(`L:${store.currentLayout.name}`);
		const cls = store.focusedSlotClass();
		if (cls) parts.push(`@${cls}`);
		if (focusedBuffer) {
			const star = focusedBuffer.modified ? ' *' : '';
			parts.push(`${focusedBuffer.label}${star}${focusedBuffer.kicker ? ` (${focusedBuffer.kicker})` : ''}`);
		}
		if (leaderOpen) parts.push(`[${leaderPathLabel}-]`);
		if (mb.mode !== 'closed') parts.push(`[mb:${mb.mode}]`);
		const netMode = app.networkStatus?.mode ?? '?';
		parts.push(netMode);
		const id = app.identityStatus;
		if (id?.state === 'unlocked') {
			const npub = id.npub ?? '';
			parts.push(`@${npub.slice(0, 12)}`);
		} else if (id?.state === 'locked') {
			parts.push('locked');
		}
		return parts.join('  ·  ');
	});

	const layoutOrder: string[] = ['base', 'chat'];

	const minibufferTitle = $derived.by(() => {
		if (mb.mode === 'class') {
			const cls = store.focusedSlotClass();
			return `Switch buffer · ${cls ?? '?'} class · ${minibufferEntries.length} open`;
		}
		if (mb.mode === 'split') {
			const cls = store.focusedSlotClass();
			return `Split with · ${cls ?? '?'} class · ${minibufferEntries.length} open`;
		}
		if (mb.mode === 'global') return `Switch buffer · global · ${minibufferEntries.length} open`;
		if (mb.mode === 'recent') return `Recently closed · ${minibufferEntries.length}`;
		if (mb.mode === 'mx') return `M-x · ${mxEntries.length} commands`;
		return '';
	});
</script>

<svelte:head><title>tendrl · WM shell</title></svelte:head>

<svelte:window onkeydown={onGlobalKeydown} />

<div class="page">
	<header class="page__head">
		<div class="eyebrow">design · interactive</div>
		<h1 class="title">WM Shell — class-typed slots + buffer switcher</h1>
		<p class="lede">
			Three slots: <code>chat</code>, <code>work</code>, <code>research</code>. Splits restricted to same class.
			Press <span class="kbd">SPC</span> for the leader popup; descend with <span class="kbd">b b</span> for class-scoped switch,
			<span class="kbd">b B</span> for global, <span class="kbd">l r/w/t/c/z</span> for layouts, etc.
		</p>
		<div class="hint">
			Click a layout button to switch · Click a slot's <span class="kbd">×</span> to collapse to rail · Click a rail to reopen ·
			Click a slot to focus it · <span class="kbd">SPC</span> opens the leader popup · <span class="kbd">:</span> opens M-x ·
			<span class="kbd">↑</span><span class="kbd">↓</span> + <span class="kbd">Enter</span> in the minibuffer
		</div>
	</header>

	<div class="shell">
		<div class="shell__header">
			<div class="shell__brand">tendrl</div>
			<div class="shell__layouts">
				{#each layoutOrder as name (name)}
					<button
						class="lt {store.currentLayoutName === name ? 'lt--on' : ''}"
						onclick={() => setLayout(name)}
					>
						{name}
					</button>
				{/each}
			</div>
			<div class="shell__layout-desc">{store.currentLayout.desc}</div>
			<button
				class="px {leaderOpen ? 'px--on' : ''}"
				onclick={() => (leaderOpen ? (prefixPath = []) : openLeader())}
				title="SPC — leader prefix (which-key popup)"
			>SPC</button>
			<button class="shell__mx {mb.mode === 'mx' ? 'shell__mx--on' : ''}" onclick={() => openMinibuffer('mx')} title="M-x · command palette">M-x</button>
		</div>

		<div class="shell__body">
			{#each store.positionOrder as pos (pos)}
				{@const slot = store.slotFor(pos)}
				{#if slot && slot.state === 'open'}
					{@render windowSlot(pos, slot)}
				{:else if slot && slot.state === 'rail'}
					{@render railSlot(pos, slot)}
				{/if}
			{/each}
		</div>

		{#if leaderOpen && currentLeaderNode?.type === 'prefix'}
			{@render leaderPopup(currentLeaderNode)}
		{/if}

		{#if mb.mode !== 'closed'}
			{@render minibufferStrip()}
		{/if}

		<div class="shell__modeline">
			<span class="ml__mode ml__mode--{mode}">-- {mode.toUpperCase()} --</span>
			<span class="ml__layout">{modelineText}</span>
			<span class="ml__spacer"></span>
			<span class="ml__pos">L42:18</span>
		</div>
	</div>

	<section class="legend">
		<h2 class="legend__title">Reading the shell</h2>
		<div class="legend__grid">
			<div class="legend__item">
				<div class="legend__h">Class badges</div>
				<p>
					<span class="cls cls--chat">chat</span>
					<span class="cls cls--work">work</span>
					<span class="cls cls--research">research</span>
					— each slot wears its class. Internal splits are restricted to same-class buffers.
				</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Buffer classes</div>
				<p><strong>chat</strong> = chat (singleton). <strong>work</strong> = main content surface — feed, reader, composer, profile, ignored. Cycle via <span class="kbd">SPC b b</span> to move through read/write/feed modes. <strong>research</strong> = auxiliary tools — search, refs, knowledgebase.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Class transitions</div>
				<p>Selecting a section/document from a research buffer (feed, kb, search) opens it as a reader in the work slot. The source list isn't duplicated.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Switcher · class-scoped</div>
				<p><span class="kbd">SPC b b</span> shows only the focused slot's class. To reach another class's buffers, focus that slot first (window-nav) or use the global switcher.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Switcher · global</div>
				<p><span class="kbd">SPC b B</span> shows every open buffer with its class tag. Selecting jumps focus to that buffer's class slot, restoring it from rail or hidden, with a brief flash.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Recently closed</div>
				<p><span class="kbd">SPC b r</span> shows recently-killed buffers. Click to re-open.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">M-x · command palette</div>
				<p>Click <span class="kbd">M-x</span> in the header (or <span class="kbd">SPC :</span>) to open the global command runner. Lists every named command with description, category, and keybinding hint. The same minibuffer mechanism as the buffer switchers.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Commands as the API</div>
				<p>Every action is a named command. Keybindings are shortcuts to commands. M-x is the discovery surface. Production reads from the existing TUI registry in <code>src/tree/command.rs</code> (61 commands).</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">Modal editing</div>
				<p>
					<span class="kbd">NORMAL</span>: <span class="kbd">h</span> <span class="kbd">l</span> focus prev/next slot · <span class="kbd">j</span> <span class="kbd">k</span> cycle buffer in slot's class · <span class="kbd">Enter</span> expands focused rail · <span class="kbd">:</span> opens M-x · <span class="kbd">SPC</span> opens leader.
				</p>
				<p>
					<span class="kbd">i</span> — standard insert (focus the focused buffer's entry field).
				</p>
				<p>
					<span class="kbd">o</span> — open-and-insert. On <strong>composer</strong>: appends a new section block and inserts. On <strong>chat</strong> (singleton): same as <span class="kbd">i</span>. On other splittable buffers: would create a same-class split — split-create is deferred, so falls back to <span class="kbd">i</span>.
				</p>
				<p>
					<span class="kbd">INSERT</span>: type into focused field. <span class="kbd">Esc</span> · <span class="kbd">C-[</span> · <span class="kbd">C-g</span> all return to normal.
				</p>
				<p>The minibuffer and leader popup each take over key handling when open — same escapes close them.</p>
			</div>
			<div class="legend__item">
				<div class="legend__h">SPC leader (which-key)</div>
				<p>
					Press <span class="kbd">SPC</span> in normal mode (or click the <span class="kbd">SPC</span> button) to open a popup of next-key bindings. Each key descends into a sub-prefix or executes a leaf. <span class="kbd">Backspace</span> goes up one level; <span class="kbd">Esc</span> · <span class="kbd">C-g</span> · <span class="kbd">C-[</span> cancel.
				</p>
				<p>
					Leaves carry a <span class="kbd">kind</span> tag — <em>engine</em> commands go through the Tendrl HTTP API, <em>client</em> commands are pure UI. The same split applies in the eventual Tendrl+Emacs port.
				</p>
				<p>
					Deferred leaves (e.g. <span class="kbd">SPC w s</span> split-create) are shown grayed out for discoverability.
				</p>
			</div>
		</div>
	</section>

	<section class="notes">
		<h2 class="notes__title">What this artboard isn't yet</h2>
		<ul>
			<li>SPC leader is wired, but <em>timeout suppression</em> (Doom's "no popup if released quickly") is deferred — the popup always opens.</li>
			<li>Sub-prefix hover/help (peek into a child branch) is deferred.</li>
			<li>User customization of the prefix tree is deferred — tree is hard-coded.</li>
			<li><code>SPC f e/d/p</code> (engine find commands) currently just prefilter M-x — actual engine API calls aren't wired in the artboard.</li>
			<li>No drag-split or splits-create — splits shown are layout-defined; <code>SPC w s</code> is shown grayed.</li>
			<li>No persistence — refresh resets to <code>write</code>.</li>
		</ul>
	</section>
</div>

{#snippet windowSlot(pos: Position, slot: Slot)}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="win win--{pos} win--{slot.className} {store.focusedSlot === pos ? 'win--focused' : ''} {store.flashSlot === pos ? 'win--flash' : ''}"
		onclick={() => store.focusSlot(pos)}
		bind:this={slotBodyEls[pos]}
	>
		{@render renderTree(slot.tree, slot, pos, true)}
	</div>
{/snippet}

{#snippet renderTree(node: SplitNode, slot: Slot, pos: Position, isRoot: boolean)}
	{#if node.type === 'leaf'}
		{@const classCount = store.openBuffers.filter((b) => b.className === slot.className).length}
		<div class="pane {isRoot ? 'pane--root' : ''}">
			<div class="pane__head">
				<span class="cls cls--{slot.className}">{slot.className}</span>
				<span class="pane__name">{node.buffer.label}</span>
				{#if node.buffer.kicker}
					<span class="pane__kicker">· {node.buffer.kicker}</span>
				{/if}
				{#if node.buffer.modified}
					<span class="pane__mod" title="Modified">●</span>
				{/if}
				<div class="pane__sp"></div>
				{#if isRoot && classCount > 1}
					<button
						class="pane__cycle"
						onclick={(e) => {
							e.stopPropagation();
							store.focusSlot(pos);
							openMinibuffer('class');
						}}
						title="Switch buffer in this slot ({classCount} {slot.className} buffers)"
					>{classCount}↻</button>
				{/if}
				{#if isRoot}
					<button
						class="pane__x"
						onclick={(e) => {
							e.stopPropagation();
							store.toggleSlot(pos);
						}}
						title="Collapse to rail"
					>×</button>
				{/if}
			</div>
			<div class="pane__body pane__body--{node.buffer.kind}">
				{@render bufferContent(node.buffer)}
			</div>
		</div>
	{:else}
		<div class="split split--{node.orient}">
			{#each node.children as child, i (i)}
				{@render renderTree(child, slot, pos, false)}
			{/each}
		</div>
	{/if}
{/snippet}

{#snippet bufferContent(b: Buffer)}
	{#if rendererFor(b.kind)}
		<BufferRenderer buffer={b} />
	{:else if b.kind === 'reader'}
		<div class="reader">
			<div class="reader__h2">{b.kicker ?? 'Section'}</div>
			<div class="reader__line reader__line--lg"></div>
			<div class="reader__line"></div>
			<div class="reader__line"></div>
			<div class="reader__line reader__line--short"></div>
			<div class="reader__line"></div>
			<div class="reader__quote">
				<div class="reader__line"></div>
				<div class="reader__line reader__line--short"></div>
			</div>
			<div class="reader__line"></div>
			<div class="reader__line reader__line--short"></div>
		</div>
	{:else if b.kind === 'composer'}
		<div class="composer">
			<div class="composer__block composer__block--editable">
				<div class="composer__block-h">§ block · editable</div>
				<div class="composer__line composer__line--lg"></div>
				<div class="composer__line"></div>
				<div class="composer__line composer__line--short"></div>
			</div>
			<div class="composer__block composer__block--imported">
				<div class="composer__block-h">§ block · imported · ring sigs §3</div>
				<div class="composer__line"></div>
				<div class="composer__line composer__line--short"></div>
			</div>
			<div class="composer__block composer__block--editable">
				<div class="composer__block-h">§ block · editable</div>
				<div class="composer__line"></div>
				<div class="composer__cursor">▎</div>
			</div>
			{#each Array(composerExtraBlocks) as _, i (i)}
				<div class="composer__block composer__block--editable composer__block--new">
					<div class="composer__block-h">§ block · new (o)</div>
					<div class="composer__line composer__line--short"></div>
					<div class="composer__cursor">▎</div>
				</div>
			{/each}
		</div>
	{:else if b.kind === 'feed'}
		<div class="feed">
			{#each [0, 1, 2, 3, 4] as i (i)}
				<div class="feed__row">
					<div class="feed__bar feed__bar--{i % 3}"></div>
					<div class="feed__col">
						<div class="feed__title"></div>
						<div class="feed__meta"></div>
					</div>
				</div>
			{/each}
		</div>
	{:else if b.kind === 'knowledgebase'}
		<div class="feed">
			{#each [0, 1, 2, 3] as i (i)}
				<div class="feed__row feed__row--kb">
					<div class="kb__icon">📄</div>
					<div class="feed__col">
						<div class="feed__title"></div>
						<div class="feed__meta"></div>
					</div>
				</div>
			{/each}
		</div>
	{:else if b.kind === 'search'}
		<div class="search">
			<input
				class="search__input"
				type="text"
				value={b.kicker?.replace('~', '~ ') ?? '~ '}
				data-entry
				placeholder="search query…"
			/>
			{#each [0, 1, 2] as i (i)}
				<div class="search__row">
					<div class="search__title"></div>
					<div class="search__meta"></div>
				</div>
			{/each}
		</div>
	{:else if b.kind === 'refs'}
		<div class="refs">
			{#each [0, 1, 2, 3] as i (i)}
				<div class="refs__row">
					<span class="refs__num">[{i + 1}]</span>
					<span class="refs__line"></span>
				</div>
			{/each}
		</div>
	{:else if b.kind === 'profile'}
		<div class="profile-mock">
			<div class="profile__avatar"></div>
			<div class="profile__name">{b.kicker ?? 'profile'}</div>
			<div class="profile__line"></div>
			<div class="profile__line profile__line--short"></div>
		</div>
	{:else if b.kind === 'chat'}
		<div class="chat">
			<div class="chat__msg chat__msg--user">
				<div class="chat__line"></div>
				<div class="chat__line chat__line--short"></div>
			</div>
			<div class="chat__msg chat__msg--bot">
				<div class="chat__line"></div>
				<div class="chat__line"></div>
				<div class="chat__line chat__line--short"></div>
			</div>
			<input class="chat__input" type="text" data-entry placeholder="type a message…" />
		</div>
	{/if}
{/snippet}

{#snippet railSlot(pos: Position, slot: Slot)}
	<button
		class="rail rail--{pos} rail--{slot.className} {store.focusedSlot === pos ? 'rail--focused' : ''} {store.flashSlot === pos ? 'rail--flash' : ''}"
		onclick={(e) => {
			e.stopPropagation();
			store.focusSlot(pos);
			store.toggleSlot(pos);
		}}
		title="Click to expand · Enter expands when focused"
	>
		<span class="cls cls--{slot.className} cls--vert">{slot.className}</span>
		<span class="rail__name">{slot.tree.type === 'leaf' ? slot.tree.buffer.label : 'split'}</span>
	</button>
{/snippet}

{#snippet leaderPopup(node: SubPrefix)}
	<div class="lp">
		<div class="lp__head">
			<span class="lp__path">{leaderPathLabel}-</span>
			<span class="lp__path-desc">{node.desc}</span>
			<span class="lp__sp"></span>
			<span class="lp__hint">key descends · backspace up · esc / C-g cancel</span>
		</div>
		<div class="lp__grid">
			{#each Object.entries(node.children) as [key, child] (key)}
				<button
					class="lp__row {child.type === 'leaf' && child.deferred ? 'lp__row--deferred' : ''}"
					onclick={() => leaderDescend(key)}
					disabled={child.type === 'leaf' && child.deferred}
				>
					<span class="lp__keychip">{key}</span>
					<span class="lp__arrow">{child.type === 'prefix' ? '+' : '→'}</span>
					<span class="lp__desc">{child.desc}</span>
					<span class="lp__sp"></span>
					{#if child.type === 'leaf'}
						<span class="lp__kind lp__kind--{child.kind}">{child.kind}</span>
					{:else}
						<span class="lp__kind lp__kind--prefix">prefix</span>
					{/if}
				</button>
			{/each}
		</div>
	</div>
{/snippet}

{#snippet minibufferStrip()}
	<div class="mb">
		<div class="mb__list">
			{#if mb.mode === 'mx'}
				{#each mxEntries as cmd, i (cmd.id)}
					<button
						class="mb__row mb__row--mx {i === mb.selectedIndex ? 'mb__row--sel' : ''}"
						onmouseenter={() => (mb.selectedIndex = i)}
						onclick={() => executeCommand(cmd)}
					>
						<span class="cat cat--{cmd.category.toLowerCase()}">{cmd.category}</span>
						<span class="mb__name">{cmd.name}</span>
						<span class="mb__kicker">{cmd.description}</span>
						<span class="mb__sp"></span>
						{#if cmd.keybinding}
							<span class="mb__kb">{cmd.keybinding}</span>
						{/if}
					</button>
				{/each}
				{#if mxEntries.length === 0}
					<div class="mb__empty">no matching commands</div>
				{/if}
			{:else}
				{#each minibufferEntries as entry, i (entry.buffer.id + i)}
					<button
						class="mb__row {i === mb.selectedIndex ? 'mb__row--sel' : ''}"
						onmouseenter={() => (mb.selectedIndex = i)}
						onclick={() => selectBuffer(entry)}
					>
						<span class="cls cls--{entry.className}">{entry.className}</span>
						<span class="mb__name">{entry.buffer.label}</span>
						<span class="mb__kicker">{entry.buffer.kicker ?? ''}</span>
					</button>
				{/each}
				{#if minibufferEntries.length === 0}
					<div class="mb__empty">no matching buffers</div>
				{/if}
			{/if}
		</div>
		<div class="mb__input-row">
			<span class="mb__title">{minibufferTitle}</span>
			<span class="mb__prompt">{mb.mode === 'global' ? 'B>' : mb.mode === 'recent' ? 'r>' : mb.mode === 'mx' ? 'M-x' : mb.mode === 'split' ? 's>' : 'b>'}</span>
			<!-- svelte-ignore a11y_autofocus -->
			<input
				class="mb__input"
				bind:value={mb.query}
				oninput={() => (mb.selectedIndex = 0)}
				autofocus
				placeholder={mb.mode === 'mx' ? 'command…' : 'filter…'}
			/>
			<span class="mb__hint">↑↓ select · enter {mb.mode === 'mx' ? 'execute' : mb.mode === 'split' ? 'split' : 'switch'} · esc close</span>
		</div>
	</div>
{/snippet}

<style>
	.page {
		min-height: 100dvh;
		background: var(--bg-alt);
		color: var(--fg);
		font-family: var(--font-sans);
		padding: var(--s-6) var(--s-6) var(--s-10);
		max-width: 1500px;
		margin: 0 auto;
	}
	.page__head { margin-bottom: var(--s-6); }
	.eyebrow {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		margin-bottom: var(--s-2);
	}
	.title {
		font-size: var(--t-2xl);
		font-weight: 600;
		margin: 0 0 var(--s-3);
		line-height: var(--lh-tight);
	}
	.lede {
		font-size: var(--t-md);
		color: var(--base7);
		max-width: 80ch;
		margin: 0 0 var(--s-3);
		line-height: var(--lh-snug);
	}
	.lede code {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		background: var(--base1);
		padding: 1px 5px;
		border-radius: var(--r-sm);
		color: var(--cyan);
	}
	.hint {
		font-size: var(--t-sm);
		color: var(--base6);
		font-family: var(--font-mono);
	}
	.kbd {
		display: inline-block;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 6px;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		background: var(--base1);
		color: var(--fg-alt);
	}

	.shell {
		background: var(--bg);
		border: 1px solid var(--base3);
		border-radius: var(--r-md);
		overflow: hidden;
		display: flex;
		flex-direction: column;
		min-height: 660px;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
		position: relative;
	}

	.shell__header {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		padding: 0 var(--s-3);
		height: 36px;
		background: var(--panel-header-bg);
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
	}
	.shell__brand {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--base7);
		font-weight: 600;
	}
	.shell__layouts { display: flex; gap: 2px; }
	.lt {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 10px;
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
		transition: color 100ms, background 100ms, border-color 100ms;
	}
	.lt:hover { color: var(--fg); background: var(--base1); }
	.lt--on {
		background: var(--base2);
		color: var(--fg);
		border-color: var(--base3);
	}
	.shell__layout-desc {
		font-size: var(--t-sm);
		color: var(--base5);
		font-style: italic;
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.shell__prefix { display: flex; gap: 2px; }
	.px {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 3px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base5);
		cursor: pointer;
	}
	.px:hover { color: var(--fg); border-color: var(--base4); }
	.px--on { background: var(--id-yours); color: var(--bg); border-color: var(--id-yours); }
	.shell__mx {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 12px;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
		background: var(--base0);
		transition: color 100ms, border-color 100ms, background 100ms;
	}
	.shell__mx:hover { color: var(--fg); border-color: var(--base4); }
	.shell__mx--on {
		background: var(--id-yours);
		color: var(--bg);
		border-color: var(--id-yours);
	}

	.shell__body {
		display: flex;
		flex: 1;
		min-height: 0;
	}

	.win {
		display: flex;
		flex-direction: column;
		border-right: 1px solid var(--panel-border);
		min-width: 0;
		cursor: pointer;
		position: relative;
	}
	.win:last-child { border-right: none; }
	.win--left { width: 360px; flex-shrink: 0; }
	.win--right { width: 360px; flex-shrink: 0; }
	.win--center { flex: 1; }
	.win--focused::before {
		content: '';
		position: absolute;
		inset: 0;
		border: 1px solid var(--id-yours);
		pointer-events: none;
		opacity: 0.5;
	}
	.win--flash { animation: flash-glow 700ms ease-out; }
	@keyframes flash-glow {
		0% { background: color-mix(in srgb, var(--id-yours) 25%, transparent); }
		20% { background: color-mix(in srgb, var(--id-yours) 18%, transparent); }
		100% { background: transparent; }
	}

	.split { display: flex; flex: 1; min-height: 0; }
	.split--h { flex-direction: column; }
	.split--h > .pane { border-bottom: 1px solid var(--panel-border); }
	.split--h > .pane:last-child { border-bottom: none; }
	.split--v > .pane { border-right: 1px solid var(--panel-border); }
	.split--v > .pane:last-child { border-right: none; }

	.pane {
		display: flex;
		flex-direction: column;
		flex: 1;
		min-height: 0;
		min-width: 0;
	}
	.pane__head {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: 4px var(--s-3);
		background: var(--panel-bg-soft);
		border-bottom: 1px solid var(--panel-border);
		font-size: var(--t-xs);
		flex-shrink: 0;
	}
	.pane__name {
		font-family: var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--fg);
		font-weight: 600;
	}
	.pane__kicker { color: var(--base5); font-family: var(--font-sans); text-transform: none; letter-spacing: 0; }
	.pane__mod { color: var(--id-draft); font-size: 8px; }
	.pane__sp { flex: 1; }
	.pane__x {
		background: transparent;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: 14px;
		line-height: 1;
		padding: 0 4px;
	}
	.pane__x:hover { color: var(--fg); }
	.pane__cycle {
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: 10px;
		padding: 1px 6px;
		margin-right: 4px;
	}
	.pane__cycle:hover { color: var(--fg); border-color: var(--id-yours); }
	.pane__body {
		flex: 1;
		padding: var(--s-3);
		overflow: hidden;
	}

	.cls {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		font-family: var(--font-mono);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		font-weight: 600;
	}
	.cls--chat { background: color-mix(in srgb, var(--id-imported) 22%, transparent); color: var(--id-imported); }
	.cls--work { background: color-mix(in srgb, var(--id-yours) 22%, transparent); color: var(--id-yours); }
	.cls--research { background: color-mix(in srgb, var(--id-remote) 22%, transparent); color: var(--id-remote); }
	.cls--vert {
		writing-mode: vertical-rl;
		transform: rotate(180deg);
		padding: 6px 1px;
		letter-spacing: 0.08em;
	}

	.rail {
		width: 32px;
		flex-shrink: 0;
		background: var(--panel-rail-bg);
		border-right: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: var(--s-3);
		padding: var(--s-3) 0;
		cursor: pointer;
		border: none;
		border-right: 1px solid var(--panel-border);
		transition: background 100ms;
	}
	.rail:last-child { border-right: none; }
	.rail:hover { background: var(--base1); }
	.rail__name {
		writing-mode: vertical-rl;
		transform: rotate(180deg);
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base6);
		letter-spacing: 0.05em;
	}
	.rail--flash { animation: flash-glow 700ms ease-out; }
	.rail--focused {
		background: color-mix(in srgb, var(--id-yours) 12%, var(--panel-rail-bg));
		border-right-color: var(--id-yours);
		box-shadow: inset 1px 0 0 var(--id-yours), inset -1px 0 0 var(--id-yours);
	}

	.shell__modeline {
		height: 22px;
		background: var(--panel-bg-soft);
		border-top: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
		display: flex;
		align-items: center;
		padding: 0 var(--s-3);
		flex-shrink: 0;
		gap: var(--s-3);
	}
	.ml__spacer { flex: 1; }
	.ml__pos { color: var(--base5); }
	.ml__mode {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
		padding: 0 var(--s-2);
		border-radius: var(--r-sm);
	}
	.ml__mode--normal {
		background: color-mix(in srgb, var(--state-online) 25%, transparent);
		color: var(--state-online);
	}
	.ml__mode--insert {
		background: color-mix(in srgb, var(--id-yours) 25%, transparent);
		color: var(--id-yours);
	}

	/* Leader popup (which-key) */
	.lp {
		border-top: 1px solid var(--base3);
		background: var(--panel-bg);
		display: flex;
		flex-direction: column;
		flex-shrink: 0;
		max-height: 280px;
	}
	.lp__head {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		padding: var(--s-2) var(--s-3);
		background: var(--panel-bg-soft);
		border-bottom: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.lp__path {
		color: var(--id-yours);
		font-weight: 600;
		letter-spacing: 0.05em;
	}
	.lp__path-desc {
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.lp__hint { color: var(--base5); font-size: 10px; }
	.lp__sp { flex: 1; }
	.lp__grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
		gap: 2px;
		padding: var(--s-2);
		overflow-y: auto;
	}
	.lp__row {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: 4px var(--s-2);
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		cursor: pointer;
		font-family: var(--font-sans);
		font-size: var(--t-sm);
		color: var(--fg);
		text-align: left;
	}
	.lp__row:hover:not(:disabled) {
		background: var(--base1);
		border-color: var(--base3);
	}
	.lp__row--deferred {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.lp__keychip {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
		min-width: 22px;
		padding: 1px 6px;
		text-align: center;
		background: var(--base2);
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--id-yours);
		flex-shrink: 0;
	}
	.lp__arrow {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		width: 10px;
		text-align: center;
		flex-shrink: 0;
	}
	.lp__desc { color: var(--fg-alt); font-size: var(--t-sm); }
	.lp__kind {
		font-family: var(--font-mono);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		font-weight: 600;
		flex-shrink: 0;
	}
	.lp__kind--engine {
		background: color-mix(in srgb, var(--state-fetching) 18%, transparent);
		color: var(--state-fetching);
	}
	.lp__kind--client {
		background: color-mix(in srgb, var(--base6) 18%, transparent);
		color: var(--base7);
	}
	.lp__kind--prefix {
		background: color-mix(in srgb, var(--id-imported) 18%, transparent);
		color: var(--id-imported);
	}

	/* Minibuffer strip */
	.mb {
		border-top: 1px solid var(--base3);
		background: var(--panel-bg);
		display: flex;
		flex-direction: column;
		max-height: 280px;
		flex-shrink: 0;
	}
	.mb__list {
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		padding: var(--s-1) 0;
		max-height: 240px;
	}
	.mb__row {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: 4px var(--s-3);
		background: transparent;
		border: none;
		border-left: 2px solid transparent;
		font-size: var(--t-sm);
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-family: var(--font-sans);
	}
	.mb__row--sel {
		background: var(--base1);
		border-left-color: var(--id-yours);
	}
	.mb__name {
		font-family: var(--font-mono);
		text-transform: uppercase;
		font-size: var(--t-xs);
		letter-spacing: 0.05em;
		color: var(--fg);
	}
	.mb__kicker { color: var(--base6); font-size: var(--t-sm); flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.mb__sp { flex: 1; }
	.mb__kb {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		padding: 1px 6px;
		border: 1px solid var(--base2);
		border-radius: var(--r-sm);
		background: var(--base0);
		flex-shrink: 0;
	}
	.mb__row--mx {
		font-family: var(--font-sans);
	}

	.cat {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		font-family: var(--font-mono);
		font-size: 9px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		font-weight: 600;
		min-width: 70px;
		flex-shrink: 0;
	}
	.cat--buffer { background: color-mix(in srgb, var(--id-yours) 18%, transparent); color: var(--id-yours); }
	.cat--window { background: color-mix(in srgb, var(--id-remote) 18%, transparent); color: var(--id-remote); }
	.cat--layout { background: color-mix(in srgb, var(--id-imported) 18%, transparent); color: var(--id-imported); }
	.cat--compose { background: color-mix(in srgb, var(--id-forked) 18%, transparent); color: var(--id-forked); }
	.cat--configuration { background: color-mix(in srgb, var(--state-fetching) 18%, transparent); color: var(--state-fetching); }
	.cat--versioning { background: color-mix(in srgb, var(--state-online) 18%, transparent); color: var(--state-online); }
	.cat--application { background: color-mix(in srgb, var(--state-error) 18%, transparent); color: var(--state-error); }
	.cat--view { background: color-mix(in srgb, var(--cyan) 18%, transparent); color: var(--cyan); }
	.cat--navigation { background: color-mix(in srgb, var(--base6) 18%, transparent); color: var(--base7); }
	.mb__empty {
		padding: var(--s-3);
		color: var(--base5);
		font-size: var(--t-sm);
		font-style: italic;
	}

	.mb__input-row {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: var(--s-2) var(--s-3);
		background: var(--panel-bg-soft);
		border-top: 1px solid var(--panel-border);
	}
	.mb__title {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		margin-right: var(--s-2);
	}
	.mb__prompt {
		font-family: var(--font-mono);
		color: var(--id-yours);
		font-size: var(--t-sm);
	}
	.mb__input {
		flex: 1;
		background: var(--base0);
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--fg);
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		padding: 4px 8px;
		outline: none;
	}
	.mb__input:focus { border-color: var(--id-yours); }
	.mb__hint {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
	}

	/* Buffer-content ghosts */
	.reader {
		font-family: var(--font-serif);
		max-width: 65ch;
		margin: 0 auto;
		padding: var(--s-4) 0;
	}
	.reader__h2 {
		font-size: var(--t-lg);
		font-weight: 600;
		color: var(--fg);
		margin-bottom: var(--s-4);
		font-family: var(--font-serif);
	}
	.reader__line {
		height: 9px;
		background: var(--base2);
		border-radius: 2px;
		margin-bottom: var(--s-2);
	}
	.reader__line--lg { width: 95%; height: 11px; background: var(--base3); }
	.reader__line--short { width: 60%; }
	.reader__quote {
		border-left: 2px solid var(--base4);
		padding: var(--s-2) 0 var(--s-2) var(--s-3);
		margin: var(--s-3) 0;
	}
	.reader__quote .reader__line { background: var(--base2); }

	.composer {
		display: flex;
		flex-direction: column;
		gap: var(--s-3);
	}
	.composer__block {
		padding: var(--s-3);
		border-radius: var(--r-sm);
		border-left: 3px solid var(--base4);
	}
	.composer__block--editable { border-left-color: var(--id-yours); background: color-mix(in srgb, var(--id-yours) 4%, transparent); }
	.composer__block--imported { border-left-color: var(--id-imported); background: color-mix(in srgb, var(--id-imported) 4%, transparent); }
	.composer__block-h {
		font-family: var(--font-mono);
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		margin-bottom: var(--s-2);
	}
	.composer__line {
		height: 8px;
		background: var(--base2);
		border-radius: 2px;
		margin-bottom: var(--s-1);
		width: 100%;
	}
	.composer__line--lg { width: 100%; }
	.composer__line--short { width: 55%; }
	.composer__cursor {
		display: inline-block;
		color: var(--id-yours);
		animation: blink 1.1s step-end infinite;
	}
	@keyframes blink { 0%, 50% { opacity: 1; } 50.01%, 100% { opacity: 0; } }

	.feed { display: flex; flex-direction: column; gap: var(--s-2); }
	.feed__row {
		display: flex;
		gap: var(--s-2);
		padding: var(--s-2);
		border-radius: var(--r-sm);
		background: var(--panel-bg-soft);
	}
	.feed__row--kb { background: color-mix(in srgb, var(--id-imported) 5%, var(--panel-bg-soft)); }
	.feed__bar { width: 3px; align-self: stretch; border-radius: 2px; }
	.feed__bar--0 { background: var(--id-yours); }
	.feed__bar--1 { background: var(--id-remote); }
	.feed__bar--2 { background: var(--id-imported); }
	.kb__icon { font-size: 14px; align-self: center; opacity: 0.7; }
	.feed__col { flex: 1; display: flex; flex-direction: column; gap: 4px; }
	.feed__title { height: 9px; background: var(--base3); border-radius: 2px; width: 70%; }
	.feed__meta { height: 7px; background: var(--base2); border-radius: 2px; width: 40%; }

	.search { display: flex; flex-direction: column; gap: var(--s-2); }
	.search__input {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: var(--s-2);
		background: var(--base0);
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--cyan);
	}
	.search__row {
		padding: var(--s-2);
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.search__title { height: 8px; background: var(--base3); border-radius: 2px; width: 75%; }
	.search__meta { height: 6px; background: var(--base2); border-radius: 2px; width: 45%; }

	.refs { display: flex; flex-direction: column; gap: var(--s-2); padding-top: var(--s-2); }
	.refs__row { display: flex; align-items: center; gap: var(--s-2); padding: 2px 0; }
	.refs__num {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--id-remote);
	}
	.refs__line { flex: 1; height: 7px; background: var(--base2); border-radius: 2px; }

	.profile-mock { display: flex; flex-direction: column; align-items: center; gap: var(--s-2); padding-top: var(--s-4); }
	.profile__avatar { width: 48px; height: 48px; border-radius: 50%; background: var(--base3); }
	.profile__name { font-family: var(--font-mono); font-size: var(--t-md); color: var(--fg); }
	.profile__line { height: 8px; background: var(--base2); border-radius: 2px; width: 70%; }
	.profile__line--short { width: 40%; }

	.chat { display: flex; flex-direction: column; gap: var(--s-3); height: 100%; }
	.chat__msg {
		padding: var(--s-2) var(--s-3);
		border-radius: var(--r-md);
		display: flex;
		flex-direction: column;
		gap: 3px;
		max-width: 90%;
	}
	.chat__msg--user { background: var(--base1); align-self: flex-end; }
	.chat__msg--bot { background: color-mix(in srgb, var(--id-imported) 8%, transparent); align-self: flex-start; }
	.chat__line { height: 7px; background: var(--base3); border-radius: 2px; width: 100%; }
	.chat__line--short { width: 50%; }
	.chat__input {
		margin-top: auto;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: var(--s-2);
		background: var(--base0);
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base5);
	}

	.legend {
		margin-top: var(--s-8);
		padding: var(--s-5);
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
	}
	.legend__title { font-size: var(--t-lg); margin: 0 0 var(--s-4); font-weight: 600; }
	.legend__grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
		gap: var(--s-4);
	}
	.legend__item p { margin: 0; font-size: var(--t-sm); color: var(--base6); line-height: var(--lh-snug); }
	.legend__h {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base7);
		margin-bottom: var(--s-1);
	}
	.legend__item code, .legend__item strong {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.legend__item code {
		background: var(--base1);
		padding: 1px 4px;
		border-radius: var(--r-sm);
		color: var(--cyan);
	}

	.notes {
		margin-top: var(--s-5);
		padding: var(--s-5);
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
	}
	.notes__title { font-size: var(--t-md); margin: 0 0 var(--s-3); color: var(--base7); font-weight: 600; }
	.notes ul { margin: 0; padding-left: var(--s-4); font-size: var(--t-sm); color: var(--base6); line-height: var(--lh-snug); }
	.notes li { margin-bottom: var(--s-1); }
	.notes code { font-family: var(--font-mono); background: var(--base1); padding: 1px 4px; border-radius: var(--r-sm); color: var(--cyan); font-size: var(--t-xs); }
</style>
