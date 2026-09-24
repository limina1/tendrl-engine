<script lang="ts">
	// One row of the ledger — grid `3px 38px 1fr auto auto 14px`: the
	// author rail, the § cell (scrolls the reader), the main (badge or
	// placement glyph, title, flags, subs), the author, the age, the
	// pointer. Modifiers .cur / .x / .child / .new. DOM per
	// docs/annotations-ledger-contract.org ("Ledger buffer").
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { relTime, type LedgerRow, type Section } from './ledger-model';

	let {
		row,
		index,
		sections,
		cursor = false,
		expanded = false,
		child = false,
		fresh = false,
		phone = false,
		answers = 0,
		who,
		requestOutput = undefined,
		replyTitle = undefined,
		onselect,
		onanchor,
		onhover
	}: {
		row: LedgerRow;
		index: number;
		sections: Section[];
		cursor?: boolean;
		expanded?: boolean;
		child?: boolean;
		fresh?: boolean;
		phone?: boolean;
		/** Requests: how many rows answer it. */
		answers?: number;
		who: string;
		/** Answering rows: the output word of the request they answer. */
		requestOutput?: string | undefined;
		/** Notes in reply: the title of the annotation they answer. */
		replyTitle?: string | undefined;
		onselect: () => void;
		onanchor: () => void;
		onhover: (on: boolean) => void;
	} = $props();

	const au = $derived(row.author ? pubkeyToColor(row.author) : 'var(--base5)');
	const auSoft = $derived(row.author ? pubkeyToColor(row.author, 0.14) : 'transparent');
	const secLabel = $derived(row.sec < 0 ? '⌂' : `§${sections[row.sec]?.n ?? row.sec + 1}`);
	const secTitle = $derived(
		row.sec < 0 ? 'whole publication — scroll the reader to the top' : `§${sections[row.sec]?.n ?? row.sec + 1} · ${sections[row.sec]?.title ?? ''} — scroll the reader there`
	);
	const placement = $derived(row.placement === 'before' ? '▴ before' : row.placement === 'after' ? '▾ after' : '◂');
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="row"
	class:cur={cursor}
	class:x={expanded}
	class:child
	class:new={fresh}
	data-id={row.id}
	data-kind={row.kind}
	data-cursor={index}
	style:--au={au}
	style:--au-soft={auSoft}
	role="row"
	tabindex="-1"
	onclick={onselect}
	onkeydown={(e) => e.key === 'Enter' && e.target === e.currentTarget && onselect()}
	onmouseenter={() => onhover(true)}
	onmouseleave={() => onhover(false)}
