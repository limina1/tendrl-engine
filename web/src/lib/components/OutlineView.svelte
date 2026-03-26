<script lang="ts">
	import type { LazySection } from '$lib/types';
	import SectionCard from './SectionCard.svelte';

	let {
		sections,
		onselect = undefined,
		onload = undefined
	}: {
		sections: LazySection[];
		onselect?: ((index: number) => void) | undefined;
		onload?: ((index: number) => void) | undefined;
	} = $props();

	// Auto-load all sections for the outline preview
	$effect(() => {
		if (!onload) return;
		for (let i = 0; i < sections.length; i++) {
			if (sections[i].status === 'pending') {
				onload(i);
			}
		}
	});
</script>

<div class="outline-view">
	{#each sections as section, i (section.addr?.d_tag ?? i)}
		<SectionCard
			{section}
			preview
			index={i + 1}
			onclick={onselect ? () => onselect?.(i) : undefined}
		/>
	{/each}
	{#if sections.length === 0}
		<p class="empty">No sections loaded</p>
	{/if}
</div>

<style>
	.outline-view {
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 4px 0;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: 0.85rem;
	}
</style>
