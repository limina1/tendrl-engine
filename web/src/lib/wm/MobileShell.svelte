<script lang="ts">
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import type { Buffer, ClassName } from './types';
	import type { BufferStore } from './buffer-store.svelte';
	import type { NetworkStatus } from '$lib/types';
	import { mobileNav } from './mobile-nav.svelte';
	import { shell } from './shell.svelte';
	import BufferRenderer from './BufferRenderer.svelte';
	import ActivityCenter from './ActivityCenter.svelte';

	// The mobile shell renders the SAME BufferStore as the desktop WM — the
	// three class slots become full-screen panels behind a bottom bar, and
	// the work class gets a vertical rail of its open buffers (the mobile
	// reading of the desktop pane tabs). Splits are desktop-only: only the
	// focused leaf of the active class renders here.
	// Status pills are computed in +page (single source shared with the
	// desktop modeline) and prop-threaded here; the drawer just renders
	// them. Tap routing likewise stays in +page (onIdentityTap knows the
	// pill kind; onToggleNetwork flips auto/confirm).
	type StatusPill = { label: string; pillClass: string; dotClass?: string };

	let {
		store,
		onCommands,
		networkPill = null,
		identityPill = null,
		embeddingPill = null,
		engineInfo = null,
		onToggleNetwork,
		onOpenRelays,
		onOpenSettings,
		onOpenCompose,
		onIdentityTap,
		activity = null,
		onKillFetch,
		wiki = null,
		onResolveWiki,
		searchRows = []
	}: {
		store: BufferStore;
		onCommands: () => void;
		networkPill?: StatusPill | null;
		identityPill?: StatusPill | null;
		embeddingPill?: StatusPill | null;
		/** Engine build info (version + git branch) — the mobile parity of
		 *  the mode-line's version/branch segments. */
		engineInfo?: { version: string | null; branch: string | null } | null;
		onToggleNetwork?: () => void;
		onOpenRelays?: () => void;
		onOpenSettings?: () => void;
		onOpenCompose?: () => void;
		onIdentityTap?: () => void;
		/** Live network-activity summary (the modeline ⇅ pill's data). */
		activity?: NetworkStatus | null;
		onKillFetch?: (id?: number) => void;
		/** Wiki-resolution progress (the modeline n/m pill); null = no wiki
		 *  links on the current screen, row hidden. */
		wiki?: { found: number; total: number; busy: boolean } | null;
		onResolveWiki?: () => void;
		/** Search history, pre-labelled by +page (the modeline 🔍 pill). */
		searchRows?: { key: string; kind: string; label: string; meta: string; replay: () => void }[];
	} = $props();

	// Drawer-local expand state for the activity and history sub-lists.
	let actOpen = $state(false);
	let histOpen = $state(false);

	// Bottom-bar order + user-facing labels. Only work and search get a slot
	// for now ('research' reads as "search" on the bar; the class name stays
	// research everywhere else). Chat has no slot — it stays reachable
	// through the ☰ drawer's commands entry, and its panel still renders
	// when something opens it.
	const bar: { cls: ClassName; label: string }[] = [
		{ cls: 'work', label: 'work' },
		{ cls: 'research', label: 'search' }
	];

	const activeClass = $derived(store.focusedSlotClass() ?? 'work');
	const activePos = $derived(store.findSlotForClass(activeClass));
	const activeLeaf = $derived(activePos ? store.focusedLeaf(activePos) : null);
	const workBuffers = $derived(store.openBuffers.filter((b) => b.className === 'work'));

	// Amethyst-style buffer drawer: the ☰ in the work header slides in a left
	// drawer listing open work buffers as horizontal rows. Select closes it;
	// kill keeps it open so the pruned list stays visible. Open-state lives
	// in mobileNav so hardware Back can close it.
	function switchClass(cls: ClassName) {
		mobileNav.drawerOpen = false;
		const pos = store.findSlotForClass(cls);
		if (pos) store.focusSlot(pos);
	}

	function drawerSelect(buf: Buffer) {
		const pos = store.findSlotForClass('work');
		if (!pos) return;
		store.focusSlot(pos);
		if (activeLeaf?.buffer.id !== buf.id) store.setLeaf(pos, buf);
		mobileNav.drawerOpen = false;
	}

	// ── Back navigation ─────────────────────────────────────────────────
	// The work slot's focused buffer is tracked even while another class is
	// active, so Back restores the work panel as the user left it.
	const workPos = $derived(store.findSlotForClass('work'));
	const workBufId = $derived(workPos ? (store.focusedLeaf(workPos)?.buffer.id ?? null) : null);

	// Central watcher: one history entry per genuine panel/buffer change,
	// whatever caused it (bottom bar, drawer, palette, renderer openBuffer —
	// effect batching coalesces openBuffer's focus+leaf writes into one
	// entry). Reads tracked, writes untracked; fully synchronous.
	$effect(() => {
		const cls = activeClass;
		const wb = workBufId;
		untrack(() => mobileNav.syncFromApp(cls, wb));
	});

	// History watcher: kit restores shallow state into page.state on
	// back/forward; mobileNav distinguishes echoes from real traversals.
	$effect(() => {
		const entry = page.state.mnav;
		untrack(() => mobileNav.syncFromHistory(entry, store));
	});

	// Teardown on shell flip to desktop — desktop stays history-free; any
	// stale mnav entries left in history are inert (nobody watches them).
	$effect(() => () => mobileNav.reset());
