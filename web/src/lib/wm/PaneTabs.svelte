<script lang="ts">
	import type { Buffer } from './types';

	// The pane-head tab strip, extracted from +page's renderTree so it can
	// carry scroll state. The strip has always been overflow-x with a hidden
	// scrollbar; what this adds is the mouse/touch affordances that make the
	// overflow *visible and reachable* without a keyboard: edge fades +
	// chevron nudges when scrolled, the active tab kept in view, vertical
	// wheel mapped to horizontal scroll, and a trailing count button that
	// opens the class buffer switcher (the same list as SPC b b) once tabs
	// don't all fit. Shell chrome, not a buffer renderer — but it still
	// follows the buffer-component rules: props + $effect teardown only.
	let {
		buffers,
		activeId,
		onSelect,
		onKill,
		onMore
	}: {
		buffers: Buffer[];
		activeId: string;
		onSelect: (buf: Buffer) => void;
		onKill: (id: string) => void;
		/** Open the class-scoped buffer switcher (SPC b b) — the full list. */
		onMore: () => void;
	} = $props();

	let stripEl = $state<HTMLElement | null>(null);
	let canLeft = $state(false);
	let canRight = $state(false);
	const overflowing = $derived(canLeft || canRight);

	function recompute() {
		const el = stripEl;
		if (!el) return;
		canLeft = el.scrollLeft > 1;
		canRight = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
	}

	// Overflow state tracks resizes and tab-set changes; rAF lets the DOM
	// settle after an {#each} update before measuring.
	$effect(() => {
		void buffers.length;
		const el = stripEl;
		if (!el) return;
		const ro = new ResizeObserver(recompute);
		ro.observe(el);
		const raf = requestAnimationFrame(recompute);
		return () => {
			ro.disconnect();
			cancelAnimationFrame(raf);
		};
	});

	// Keep the active tab visible however it was activated (click, SPC b n/p,
	// minibuffer switch) — scrolled out of view it reads as "buffer gone".
	$effect(() => {
		void activeId;
		const el = stripEl;
		if (!el) return;
		const raf = requestAnimationFrame(() => {
			el.querySelector('.pane__tab--on')?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
		});
		return () => cancelAnimationFrame(raf);
	});

	function nudge(dir: 1 | -1) {
		stripEl?.scrollBy({ left: dir * stripEl.clientWidth * 0.6, behavior: 'smooth' });
	}

	function onWheel(e: WheelEvent) {
		const el = stripEl;
		if (!el || el.scrollWidth <= el.clientWidth) return;
		if (Math.abs(e.deltaY) > Math.abs(e.deltaX)) {
			el.scrollLeft += e.deltaY;
			e.preventDefault();
		}
	}
</script>

<div class="ptabs" data-tour="pane-tabs">
	{#if overflowing}
		<button
			class="ptabs__nav"
			disabled={!canLeft}
			onclick={(e) => {
				e.stopPropagation();
				nudge(-1);
			}}
			aria-label="Scroll tabs left"
		>‹</button>
	{/if}
	<div
		class="pane__tabs"
		class:pane__tabs--fade-l={canLeft}
		class:pane__tabs--fade-r={canRight}
		bind:this={stripEl}
		onscroll={recompute}
		onwheel={onWheel}
	>
		{#each buffers as buf (buf.id)}
			<span class="pane__tab {buf.id === activeId ? 'pane__tab--on' : ''}">
				<button
					class="pane__tab-go"
					onclick={(e) => {
						e.stopPropagation();
						onSelect(buf);
					}}
					title={buf.kicker ?? buf.label}
				>{buf.label}</button>
				<button
					class="pane__tab-x"
					onclick={(e) => {
						e.stopPropagation();
						onKill(buf.id);
					}}
					title="Close this buffer (SPC b k)"
					aria-label={`Close ${buf.label}`}
				>×</button>
			</span>
		{/each}
	</div>
	{#if overflowing}
		<button
			class="ptabs__nav"
			disabled={!canRight}
			onclick={(e) => {
				e.stopPropagation();
				nudge(1);
			}}
			aria-label="Scroll tabs right"
		>›</button>
		<button
			class="ptabs__all"
			onclick={(e) => {
				e.stopPropagation();
				onMore();
			}}
			title="List all {buffers.length} open buffers (SPC b b)"
		>{buffers.length}▾</button>
	{/if}
</div>

<style>
	.ptabs {
		display: flex;
		align-items: center;
		gap: 2px;
		min-width: 0;
	}
	.pane__tabs {
		display: flex;
		gap: 2px;
		align-items: center;
		min-width: 0;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.pane__tabs::-webkit-scrollbar {
		display: none;
	}
	/* Edge fades say "there's more" without a scrollbar; mask keeps them
	   theme-agnostic (no background-matched gradient overlays). */
	.pane__tabs--fade-l {
		mask-image: linear-gradient(to right, transparent 0, black 16px);
	}
	.pane__tabs--fade-r {
		mask-image: linear-gradient(to right, black calc(100% - 16px), transparent 100%);
	}
	.pane__tabs--fade-l.pane__tabs--fade-r {
		mask-image: linear-gradient(
			to right,
			transparent 0,
			black 16px,
			black calc(100% - 16px),
			transparent 100%
		);
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
	.pane__tab:hover {
		color: var(--fg);
		border-color: var(--base3);
	}
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
	.pane__tab-x:hover {
		color: var(--red, var(--fg));
	}
	.ptabs__nav {
		background: transparent;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: var(--t-sm);
		line-height: 1;
		padding: 0 2px;
		flex-shrink: 0;
	}
	.ptabs__nav:hover:enabled {
		color: var(--fg);
	}
	.ptabs__nav:disabled {
		opacity: 0.3;
		cursor: default;
	}
	.ptabs__all {
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base5);
		cursor: pointer;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		line-height: 1;
		padding: 2px 5px;
		flex-shrink: 0;
	}
	.ptabs__all:hover {
		color: var(--fg);
		border-color: var(--id-yours);
	}
</style>
