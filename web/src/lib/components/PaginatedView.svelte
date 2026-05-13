<script lang="ts">
	import type { LazySection } from '$lib/types';
	import SectionCard from './SectionCard.svelte';

	let {
		sections,
		currentSection = 0,
		onnavigate,
		onload,
		onsectionjson
	}: {
		sections: LazySection[];
		currentSection?: number;
		onnavigate: (index: number) => void;
		onload?: (index: number) => void;
		/** Open the section's underlying event in the structured JSON modal.
		 *  Surfaces on the right margin of the pager. */
		onsectionjson?: (index: number) => void;
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

	// Keydown is handled by ReaderBuffer's nav handler (registered via the
	// global buffer-store dispatcher). PaginatedView no longer attaches its
	// own listener — global j/k/arrow already drives onnavigate from there.
</script>

<div class="paginated-view">
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
		<span class="pager-spacer"></span>
		{#if onsectionjson && section}
			<button
				class="pager-json"
				onclick={() => onsectionjson?.(currentSection)}
				title="Open this section's raw event in the JSON viewer"
			>§ json</button>
		{/if}
	</div>
	{#if section?.title}
		<div class="paginated-title">{section.title}</div>
	{/if}
	<div class="paginated-content" bind:this={contentEl}>
		{#if section}
			<SectionCard {section} />
		{/if}
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
		gap: 12px;
		padding: 10px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		flex-shrink: 0;
	}
	.pager-spacer { flex: 1; }
	.pager-json {
		background: none;
		border: 1px solid var(--border);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: 0.72rem;
		padding: 2px 8px;
		border-radius: var(--radius);
		cursor: pointer;
	}
	.pager-json:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-color: var(--id-yours);
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
