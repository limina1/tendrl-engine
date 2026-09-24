<script lang="ts">
	// The annotations table on a phone: no canvas, one scroll of grouped
	// 44 px rows. Mounted by `AnnotationsBuffer` behind the mobile flag with
	// the same inputs the desktop table gets (cards already filtered by
	// `by` / since, the TOC, the author bars, the view state, the collapsed
	// spell text) and callbacks for every action — the list owns no state
	// of its own beyond what the DOM needs. Contract:
	// docs/annotations-table-contract.org ("Mobile" under the DOM contract).
	//
	// No Svelte lifecycle hooks (project_wm_buffer_render_constraints):
	// props + derived only.
	import type { TableCard, Section, AnnotationsViewState, LayoutMode } from './table-model';
	import { groupCards, listMode, anchorLabel, relTime, authorFallback } from './list-model';
	import { pubkeyToColor } from '$lib/discussions/colors';

	type Bar = { pubkey: string; label: string; count: number };

	let {
		cards,
		sections,
		bars,
		view,
		spell,
		onMode,
		onToggleBy,
		onWand,
		onAnchor,
		onOpen
	}: {
		cards: TableCard[];
		sections: Section[];
		bars: Bar[];
		view: AnnotationsViewState;
		spell: string;
		onMode: (m: LayoutMode) => void;
		onToggleBy: (pk: string) => void;
		onWand: () => void;
		onAnchor: (card: TableCard) => void;
		onOpen: (card: TableCard) => void;
	} = $props();

	const MODES: { mode: LayoutMode; label: string }[] = [
		{ mode: 'section', label: 'section' },
		{ mode: 'author', label: 'by author' },
		{ mode: 'timeline', label: 'timeline' }
	];

	const mode = $derived(listMode(view.mode));
	const groups = $derived(groupCards(cards, sections, bars, mode));
	const total = $derived(bars.reduce((n, b) => n + b.count, 0));
	const selected = $derived(new Set(view.by));

	const labelFor = $derived.by(() => {
		const m = new Map<string, string>();
		for (const b of bars) m.set(b.pubkey, b.label);
		return (pk: string) => m.get(pk) ?? authorFallback(pk);
	});

	function au(pk: string): string {
		// `''` (unsigned local request) → hue 0; the same recipe as the
		// highlight overlays and the reader's cards, so an author reads the
		// same colour everywhere.
		return `--au: ${pubkeyToColor(pk)}; --au-soft: ${pubkeyToColor(pk, 0.16)}`;
	}
</script>