</script>

<div class="mshell">
	<div class="mshell__head">
		<span class="cls cls--{activeClass}">{activeClass === 'research' ? 'search' : activeClass}</span>
		{#if activeLeaf}
			<span class="mshell__head-name">{activeLeaf.buffer.label}</span>
			{#if activeLeaf.buffer.kicker}
				<span class="mshell__head-kicker">· {activeLeaf.buffer.kicker}</span>
			{/if}
			{#if activeLeaf.buffer.modified}
				<span class="mshell__head-mod" title="Modified">●</span>
			{/if}
		{/if}
	</div>

	<div class="mshell__body">
		<div class="mshell__panel">
			{#if activeLeaf}
				<BufferRenderer buffer={activeLeaf.buffer} />
			{/if}
		</div>
	</div>

	{#if mobileNav.drawerOpen}
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="mshell__scrim" onclick={() => (mobileNav.drawerOpen = false)}></div>
		<nav class="mshell__drawer mshell__drawer--{shell.menuEdge}" aria-label="Open work buffers">
			<div class="mshell__drawer-head">
				<span class="cls cls--work">work</span>
				<span class="mshell__drawer-title">buffers</span>
			</div>
			{#each workBuffers as ob (ob.buffer.id)}
				<div
					class="mshell__drawer-row {activeLeaf?.buffer.id === ob.buffer.id
						? 'mshell__drawer-row--on'
						: ''}"
				>
					<button class="mshell__drawer-go" onclick={() => drawerSelect(ob.buffer)}>
						<span class="mshell__drawer-name">{ob.buffer.label}</span>
						{#if ob.buffer.kicker}
							<span class="mshell__drawer-kicker">{ob.buffer.kicker}</span>
						{/if}
					</button>
					{#if ob.buffer.modified}
						<span class="mshell__drawer-mod" title="Modified">●</span>
					{/if}
					<button
						class="mshell__drawer-x"
						onclick={() => store.killBuffer(ob.buffer.id)}
						title="Close this buffer"
						aria-label={`Close ${ob.buffer.label}`}
					>×</button>
				</div>
			{/each}
			<button
				class="mshell__drawer-add"
				onclick={() => {
					mobileNav.drawerOpen = false;
					onOpenCompose?.();
				}}
				title="Open the composer (current draft)"
			>+ compose</button>
			<button
				class="mshell__drawer-add"
				data-tour="mobile-cmds"
				onclick={() => {
					mobileNav.drawerOpen = false;
					onCommands();
				}}
				title="Commands — open buffers, act (M-x)"
			>+ commands</button>

			<!-- Mobile home for the desktop modeline's status pills, in the
			     same visual vocabulary (dots + pills); › marks rows that open
			     a work buffer rather than toggling in place. -->
			<div class="mshell__drawer-sp"></div>
			<div class="mshell__status" data-tour="mobile-status">
				<div class="mshell__status-head">status</div>
				<button
					class="mshell__status-row"
					onclick={() => {
						mobileNav.drawerOpen = false;
						onOpenSettings?.();
					}}
					title="Open settings (identity, embedding, appearance)"
				>
					<span class="mshell__status-label">settings</span>
					<span class="mshell__status-go" aria-hidden="true">›</span>
				</button>
				{#if networkPill}
					<button
						class="mshell__status-row"
						onclick={onToggleNetwork}
						title="Toggle network mode (auto / confirm)"
					>
						{#if networkPill.dotClass}<span class="dot {networkPill.dotClass}"></span>{/if}
						<span class="mshell__status-label">network</span>
						<span class="pill {networkPill.pillClass}">{networkPill.label}</span>
					</button>
				{/if}
				<button
					class="mshell__status-row"
					onclick={() => {
						mobileNav.drawerOpen = false;
						onOpenRelays?.();
					}}
					title="Relay configuration"
				>
					<span class="mshell__status-label">relays</span>
					<span class="mshell__status-go" aria-hidden="true">›</span>
				</button>
				{#if identityPill}
					<button
						class="mshell__status-row"
						onclick={() => {
							mobileNav.drawerOpen = false;
							onIdentityTap?.();
						}}
						title="Identity / signing"
					>
						<span class="mshell__status-label">identity</span>
						<span class="pill {identityPill.pillClass}">{identityPill.label}</span>
					</button>
				{/if}
				{#if embeddingPill}
					<button
						class="mshell__status-row"
						onclick={() => {
							mobileNav.drawerOpen = false;
							onOpenSettings?.();
						}}
						title="Embedding index — status, sync, and reindex live in Settings"
					>
						{#if embeddingPill.dotClass}<span class="dot {embeddingPill.dotClass}"></span>{/if}
						<span class="mshell__status-label">{embeddingPill.label}</span>
						<span class="mshell__status-go" aria-hidden="true">›</span>
					</button>
				{/if}
				{#if wiki}
					<button
						class="mshell__status-row"
						onclick={onResolveWiki}
						title={wiki.found < wiki.total
							? `${wiki.total - wiki.found} wiki links unresolved — tap to fetch them all from relays`
							: 'All wiki links resolved — tap to re-fetch from relays'}
					>
						{#if wiki.busy}<span class="dot dot--fetching"></span>{/if}
						<span class="mshell__status-label">wiki links</span>
						<span class="pill pill--ghost">{wiki.found}/{wiki.total}</span>
					</button>
				{/if}
				<button
					class="mshell__status-row"
					onclick={() => (actOpen = !actOpen)}
					title="Network activity — what the engine pulled, and why"
				>
					<span class="mshell__status-label">⇅ activity</span>
					{#if (activity?.active_fetches ?? 0) > 0}
						<span class="pill pill--ghost">{activity?.active_fetches}</span>
					{/if}
					<span class="mshell__status-go" aria-hidden="true">{actOpen ? '▾' : '▸'}</span>
				</button>
				{#if actOpen}
					<div class="mshell__act">
						<ActivityCenter {activity} onKill={onKillFetch} />
					</div>
				{/if}
				{#if searchRows.length > 0}
					<button
						class="mshell__status-row"
						onclick={() => (histOpen = !histOpen)}
						title="Search history — tap an entry to run it again"
					>
						<span class="mshell__status-label">🔍 searches</span>
						<span class="pill pill--ghost">{searchRows.length}</span>
						<span class="mshell__status-go" aria-hidden="true">{histOpen ? '▾' : '▸'}</span>
					</button>
					{#if histOpen}
						<div class="mshell__hist">
							{#each searchRows as row (row.key)}
								<button
									class="mshell__hist-row"
									onclick={() => {
										mobileNav.drawerOpen = false;
										row.replay();
									}}
									title={row.label}
								>
									<span class="mshell__hist-kind">{row.kind}</span>
									<span class="mshell__hist-label">{row.label}</span>
									{#if row.meta}<span class="mshell__hist-meta">{row.meta}</span>{/if}
								</button>
							{/each}
						</div>
					{/if}
				{/if}
				{#if engineInfo?.version || engineInfo?.branch}
					<div
						class="mshell__status-row mshell__status-row--static"
						title={engineInfo.branch ? `Git branch the engine is running from: ${engineInfo.branch}` : ''}
					>
						<span class="mshell__status-label">
							engine{engineInfo.version ? ` v${engineInfo.version}` : ''}{engineInfo.branch
								? ` · ${engineInfo.branch.split('/').pop()}`
								: ''}
						</span>
					</div>
				{/if}
			</div>
		</nav>
	{/if}

	<!-- ☰ lives in the bar, thumb-reachable, not the top-left header. Small
	     fixed slot on the edge picked by Settings → "Menu edge" (right by
	     default). Global, not work-only: the drawer is the sole route to
	     STATUS — search needs it too. -->
	{#snippet menuBtn()}
		<button
			class="mshell__bar-item mshell__bar-item--menu mshell__bar-item--menu-{shell.menuEdge} {mobileNav.drawerOpen
				? 'mshell__bar-item--on'
				: ''}"
			data-tour="mobile-menu"
			onclick={() => (mobileNav.drawerOpen = !mobileNav.drawerOpen)}
			title="Buffers + status"
			aria-label="Open the buffer/status drawer"
		>☰</button>
	{/snippet}
	<nav class="mshell__bar" data-tour="mobile-bar" aria-label="Main panels">
		{#if shell.menuEdge === 'left'}{@render menuBtn()}{/if}
		{#each bar as b (b.cls)}
			<button
				class="mshell__bar-item mshell__bar-item--{b.cls} {activeClass === b.cls
					? 'mshell__bar-item--on'
					: ''}"
				onclick={() => switchClass(b.cls)}
			>{b.label}</button>
		{/each}
		{#if shell.menuEdge === 'right'}{@render menuBtn()}{/if}
	</nav>
</div>

<style>
	.mshell {
		height: 100%;
		display: flex;
		flex-direction: column;
		background: var(--bg);
		color: var(--fg);
		overflow: hidden;
	}

	.mshell__head {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
		padding: var(--s-1) var(--s-2);
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-header-bg);
		flex-shrink: 0;
		min-height: var(--panel-header-h);
		box-sizing: border-box;
	}
	.mshell__head-name {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
	}
	.mshell__head-kicker {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-alt);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mshell__head-mod {
		color: var(--accent);
		font-size: var(--t-2xs);
	}

	/* .cls badge styles live in +page.svelte's scope — restate the pill here
	   since scoped styles don't cross components. */
	.cls {
		display: inline-flex;
		align-items: center;
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

	.mshell__body {
		flex: 1;
		display: flex;
		min-height: 0;
	}

	.mshell__scrim {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.45);
		z-index: 50;
	}
	/* The drawer slides out from the ☰ button's own edge. */
	.mshell__drawer {
		position: fixed;
		top: 0;
		bottom: 0;
		width: min(78vw, 300px);
		display: flex;
		flex-direction: column;
		background: var(--panel-bg);
		z-index: 51;
		padding: var(--s-2) 0 calc(var(--s-2) + env(safe-area-inset-bottom));
		overflow-y: auto;
	}
	.mshell__drawer--left {
		left: 0;
		border-right: 1px solid var(--panel-border-strong);
		animation: mshell-drawer-in-left 160ms ease-out;
	}
	.mshell__drawer--right {
		right: 0;
		border-left: 1px solid var(--panel-border-strong);
		animation: mshell-drawer-in-right 160ms ease-out;
	}
	@keyframes mshell-drawer-in-left {
		from {
			transform: translateX(-100%);
		}
		to {
			transform: translateX(0);
		}
	}
	@keyframes mshell-drawer-in-right {
		from {
			transform: translateX(100%);
		}
		to {
			transform: translateX(0);
		}
	}
	.mshell__drawer-head {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
		padding: var(--s-2) var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		margin-bottom: var(--s-1);
	}
	.mshell__drawer-title {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-alt);
		text-transform: uppercase;
		letter-spacing: 0.08em;
	}
	.mshell__drawer-row {
		display: flex;
		align-items: center;
		border-left: 2px solid transparent;
	}
	.mshell__drawer-row--on {
		border-left-color: var(--accent);
		background: var(--panel-bg-soft);
	}
	.mshell__drawer-go {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 2px;
		border: none;
		background: none;
		color: var(--fg);
		text-align: left;
		font-family: var(--font-mono);
		padding: var(--s-2) var(--s-3);
		cursor: pointer;
		min-height: 44px;
		justify-content: center;
	}
	.mshell__drawer-name {
		font-size: var(--t-xs);
		font-weight: 600;
	}
	.mshell__drawer-row--on .mshell__drawer-name {
		color: var(--accent);
	}
	.mshell__drawer-kicker {
		font-size: var(--t-2xs);
		color: var(--fg-alt);
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mshell__drawer-mod {
		color: var(--accent);
		font-size: var(--t-2xs);
		flex-shrink: 0;
	}
	.mshell__drawer-x {
		border: none;
		background: none;
		color: var(--fg-alt);
		font-size: var(--t-sm);
		padding: var(--s-2) var(--s-3);
		cursor: pointer;
		flex-shrink: 0;
	}
	.mshell__drawer-x:hover {
		color: var(--danger);
	}
	.mshell__drawer-add {
		margin: var(--s-2) var(--s-3) 0;
		border: 1px dashed var(--panel-border-strong);
		border-radius: var(--r-sm);
		background: none;
		color: var(--fg-alt);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: var(--s-2);
		cursor: pointer;
	}
	.mshell__drawer-add:hover {
		color: var(--accent);
		border-color: var(--accent);
	}
	.mshell__drawer-sp {
		flex: 1;
	}
	.mshell__status {
		border-top: 1px solid var(--panel-border);
		padding: var(--s-2) 0 0;
	}
	.mshell__status-head {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-alt);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		padding: 0 var(--s-3) var(--s-1);
	}
	.mshell__status-row {
		width: 100%;
		display: flex;
		align-items: center;
		gap: var(--s-2);
		border: none;
		background: none;
		color: var(--fg);
		text-align: left;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: var(--s-2) var(--s-3);
		min-height: 40px;
		cursor: pointer;
	}
	.mshell__status-row--static {
		cursor: default;
	}
	.mshell__status-label {
		flex: 1;
		color: var(--fg-alt);
	}
	.mshell__status-go {
		color: var(--fg-alt);
		font-size: var(--t-sm);
		line-height: 1;
	}
	/* Inline expansions of the activity / search-history rows. */
	.mshell__act {
		font-family: var(--font-sans);
		font-size: var(--t-2xs);
		border-left: 2px solid var(--panel-border);
		margin: 0 0 var(--s-1);
	}
	.mshell__hist {
		border-left: 2px solid var(--panel-border);
		margin: 0 0 var(--s-1);
	}
	.mshell__hist-row {
		width: 100%;
		display: flex;
		align-items: center;
		gap: var(--s-2);
		border: none;
		background: none;
		color: var(--fg);
		text-align: left;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: var(--s-1) var(--s-3);
		min-height: 40px;
		cursor: pointer;
	}
	.mshell__hist-kind {
		flex-shrink: 0;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		font-size: var(--t-3xs);
		padding: 0 4px;
	}
	.mshell__hist-label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mshell__hist-meta {
		flex-shrink: 0;
		color: var(--fg-alt);
		font-size: var(--t-3xs);
	}

	.mshell__panel {
		flex: 1;
		min-width: 0;
		min-height: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	/* BufferRenderer's child is the panel's sole flex item — let it fill. */
	.mshell__panel > :global(*) {
		flex: 1;
		min-height: 0;
	}

	.mshell__bar {
		display: flex;
		flex-shrink: 0;
		border-top: 1px solid var(--panel-border-strong);
		background: var(--panel-header-bg);
		padding-bottom: env(safe-area-inset-bottom);
	}
	.mshell__bar-item {
		flex: 1;
		border: none;
		border-top: 2px solid transparent;
		background: none;
		color: var(--fg-alt);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		letter-spacing: 0.06em;
		padding: var(--s-2) 0 calc(var(--s-2) + 2px);
		cursor: pointer;
		min-height: 44px;
	}
	.mshell__bar-item--on {
		color: var(--fg);
		font-weight: 600;
	}
	.mshell__bar-item--on.mshell__bar-item--chat { border-top-color: var(--id-imported); }
	.mshell__bar-item--on.mshell__bar-item--work { border-top-color: var(--id-yours); }
	.mshell__bar-item--on.mshell__bar-item--research { border-top-color: var(--id-remote); }
	/* Small fixed slot — the drawer trigger, not a class panel. The divider
	   sits on its inner edge, whichever side Settings puts it on. */
	.mshell__bar-item--menu {
		flex: 0 0 56px;
		font-size: var(--t-sm);
	}
	.mshell__bar-item--menu-left { border-right: 1px solid var(--panel-border); }
	.mshell__bar-item--menu-right { border-left: 1px solid var(--panel-border); }
	.mshell__bar-item--menu.mshell__bar-item--on {
		border-top-color: var(--accent);
	}
</style>
