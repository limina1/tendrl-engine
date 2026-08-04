<script lang="ts">
	import type {
		Buffer,
		ClassName,
		Command,
		LayoutConfig,
		MinibufferMode,
		OpenBuf,
		Position,
		Slot,
		SlotState,
		SplitNode
	} from '$lib/wm/types';
	import {
		applyBindingOverrides,
		buildLeaderRoot,
		resolveLeaderNode,
		type LeaderNode,
		type SubPrefix
	} from '$lib/wm/leader';
	import { commands } from '$lib/wm/commands';
	import {
		effectiveKeybinding,
		isCommandHidden,
		leaderOverrides,
		singleKeyBindings
	} from '$lib/wm/command-prefs.svelte';
	import { BufferStore, setActiveStore, type NavAction } from '$lib/wm/buffer-store.svelte';
	import BufferRenderer from '$lib/wm/BufferRenderer.svelte';
	import { rendererFor, toursForClass } from '$lib/wm/registry';
	import { getAppState, type ModalNavEntry } from '$lib/state.svelte';
	import { themeById } from '$lib/themes';
	import {
		getAuthorDisplayName,
		getAuthorProfile,
		prefetchAuthors
	} from '$lib/discussions/authors.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { identityCanSign, detectNip07 } from '$lib/identity/signer';
	import {
		replayWalkthrough,
		runTour,
		trigger as triggerTip,
		armSurfaceTour,
		discovery,
		TIPS
	} from '$lib/wm/discovery.svelte';
	import { openModelineHelp } from '$lib/wm/modeline-help.svelte';

	const app = getAppState();

	// Drives the sun/moon toggle glyph: show the sun while dark (click → light)
	// and the moon while light (click → dark).
	const themeMode = $derived(themeById(app.currentTheme)?.mode ?? 'dark');

	// The `search-history` tip no longer auto-fires on the first search — it's
	// now the closing beat of the opt-in search tour (run from the search W).

	// Singleton buffers seeded on every frame.
	const chatBuf: Buffer = { id: 'chat', kind: 'chat', label: 'chat' };
	const feedBuf: Buffer = { id: 'feed', kind: 'feed', label: 'feed' };
	const composerBuf: Buffer = { id: 'composer:current', kind: 'composer', label: 'composer', kicker: 'untitled draft' };
	const searchBuf: Buffer = { id: 'search:default', kind: 'search', label: 'search' };
	const ignoredBuf: Buffer = { id: 'ignored', kind: 'ignored', label: 'ignored' };

	const openBuffers: OpenBuf[] = [
		{ className: 'chat', buffer: chatBuf },
		// Work class — main content surface. Cycles via SPC b b: feed → reader
		// → composer → ... Click a publication in the feed to spawn a reader
		// that joins this cycle (replaces feed in the active leaf).
		{ className: 'work', buffer: feedBuf },
		{ className: 'work', buffer: composerBuf },
		{ className: 'work', buffer: ignoredBuf },
		// Research class — single search buffer. It hosts Search · Refs · KB
		// as internal h/l-cycled tabs; the standalone refs and KB buffers
		// were retired once SearchPanel grew their equivalents.
		{ className: 'research', buffer: searchBuf }
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
		}
	};

	const store = new BufferStore(layouts, 'base');
	store.seed(openBuffers);
	setActiveStore(store);

	// Walkthrough affordances:
	//   • mode-line W — permanent single button, always the mode-line tour.
	//   • logo W      — a dropdown of all top-level tutorials (the first-run
	//                   walk + the work/center window's tours).
	//   • per-window W (Chat / Research panes) — a dropdown of that window's
	//                   tours.
	// Every dropdown row carries a ✓ once its tour has been run; the glyph
	// colours by aggregate state (bright = something unrun, faded = all run,
	// grey = nothing here yet).
	function walkState(tour: string | undefined): 'new' | 'done' | 'none' {
		return !tour ? 'none' : discovery.seen.includes(tour) ? 'done' : 'new';
	}
	const modelineWalkState = $derived(walkState('modeline-overview'));

	// `buffer` is the owning buffer's label, shown as an opaque tag in the logo
	// `W` (which aggregates every buffer's tours) so you can see where each tour
	// lives. Omitted by single-scope menus (Chat / Research pane heads).
	type GuideEntry = { key: string; label: string; buffer?: string; done: boolean; run: () => void };
	// Running a tour brings up the buffer it describes so it starts immediately
	// rather than waiting for the user to navigate there themselves.
	function openForTour(kind: string) {
		if (kind === 'composer') store.openBuffer({ className: 'work', buffer: composerBuf });
		else if (kind === 'settings')
			store.openBuffer({
				className: 'work',
				buffer: { id: 'settings', kind: 'settings', label: 'settings' }
			});
		else if (kind === 'search') store.openBuffer({ className: 'research', buffer: searchBuf });
		// reader/etc. need a loaded document — leave those to an already-open buffer.
	}
	function guidesForClass(cls: ClassName): GuideEntry[] {
		return toursForClass(cls).map(({ kind, label, key }) => ({
			key,
			label: TIPS[key]?.title ?? key,
			buffer: label,
			done: discovery.seen.includes(key),
			run: () => {
				openForTour(kind);
				runTour(key);
			}
		}));
	}
	// True once the user is set up — a signer connected and a populated pool.
	const onboarded = $derived(identityCanSign(app.identityStatus) && app.feed.length > 0);
	// The logo dropdown: the work/center window's tours, prefixed by the
	// first-run walk — but only while the user isn't already set up (a fully
	// onboarded user has nothing to gain from the intro, so it's omitted).
	// The event menu is a modal, not a work buffer, so it isn't in the registry's
	// class map — but its tour belongs in the global W. If a menu is already open
	// run it straight; otherwise arm it and point the user at a feed row's `menu`
	// pill (menu-open), and EventViewModal resumes the tour when it mounts.
	function menuGuide(): GuideEntry {
		return {
			key: 'menu-overview',
			label: TIPS['menu-overview']?.title ?? 'The event menu',
			buffer: 'menu',
			done: discovery.seen.includes('menu-overview'),
			run: () => {
				if (app.eventModalData) {
					runTour('menu-overview');
				} else {
					armSurfaceTour('menu-overview');
					runTour('menu-open');
				}
			}
		};
	}
	function logoGuides(): GuideEntry[] {
		const work = guidesForClass('work');
		const menu = menuGuide();
		if (onboarded) return [...work, menu];
		return [
			{
				key: '__first_run__',
				label: 'First-run walkthrough',
				done: discovery.seen.includes('walk-done'),
				run: () => replayWalkthrough()
			},
			...work,
			menu
		];
	}
	function menuAggState(entries: GuideEntry[]): 'new' | 'done' | 'none' {
		if (!entries.length) return 'none';
		return entries.some((e) => !e.done) ? 'new' : 'done';
	}
	// Which W dropdown is open (by id), or null. The open menu is positioned
	// fixed and clamped to the viewport from the trigger button's rect, so it
	// never spills off-screen regardless of which corner the button sits in.
	let walkMenuOpen = $state<string | null>(null);
	let walkBtnRect = $state<DOMRect | null>(null);
	let walkMenuEl = $state<HTMLElement | null>(null);
	let walkMenuW = $state(240);
	let walkMenuH = $state(0);
	let winW = $state(0);
	let winH = $state(0);

	$effect(() => {
		winW = window.innerWidth;
		winH = window.innerHeight;
		const onResize = () => {
			winW = window.innerWidth;
			winH = window.innerHeight;
		};
		window.addEventListener('resize', onResize);
		return () => window.removeEventListener('resize', onResize);
	});

	$effect(() => {
		if (!walkMenuEl) return;
		const ro = new ResizeObserver(() => {
			walkMenuW = walkMenuEl?.offsetWidth ?? walkMenuW;
			walkMenuH = walkMenuEl?.offsetHeight ?? walkMenuH;
		});
		ro.observe(walkMenuEl);
		return () => ro.disconnect();
	});

	const walkMenuStyle = $derived.by(() => {
		if (!walkBtnRect) return 'visibility:hidden;';
		const m = 8;
		const left = Math.max(m, Math.min(walkBtnRect.left, winW - walkMenuW - m));
		let top = walkBtnRect.bottom + 6;
		if (top + walkMenuH > winH - m) {
			const above = walkBtnRect.top - 6 - walkMenuH;
			top = above >= m ? above : Math.max(m, winH - walkMenuH - m);
		}
		return `left:${left}px;top:${top}px;`;
	});

	// Redirect AppState navigation calls to spawn buffers in the shell
	// instead of route-navigating away from the single-page app.
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
		onDiscussion: (event_id, kind) => {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `discussion:${event_id}`,
					kind: 'discussion-view',
					label: kind === 9802 ? 'highlight' : 'comment',
					kicker: event_id.slice(0, 8) + '…'
				}
			});
		},
		onReader: (buffer_id, label, kicker) => {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: buffer_id,
					kind: 'reader',
					label,
					kicker
				}
			});
		},
		onDoc: (buffer_id, kicker) => {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: buffer_id,
					kind: 'doc',
					label: 'doc',
					kicker
				}
			});
		},
		onSearch: (kicker) => {
			store.openBuffer({
				className: 'research',
				buffer: { id: 'search', kind: 'search', label: 'search', kicker: kicker ?? '' }
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

	// Pending `g` for the `gg` motion. Cleared after a short window or by
	// any non-`g` keystroke. `G` (shift) fires immediately as 'bottom'.
	let pendingG = $state(false);
	let pendingGTimer: ReturnType<typeof setTimeout> | null = null;
	function armPendingG() {
		pendingG = true;
		if (pendingGTimer) clearTimeout(pendingGTimer);
		pendingGTimer = setTimeout(() => { pendingG = false; pendingGTimer = null; }, 700);
	}
	function clearPendingG() {
		pendingG = false;
		if (pendingGTimer) { clearTimeout(pendingGTimer); pendingGTimer = null; }
	}

	let mb = $state<{ mode: MinibufferMode; query: string; selectedIndex: number }>({
		mode: 'closed',
		query: '',
		selectedIndex: 0
	});

	// Search-history popover toggle, anchored to the .hs-pill-wrap in the
	// modeline. Click-outside closes it via the $effect below.
	let historyPopoverOpen = $state(false);
	let hsWrapEl: HTMLElement | null = $state(null);

	$effect(() => {
		if (!historyPopoverOpen) return;
		function onDocMouseDown(e: MouseEvent) {
			if (!hsWrapEl) return;
			if (e.target instanceof Node && hsWrapEl.contains(e.target)) return;
			historyPopoverOpen = false;
		}
		document.addEventListener('mousedown', onDocMouseDown);
		return () => document.removeEventListener('mousedown', onDocMouseDown);
	});

	// Local mirror of state.svelte.ts entryKey — used to test "is this row
	// the previousEntry" for the depth-1 highlight.
	function entryKey(e: ModalNavEntry): string {
		if (e.kind === 'query') {
			const norm = e.query.trim().replace(/\s+/g, ' ');
			return `q|${norm}|s=${e.opts.scopeToMe}`;
		}
		if (e.kind === 'nevent') return `e|${e.eventId.toLowerCase()}`;
		return `a|${e.coord.kind}:${e.coord.pubkey}:${e.coord.d_tag}`;
	}

	function entryLabel(e: ModalNavEntry): string {
		if (e.kind === 'query') return e.query;
		if (e.kind === 'nevent') return e.title ?? e.eventId.slice(0, 12) + '…';
		return e.title ?? e.coord.d_tag;
	}

	function entryMeta(e: ModalNavEntry): string {
		if (e.kind === 'query') return e.opts.scopeToMe ? 'scoped' : '';
		if (e.kind === 'nevent') return e.eventId.slice(0, 8);
		return `k:${e.coord.kind}`;
	}

	async function replayEntry(entry: ModalNavEntry) {
		historyPopoverOpen = false;
		if (entry.kind === 'query') {
			await app.handleSearch(entry.query, entry.opts);
			return;
		}
		if (entry.kind === 'nevent') {
			await app.getEventForModal(entry.eventId);
			return;
		}
		// naddr — replay as a structured query. by: accepts hex per src/search.rs.
		const q = `k:${entry.coord.kind} by:${entry.coord.pubkey} #d:${entry.coord.d_tag}`;
		await app.handleSearch(q, { scopeToMe: false });
	}

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

	let mbInputEl: HTMLInputElement | null = null;

	function openMinibuffer(mode: MinibufferMode) {
		mb = { mode, query: '', selectedIndex: 0 };
		prefixPath = [];
		// Deterministic focus into the query field — the input's `autofocus`
		// only fires reliably on some mount paths (button click), not a
		// leader-key open (SPC :), which left focus on body so typing never
		// reached the filter. setTimeout ordering matches enterInsertMode.
		setTimeout(() => mbInputEl?.focus(), 0);
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
		// Per-user visibility: hidden commands stay runnable via their
		// keybinding, they just don't clutter the palette.
		const visible = commands.filter((c) => !isCommandHidden(c));
		const q = mb.query.trim().toLowerCase();
		if (!q) return visible;
		return visible.filter(
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
		if (cmd.id === 'tendrl-kill-buffer') {
			store.killFocused();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-split-window') {
			openMinibuffer('split');
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
		if (cmd.id === 'tendrl-highlight') {
			// Prefill from any live selection — the "I was just reading this
			// elsewhere" path; otherwise open blank for a pure paste.
			const sel = window.getSelection()?.toString().trim() ?? '';
			app.highlightComposer = sel ? { text: sel } : {};
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-highlight-mode') {
			app.toggleHighlightMode();
			app.pushToast(
				app.highlightMode
					? 'Highlight mode ON — select text in a reader/doc to publish a highlight'
					: 'Highlight mode off',
				'info'
			);
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-open-settings') {
			store.openBuffer({
				className: 'work',
				buffer: { id: 'settings', kind: 'settings', label: 'settings', kicker: 'settings' }
			});
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-show-relays') {
			// Keyboard path to the relay-config buffer; previously this
			// command was declared but had no handler (the buffer was only
			// reachable by right-clicking the network-mode pill).
			openRelays();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-demo-publish-progress') {
			import('$lib/wm/publish-progress.svelte').then(({ setProgress, mockProgress }) => {
				setProgress(mockProgress());
				store.openBuffer({
					className: 'work',
					buffer: {
						id: 'publish-progress:current',
						kind: 'publish-progress',
						label: 'publish',
						kicker: 'demo'
					}
				});
			});
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
		if (cmd.id === 'tendrl-embed-missing') {
			app.handleSyncEmbeddings();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-reembed-all') {
			app.handleReindexEmbeddings();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-login') {
			// Open the Settings buffer where the Identity section's login
			// form lives. Earlier this called handleIdentityLock — wrong;
			// "login" should drive the user to the place that takes a key.
			openSettings();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-switch-source') {
			openSettings();
			closeMinibuffer();
			return;
		}
		if (cmd.id === 'tendrl-edit-profile') {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: 'profile-edit:current',
					kind: 'profile-edit',
					label: 'profile',
					kicker: 'kind:0'
				}
			});
			closeMinibuffer();
			return;
		}
		// Deferred commands land here: acknowledge instead of silently closing,
		// so an inapplicable pick is distinguishable from a successful no-op.
		if (cmd.deferred) app.pushToast(`${cmd.name} isn't wired up yet`, 'info');
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

	// Treat any focused editable as implicit insert intent — SPC / h j k l
	// must reach the textarea, not trigger global nav. Mirrors evil-mode's
	// auto-state-switch: the focused element decides the state.
	function isEditable(el: EventTarget | null): boolean {
		if (!(el instanceof HTMLElement)) return false;
		const tag = el.tagName;
		if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
		if (el.isContentEditable) return true;
		return false;
	}

	function onFocusIn(e: FocusEvent) {
		if (isEditable(e.target) && mode !== 'insert') mode = 'insert';
	}

	function onFocusOut(e: FocusEvent) {
		// Only flip back when focus actually leaves all editables.
		if (!isEditable(e.target)) return;
		const next = e.relatedTarget;
		if (!isEditable(next) && mode === 'insert') mode = 'normal';
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
		// Editable focus = implicit insert. The keystroke must reach the
		// field — don't preventDefault, don't trigger global nav. Esc and
		// C-[ / C-g still escape (which blurs the field via focusout →
		// mode = 'normal'). This is the safety net; focusin/out keeps
		// `mode` aligned for the modeline.
		// Also gate on e.target: a field can blur synchronously inside its
		// own keydown handler (SearchInput does this on Enter), so by the
		// time the event bubbles here, document.activeElement is no longer
		// editable — but the keystroke was still for that field, not us.
		if ((isEditable(document.activeElement) || isEditable(e.target)) && mb.mode === 'closed') {
			if (e.key === 'Escape' || (e.key === '[' && e.ctrlKey) || (e.key === 'g' && e.ctrlKey)) {
				e.preventDefault();
				exitInsertMode();
			}
			return;
		}

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

		// User single-key bindings (validated at bind time to never shadow
		// the reserved normal-mode keys below).
		const customCmd = singleKeyBindings()[e.key];
		if (customCmd && !e.ctrlKey) {
			clearPendingG();
			e.preventDefault();
			executeCommand(customCmd);
			return;
		}

		// Vim/ranger-style in-buffer nav. h/j/k/l + arrows dispatch to the
		// focused buffer's registered handler. preventDefault fires
		// unconditionally in normal mode so arrow keys never trigger
		// browser-default scroll — selection-by-row is the only way the
		// scrollbar should move via keyboard. Mouse wheel still scrolls.
		const navMap: Record<string, NavAction> = {
			h: 'left', l: 'right', j: 'down', k: 'up',
			ArrowLeft: 'left', ArrowRight: 'right', ArrowDown: 'down', ArrowUp: 'up',
			// `m` opens the universal event menu on the cursored item. Each
			// cursor-aware buffer translates this to the right modal call.
			m: 'menu'
		};
		const navAction = navMap[e.key];
		if (navAction) {
			clearPendingG();
			e.preventDefault();
			store.dispatchNav(navAction);
			return;
		}

		// G (shift+g) → bottom; gg (two-step) → top. CM6's vim plugin
		// handles G/gg internally for in-section motion; this is the
		// app-level fallback for ranger-style buffers (feed, reader
		// outline/paginated/continuous, search).
		if (e.key === 'G') {
			clearPendingG();
			e.preventDefault();
			store.dispatchNav('bottom');
			return;
		}
		if (e.key === 'g') {
			e.preventDefault();
			if (pendingG) {
				clearPendingG();
				store.dispatchNav('top');
			} else {
				armPendingG();
			}
			return;
		}

		if (e.key === 'Enter') {
			clearPendingG();
			e.preventDefault();
			if (!store.dispatchNav('select')) store.expandFocusedIfRail();
		} else if (e.key === 'i') {
			clearPendingG();
			e.preventDefault();
			store.expandFocusedIfRail();
			// Buffer-specific 'insert' (e.g., compose focuses the cursored
			// section's textarea) wins over the generic first-data-entry
			// fallback. focusin will flip mode → 'insert'.
			if (!store.dispatchNav('insert')) enterInsertMode();
		} else if (e.key === 'o') {
			clearPendingG();
			e.preventDefault();
			openAndInsert();
		} else if (e.key === ':') {
			clearPendingG();
			e.preventDefault();
			openMinibuffer('mx');
		} else {
			clearPendingG();
		}
	}

	function prefilterMx(name: string) {
		openMinibuffer('mx');
		mb.query = name;
		mb.selectedIndex = 0;
	}

	function toggleNetworkMode() {
		const next = app.networkStatus?.mode === 'auto' ? 'confirm' : 'auto';
		app.handleSetNetworkMode(next);
	}

	// Rebuilt whenever command prefs change: custom bindings replace the
	// default tagged leaves and graft their own chords, so the which-key
	// popup and dispatch always reflect the user's effective bindings.
	const leaderRoot: SubPrefix = $derived(applyBindingOverrides(buildLeaderRootDefault(), leaderOverrides(executeCommand)));

	function buildLeaderRootDefault(): SubPrefix {
		return buildLeaderRoot({
		openMinibuffer,
		prefilterMx,
		killFocusedBuffer: () => store.killFocused(),
		cycleBufferInSlot: (dir) => store.cycleBufferInSlot(dir),
		toggleFocusedSlot: () => store.toggleFocusedSlot(),
		navigateSlot: (dir) => store.navigateSlot(dir),
		setLayout,
		toggleNetworkMode,
		openSplitPicker: () => openMinibuffer('split'),
		openSettings: () =>
			store.openBuffer({
				className: 'work',
				buffer: { id: 'settings', kind: 'settings', label: 'settings', kicker: 'settings' }
			}),
		openProfileEdit: () =>
			store.openBuffer({
				className: 'work',
				buffer: {
					id: 'profile-edit:current',
					kind: 'profile-edit',
					label: 'profile',
					kicker: 'kind:0'
				}
			})
		});
	}

	function openLeader() {
		closeMinibuffer();
		prefixPath = ['SPC'];
	}

	// The mode-line doubles as the leader trigger: clicking empty space or a
	// passive text segment opens (or closes) the `menu` (SPC leader). The
	// interactive pills inside it — relays, fetch mode, identity, search
	// history, the W/? affordances — keep their own clicks, so ignore anything
	// that lands on a button/link/input or inside the history popover wrap.
	function onModelineClick(e: MouseEvent) {
		if ((e.target as HTMLElement).closest('button, a, input, .hs-pill-wrap')) return;
		if (leaderOpen) prefixPath = [];
		else openLeader();
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

	// Mode-line segments. Network and identity states drive pill/dot
	// chrome (see modeline render below); leader/minibuffer/buffer info
	// stays as text segments — they're transient and don't fit the
	// stable-status-pill metaphor.
	const focusedBufferText = $derived.by(() => {
		if (!focusedBuffer) return '';
		const star = focusedBuffer.modified ? ' *' : '';
		// Suppress a kicker that just repeats the label (e.g. "settings (settings)").
		const kicker =
			focusedBuffer.kicker && focusedBuffer.kicker !== focusedBuffer.label
				? ` (${focusedBuffer.kicker})`
				: '';
		return `${focusedBuffer.label}${star}${kicker}`;
	});

	const networkPill = $derived.by(() => {
		// Live status wins when present; otherwise the saved mode (read
		// synchronously from localStorage at module-load, or defaulted
		// to 'auto' for fresh users since that's the engine default).
		// Either way the pill renders the right colour on the first
		// frame — no "loading" placeholder, no flash.
		const mode = app.networkStatus?.mode ?? app.savedNetworkMode;
		const active = (app.networkStatus?.active_fetches ?? 0) > 0;
		if (mode === 'confirm') {
			// Confirm = orange (id-diverged): the engine is gating
			// fetches behind the user's explicit approval. The "warm"
			// tint reads as deliberate / attention-requiring without
			// the alarm of red.
			return {
				label: active ? 'fetching' : 'confirm',
				pillClass: 'pill--diverged',
				dotClass: active ? 'dot--fetching' : 'dot--diverged'
			};
		}
		// 'auto' — including the localStorage fallback for fresh
		// users where 'auto' is also the engine default.
		return {
			label: active ? 'fetching' : 'auto',
			pillClass: 'pill--online',
			dotClass: active ? 'dot--fetching' : 'dot--online'
		};
	});

	const identityPill = $derived.by(() => {
		const id = app.identityStatus;
		// In-flight connect (manual click or boot auto-reconnect) — show
		// a non-interactive "connecting…" chip so the logged-out chip
		// doesn't flash a second prompt mid-handshake.
		if (app.identityLoading || app.identityAutoReconnecting) {
			return { kind: 'connecting' as const, label: 'connecting…', pillClass: 'pill--draft' };
		}
		if (identityCanSign(id)) {
			const npub = id?.npub ?? '';
			return {
				kind: 'me' as const,
				label: npub ? `@${npub.slice(0, 12)}` : 'unlocked',
				pillClass: 'pill--local'
			};
		}
		// Engine key present but locked — clicking opens Settings to
		// unlock (needs the password field).
		if (id?.state === 'locked') {
			return { kind: 'locked' as const, label: 'locked', pillClass: 'pill--draft' };
		}
		// Logged out — the top-level login affordance. `pill--connect`
		// carries its own cursor/border, so it omits `pill--btn` (whose
		// scoped `border: none` would otherwise win on specificity).
		return { kind: 'connect' as const, label: 'connect', pillClass: 'pill--connect' };
	});

	// Top-left profile chip. Only shown when there's an active signing
	// session — engine key unlocked, or a NIP-07/46 signer connected.
	// `config.toml [identity] pubkey` (app.myPubkey) is just the
	// configured authoring identity and is set at boot regardless of
	// login, so gating on it alone made the chip look "logged in" on
	// engine source with no unlocked key. Gate on can-sign instead;
	// app.myPubkey still drives feed/authoring under the hood.
	const meLoggedIn = $derived(identityCanSign(app.identityStatus));
	const mePubkey = $derived(
		meLoggedIn ? (app.identityStatus?.pubkey ?? app.myPubkey ?? null) : null
	);
	const meProfile = $derived(mePubkey ? getAuthorProfile(mePubkey) : null);
	const meName = $derived(mePubkey ? getAuthorDisplayName(mePubkey) : '');
	const meColor = $derived(mePubkey ? pubkeyToColor(mePubkey) : 'transparent');

	$effect(() => {
		if (mePubkey) prefetchAuthors([mePubkey]);
	});

	// Walkthrough: the moment a sign-in lands (false→true), point at the me-chip
	// to explain the name-less pubkey. Guarded so it fires once per login, not on
	// every reactive re-run; resets on logout. trigger() itself no-ops when the
	// walkthrough is off or this tip is already seen.
	let signedInTipFired = false;
	$effect(() => {
		// Only when there's genuinely no name yet — the tip literally explains the
		// name-less pubkey, so showing it to someone whose profile is already
		// resolved (e.g. an established user replaying the walk) is wrong.
		if (mePubkey && !meName && !signedInTipFired) {
			signedInTipFired = true;
			triggerTip('signed-in-noname');
		} else if (!mePubkey) {
			signedInTipFired = false;
		}
	});

	function openMyProfile() {
		if (!mePubkey) return;
		app.navigateToProfile(mePubkey);
	}

	const embeddingPill = $derived.by(() => {
		const e = app.embeddingStatus;
		if (!e || !e.enabled) return null;
		const stale = (e.stale_count ?? 0) > 0;
		const missing = (e.missing_sections ?? 0) > 0;
		const fetching = stale || missing;
		return {
			label: `embed ${e.indexed_count}/${e.total_events}`,
			pillClass: 'pill--ghost',
			dotClass: fetching ? 'dot--fetching' : 'dot--online'
		};
	});

	function openSettings() {
		store.openBuffer({
			className: 'work',
			buffer: { id: 'settings', kind: 'settings', label: 'settings', kicker: 'settings' }
		});
	}

	function openRelays() {
		store.openBuffer({
			className: 'work',
			buffer: { id: 'relays', kind: 'relays', label: 'relays', kicker: 'config' }
		});
	}

	// Top-level login affordance. NIP-07 is the common case: connect
	// inline (getPublicKey + register the signer) without a Settings
	// detour. Engine/ncryptsec login needs a key + password field, so
	// when there's no extension we fall back to the Settings buffer
	// where that form lives. A successful nip07 connect persists the
	// source (state.svelte.ts) so the next reload auto-reconnects.
	async function connectIdentity() {
		if (detectNip07()) {
			await app.handleSelectNip07Source();
			if (app.identityError) {
				app.pushToast(`Connect failed: ${app.identityError}`, 'error', 5000);
				app.identityError = null;
				openSettings();
			}
		} else {
			openSettings();
		}
	}

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
		if (mb.mode === 'mx') return `Commands · ${mxEntries.length}`;
		return '';
	});
</script>

<svelte:head><title>{app.engineBranch ? `tendrl · ${app.engineBranch}` : 'tendrl'}</title></svelte:head>

<svelte:window onkeydown={onGlobalKeydown} onfocusin={onFocusIn} onfocusout={onFocusOut} />

<div class="page">
	<div class="shell">
		<div class="shell__header">
			<button
				class="shell__brand"
				data-tour="home"
				onclick={() => store.openBuffer({
					className: 'work',
					buffer: { id: 'feed', kind: 'feed', label: 'feed' }
				})}
				title="Home — the feed (publications stream)"
			>tendrl</button>
			{#if mePubkey}
				<button
					class="me-chip"
					data-tour="me-chip"
					onclick={openMyProfile}
					title="Open my profile ({mePubkey.slice(0, 12)}…)"
				>
					{#if meProfile?.picture}
						<img class="me-chip__avatar" src={meProfile.picture} alt="" />
					{:else}
						<span class="me-chip__dot" style="background: {meColor};"></span>
					{/if}
					<span class="me-chip__name">{meName}</span>
				</button>
			{/if}
			{@render walkMenu('logo', 'Walkthroughs', logoGuides(), null, 'lt-walk')}
			<button
				class="lt lt--settings"
				data-tour="settings"
				onclick={openSettings}
				title="Open settings buffer (SPC s s)"
			>settings</button>
			<div class="shell__layout-desc">{store.currentLayout.desc}</div>
			<button
				class="lt lt--theme"
				onclick={() => app.toggleTheme()}
				title={themeMode === 'light' ? 'Switch to dark theme' : 'Switch to light theme'}
				aria-label="Toggle light / dark theme"
			>
				{#if themeMode === 'light'}
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
						<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
					</svg>
				{:else}
					<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
						<circle cx="12" cy="12" r="4" />
						<path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
					</svg>
				{/if}
			</button>
			<button
				class="px {leaderOpen ? 'px--on' : ''}"
				onclick={() => (leaderOpen ? (prefixPath = []) : openLeader())}
				title="Menu — the SPC leader (which-key popup); also opens by clicking the mode-line"
			>menu</button>
			<button class="shell__mx {mb.mode === 'mx' ? 'shell__mx--on' : ''}" onclick={() => openMinibuffer('mx')} title="Commands · run an app command (SPC :)">commands</button>
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

		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="shell__modeline"
			data-tour="modeline"
			onclick={onModelineClick}
			title="Click to open the menu (SPC)"
		>
			{#if store.focusedSlotClass()}
				{@const focusedClass = store.focusedSlotClass()}
				<span class="ml__class ml__class--{focusedClass}" data-tour="ml-mode">{focusedClass}</span>
			{/if}
			{#if focusedBufferText}
				<span class="ml__seg ml__seg--buf">{focusedBufferText}</span>
			{/if}
			{#if leaderOpen}
				<span class="ml__seg ml__seg--prefix">{leaderPathLabel || 'SPC'}-</span>
			{/if}
			{#if mb.mode !== 'closed'}
				<span class="ml__seg ml__seg--prefix">mb:{mb.mode}</span>
			{/if}
			<!-- Relays pill sits before the spacer so it lands closer to the
			     visible center of the modeline rather than the right edge. -->
			<button
				class="pill pill--btn pill--relays"
				data-tour="ml-pills"
				onclick={openRelays}
				title="Relay configuration · read/write toggles · NIP-11 details"
			>
				relays
			</button>
			<span class="ml__spacer"></span>
			{#if focusedBuffer && store.modelineStatus(focusedBuffer.id)}
				<span class="ml__seg ml__status">{store.modelineStatus(focusedBuffer.id)}</span>
			{/if}
			{#if app.searchHistory.length > 0}
				<span class="hs-pill-wrap" data-tour="search-history" bind:this={hsWrapEl}>
					<button
						class="pill pill--btn pill--hs"
						onclick={() => (historyPopoverOpen = !historyPopoverOpen)}
						title="Search history · click to expand"
					>
						🔍 {app.searchHistory.length}
					</button>
					{#if historyPopoverOpen}
						{@const prevKey = app.previousEntry ? entryKey(app.previousEntry) : null}
						<div class="hs-popover" role="dialog" aria-label="Search history">
							<div class="hs-popover__list">
								{#each app.searchHistory as entry (entryKey(entry))}
									{@const k = entryKey(entry)}
									<!-- svelte-ignore a11y_click_events_have_key_events -->
									<button
										class="hs-row {prevKey === k ? 'hs-row--prev' : ''}"
										onclick={() => replayEntry(entry)}
										title={entryLabel(entry)}
									>
										<span class="hs-row__kind hs-row__kind--{entry.kind}">{entry.kind}</span>
										<span class="hs-row__label">{entryLabel(entry)}</span>
										{#if prevKey === k}
											<span class="hs-row__tag">prev search</span>
										{/if}
										<span class="hs-row__meta">{entryMeta(entry)}</span>
									</button>
								{/each}
							</div>
						</div>
					{/if}
				</span>
			{/if}
			{#if networkPill}
				<button
					class="pill pill--btn {networkPill.pillClass}"
					onclick={toggleNetworkMode}
					oncontextmenu={(e) => {
						e.preventDefault();
						openRelays();
					}}
					title="Click to toggle auto/confirm fetching · right-click for relay configuration"
				>
					<span class="dot {networkPill.dotClass}"></span>
					{networkPill.label}
				</button>
			{/if}
			{#if embeddingPill}
				<button
					class="pill pill--btn {embeddingPill.pillClass}"
					onclick={openSettings}
					title="Embedding index — click for status, sync, and reindex"
				>
					<span class="dot {embeddingPill.dotClass}"></span>
					{embeddingPill.label}
				</button>
			{/if}
			<!-- The signed-in `@npub…` identity is shown by the top-left me-chip;
			     don't mirror it here (kind 'me' is skipped). This pill stays for
			     the logged-out / locked / connecting affordances only. -->
			{#if identityPill && identityPill.kind !== 'me'}
				{#if identityPill.kind === 'connect'}
					<button
						class="pill {identityPill.pillClass}"
						onclick={connectIdentity}
						title="Log in — connect a NIP-07 extension, or open Settings for an engine key"
					>
						{identityPill.label}
					</button>
				{:else if identityPill.kind === 'locked'}
					<button
						class="pill pill--btn {identityPill.pillClass}"
						onclick={openSettings}
						title="Identity locked — click to unlock"
					>
						{identityPill.label}
					</button>
				{:else}
					<span class="pill {identityPill.pillClass}" title="Identity">
						{identityPill.label}
					</span>
				{/if}
			{/if}
			{#if app.engineBranch}
				<span class="ml__seg ml__branch" title="Git branch the engine is running from">{app.engineBranch}</span>
			{/if}
			{#if app.engineVersion}
				<span class="ml__seg ml__version" title="Engine build version">v{app.engineVersion}</span>
			{/if}
			<!-- Mode-line's own affordances, mirroring search's ? / ⚙ pair: W is
			     permanent and always tours the mode-line itself; ? opens the
			     reference. (Per-panel tours live on the logo W and window Ws.) -->
			<button
				class="affordance affordance--walkthrough walk--{modelineWalkState}"
				onclick={() => runTour('modeline-overview')}
				title="Tour the mode-line — a guided walk through each segment"
				aria-label="Mode-line walkthrough"
			>W</button>
			<button
				class="affordance affordance--help"
				onclick={openModelineHelp}
				title="Mode-line reference — what each segment means and the global keys"
				aria-label="Mode-line help"
			>?</button>
		</div>
	</div>

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

<!-- Generic walkthrough dropdown — used by the logo (all top-level tours) and
     the Chat / Research pane heads (that window's tours). Each row shows a ✓
     once run; the glyph colours by aggregate state. `pos` (when given) focuses
     that slot on open; `extra` adds a layout class. Grey + inert when empty. -->
{#snippet walkMenu(
	menuId: string,
	title: string,
	entries: GuideEntry[],
	pos: Position | null,
	extra: string = ''
)}
	{@const st = menuAggState(entries)}
	{@const pending = entries.filter((e) => !e.done).length}
	<span class="walk-wrap">
		<button
			class="affordance affordance--walkthrough {extra} walk--{st}"
			onclick={(e) => {
				e.stopPropagation();
				if (pos) store.focusSlot(pos);
				if (!entries.length) return;
				if (walkMenuOpen === menuId) {
					walkMenuOpen = null;
				} else {
					walkBtnRect = (e.currentTarget as HTMLElement).getBoundingClientRect();
					walkMenuOpen = menuId;
				}
			}}
			title={entries.length ? `${title} — ${pending} not yet run` : 'No walkthroughs here yet'}
			aria-label={title}
		>W</button>
		{#if walkMenuOpen === menuId && entries.length}
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div class="walk-backdrop" onclick={() => (walkMenuOpen = null)}></div>
			<div class="walk-menu" role="menu" bind:this={walkMenuEl} style={walkMenuStyle}>
				<div class="walk-menu__head">{title}</div>
				{#each entries as g (g.key)}
					<button
						class="walk-menu__row"
						role="menuitem"
						onclick={(e) => {
							e.stopPropagation();
							walkMenuOpen = null;
							g.run();
						}}
						title={g.done ? 'Done — click to replay' : 'Not yet run'}
					>
						<span class="walk-menu__check {g.done ? 'walk-menu__check--done' : ''}"
							>{g.done ? '✓' : ''}</span
						>
						<span class="walk-menu__label">{g.label}</span>
						{#if g.buffer}<span class="walk-menu__buffer">{g.buffer}</span>{/if}
					</button>
				{/each}
			</div>
		{/if}
	</span>
{/snippet}

{#snippet renderTree(node: SplitNode, slot: Slot, pos: Position, isRoot: boolean)}
	{#if node.type === 'leaf'}
		{@const classBuffers = store.openBuffers.filter((b) => b.className === slot.className)}
		<div class="pane {isRoot ? 'pane--root' : ''}">
			<div class="pane__head">
				<span class="cls cls--{slot.className}">{slot.className}</span>
				{#if isRoot && classBuffers.length > 1}
					<div class="pane__tabs">
						{#each classBuffers as ob (ob.buffer.id)}
							<span class="pane__tab {ob.buffer.id === node.buffer.id ? 'pane__tab--on' : ''}">
								<button
									class="pane__tab-go"
									onclick={(e) => {
										e.stopPropagation();
										store.focusSlot(pos);
										if (ob.buffer.id !== node.buffer.id) store.setLeaf(pos, ob.buffer);
									}}
									title={ob.buffer.kicker ?? ob.buffer.label}
								>{ob.buffer.label}</button>
								<button
									class="pane__tab-x"
									onclick={(e) => {
										e.stopPropagation();
										store.killBuffer(ob.buffer.id);
									}}
									title="Close this buffer (SPC b k)"
									aria-label={`Close ${ob.buffer.label}`}
								>×</button>
							</span>
						{/each}
					</div>
				{:else}
					<span class="pane__name">{node.buffer.label}</span>
				{/if}
				{#if node.buffer.kicker}
					<span class="pane__kicker">· {node.buffer.kicker}</span>
				{/if}
				{#if node.buffer.modified}
					<span class="pane__mod" title="Modified">●</span>
				{/if}
				<div class="pane__sp"></div>
				{#if isRoot && slot.className !== 'work'}
					{@render walkMenu(
						slot.className,
						`${slot.className} guides`,
						guidesForClass(slot.className),
						pos
					)}
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
						class="mb__row mb__row--mx {i === mb.selectedIndex ? 'mb__row--sel' : ''} {cmd.deferred ? 'mb__row--deferred' : ''}"
						onmouseenter={() => (mb.selectedIndex = i)}
						onclick={() => executeCommand(cmd)}
					>
						<span class="cat cat--{cmd.category.toLowerCase()}">{cmd.category}</span>
						<span class="mb__name">{cmd.name}</span>
						<span class="mb__kicker">{cmd.description}</span>
						<span class="mb__sp"></span>
						{#if cmd.deferred}
							<span class="mb__deferred">deferred</span>
						{/if}
						{#if effectiveKeybinding(cmd)}
							<span class="mb__kb">{effectiveKeybinding(cmd)}</span>
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
			<span class="mb__prompt">{mb.mode === 'global' ? 'B>' : mb.mode === 'recent' ? 'r>' : mb.mode === 'mx' ? 'cmd>' : mb.mode === 'split' ? 's>' : 'b>'}</span>
			<!-- svelte-ignore a11y_autofocus -->
			<input
				class="mb__input"
				bind:this={mbInputEl}
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
		height: 100dvh;
		background: var(--bg-alt);
		color: var(--fg);
		font-family: var(--font-sans);
		display: flex;
		flex-direction: column;
	}
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
		flex: 1;
		min-height: 0;
		overflow: hidden;
		display: flex;
		flex-direction: column;
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
	/* Brand doubles as the home button — clicking opens the feed buffer
	   in the work slot. Visually neutral so it doesn't read as a chrome
	   widget; subtle hover signals the affordance. */
	.shell__brand {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--base7);
		font-weight: 600;
		background: none;
		border: none;
		padding: 4px 6px;
		margin: 0 -6px 0 0;
		cursor: pointer;
		border-radius: var(--r-sm);
		transition: color 0.1s, background 0.1s;
	}
	.shell__brand:hover {
		color: var(--fg);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}

	.me-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 2px 8px 2px 4px;
		border: 1px solid transparent;
		border-radius: 999px;
		background: transparent;
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		cursor: pointer;
		max-width: 200px;
	}
	.me-chip:hover {
		background: var(--base1);
		border-color: var(--base3);
		color: var(--fg);
	}
	.me-chip__avatar {
		width: 18px;
		height: 18px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
		background: var(--base2);
	}
	.me-chip__dot {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		flex-shrink: 0;
		margin-left: 4px;
	}
	.me-chip__name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
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
	/* The settings entry carries the app-wide settings hue (magenta) so it
	   reads as the same affordance as the search gear. */
	.lt--settings:hover { color: var(--affordance-settings); }
	/* Sun/moon theme toggle — square, icon-centred; warm hover so it reads as
	   the "light/dark" affordance. */
	.lt--theme {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		padding: 4px 7px;
	}
	.lt--theme:hover { color: var(--affordance-help); }
	.lt--theme svg { display: block; }
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
	/* Kicker often carries a full publication title — let it shrink and
	   ellipsize instead of pushing the W/× affordances off the header. */
	.pane__kicker {
		color: var(--base5);
		font-family: var(--font-sans);
		text-transform: none;
		letter-spacing: 0;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pane__mod { color: var(--id-draft); font-size: var(--t-3xs); }
	.pane__sp { flex: 1; }
	.pane__x {
		background: transparent;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: var(--t-md);
		line-height: 1;
		padding: 0 4px;
	}
	.pane__x:hover { color: var(--fg); }
	.pane__tabs {
		display: flex;
		gap: 2px;
		align-items: center;
		min-width: 0;
		overflow-x: auto;
		scrollbar-width: none;
	}
	/* Tab = wrapper span holding the switch button + the close ×, so the
	   border/active tint frames both without nesting buttons. */
	.pane__tab {
		display: inline-flex;
		align-items: center;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--base5);
		flex-shrink: 0;
	}
	.pane__tab:hover { color: var(--fg); border-color: var(--base3); }
	.pane__tab--on {
		color: var(--fg);
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
		font-weight: 600;
	}
	.pane__tab-go {
		background: transparent;
		border: none;
		color: inherit;
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		font-weight: inherit;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 1px 2px 1px 6px;
		max-width: 18ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pane__tab-x {
		background: transparent;
		border: none;
		color: var(--base4);
		cursor: pointer;
		font-size: var(--t-xs);
		line-height: 1;
		padding: 1px 5px 1px 3px;
		border-radius: var(--r-sm);
	}
	.pane__tab-x:hover { color: var(--red, var(--fg)); }
	.pane__body {
		flex: 1;
		padding: var(--s-3);
		min-height: 0;
		min-width: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.cls {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
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
		font-size: var(--t-2xs);
		color: var(--base6);
		letter-spacing: 0.05em;
	}
	.rail--flash { animation: flash-glow 700ms ease-out; }
	.rail--focused {
		background: color-mix(in srgb, var(--id-yours) 12%, var(--panel-rail-bg));
		border-right-color: var(--id-yours);
		box-shadow: inset 1px 0 0 var(--id-yours), inset -1px 0 0 var(--id-yours);
	}

	/* Search history pill — sits in the modeline cluster, anchors the
	   Slice 3 popover via .hs-pill-wrap (position: relative). Tinted with
	   id-yours so it reads as a navigation-context pill, not a status pill. */
	.hs-pill-wrap {
		position: relative;
		display: inline-flex;
	}
	.pill--hs {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours);
	}

	/* Slice 3 popover — anchored above the pill (bottom: 100%). z-index 120
	   keeps it above every modal backdrop (100–110) so it stays clickable
	   while a modal is open (Slice 4: backdrops clip to the modeline). */
	/* Per-window walkthrough guide menu (Chat / Research pane heads). */
	.walk-wrap {
		position: relative;
		display: inline-flex;
	}
	.walk-backdrop {
		position: fixed;
		inset: 0;
		z-index: 119;
	}
	.walk-menu {
		position: fixed;
		z-index: 120;
		min-width: 220px;
		max-width: min(320px, calc(100vw - 16px));
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-md);
		font-family: var(--font-sans);
		padding: var(--s-1) 0;
	}
	.walk-menu__head {
		padding: 4px var(--s-3) 6px;
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--base5);
		border-bottom: 1px solid var(--panel-border);
		margin-bottom: var(--s-1);
	}
	.walk-menu__row {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		width: 100%;
		padding: 5px var(--s-3);
		background: transparent;
		border: none;
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-size: var(--t-sm);
	}
	.walk-menu__row:hover {
		background: var(--base1);
	}
	.walk-menu__check {
		width: 1em;
		flex: 0 0 auto;
		text-align: center;
		font-size: 0.9em;
		line-height: 1;
		color: transparent;
	}
	.walk-menu__check--done {
		color: var(--green);
	}
	.walk-menu__label {
		flex: 1;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	/* Opaque buffer tag — which surface a tour lives on (composer/reader/…),
	   shown only in the aggregate logo menu so the source is legible. */
	.walk-menu__buffer {
		flex: 0 0 auto;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: lowercase;
		letter-spacing: 0.03em;
		color: var(--fg-muted);
		opacity: 0.7;
	}

	.hs-popover {
		position: absolute;
		bottom: calc(100% + 6px);
		right: 0;
		z-index: 120;
		min-width: 320px;
		max-width: 480px;
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-md);
		font-family: var(--font-sans);
	}
	.hs-popover__list {
		display: flex;
		flex-direction: column;
		max-height: 320px;
		overflow-y: auto;
		padding: var(--s-1) 0;
	}
	.hs-row {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: 4px var(--s-3);
		background: transparent;
		border: none;
		border-left: 2px solid transparent;
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-size: var(--t-sm);
	}
	.hs-row:hover {
		background: var(--base1);
	}
	.hs-row--prev {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-left-color: var(--id-yours);
	}
	.hs-row--prev:hover {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.hs-row__kind {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		font-weight: 600;
		min-width: 56px;
		text-align: center;
		flex-shrink: 0;
	}
	.hs-row__kind--query {
		background: color-mix(in srgb, var(--cyan) 18%, transparent);
		color: var(--cyan);
	}
	.hs-row__kind--nevent {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		color: var(--id-yours);
	}
	.hs-row__kind--naddr {
		background: color-mix(in srgb, var(--id-imported) 18%, transparent);
		color: var(--id-imported);
	}
	.hs-row__label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.hs-row__meta {
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		flex-shrink: 0;
	}
	.hs-row__tag {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		padding: 1px 5px;
		border: 1px solid color-mix(in srgb, var(--id-yours) 40%, transparent);
		border-radius: var(--r-sm);
		flex-shrink: 0;
	}

	.shell__modeline {
		height: 22px;
		background: var(--panel-bg-soft);
		cursor: pointer;
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
	/* The whole strip is a leader trigger (see onModelineClick) — lift it on
	   hover so the click affordance reads. */
	.shell__modeline:hover { background: color-mix(in srgb, var(--fg) 7%, var(--panel-bg-soft)); }
	.ml__spacer { flex: 1; }
	/* Focused slot-class badge — restyled from the old vim-mode badge into a
	   colored pill. Three classes, three accents: chat purple, work blue,
	   research green. */
	.ml__class {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		padding: 0 var(--s-2);
		border-radius: var(--r-sm);
	}
	.ml__class--chat { background: color-mix(in srgb, var(--id-imported) 25%, transparent); color: var(--id-imported); }
	.ml__class--work { background: color-mix(in srgb, var(--id-yours) 25%, transparent); color: var(--id-yours); }
	.ml__class--research { background: color-mix(in srgb, var(--state-online) 25%, transparent); color: var(--state-online); }
	.ml__seg { color: var(--base6); white-space: nowrap; }
	/* The buf segment carries the buffer label + kicker — for a reader
	   that's the full publication title. It's the one segment allowed to
	   shrink (overflow:hidden lifts the min-width:auto floor) so a long
	   title ellipsizes instead of pushing the pills off the strip. */
	.ml__seg--buf {
		color: var(--fg);
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.ml__seg--prefix { color: var(--id-yours); }
	/* Right-justified loading indicator — sits after .ml__spacer (flex:1). */
	.ml__status {
		color: var(--base6);
		font-variant-numeric: tabular-nums;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	/* Engine version — quiet, far-right informational tag. */
	.ml__version { color: var(--base5, var(--base6)); font-variant-numeric: tabular-nums; opacity: 0.75; }
	.ml__branch { color: var(--orange, var(--base6)); opacity: 0.85; }
	.pill--btn {
		border: none;
		cursor: pointer;
		font: inherit;
	}
	.pill--btn:hover {
		filter: brightness(1.15);
	}
	.pill--relays {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 14%, transparent);
		color: var(--id-remote, var(--fg));
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
	.lp__hint { color: var(--base5); font-size: var(--t-2xs); }
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
		font-size: var(--t-3xs);
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
		font-size: var(--t-2xs);
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
	.mb__row--deferred {
		opacity: 0.45;
	}
	.mb__deferred {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		border: 1px dashed var(--base3);
		border-radius: var(--r-sm);
		padding: 1px 6px;
		flex-shrink: 0;
	}

	.cat {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
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
		font-size: var(--t-2xs);
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
		font-size: var(--t-2xs);
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
	.kb__icon { font-size: var(--t-md); align-self: center; opacity: 0.7; }
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
