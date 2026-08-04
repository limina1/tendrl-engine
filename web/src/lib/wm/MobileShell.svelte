<script lang="ts">
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import type { Buffer, ClassName } from './types';
	import type { BufferStore } from './buffer-store.svelte';
	import { mobileNav } from './mobile-nav.svelte';
	import BufferRenderer from './BufferRenderer.svelte';

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
		onIdentityTap
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
		onIdentityTap?: () => void;
	} = $props();

	// Bottom-bar order + user-facing labels. 'research' reads as "search" on
	// the bar; the class name stays research everywhere else.
	const bar: { cls: ClassName; label: string }[] = [
		{ cls: 'chat', label: 'chat' },
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
		{#if activeClass === 'work'}
			<button
				class="mshell__menu-btn {mobileNav.drawerOpen ? 'mshell__menu-btn--on' : ''}"
				onclick={() => (mobileNav.drawerOpen = !mobileNav.drawerOpen)}
				title="Open work buffers"
				aria-label="Open the work-buffer drawer"
			>☰</button>
		{/if}
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

	{#if mobileNav.drawerOpen && activeClass === 'work'}
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div class="mshell__scrim" onclick={() => (mobileNav.drawerOpen = false)}></div>
		<nav class="mshell__drawer" aria-label="Open work buffers">
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
					onCommands();
				}}
				title="Commands — open buffers, act (M-x)"
			>+ commands</button>

			<!-- Mobile home for the desktop modeline's status pills. -->
			<div class="mshell__drawer-sp"></div>
			<div class="mshell__status">
				<div class="mshell__status-head">status</div>
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
					<div class="mshell__status-row mshell__status-row--static">
						{#if embeddingPill.dotClass}<span class="dot {embeddingPill.dotClass}"></span>{/if}
						<span class="mshell__status-label">{embeddingPill.label}</span>
					</div>
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

	<nav class="mshell__bar" aria-label="Main panels">
		{#each bar as b (b.cls)}
			<button
				class="mshell__bar-item mshell__bar-item--{b.cls} {activeClass === b.cls
					? 'mshell__bar-item--on'
					: ''}"
				onclick={() => switchClass(b.cls)}
			>{b.label}</button>
		{/each}
		<button class="mshell__bar-item mshell__bar-item--cmd" onclick={onCommands}>cmds</button>
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

	.mshell__menu-btn {
		border: none;
		background: none;
		color: var(--fg-alt);
		font-size: var(--t-sm);
		line-height: 1;
		padding: var(--s-1) var(--s-1);
		margin-left: calc(-1 * var(--s-1));
		cursor: pointer;
		align-self: center;
	}
	.mshell__menu-btn--on,
	.mshell__menu-btn:hover {
		color: var(--fg);
	}

	.mshell__scrim {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.45);
		z-index: 50;
	}
	.mshell__drawer {
		position: fixed;
		top: 0;
		bottom: 0;
		left: 0;
		width: min(78vw, 300px);
		display: flex;
		flex-direction: column;
		background: var(--panel-bg);
		border-right: 1px solid var(--panel-border-strong);
		z-index: 51;
		padding: var(--s-2) 0 calc(var(--s-2) + env(safe-area-inset-bottom));
		overflow-y: auto;
		animation: mshell-drawer-in 160ms ease-out;
	}
	@keyframes mshell-drawer-in {
		from {
			transform: translateX(-100%);
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
		margin: var(--s-2) var(--s-3);
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
	.mshell__bar-item--cmd {
		flex: 0 0 22%;
		border-left: 1px solid var(--panel-border);
		color: var(--fg-alt);
	}
</style>
