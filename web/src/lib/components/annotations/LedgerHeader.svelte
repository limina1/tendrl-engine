<script lang="ts">
	// The ledger's two rows of chrome — every element a filter. Row one:
	// the spell input (the search DSL the engine mirrors; Enter applies),
	// the wand, "+ annotate", the group-by segment, rows|board, the count.
	// Row two, the facets: who (author bars), where (§ counts), what (kind
	// counts), when (a 21-day sparkline + all·7d·30d). DOM per
	// docs/annotations-ledger-contract.org ("Ledger buffer").
	import { pubkeyToColor } from '$lib/discussions/colors';
	import {
		GROUPS,
		KINDS,
		SPARK_DAYS,
		type FacetCounts,
		type GroupBy,
		type LedgerKind,
		type LedgerView,
		type Section
	} from './ledger-model';

	let {
		spell,
		view,
		counts,
		sections,
		authors,
		shown,
		total,
		phone = false,
		onapply,
		onwand,
		onannotate,
		ongroup,
		onview,
		ontoggleby,
		ontogglesec,
		ontogglekind,
		onpickday,
		oncyclesince,
		onhoversec
	}: {
		spell: string;
		view: LedgerView;
		counts: FacetCounts;
		sections: Section[];
		/** Authors in bar order (descending count) with their labels. */
		authors: { pubkey: string; label: string }[];
		shown: number;
		total: number;
		phone?: boolean;
		/** Apply a hand-edited spell; false = refused (the input reverts). */
		onapply: (text: string) => boolean;
		onwand: () => void;
		onannotate: () => void;
		ongroup: (g: GroupBy) => void;
		onview: (v: 'rows' | 'board') => void;
		ontoggleby: (pk: string) => void;
		ontogglesec: (sec: number) => void;
		ontogglekind: (k: LedgerKind) => void;
		onpickday: (day: number) => void;
		oncyclesince: () => void;
		onhoversec: (sec: number | null) => void;
	} = $props();

	let text = $state('');
	$effect(() => {
		text = spell;
	});
	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			if (!onapply(text)) text = spell;
			(e.currentTarget as HTMLInputElement).blur();
		} else if (e.key === 'Escape') {
			text = spell;
			(e.currentTarget as HTMLInputElement).blur();
		}
		e.stopPropagation();
	}
	const GROUP_LABELS: Record<GroupBy, string> = { section: '§', author: 'author', kind: 'kind', time: 'time' };
	const KIND_FACET: Record<LedgerKind, { label: string; g: string; cls: string }> = {
		note: { label: 'notes', g: '◂', cls: 'note' },
		def: { label: 'def', g: 'def', cls: 'def' },
		request: { label: 'requests', g: '?', cls: 'req' },
		mr: { label: 'merge', g: '⑂', cls: 'mr' },
		kasten: { label: 'kasten', g: '⊞', cls: 'kasten' }
	};
	const authorTotal = $derived(Object.values(counts.by).reduce((a, b) => a + b, 0) || 1);
	const sparkMax = $derived(Math.max(1, ...counts.day));
	const sinceLabel = $derived(
		view.day != null ? (view.day === 0 ? 'today' : `${view.day}d ago`) : view.sinceDays == null ? 'all time' : `${view.sinceDays}d`
	);
	const secLabel = (sec: number) => (sec < 0 ? '⌂' : `§${sections[sec]?.n ?? sec + 1}`);
	const secTitle = (sec: number) => (sec < 0 ? 'whole publication' : `§${sections[sec]?.n ?? sec + 1} · ${sections[sec]?.title ?? ''}`);
</script>

