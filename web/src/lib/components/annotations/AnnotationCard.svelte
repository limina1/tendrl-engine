<script lang="ts">
	// One card on the annotations table — a note or a request, absolutely
	// placed by the table from a `Placed`. DOM per the contract
	// (docs/annotations-table-contract.org, "Table buffer"): `.acard` with
	// `.m` / `.x` / `.cur` / `.pinned` modifiers, inline `--au` from the
	// highlight colour recipe, `.acard__h` / `.acard__t` / `.acard__b` /
	// `.acard__f`. The table wires drag (free layout) and hover.
	import type { Placed, Section } from './table-model';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { styleTitle } from '$lib/styles.svelte';
	import RichContent from '$lib/components/RichContent.svelte';
	import { draggable, type DraggableOptions } from '$lib/actions/draggable';

	let {
		placed,
		index,
		small = false,
		expanded = false,
		cursor = false,
		section = null,
		drag = {},
		onselect,
		onanchor,
		onexpand,
		onopen,
		onmenu,
		onrequest,
		onhover,
		onunpin = undefined
	}: {
		placed: Placed;
		index: number;
		small?: boolean;
		expanded?: boolean;
		cursor?: boolean;
		/** The section the card anchors to (null = the whole publication). */
		section?: Section | null;
		/** Free-layout drag: the action moves this node; `onEnd` commits the pin. */
		drag?: DraggableOptions;
		/** Double-click in the free layout releases a pin. */
		onunpin?: (() => void) | undefined;
		onselect: () => void;
		/** The "→ §n" chip: scroll the reader to the anchor. */
		onanchor: () => void;
		onexpand: () => void;
		/** Open the note in its own reader / the request's full record. */
		onopen: () => void;
		/** The universal event menu on the signed event. */
		onmenu: () => void;
		/** The "↳ answer" chip: open the request this note answers. */
		onrequest: () => void;
		onhover: (on: boolean) => void;
	} = $props();

	const card = $derived(placed.card);
	const au = $derived(card.author ? pubkeyToColor(card.author) : 'var(--base5)');
	const auSoft = $derived(card.author ? pubkeyToColor(card.author, 0.14) : 'transparent');
	const who = $derived(card.author ? getAuthorDisplayName(card.author) : 'unsigned');
	const placement = $derived(
		card.placement === 'before' ? '▴ before' : card.placement === 'after' ? '▾ after' : '◂ attached'
	);
	const styleLabel = $derived.by(() => {
		const s = card.style;
		if (!s) return null;
		const t = card.req?.style_title ?? styleTitle(s);
		return (t ?? s.split(':').slice(2).join(':')).split(',')[0];
	});
	const anchorLabel = $derived(section ? `→ §${section.n}` : '→ §');
	const anchorTitle = $derived(
		section ? `Scroll the reader to §${section.n} ${section.title}` : 'Scroll the reader to the top'
	);

	// A second click on the card within the double-click window releases a
	// pin. Detected here rather than via `dblclick`: under the drag action's
	// pointer capture the browser's own double-click does not always fire
	// (the drag's swallowed click after a move also resets the count).
	let lastClick = 0;
	function onclick() {
		const now = performance.now();
		if (now - lastClick < 350) {
			lastClick = 0;
			onunpin?.();
			return;
		}
		lastClick = now;
		onselect();
	}

	function rel(ts: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000) - ts);
		if (s < 60) return 'now';
		if (s < 3600) return `${Math.floor(s / 60)}m`;
		if (s < 86400) return `${Math.floor(s / 3600)}h`;
		if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
		return new Date(ts * 1000).toLocaleDateString();
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="acard"
	class:m={small && !expanded}
	class:x={expanded}
	class:cur={cursor}
	class:pinned={placed.pinned}
	class:req={card.kind === 'request'}
	data-cursor={index}
	data-id={card.id}
	style:left="{placed.x}px"
	style:top="{placed.y}px"
	style:--au={au}
	style:--au-soft={auSoft}
	use:draggable={drag}
	onclick={onclick}
	onkeydown={(e) => e.key === 'Enter' && e.target === e.currentTarget && onselect()}
	onmouseenter={() => onhover(true)}
	onmouseleave={() => onhover(false)}
