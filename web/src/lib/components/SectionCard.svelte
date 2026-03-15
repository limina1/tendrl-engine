<script lang="ts">
	import type { Section } from '$lib/types';

	let { section, truncate = false }: { section: Section; truncate?: boolean } = $props();

	const displayContent = $derived(
		truncate && section.content && section.content.length > 200
			? section.content.slice(0, 200) + '...'
			: section.content
	);
</script>

<div class="section-card">
	{#if section.title}
		<h3 class="section-title">{section.title}</h3>
	{/if}
	{#if displayContent}
		<pre class="section-content">{displayContent}</pre>
	{:else if !section.loaded}
		<p class="section-loading">Loading...</p>
	{/if}
</div>

<style>
	.section-card {
		padding: 12px 16px;
		border-bottom: 1px solid var(--border);
	}

	.section-title {
		font-size: 0.95rem;
		font-weight: 600;
		margin-bottom: 6px;
	}

	.section-content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: 0.85rem;
		line-height: 1.5;
		color: var(--fg);
		margin: 0;
	}

	.section-loading {
		color: var(--fg-muted);
		font-size: 0.85rem;
		font-style: italic;
	}
</style>
