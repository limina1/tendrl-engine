<script lang="ts">
	// The board: one column per section (whole publication first), cards
	// sized to their content — no absolute positioning, no drag. A card
	// expands in place; its answers list under it. DOM per
	// docs/annotations-ledger-contract.org ("Ledger buffer").
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { answersOf, glyphOf, relTime, sectionLabel, type LedgerRow, type LedgerView, type Section } from './ledger-model';

	let {
		rows,
		view,
		sections,
		hotSec = null,
		cursorId = null,
		who,
		onselect,
		onhover,
		onjump,
		onaction
	}: {
		rows: LedgerRow[];
		view: LedgerView;
		sections: Section[];
		hotSec?: number | null;
		cursorId?: string | null;
		who: (pk: string) => string;
		onselect: (row: LedgerRow) => void;
		onhover: (row: LedgerRow | null) => void;
		onjump: (sec: number) => void;
		/** answer · review · relink · open · menu on an expanded card. */
		onaction: (what: 'answer' | 'review' | 'relink' | 'open' | 'menu', row: LedgerRow) => void;
	} = $props();

	const cols = $derived(
		[-1, ...sections.map((_, i) => i)].map((sec) => ({
			sec,
			items: rows.filter((r) => r.sec === sec && !(r.request && rows.some((q) => q.id === r.request)))
		}))
	);
	const kindWord = (r: LedgerRow) =>
		r.kind === 'request' ? (r.output ?? 'request') : r.kind === 'note' ? (r.placement === 'annotation' ? 'attached' : r.placement) : r.kind === 'def' ? 'definition' : r.kind === 'mr' ? 'merge request' : 'kasten';
</script>

<div class="board">
	{#each cols as c (c.sec)}
		<div class="bcol" data-sec={c.sec}>
			<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
			<div class="bcol__bh" class:hot={hotSec === c.sec} onclick={() => onjump(c.sec)} title="scroll the reader there">
				{sectionLabel(c.sec, sections)} <span class="bcol__n">· {c.items.length}</span>
			</div>
			{#each c.items as r (r.id)}
				{@const g = glyphOf(r)}
				{@const ans = r.kind === 'request' ? answersOf(rows, r.id) : []}
				{@const x = view.expanded === r.id}
				<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
				<div
					class="bcard"
					class:x
					class:cur={cursorId === r.id}
					data-id={r.id}
					style="--au: {r.author ? pubkeyToColor(r.author) : 'var(--base5)'}"
					onclick={() => onselect(r)}
					onmouseenter={() => onhover(r)}
					onmouseleave={() => onhover(null)}
				>
					<div class="bcard__h">
						<span class="g {g.cls}">{g.g}</span>{kindWord(r)}
						<span class="bcard__sp"></span>
						<span class="bcard__dot"></span>{r.author ? who(r.author) : 'unsigned'} · {relTime(r.createdAt)}
					</div>
					<div class="bcard__t">{r.title}</div>
					{#if !(r.kind === 'request' && (r.output === 'vocabulary' || r.output === 'kasten'))}
						<div class="bcard__e">{r.body}</div>
					{/if}
					{#each ans as a (a.id)}
						{@const ag = glyphOf(a)}
						<div class="bcard__child"><span class="g {ag.cls}">{ag.g}</span> {a.title} · {a.author ? who(a.author) : 'unsigned'}</div>
					{/each}
					{#if x}
						<div class="bcard__acts">
							{#if r.kind === 'request' && (r.status === 'open' || r.output === 'vocabulary')}
								<button class="act act--answer" onclick={(e) => { e.stopPropagation(); onaction('answer', r); }}>{r.status === 'open' ? 'answer it yourself ▸' : 'define a term ▸'}</button>
							{/if}
							{#if r.kind === 'mr'}
								<button class="act act--review" onclick={(e) => { e.stopPropagation(); onaction('review', r); }}>review ▸</button>
							{/if}
							{#if r.kind === 'def'}
								<button class="act act--relink" onclick={(e) => { e.stopPropagation(); onaction('relink', r); }}>relink</button>
							{/if}
							{#if r.kind !== 'mr' && r.kind !== 'request'}
								<button class="act" onclick={(e) => { e.stopPropagation(); onaction('open', r); }}>open ▸</button>
							{/if}
							<button class="act" onclick={(e) => { e.stopPropagation(); onaction('menu', r); }}>menu</button>
						</div>
					{/if}
				</div>
			{:else}
				<div class="bcol__empty">—</div>
			{/each}
		</div>
	{/each}
</div>

<style>
	.board {
		flex: 1;
		overflow: auto;
		padding: 10px;
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(230px, 1fr);
		gap: 10px;
		align-items: start;
		min-height: 0;
	}
	.bcol { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
	.bcol__bh {
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		padding: 2px 2px 4px;
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		gap: 6px;
		cursor: pointer;
	}
	.bcol__bh.hot { color: var(--blue); }
	.bcol__n { color: var(--base4); font-weight: 400; }
	.bcol__empty { padding: 14px 4px; color: var(--base5); font: var(--t-xs) / 1.4 var(--font-mono); }
	.bcard {
		background: var(--base1);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au);
		border-radius: var(--r-md);
		padding: 6px 9px 7px;
		cursor: pointer;
		font: var(--t-xs) / 1.4 var(--font-sans);
	}
	.bcard:hover { border-color: var(--base5); border-left-color: var(--au); }
	.bcard.cur { box-shadow: 0 0 0 1px var(--blue); }
	.bcard__h { display: flex; gap: 6px; align-items: center; font: var(--t-3xs) / 1.2 var(--font-mono); color: var(--base5); margin-bottom: 3px; }
	.bcard__sp { flex: 1; }
	.bcard__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); display: inline-block; flex: none; }
	.bcard__t { font: 600 var(--t-sm) / 1.3 var(--font-sans); color: var(--fg); }
	.bcard__e { color: var(--fg-alt); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; margin-top: 2px; }
	.bcard.x .bcard__e { display: block; -webkit-line-clamp: unset; line-clamp: unset; font: var(--t-base) / 1.5 var(--font-serif); color: var(--fg); }
	.bcard__child { margin-top: 4px; padding-left: 8px; border-left: 2px solid var(--panel-border); color: var(--base6); font: var(--t-2xs) / 1.3 var(--font-mono); }
	.bcard__acts { display: flex; gap: 6px; margin-top: 6px; flex-wrap: wrap; }
	.act {
		font: var(--t-3xs) / 1.2 var(--font-mono);
		min-height: 22px;
		padding: 1px 8px;
		color: var(--base6);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		background: transparent;
		cursor: pointer;
	}
	.act:hover { border-color: var(--base5); color: var(--fg); }
	.act--answer,
	.act--review { color: var(--ask); }
	.act--relink { color: var(--green); }
	.g { font-weight: 700; }
	.g.note { color: var(--base6); }
	.g.def { color: var(--green); }
	.g.req { color: var(--ask); }
	.g.ok { color: var(--blue); }
	.g.mr { color: var(--ask); }
	.g.kasten { color: var(--violet); }
	.g.merged { color: var(--green); }
	.g.declined { color: var(--red); }
</style>