<div class="qrow">
	{#if phone}
		<button class="qrow__spell" title={spell} onclick={onwand}>{spell.slice(0, 30)}…</button>
	{:else}
		<input
			class="spell__input"
			bind:value={text}
			{onkeydown}
			spellcheck="false"
			aria-label="Ledger query"
			title="The ledger's query — a spell. Enter applies; the wand saves it."
		/>
	{/if}
	<button class="spell__wand" onclick={onwand} title="Save this query as a spell">🪄</button>
	<button class="qrow__plus" onclick={onannotate} title="Open the annotate window (n) — a request to your agent, or a definition by hand">+ annotate</button>
	<span class="seg seg--group" title="Group rows by">
		{#each GROUPS as g (g)}
			<button data-group={g} class:on={view.group === g} onclick={() => ongroup(g)}>{GROUP_LABELS[g]}</button>
		{/each}
	</span>
	{#if !phone}
		<span class="seg seg--view" title="rows: a ledger · board: content-sized cards in section columns">
			<button data-view="rows" class:on={view.view === 'rows'} onclick={() => onview('rows')}>≡ rows</button>
			<button data-view="board" class:on={view.view === 'board'} onclick={() => onview('board')}>▦ board</button>
		</span>
	{/if}
	<span class="qrow__n">{shown}{shown !== total ? ` of ${total}` : ''}</span>
</div>
<div class="facets">
	<span class="facet facet--who">
		<span class="facet__lab">who</span>
		{#each authors as a (a.pubkey)}
			{@const n = counts.by[a.pubkey] ?? 0}
			<button
				class="bar"
				class:on={view.by.includes(a.pubkey)}
				class:zero={n === 0}
				data-pk={a.pubkey}
				style="--au: {a.pubkey ? pubkeyToColor(a.pubkey) : 'var(--base5)'}; --au-soft: {a.pubkey ? pubkeyToColor(a.pubkey, 0.14) : 'transparent'}"
				onclick={() => ontoggleby(a.pubkey)}
				title={a.pubkey ? `${a.pubkey} — click to filter` : 'Unsigned local requests'}
			>
				<i style="width: {Math.round((100 * n) / authorTotal)}%"></i>
				<span class="bar__dot"></span>{a.label}<span class="facet__k">{n}</span>
			</button>
		{/each}
	</span>
	<span class="facet facet--where">
		<span class="facet__lab">where</span>
		{#each [-1, ...sections.map((_, i) => i)] as sec (sec)}
			{@const n = counts.sec[sec] ?? 0}
			<button
				class="sec"
				class:on={view.secs.includes(sec)}
				class:zero={n === 0}
				data-sec={sec}
				onclick={() => ontogglesec(sec)}
				onmouseenter={() => onhoversec(sec)}
				onmouseleave={() => onhoversec(null)}
				title={secTitle(sec)}
			>{secLabel(sec)}<span class="facet__k">{n}</span></button>
		{/each}
	</span>
	<span class="facet facet--what">
		<span class="facet__lab">what</span>
		{#each KINDS as k (k)}
			{@const n = counts.kind[k]}
			<button class="kind" class:on={view.kinds.includes(k)} class:zero={n === 0} data-kind={k} onclick={() => ontogglekind(k)}>
				<span class="g {KIND_FACET[k].cls}">{KIND_FACET[k].g}</span>{KIND_FACET[k].label}<span class="facet__k">{n}</span>
			</button>
		{/each}
	</span>
	<span class="facet facet--when">
		<span class="facet__lab">when</span>
		<span class="spark" title="the last {SPARK_DAYS} days, newest on the right — click a day to filter">
			{#each counts.day.slice().reverse() as n, idx (idx)}
				{@const day = SPARK_DAYS - 1 - idx}
				<b
					data-day={day}
					class:on={view.day === day}
					style="height: {Math.max(1, Math.round((14 * n) / sparkMax))}px"
					onclick={() => onpickday(day)}
					onkeydown={(e) => e.key === 'Enter' && onpickday(day)}
					title="{day === 0 ? 'today' : `${day}d ago`} · {n}"
					role="button"
					tabindex="-1"
				></b>
			{/each}
		</span>
		<button class="since" class:on={view.sinceDays != null || view.day != null} onclick={oncyclesince} title="all · 7d · 30d">{sinceLabel}</button>
	</span>
</div>

<style>
	.qrow {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 10px 4px;
		flex-wrap: wrap;
		flex-shrink: 0;
	}
	.spell__input {
		flex: 1 1 160px;
		min-width: 100px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--base1);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: 4px 8px;
	}
	.spell__input:focus { outline: none; border-color: var(--blue); }
	.qrow__spell {
		flex: 1;
		min-width: 0;
		text-align: left;
		overflow: hidden;
		white-space: nowrap;
		text-overflow: ellipsis;
		font: var(--t-2xs) / 1.2 var(--font-mono);
		color: var(--base5);
		min-height: 26px;
		padding: 0 6px;
	}
	.spell__wand { min-height: 26px; padding: 0 7px; font-size: var(--t-sm); }
	.qrow__plus {
		min-height: 26px;
		padding: 0 8px;
		font: var(--t-2xs) / 1 var(--font-mono);
		color: var(--ask);
		border-color: rgba(226, 164, 120, 0.4);
	}
	.qrow__plus:hover { border-color: var(--ask); }
	.qrow__n { font: var(--t-2xs) / 1 var(--font-mono); color: var(--base5); white-space: nowrap; margin-left: auto; }
	.seg {
		display: inline-flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		overflow: hidden;
	}
	.seg button {
		border: 0;
		border-radius: 0;
		border-right: 1px solid var(--panel-border);
		min-height: 24px;
		padding: 1px 9px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}
	.seg button:last-child { border-right: 0; }
	.seg button.on { background: var(--base2); color: var(--fg); }
	.facets {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 10px;
		padding: 2px 10px 6px;
		border-bottom: 1px solid var(--panel-border);
		font: var(--t-2xs) / 1.3 var(--font-mono);
		flex-shrink: 0;
		align-items: center;
	}
	.facet { display: inline-flex; align-items: center; gap: 3px; flex-wrap: wrap; }
	.facet__lab {
		color: var(--base4);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		font-size: var(--t-3xs);
		font-weight: 600;
		margin-right: 2px;
	}
	.facets button {
		white-space: nowrap;
		flex: none;
		min-height: 20px;
		padding: 0 6px;
		font: var(--t-2xs) / 1 var(--font-mono);
		border: 1px solid transparent;
		border-radius: var(--r-md);
		color: var(--base6);
		position: relative;
		overflow: hidden;
		background: transparent;
		cursor: pointer;
		display: inline-flex;
		align-items: center;
	}
	.facets button:hover { border-color: var(--panel-border-strong); color: var(--fg); }
	.facets button.on { border-color: var(--au, var(--blue)); color: var(--fg); background: var(--au-soft, rgba(132, 160, 198, 0.14)); }
	.facets button.zero { color: var(--base4); }
	.facets button i { position: absolute; left: 0; top: 0; bottom: 0; background: var(--au-soft); z-index: -1; }
	.bar { z-index: 0; }
	.bar__dot { width: 6px; height: 6px; border-radius: 50%; background: var(--au); display: inline-block; margin-right: 5px; }
	.facet__k { color: var(--base5); margin-left: 4px; }
	.sec { font-weight: 700; }
	.kind .g { font-weight: 700; margin-right: 3px; }
	.g.note { color: var(--base6); }
	.g.def { color: var(--green); }
	.g.req { color: var(--ask); }
	.g.mr { color: var(--ask); }
	.g.kasten { color: var(--violet); }
	.spark { display: inline-flex; align-items: flex-end; gap: 1px; height: 14px; margin: 0 4px; }
	.spark b {
		display: block;
		width: 5px;
		background: var(--base4);
		border-radius: 1px 1px 0 0;
		min-height: 1px;
		cursor: pointer;
	}
	.spark b:hover,
	.spark b.on { background: var(--blue); }
</style>
