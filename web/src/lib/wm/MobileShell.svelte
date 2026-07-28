<script lang="ts">
	import type { Buffer, ClassName } from './types';
	import type { BufferStore } from './buffer-store.svelte';
	import BufferRenderer from './BufferRenderer.svelte';

	// The mobile shell renders the SAME BufferStore as the desktop WM — the
	// three class slots become full-screen panels behind a bottom bar, and
	// the work class gets a vertical rail of its open buffers (the mobile
	// reading of the desktop pane tabs). Splits are desktop-only: only the
	// focused leaf of the active class renders here.
	let {
		store,
		onCommands
	}: {
		store: BufferStore;
		onCommands: () => void;
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

	function switchClass(cls: ClassName) {
		const pos = store.findSlotForClass(cls);
		if (pos) store.focusSlot(pos);
	}

	function railSelect(buf: Buffer) {
		const pos = store.findSlotForClass('work');
		if (!pos) return;
		store.focusSlot(pos);
		if (activeLeaf?.buffer.id !== buf.id) store.setLeaf(pos, buf);
	}
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
		{#if activeClass === 'work'}
			<nav class="mshell__rail" aria-label="Open work buffers">
				{#each workBuffers as ob (ob.buffer.id)}
					<button
						class="mshell__rail-item {activeLeaf?.buffer.id === ob.buffer.id
							? 'mshell__rail-item--on'
							: ''}"
						onclick={() => railSelect(ob.buffer)}
						title={ob.buffer.kicker ?? ob.buffer.label}
					>{ob.buffer.label}</button>
				{/each}
				<div class="mshell__rail-sp"></div>
				<button
					class="mshell__rail-add"
					onclick={onCommands}
					title="Commands — open buffers, act (M-x)"
					aria-label="Open command palette"
				>+</button>
			</nav>
		{/if}
		<div class="mshell__panel">
			{#if activeLeaf}
				<BufferRenderer buffer={activeLeaf.buffer} />
			{/if}
		</div>
	</div>

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

	.mshell__rail {
		width: 38px;
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		align-items: stretch;
		gap: var(--s-1);
		padding: var(--s-2) 0;
		background: var(--panel-rail-bg);
		border-right: 1px solid var(--panel-border);
		overflow-y: auto;
	}
	.mshell__rail-item {
		writing-mode: vertical-rl;
		transform: rotate(180deg);
		border: none;
		border-right: 2px solid transparent;
		background: none;
		color: var(--fg-alt);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		letter-spacing: 0.08em;
		padding: var(--s-2) var(--s-1);
		max-height: 7.5rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		cursor: pointer;
	}
	.mshell__rail-item--on {
		color: var(--fg);
		font-weight: 600;
		/* rotate(180deg) flips the painted edge: border-right renders on the
		   left, flush against the rail's outer edge. */
		border-right-color: var(--accent);
		background: var(--panel-bg-soft);
	}
	.mshell__rail-sp {
		flex: 1;
	}
	.mshell__rail-add {
		border: 1px dashed var(--panel-border-strong);
		border-radius: var(--r-sm);
		background: none;
		color: var(--fg-alt);
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		margin: 0 4px;
		padding: var(--s-1) 0;
		cursor: pointer;
	}
	.mshell__rail-add:hover {
		color: var(--accent);
		border-color: var(--accent);
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
