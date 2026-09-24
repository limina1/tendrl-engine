<script lang="ts">
	// The rows: a scroll container of sticky group headers and rows, a
	// request's answers nested under it (section grouping), the expanded
	// body right under its row when the detail is inline. DOM per
	// docs/annotations-ledger-contract.org ("Ledger buffer").
	import { untrack } from 'svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import LedgerRow from './LedgerRow.svelte';
	import LedgerBody from './LedgerBody.svelte';
	import { answersOf, type LedgerGroup, type LedgerRow as Row, type LedgerView, type Section } from './ledger-model';

	let {
		groups,
		rows,
		flat,
		view,
		sections,
		root,
		rootTitle = null,
		ownsRoot = false,
		hotSec = null,
		fresh,
		phone = false,
		inline = true,
		empty = null,
		who,
		titleOf,
		initialScroll = 0,
		onscroll,
		onselect,
		onanchor,
		onhover,
		ontogglegroup,
		onopen,
		onreveal,
		onchanged
	}: {
		groups: LedgerGroup[];
		/** Every filtered row (answers are looked up here). */
		rows: Row[];
		/** The cursor's order. */
		flat: Row[];
		view: LedgerView;
		sections: Section[];
		root: string;
		rootTitle?: string | null;
		ownsRoot?: boolean;
		/** The section ordinal the reader is lit on (-1 = whole publication). */
		hotSec?: number | null;
		/** Ids that just landed via the store stream (`.new`). */
		fresh: Set<string>;
		phone?: boolean;
		/** Render the expanded body under its row (false = the detail pane has it). */
		inline?: boolean;
		empty?: string | null;
		who: (pk: string) => string;
		/** Title lookup for a row id (the request a note answers, the reply target). */
		titleOf: (id: string) => string | undefined;
		initialScroll?: number;
		onscroll: (top: number) => void;
		onselect: (row: Row) => void;
		onanchor: (row: Row) => void;
		onhover: (row: Row | null) => void;
		ontogglegroup: (key: string) => void;
		onopen: (row: Row) => void;
		onreveal: (id: string) => void;
		onchanged: () => void;
	} = $props();

	let el: HTMLDivElement | undefined = $state();
	$effect(() => {
		const node = el;
		if (!node) return;
		untrack(() => {
			node.scrollTop = initialScroll;
		});
	});
	// Keep the cursored row in view after j/k.
	$effect(() => {
		const i = view.cursor;
		const node = el;
		if (!node) return;
		queueMicrotask(() => {
			node.querySelector<HTMLElement>(`.row[data-cursor="${i}"]`)?.scrollIntoView({ block: 'nearest' });
		});
	});
	const indexOf = (id: string) => flat.findIndex((r) => r.id === id);
	const collapsed = $derived(new Set(view.collapsed));
	const requestOutput = (r: Row) => (r.request ? rows.find((q) => q.id === r.request)?.output : undefined);
</script>

<div class="rows" bind:this={el} tabindex="0" role="grid" aria-label="Annotations" onscroll={() => el && onscroll(el.scrollTop)}>
	{#if empty}
		<div class="rows__empty">{empty}</div>
	{/if}
	{#each groups as g (g.key)}
		<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
		<div
			class="gh"
			class:hot={hotSec != null && g.sec === hotSec}
			data-group={g.key}
			style={g.author ? `--au: ${pubkeyToColor(g.author)}` : undefined}
			onclick={() => ontogglegroup(g.key)}
			onmouseenter={() => g.sec != null && onhover({ sec: g.sec } as Row)}
			onmouseleave={() => onhover(null)}
		>
			{#if g.author}<span class="gh__dot"></span>{/if}
			{g.label}
			<span class="gh__n">· {g.rows.length}</span>
			{#if g.sec != null && g.sec >= 0}
				<button class="gh__anc" onclick={(e) => { e.stopPropagation(); onanchor(g.rows[0]); }} title="scroll the reader there">→</button>
			{/if}
			<span class="gh__ptr">{collapsed.has(g.key) ? '▸' : '▾'}</span>
		</div>
		{#if !collapsed.has(g.key)}
			{#each g.rows as row (row.id)}
				{@const answers = view.group === 'section' && row.kind === 'request' ? answersOf(rows, row.id) : []}
				{@const idx = indexOf(row.id)}
				<LedgerRow
					{row}
					index={idx}
					{sections}
					cursor={view.cursor === idx}
					expanded={view.expanded === row.id}
					fresh={fresh.has(row.id)}
					{phone}
					answers={row.kind === 'request' ? answersOf(rows, row.id).length : 0}
					who={row.author ? who(row.author) : 'unsigned'}
					requestOutput={requestOutput(row)}
					replyTitle={row.replyTo ? titleOf(row.replyTo) : undefined}
					onselect={() => onselect(row)}
					onanchor={() => onanchor(row)}
					onhover={(on) => onhover(on ? row : null)}
				/>
				{#if inline && view.expanded === row.id}
					<LedgerBody {row} answers={answersOf(rows, row.id)} {sections} {root} {rootTitle} {ownsRoot} {who} requestTitle={row.request ? titleOf(row.request) : undefined} {onopen} {onreveal} {onchanged} />
				{/if}
				{#each answers as a (a.id)}
					{@const aidx = indexOf(a.id)}
					<LedgerRow
						row={a}
						index={aidx}
						{sections}
						cursor={view.cursor === aidx}
						expanded={view.expanded === a.id}
						child
						fresh={fresh.has(a.id)}
						{phone}
						who={a.author ? who(a.author) : 'unsigned'}
						replyTitle={a.replyTo ? titleOf(a.replyTo) : undefined}
						onselect={() => onselect(a)}
						onanchor={() => onanchor(a)}
						onhover={(on) => onhover(on ? a : null)}
					/>
					{#if inline && view.expanded === a.id}
						<LedgerBody row={a} {sections} {root} {rootTitle} {ownsRoot} {who} requestTitle={row.title} {onopen} {onreveal} {onchanged} />
					{/if}
				{/each}
			{/each}
		{/if}
	{/each}
</div>

<style>
	.rows { flex: 1; overflow: auto; min-height: 0; outline: none; padding-bottom: 30px; }
	.rows__empty {
		padding: var(--s-2) var(--s-3);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
		border-bottom: 1px solid var(--panel-border);
	}
	.gh {
		position: sticky;
		top: 0;
		z-index: 2;
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 10px 3px;
		background: var(--panel-bg);
		border-bottom: 1px solid var(--panel-border);
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		cursor: pointer;
		user-select: none;
	}
	.gh:hover { color: var(--base7); }
	.gh.hot { color: var(--blue); }
	.gh__n { color: var(--base4); font-weight: 400; letter-spacing: 0; }
	.gh__dot { width: 6px; height: 6px; border-radius: 50%; background: var(--au); display: inline-block; }
	.gh__anc {
		min-height: 16px;
		padding: 0 4px;
		border: 1px solid transparent;
		border-radius: var(--r-md);
		color: var(--base4);
		font-size: var(--t-3xs);
		background: transparent;
		cursor: pointer;
	}
	.gh__anc:hover { color: var(--blue); border-color: var(--panel-border-strong); }
	.gh__ptr { margin-left: auto; color: var(--base4); }
</style>
