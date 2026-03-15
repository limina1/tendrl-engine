<script lang="ts">
	import type { Section } from '$lib/types';
	import SectionCard from './SectionCard.svelte';

	let {
		sections,
		currentSection = 0,
		onnavigate
	}: {
		sections: Section[];
		currentSection?: number;
		onnavigate: (index: number) => void;
	} = $props();

	const section = $derived(sections[currentSection]);
	const total = $derived(sections.length);
</script>

<div class="paginated-view">
	<div class="paginated-content">
		{#if section}
			<SectionCard {section} />
		{/if}
	</div>
	<div class="paginated-nav">
		<button onclick={() => onnavigate(currentSection - 1)} disabled={currentSection <= 0}>
			Prev
		</button>
		<span class="page-counter">{currentSection + 1} / {total}</span>
		<button
			onclick={() => onnavigate(currentSection + 1)}
			disabled={currentSection >= total - 1}
		>
			Next
		</button>
	</div>
</div>

<style>
	.paginated-view {
		display: flex;
		flex-direction: column;
		flex: 1;
		min-height: 0;
	}

	.paginated-content {
		flex: 1;
		overflow-y: auto;
	}

	.paginated-nav {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 12px;
		padding: 10px;
		border-top: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.page-counter {
		font-size: 0.8rem;
		color: var(--fg-muted);
		min-width: 60px;
		text-align: center;
	}
</style>
