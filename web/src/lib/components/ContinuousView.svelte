<script lang="ts">
	import type { Section } from '$lib/types';
	import SectionCard from './SectionCard.svelte';

	let { sections }: { sections: Section[] } = $props();
</script>

<div class="continuous-view">
	{#each sections as section, i (section.addr?.d_tag ?? i)}
		<SectionCard {section} />
	{/each}
	{#if sections.length === 0}
		<p class="empty">No sections loaded</p>
	{/if}
</div>

<style>
	.continuous-view {
		flex: 1;
		overflow-y: auto;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: 0.85rem;
	}
</style>
