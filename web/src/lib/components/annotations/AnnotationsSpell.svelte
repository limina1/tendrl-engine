<script lang="ts">
	// The table's query row: the spell input (the search DSL the engine
	// mirrors — editable, Enter applies), the wand (save it as a kind-777
	// spell), the by / § / since chips, the layout picker, and the author
	// histogram bars that filter and rewrite the spell. DOM per
	// docs/annotations-table-contract.org ("Table buffer").
	import type { LayoutMode } from './table-model';
	import { pubkeyToColor } from '$lib/discussions/colors';

	let {
		spell,
		mode,
		modes,
		by,
		bars,
		sinceDays,
		secWith,
		secTotal,
		selectedCount,
		totalCount,
		onapply,
		onwand,
		ontoggleby,
		oncyclesince,
		onmode
	}: {
		spell: string;
		mode: LayoutMode;
		modes: LayoutMode[];
		by: string[];
		bars: { pubkey: string; label: string; count: number }[];
		sinceDays: number | null;
		secWith: number;
		secTotal: number;
		/** Cards under the current by-filter / all cards. */
		selectedCount: number;
		totalCount: number;
		/** Apply a hand-edited spell; false = refused (the input reverts). */
		onapply: (text: string) => boolean;
		onwand: () => void;
		ontoggleby: (pubkey: string) => void;
		oncyclesince: () => void;
		onmode: (m: LayoutMode) => void;
	} = $props();

	// The typed text; re-seeded whenever the derived spell changes (a bar
	// click, a chip) so the input always shows the live query.
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

	const modeLabel = (m: LayoutMode) => (m === 'section' ? 'by section' : m === 'author' ? 'by author' : m);
	const barTotal = $derived(bars.reduce((n, b) => n + b.count, 0));
	const sinceLabel = $derived(sinceDays == null ? 'all time' : `${sinceDays}d`);
</script>

<div class="spell">
	<input
		class="spell__input"
		bind:value={text}
		{onkeydown}
		spellcheck="false"
		aria-label="Table query"
		title="The table's query — a spell. Enter applies; the wand saves it."
	/>
	<button class="spell__wand" onclick={onwand} title="Save this query as a spell">🪄</button>
	<span class="chip chip--by" class:on={by.length > 0} title="Authors selected · cards shown">by {by.length || bars.length} of {bars.length} · {selectedCount} of {totalCount}</span>
	<span class="chip chip--sec" title="Sections with at least one card">§ {secWith} of {secTotal}</span>
	<button class="chip chip--since" class:on={sinceDays != null} onclick={oncyclesince} title="Time window: all · 7d · 30d">{sinceLabel}</button>
	<span class="lay">
		{#each modes as m (m)}
			<button data-mode={m} class:on={mode === m} onclick={() => onmode(m)}>{modeLabel(m)}</button>
		{/each}
	</span>
</div>
{#if bars.length}
	<div class="hist">
		{#each bars as b (b.pubkey)}
			<button
				class="bar"
				class:on={by.includes(b.pubkey)}
				data-pk={b.pubkey}
				style:--au={b.pubkey ? pubkeyToColor(b.pubkey) : 'var(--base5)'}
				style:--au-soft={b.pubkey ? pubkeyToColor(b.pubkey, 0.14) : 'transparent'}
				onclick={() => ontoggleby(b.pubkey)}
				title={b.pubkey ? `${b.pubkey} — click to filter` : 'Unsigned local requests'}
			>
				<i style:width="{barTotal ? Math.round((100 * b.count) / barTotal) : 0}%"></i>
				<span class="bar__dot"></span>
				<span class="bar__lab">{b.label}</span>
				<span class="bar__n">{b.count}</span>
			</button>
		{/each}
	</div>
{/if}

<style>
	.spell {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 10px;
		border-bottom: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		flex-wrap: wrap;
	}
	.spell__input {
		flex: 1;
		min-width: 160px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--base1);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 4px 8px;
	}
	.spell__input:focus { outline: none; border-color: var(--blue); }
	.spell__wand { min-height: 28px; padding: 2px 8px; font-size: var(--t-sm); }
	.chip {
		font: 600 var(--t-2xs) / 1.4 var(--font-mono);
		color: var(--base5);
		padding: 0 7px;
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		min-height: 22px;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: transparent;
		white-space: nowrap;
	}
	.chip.on { color: var(--fg); border-color: var(--base5); }
	button.chip { cursor: pointer; }
	.lay {
		display: inline-flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		overflow: hidden;
		margin-left: auto;
	}
	.lay button {
		border: 0;
		border-radius: 0;
		border-right: 1px solid var(--panel-border);
		min-height: 26px;
		padding: 2px 9px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}
	.lay button:last-child { border-right: 0; }
	.lay button.on { background: var(--blue); color: var(--bg); }
	.hist {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
		padding: 6px 10px;
		border-bottom: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}
	.bar {
		position: relative;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 1px 8px;
		min-height: 24px;
		color: var(--base6);
		cursor: pointer;
		overflow: hidden;
		font: inherit;
		background: transparent;
	}
	.bar i { position: absolute; left: 0; top: 0; bottom: 0; background: var(--au-soft); z-index: 0; }
	.bar > span { position: relative; z-index: 1; }
	.bar__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); flex: none; }
	.bar.on { color: var(--fg); border-color: var(--au); }
	.bar__n { color: var(--base5); }
</style>
