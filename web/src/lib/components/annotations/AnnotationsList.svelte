<script lang="ts">
	// Track C placeholder for the mobile list (Track D delivers the real
	// component with exactly this prop signature; its file replaces this one
	// at merge). Renders enough to prove the mount point.
	import type { AnnotationsViewState, LayoutMode, Section, TableCard } from './table-model';
	import type { authorBars } from './table-model';

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
		/** Already filtered by by/since. */
		cards: TableCard[];
		/** Reading order. */
		sections: Section[];
		bars: ReturnType<typeof authorBars>;
		view: AnnotationsViewState;
		spell: string;
		/** section | author | timeline only. */
		onMode: (m: LayoutMode) => void;
		onToggleBy: (pk: string) => void;
		onWand: () => void;
		/** Jump the reader to the card's anchor and switch to it. */
		onAnchor: (card: TableCard) => void;
		/** Open the note / request. */
		onOpen: (card: TableCard) => void;
	} = $props();
</script>

<div class="alist alist--stub">
	<div class="vseg">
		{#each ['section', 'author', 'timeline'] as const as m (m)}
			<button data-mode={m} class:on={view.mode === m} onclick={() => onMode(m)}>{m}</button>
		{/each}
	</div>
	<div class="mspell">{spell} <button onclick={onWand}>🪄</button></div>
	<div class="mhist">
		{#each bars as b (b.pubkey)}
			<button class="bar" class:on={view.by.includes(b.pubkey)} onclick={() => onToggleBy(b.pubkey)}>{b.label} {b.count}</button>
		{/each}
	</div>
	<div class="grp">mobile list (Track D) · {cards.length} cards · {sections.length} sections</div>
	{#each cards as c (c.id)}
		<div class="acard m">
			<button class="acard__anchor" onclick={() => onAnchor(c)}>→ §</button>
			<button class="acard__open" onclick={() => onOpen(c)}>{c.title}</button>
		</div>
	{/each}
</div>

<style>
	.alist { padding: var(--s-2); font-family: var(--font-mono); font-size: var(--t-xs); color: var(--base5); }
	.mspell { color: var(--base6); margin-bottom: var(--s-1); word-break: break-all; }
	.grp { font-style: italic; margin-bottom: var(--s-1); }
	.acard { display: block; width: 100%; text-align: left; margin-bottom: 4px; }
</style>