>
	<div class="acard__h">
		<button class="acard__anchor" onclick={(e) => { e.stopPropagation(); onanchor(); }} title={anchorTitle}>{anchorLabel}</button>
		{#if card.kind === 'request'}
			<span class="badge" class:answered={card.status === 'answered'}>{card.status === 'answered' ? '✓ request' : '? request'}</span>
		{:else}
			<span class="acard__cap">{placement}</span>
		{/if}
		<span class="acard__by"><span class="acard__dot"></span>{who} · {rel(card.createdAt)}</span>
	</div>
	<div class="acard__t">{card.title}</div>
	{#if expanded}
		<div class="acard__b acard__b--full">
			{#if card.note}
				<RichContent content={card.note.event.content} muted />
			{:else if card.req}
				<p class="acard__task">{card.req.task}</p>
				<p class="acard__meta">scope: {card.req.scope.join(' · ') || 'section'} · {card.req.output}</p>
				{#if card.req.context}<blockquote class="acard__ctx">{card.req.context}</blockquote>{/if}
				{#if card.req.answer}<p class="acard__meta">answered: {card.req.answer.summary}</p>{/if}
			{/if}
		</div>
	{:else}
		<div class="acard__b">{card.excerpt}</div>
	{/if}
	<div class="acard__f">
		{#if styleLabel}<span class="acard__style" title={card.style}>✎ {styleLabel}</span>{/if}
		{#if card.request}
			<button class="acard__style acard__answer" onclick={(e) => { e.stopPropagation(); onrequest(); }} title="Written for this request — {card.request}">↳ answer</button>
		{/if}
		{#if expanded}
			<button class="acard__act" onclick={(e) => { e.stopPropagation(); onopen(); }} title={card.kind === 'note' ? 'Open this section in its own reader buffer' : 'Open the request'}>open</button>
			{#if card.kind === 'note' || card.req?.coordinate}
				<button class="acard__act" onclick={(e) => { e.stopPropagation(); onmenu(); }} title="Event menu — JSON, comment, broadcast">menu</button>
			{/if}
		{/if}
		<button class="acard__open" onclick={(e) => { e.stopPropagation(); onexpand(); }}>{expanded ? 'collapse ▴' : 'open as section ▸'}</button>
	</div>
</div>

<style>
	.acard {
		position: absolute;
		width: 200px;
		box-sizing: border-box;
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--yellow));
		border-radius: var(--r-sm);
		padding: 6px 8px 8px;
		font: var(--t-xs) / 1.4 var(--font-sans);
		color: var(--fg);
		transition: left 0.35s ease, top 0.35s ease, width 0.2s;
		cursor: default;
		user-select: none;
	}
	.acard.m { width: 150px; max-height: 64px; overflow: hidden; }
	.acard.m .acard__t { white-space: nowrap; }
	.acard.x { width: 330px; z-index: 5; }
	.acard.cur { outline: 2px solid var(--blue); outline-offset: 1px; }
	.acard.pinned { z-index: 3; }
	.acard.pinned::after {
		content: '';
		position: absolute;
		right: 6px;
		top: -5px;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--yellow);
		box-shadow: 0 0 0 2px var(--panel-bg);
	}
	:global(.acard.dragging) { transition: none; z-index: 10; box-shadow: var(--shadow-lg); cursor: grabbing; }
	:global(.acard.draggable) { cursor: grab; }
	.acard__h {
		display: flex;
		gap: 6px;
		align-items: baseline;
		font: var(--t-3xs) / 1.2 var(--font-mono);
		margin-bottom: 3px;
		color: var(--base5);
		white-space: nowrap;
		overflow: hidden;
	}
	.acard__cap { flex: none; }
	.acard__anchor {
		border: 0;
		border-radius: 0;
		padding: 0;
		min-height: 0;
		font: inherit;
		color: var(--au);
		background: transparent;
		border-bottom: 1px dotted transparent;
		cursor: pointer;
	}
	.acard__anchor:hover { border-bottom-color: var(--au); background: transparent; }
	.acard__by {
		color: var(--base5);
		margin-left: auto;
		white-space: nowrap;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.acard__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); flex: none; }
	.badge {
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--cyan);
		border: 1px solid color-mix(in srgb, var(--cyan) 50%, transparent);
		border-radius: 3px;
		padding: 1px 6px;
		white-space: nowrap;
	}
	.badge.answered { color: var(--blue); border-color: color-mix(in srgb, var(--blue) 50%, transparent); }
	.acard__t { font-weight: 600; margin-bottom: 2px; overflow: hidden; text-overflow: ellipsis; }
	.acard__b {
		color: var(--fg-alt);
		max-height: 3.9em;
		overflow: hidden;
		-webkit-mask-image: linear-gradient(#000 60%, transparent);
		mask-image: linear-gradient(#000 60%, transparent);
	}
	.acard__b--full { max-height: 130px; overflow: auto; mask-image: none; -webkit-mask-image: none; user-select: text; }
	.acard.m .acard__b { display: none; }
	.acard__task { margin: 0 0 4px; }
	.acard__meta { margin: 0 0 4px; font: var(--t-3xs) / 1.3 var(--font-mono); color: var(--base5); }
	.acard__ctx { margin: 0 0 4px; padding-left: 6px; border-left: 2px solid var(--panel-border); color: var(--base5); font-style: italic; }
	.acard__f {
		display: flex;
		gap: 6px;
		align-items: center;
		margin-top: 6px;
		font: var(--t-3xs) / 1.2 var(--font-mono);
		color: var(--base5);
		flex-wrap: wrap;
	}
	.acard__style, .acard__act, .acard__open {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 0 5px;
		min-height: 0;
		font: inherit;
		color: inherit;
		background: transparent;
		white-space: nowrap;
	}
	span.acard__style { cursor: default; }
	.acard__answer { color: var(--au); cursor: pointer; }
	.acard__act { cursor: pointer; }
	.acard__open { margin-left: auto; color: var(--blue); cursor: pointer; border-color: transparent; }
	.acard__open:hover, .acard__act:hover, .acard__answer:hover { background: var(--base1); border-color: var(--base4); }
</style>