<div class="alist-root">
	<div class="vseg" role="tablist" aria-label="Group by">
		{#each MODES as m (m.mode)}
			<button
				type="button"
				role="tab"
				data-mode={m.mode}
				class:on={mode === m.mode}
				aria-selected={mode === m.mode}
				onclick={() => onMode(m.mode)}
			>{m.label}</button>
		{/each}
	</div>

	<div class="mspell">
		<span class="mspell__q" title={spell}>{spell}</span>
		<button type="button" class="mspell__wand" onclick={onWand} aria-label="Save this query as a spell" title="Save as a spell">🪄</button>
	</div>

	{#if bars.length}
		<div class="mhist" role="group" aria-label="Filter by author">
			{#each bars as b (b.pubkey)}
				<button
					type="button"
					class="bar"
					class:on={selected.has(b.pubkey)}
					data-pk={b.pubkey}
					style={au(b.pubkey)}
					aria-pressed={selected.has(b.pubkey)}
					onclick={() => onToggleBy(b.pubkey)}
				>
					<i style="width: {total ? Math.round((100 * b.count) / total) : 0}%"></i>
					<span class="bar__dot"></span>
					<span class="bar__label">{b.label}</span>
					<span class="bar__n">{b.count}</span>
				</button>
			{/each}
		</div>
	{/if}

	<div class="alist" role="list">
		{#if !cards.length}
			<div class="alist__empty">no annotations loaded — open the reader's Tools → annotations</div>
		{:else}
			{#each groups as g (g.key)}
				<div class="grp" data-group={g.key}>{g.label}</div>
				{#each g.cards as card (card.id)}
					<div class="acard m" class:req={card.kind === 'request'} data-id={card.id} style={au(card.author)} role="listitem">
						<button
							type="button"
							class="acard__anchor"
							onclick={() => onAnchor(card)}
							aria-label="Go to {anchorLabel(card, sections)} in the reader"
							title="Scroll the reader to this section"
						>→ {anchorLabel(card, sections)}</button>
						<button type="button" class="acard__body" onclick={() => onOpen(card)}>
							<span class="acard__t">
								{#if card.kind === 'request'}
									<span class="badge" class:answered={card.status === 'answered'}>{card.status === 'answered' ? '✓' : '?'} request</span>
								{:else if card.role === 'definition'}
									<span class="badge def">def</span>
								{/if}
								{card.title}
							</span>
							<span class="acard__by"><span class="acard__dot"></span>{labelFor(card.author)} · {relTime(card.createdAt)}</span>
						</button>
					</div>
				{/each}
			{/each}
		{/if}
	</div>
</div>

<style>
	.alist-root {
		display: flex;
		flex-direction: column;
		min-height: 0;
		height: 100%;
		background: var(--panel-bg);
		color: var(--fg);
	}

	/* ---- mode segments ---- */
	.vseg {
		display: flex;
		flex: none;
		border-bottom: 1px solid var(--panel-border);
	}
	.vseg button {
		flex: 1;
		min-height: 44px;
		border: 0;
		border-right: 1px solid var(--panel-border);
		border-radius: 0;
		background: transparent;
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		letter-spacing: 0.04em;
		cursor: pointer;
	}
	.vseg button:last-child { border-right: 0; }
	.vseg button.on {
		color: var(--blue);
		box-shadow: inset 0 -2px 0 var(--blue);
	}

	/* ---- collapsed spell ---- */
	.mspell {
		display: flex;
		flex: none;
		align-items: center;
		gap: var(--s-2);
		padding: 0 var(--s-1) 0 var(--s-3);
		min-height: 44px;
		border-bottom: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--base6);
	}
	.mspell__q {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mspell__wand {
		flex: none;
		min-width: 44px;
		min-height: 44px;
		border: 0;
		background: transparent;
		font-size: var(--t-md);
		cursor: pointer;
	}

	/* ---- author histogram ---- */
	.mhist {
		display: flex;
		flex: none;
		gap: var(--s-1);
		padding: var(--s-1) var(--s-3);
		overflow-x: auto;
		border-bottom: 1px solid var(--panel-border);
		scrollbar-width: none;
	}
	.mhist::-webkit-scrollbar { display: none; }
	.bar {
		position: relative;
		flex: none;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		min-height: 44px;
		padding: 0 10px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		background: transparent;
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		overflow: hidden;
		cursor: pointer;
	}
	.bar i {
		position: absolute;
		left: 0; top: 0; bottom: 0;
		background: var(--au-soft);
		z-index: 0;
	}
	.bar > span { position: relative; z-index: 1; }
	.bar.on { color: var(--fg); border-color: var(--au); }
	.bar__dot, .acard__dot {
		flex: none;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au);
		display: inline-block;
	}
	.bar__n { color: var(--base5); }

	/* ---- the list ---- */
	.alist {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		-webkit-overflow-scrolling: touch;
		padding: var(--s-2) var(--s-3) calc(var(--s-6) + env(safe-area-inset-bottom, 0px));
	}
	.alist__empty {
		padding: var(--s-4) var(--s-1);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
	}
	.grp {
		margin: var(--s-3) 0 var(--s-1);
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.grp:first-child { margin-top: var(--s-1); }

	.acard.m {
		display: flex;
		align-items: stretch;
		margin: 6px 0;
		background: var(--panel-bg-soft);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--yellow));
		border-radius: var(--r-lg);
		overflow: hidden;
		scroll-margin-top: var(--s-2);
		scroll-margin-bottom: calc(var(--s-6) + env(safe-area-inset-bottom, 0px));
	}
	.acard.m button {
		border: 0;
		background: transparent;
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
		min-height: 44px;
	}
	.acard.m button:active { background: var(--base2); }
	.acard.m .acard__anchor {
		flex: none;
		min-width: 48px;
		padding: 0 var(--s-2);
		border-right: 1px solid var(--panel-border);
		color: var(--au);
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.06em;
		white-space: nowrap;
	}
	.acard__body {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		justify-content: center;
		gap: 2px;
		padding: 6px var(--s-2);
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.3;
	}
	.acard__t {
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
		font-weight: 600;
	}
	.acard__by {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--base5);
	}
	.badge {
		display: inline-block;
		vertical-align: 1px;
		margin-right: 4px;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--cyan);
		border: 1px solid color-mix(in srgb, var(--cyan) 50%, transparent);
		border-radius: var(--r-md);
		padding: 1px 6px;
		white-space: nowrap;
	}
	.badge.answered {
		color: var(--blue);
		border-color: color-mix(in srgb, var(--blue) 50%, transparent);
	}
	.badge.def {
		color: var(--green);
		border-color: color-mix(in srgb, var(--green) 50%, transparent);
	}
</style>