>
	<span class="row__rail"></span>
	<button class="row__anc" onclick={(e) => { e.stopPropagation(); onanchor(); }} title={secTitle}>{secLabel}</button>
	<span class="row__main">
		{#if row.kind === 'request'}
			<span class="badge" class:req={row.status === 'open'} class:ok={row.status !== 'open'}>{row.status === 'open' ? '?' : '✓'} {row.output}</span>
		{:else if row.kind === 'def'}
			<span class="badge def">def</span>
		{:else if row.kind === 'mr'}
			<span class="badge" class:mr={row.status === 'open'} class:merged={row.status === 'merged'} class:declined={row.status === 'declined'}>⑂ merge · {row.status}</span>
		{:else if row.kind === 'kasten'}
			<span class="badge kasten">kasten{row.note?.children?.length ? ` · ${row.note.children.length}` : ''}</span>
		{:else}
			<span class="row__placement" title={row.placement}>{placement}</span>
		{/if}
		<span class="row__t">{row.title}</span>
		{#if row.kind === 'def' && row.glossary}
			<span class="flag glossary" title="listed in the glossary bundle">glossary</span>
		{/if}
		{#if row.kind === 'request' && row.status === 'open' && !phone}
			<span class="flag wait" title="waiting for your agent, or for you">waiting</span>
		{/if}
		{#if answers > 0}
			<span class="row__sub" title="{answers} written for it">{answers} ↳</span>
		{/if}
		{#if row.request && !child && requestOutput}
			<span class="row__sub" title="answers a request">↳ {requestOutput}</span>
		{/if}
		{#if row.replyTo}
			<span class="row__sub" title="a note written in reply to another annotation">↩ {replyTitle ?? row.replyTo.split(':').slice(2).join(':').slice(0, 16)}</span>
		{/if}
		{#if row.replies > 0}
			<span class="row__sub row__sub--rep" title="{row.replies} replies (kind 1111)">{row.replies} ↩</span>
		{/if}
		{#if phone}
			<span class="row__sec">· {secLabel} · <span class="row__dot"></span> {who}</span>
		{/if}
	</span>
	<span class="row__by"><span class="row__dot"></span>{who}</span>
	<span class="row__age">{relTime(row.createdAt)}</span>
	<span class="row__ptr">{expanded ? '▾' : '▸'}</span>
</div>

<style>
	.row {
		display: grid;
		grid-template-columns: 3px 38px minmax(0, 1fr) auto auto 14px;
		align-items: center;
		gap: 0 8px;
		min-height: 30px;
		padding: 0 8px 0 0;
		border-bottom: 1px solid var(--panel-border);
		cursor: pointer;
		position: relative;
		outline: none;
	}
	.row:hover { background: var(--base1); }
	.row.cur { background: var(--base2); box-shadow: inset 2px 0 0 var(--blue); }
	.row.x { background: var(--base1); }
	.row.new { animation: fan 0.4s ease-out; }
	@keyframes fan {
		from { opacity: 0; transform: translateY(-4px); }
		to { opacity: 1; transform: none; }
	}
	@media (prefers-reduced-motion: reduce) {
		.row.new { animation: none; }
	}
	.row__rail { align-self: stretch; background: var(--au); }
	.row__anc {
		font: var(--t-2xs) / 1 var(--font-mono);
		color: var(--base5);
		text-align: center;
		border: 1px solid transparent;
		border-radius: var(--r-md);
		min-height: 20px;
		padding: 0 3px;
		background: transparent;
		cursor: pointer;
	}
	.row__anc:hover { color: var(--blue); border-color: var(--panel-border-strong); }
	.row__main { display: flex; align-items: center; gap: 6px; min-width: 0; font: var(--t-sm) / 1.3 var(--font-sans); }
	.badge {
		white-space: nowrap;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.06em;
		text-transform: uppercase;
		border: 1px solid;
		border-radius: 3px;
		padding: 1px 5px;
		flex: none;
	}
	.badge.req { color: var(--ask); border-color: rgba(226, 164, 120, 0.5); }
	.badge.ok { color: var(--blue); border-color: rgba(132, 160, 198, 0.5); }
	.badge.def { color: var(--green); border-color: rgba(180, 190, 130, 0.5); }
	.badge.mr { color: var(--ask); border-color: rgba(226, 164, 120, 0.5); }
	.badge.kasten { color: var(--violet); border-color: rgba(178, 148, 187, 0.5); }
	.badge.merged { color: var(--green); border-color: rgba(180, 190, 130, 0.5); }
	.badge.declined { color: var(--red); border-color: rgba(226, 120, 120, 0.5); }
	.row__placement { color: var(--base4); font: var(--t-2xs) / 1 var(--font-mono); flex: none; white-space: nowrap; }
	.row__t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--fg); }
	.row__sub { color: var(--base5); font: var(--t-2xs) / 1.2 var(--font-mono); flex: none; white-space: nowrap; }
	.row__sub--rep { color: var(--cyan); }
	.flag {
		font: var(--t-3xs) / 1 var(--font-mono);
		color: var(--green);
		border: 1px solid rgba(180, 190, 130, 0.4);
		border-radius: 3px;
		padding: 1px 4px;
		flex: none;
	}
	.flag.wait { color: var(--base5); border-color: var(--panel-border); }
	.flag.wait::after { content: ''; animation: dots 1.2s steps(4) infinite; }
	@keyframes dots {
		0% { content: ''; }
		25% { content: '.'; }
		50% { content: '..'; }
		75% { content: '...'; }
	}
	.row__by { font: var(--t-2xs) / 1 var(--font-mono); color: var(--base5); display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
	.row__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); display: inline-block; flex: none; }
	.row__age { font: var(--t-2xs) / 1 var(--font-mono); color: var(--base4); text-align: right; min-width: 24px; font-variant-numeric: tabular-nums; }
	.row__ptr { color: var(--base4); font-size: var(--t-2xs); text-align: center; }
	.row.child .row__anc { color: var(--base4); }
	.row.child .row__main { padding-left: 14px; }
	.row.child .row__main::before { content: '↳'; color: var(--base4); font-family: var(--font-mono); font-size: var(--t-2xs); margin-right: 2px; }
	.row__sec { display: none; }
	/* phone */
	:global(.abuf.phone) .row { grid-template-columns: 3px minmax(0, 1fr) auto 14px; min-height: 40px; }
	:global(.abuf.phone) .row__anc { display: none; }
	:global(.abuf.phone) .row__by { display: none; }
	:global(.abuf.phone) .row__main { flex-wrap: wrap; row-gap: 1px; font-size: var(--t-base); }
	:global(.abuf.phone) .row__t { white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
	:global(.abuf.phone) .row__sub { display: none; }
	:global(.abuf.phone) .row__sec { display: inline-flex; align-items: center; gap: 4px; font: var(--t-2xs) / 1 var(--font-mono); color: var(--base5); }
	:global(.abuf.phone) .row__sec .row__dot { width: 6px; height: 6px; }
</style>
