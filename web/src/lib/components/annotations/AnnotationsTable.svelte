<script lang="ts">
	// The table proper: a scroll container (`.atable`) around a canvas
	// (`.cv`) sized from the LayoutResult, lanes / ticks / axis marks, and
	// the cards absolutely placed with a CSS transition on left/top so a
	// mode switch animates. Drag (free layout) moves the node and commits a
	// pin on release — the layout never sees the transient. `width` is
	// bound out so the buffer packs against the real viewport.
	import { untrack } from 'svelte';
	import type { LayoutResult, LayoutMode, Section, TableCard } from './table-model';
	import AnnotationCard from './AnnotationCard.svelte';

	let {
		layout,
		mode,
		sections,
		cursor,
		expanded,
		hotAddr = null,
		width = $bindable(0),
		initialScroll = { x: 0, y: 0 },
		onscroll,
		onselect,
		onanchor,
		onexpand,
		onopen,
		onmenu,
		onrequest,
		onhover,
		onpin,
		onunpin
	}: {
		layout: LayoutResult;
		mode: LayoutMode;
		sections: Section[];
		cursor: number;
		expanded: string | null;
		/** The parent coordinate the reader is lit on (hoverAnchor). */
		hotAddr?: string | null;
		width?: number;
		initialScroll?: { x: number; y: number };
		onscroll: (x: number, y: number) => void;
		onselect: (index: number) => void;
		onanchor: (card: TableCard) => void;
		onexpand: (card: TableCard) => void;
		onopen: (card: TableCard) => void;
		onmenu: (card: TableCard) => void;
		onrequest: (card: TableCard) => void;
		onhover: (card: TableCard | null) => void;
		onpin: (id: string, x: number, y: number) => void;
		onunpin: (id: string) => void;
	} = $props();

	const small = $derived(mode === 'timeline' || mode === 'collective');
	const sectionByCoord = $derived.by(() => {
		const m = new Map<string, Section>();
		for (const s of sections) m.set(s.coordinate, s);
		return m;
	});

	let el: HTMLDivElement | undefined = $state();
	// Restore the parked scroll once the container exists (untracked: the
	// initial offsets are a mount-time input, not a live dependency).
	$effect(() => {
		const node = el;
		if (!node) return;
		untrack(() => {
			node.scrollLeft = initialScroll.x;
			node.scrollTop = initialScroll.y;
		});
	});

	// Keep the cursored card in view after j/k.
	$effect(() => {
		const i = cursor;
		const node = el;
		if (!node) return;
		queueMicrotask(() => {
			node.querySelector<HTMLElement>(`.acard[data-cursor="${i}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
		});
	});
</script>

<div
	class="atable"
	bind:this={el}
	bind:clientWidth={width}
	role="region"
	aria-label="Annotations table"
	onscroll={() => el && onscroll(el.scrollLeft, el.scrollTop)}
>
	<div class="cv" style:width="{layout.width}px" style:height="{layout.height}px">
		{#each layout.lanes as lane (lane.key)}
			<div
				class="lane"
				class:hot={!!hotAddr && lane.parent === hotAddr}
				data-lane={lane.key}
				style:top="{lane.y}px"
				style:height="{lane.h}px"
			>
				<span class="lane__lab">{lane.label}</span>
			</div>
		{/each}
		{#each layout.ticks as tick, i (mode + ':' + i)}
			{#if mode === 'collective'}
				<div class="axis" style:left="{tick.x}px">{tick.label}</div>
			{:else}
				<div class="tick" style:left="{tick.x}px">{tick.label}</div>
			{/if}
		{/each}
		{#each layout.placed as p, i (p.card.id)}
			<AnnotationCard
				placed={p}
				index={i}
				{small}
				expanded={expanded === p.card.id}
				cursor={cursor === i}
				section={sectionByCoord.get(p.card.parent) ?? null}
				drag={{
					enabled: mode === 'free',
					ignore: 'button, .acard__anchor, .acard__open',
					onEnd: (x, y) => onpin(p.card.id, x, y)
				}}
				onselect={() => onselect(i)}
				onanchor={() => onanchor(p.card)}
				onexpand={() => onexpand(p.card)}
				onopen={() => onopen(p.card)}
				onmenu={() => onmenu(p.card)}
				onrequest={() => onrequest(p.card)}
				onhover={(on) => onhover(on ? p.card : null)}
				onunpin={() => mode === 'free' && p.pinned && onunpin(p.card.id)}
			/>
		{/each}
	</div>
</div>

<style>
	.atable {
		flex: 1;
		min-height: 0;
		overflow: auto;
		position: relative;
		background-image: radial-gradient(var(--panel-border) 1px, transparent 1px);
		background-size: 22px 22px;
	}
	.cv { position: relative; min-width: 100%; }
	.lane { position: absolute; left: 0; right: 0; border-top: 1px solid var(--panel-border); pointer-events: none; }
	.lane__lab {
		position: absolute;
		left: 10px;
		top: 4px;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		white-space: nowrap;
		max-width: 260px;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.lane.hot { background: color-mix(in srgb, var(--yellow) 6%, transparent); }
	.lane.hot .lane__lab { color: var(--yellow); }
	.axis {
		position: absolute;
		top: 0;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		pointer-events: none;
	}
	.tick {
		position: absolute;
		top: 4px;
		font: var(--t-3xs) / 1.2 var(--font-mono);
		color: var(--base5);
		border-left: 1px solid var(--panel-border);
		padding-left: 4px;
		height: 100%;
		pointer-events: none;
	}
</style>
