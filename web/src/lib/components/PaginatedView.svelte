<script lang="ts">
	import type { LazySection } from '$lib/types';
	import SectionCard from './SectionCard.svelte';

	let {
		sections,
		currentSection = 0,
		onnavigate,
		onload
	}: {
		sections: LazySection[];
		currentSection?: number;
		onnavigate: (index: number) => void;
		onload?: (index: number) => void;
	} = $props();

	const section = $derived(sections[currentSection]);
	const total = $derived(sections.length);

	let contentEl: HTMLDivElement | undefined = $state();

	// Load current section + prefetch adjacent
	$effect(() => {
		const idx = currentSection;
		onload?.(idx);
		if (idx > 0) onload?.(idx - 1);
		if (idx < total - 1) onload?.(idx + 1);
	});

	// Scroll to top on page change
	$effect(() => {
		currentSection;
		contentEl?.scrollTo(0, 0);
	});

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'ArrowLeft' && currentSection > 0) {
			e.preventDefault();
			onnavigate(currentSection - 1);
		} else if (e.key === 'ArrowRight' && currentSection < total - 1) {
			e.preventDefault();
			onnavigate(currentSection + 1);
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="paginated-view">
	{#if section?.title}
		<div class="paginated-title">{section.title}</div>
	{/if}
	<div class="paginated-content" bind:this={contentEl}>
		{#if section}
			<SectionCard {section} />
		{/if}
	</div>
	<div class="paginated-nav">
		<button onclick={() => onnavigate(currentSection - 1)} disabled={currentSection <= 0}>
			Prev
		</button>
		<span class="page-counter">
			{currentSection + 1} / {total}
			<span class="section-label">Section {currentSection + 1} of {total}</span>
		</span>
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

	.paginated-title {
		padding: 10px 16px;
		font-size: 0.95rem;
		font-weight: 700;
		border-bottom: 1px solid var(--border);
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

	.section-label {
		margin-left: 8px;
		font-size: 0.75rem;
		color: var(--fg-muted);
		opacity: 0.7;
	}
</style>
