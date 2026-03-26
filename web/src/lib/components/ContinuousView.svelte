<script lang="ts">
	import { onMount } from 'svelte';
	import type { LazySection } from '$lib/types';

	let {
		sections,
		publication = null,
		onload
	}: {
		sections: LazySection[];
		publication?: { title: string | null; summary: string | null } | null;
		onload?: (index: number) => void;
	} = $props();

	let containerEl: HTMLDivElement | undefined = $state();

	onMount(() => {
		if (!containerEl || !onload) return;

		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const idx = Number((entry.target as HTMLElement).dataset.sectionIndex);
					if (isNaN(idx)) continue;
					const section = sections[idx];
					if (section && section.status === 'pending') {
						onload!(idx);
						// Read-ahead: prefetch next 2
						if (idx + 1 < sections.length) onload!(idx + 1);
						if (idx + 2 < sections.length) onload!(idx + 2);
					}
				}
			},
			{
				root: containerEl,
				rootMargin: '200px 0px 400px 0px'
			}
		);

		// Observe all section containers
		const sectionEls = containerEl.querySelectorAll('[data-section-index]');
		sectionEls.forEach((el) => observer.observe(el));

		return () => observer.disconnect();
	});
</script>

<div class="continuous-view" bind:this={containerEl}>
	{#if publication?.title}
		<h2 class="pub-title">{publication.title}</h2>
		{#if publication.summary}
			<p class="pub-summary">{publication.summary}</p>
		{/if}
		<hr class="pub-divider" />
	{/if}

	{#each sections as section, i (section.addr?.d_tag ?? i)}
		<div class="continuous-section" data-section-index={i}>
			{#if section.title}
				<h3 class="section-title">{section.title}</h3>
			{/if}
			{#if section.status === 'loaded' && section.content}
				<pre class="section-content">{section.content}</pre>
			{:else if section.status === 'loading'}
				<div class="skeleton"></div>
			{:else if section.status === 'error'}
				<p class="section-error">{section.error ?? 'Failed to load'}</p>
			{:else}
				<div class="skeleton pending"></div>
			{/if}
		</div>
		{#if i < sections.length - 1}
			<hr class="section-divider" />
		{/if}
	{/each}
	{#if sections.length === 0}
		<p class="empty">No sections loaded</p>
	{/if}
</div>

<style>
	.continuous-view {
		flex: 1;
		overflow-y: auto;
		padding: 16px;
	}

	.pub-title {
		font-size: 1.1rem;
		font-weight: 700;
		margin: 0 0 8px 0;
	}

	.pub-summary {
		font-size: 0.85rem;
		color: var(--fg-muted);
		font-style: italic;
		margin: 0 0 12px 0;
		line-height: 1.5;
	}

	.pub-divider {
		border: none;
		border-top: 1px solid var(--border);
		margin: 12px 0;
	}

	.continuous-section {
		padding: 8px 0;
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

	.skeleton {
		height: 60px;
		background: var(--border);
		border-radius: 4px;
		animation: pulse 1.5s ease-in-out infinite;
	}

	.skeleton.pending {
		opacity: 0.4;
	}

	@keyframes pulse {
		0%, 100% { opacity: 0.3; }
		50% { opacity: 0.6; }
	}

	.section-error {
		color: #ef4444;
		font-size: 0.8rem;
	}

	.section-divider {
		border: none;
		border-top: 1px solid var(--border);
		margin: 4px 0;
		opacity: 0.5;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: 0.85rem;
	}
</style>
