<script lang="ts">
	import type { LazySection, SectionStatus } from '$lib/types';

	let {
		section,
		truncate = false,
		index = undefined,
		preview = false,
		onclick = undefined,
		onviewjson = undefined
	}: {
		section: LazySection;
		truncate?: boolean;
		index?: number | undefined;
		preview?: boolean;
		onclick?: (() => void) | undefined;
		/** When provided, renders a kebab `⋮` in the top-right that opens
		 *  this section's underlying event in the structured JSON modal. */
		onviewjson?: ((section: LazySection) => void) | undefined;
	} = $props();

	const status: SectionStatus = $derived(section.status ?? 'loaded');

	const displayContent = $derived.by(() => {
		if (status !== 'loaded' || !section.content) return null;
		if (preview) {
			const firstLine = section.content.split('\n')[0] ?? '';
			return firstLine.length > 80 ? firstLine.slice(0, 80) + '...' : firstLine;
		}
		if (truncate && section.content.length > 200) {
			return section.content.slice(0, 200) + '...';
		}
		return section.content;
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="section-card"
	class:clickable={!!onclick}
	class:pending={status === 'pending'}
	onclick={onclick}
	onkeydown={onclick ? (e) => { if (e.key === 'Enter' || e.key === ' ') onclick?.(); } : undefined}
	role={onclick ? 'button' : undefined}
	tabindex={onclick ? 0 : undefined}
>
	<h3 class="section-title">
		{#if index !== undefined}<span class="section-index">{index}.</span>{/if}
		{section.title ?? `Section ${(section.position ?? 0) + 1}`}
		{#if status === 'loading'}
			<span class="status-indicator loading-dots">...</span>
		{:else if status === 'error'}
			<span class="status-indicator status-error">!</span>
		{/if}
		{#if onviewjson}
			<button
				class="section-kebab"
				onclick={(e) => {
					e.stopPropagation();
					onviewjson?.(section);
				}}
				title="Open this section's raw event in the JSON viewer"
			>⋮</button>
		{/if}
	</h3>
	{#if status === 'loaded' && displayContent}
		<pre class="section-content" class:muted={preview}>{displayContent}</pre>
	{:else if status === 'loading'}
		<div class="skeleton"></div>
	{:else if status === 'error'}
		<p class="section-error">{section.error ?? 'Failed to load'}</p>
	{:else if status === 'pending'}
		<p class="section-pending">Not loaded</p>
	{/if}
</div>

<style>
	.section-card {
		padding: 12px 16px;
		border-bottom: 1px solid var(--border);
	}

	.section-card.clickable {
		cursor: pointer;
	}

	.section-card.clickable:hover {
		background: var(--bg-surface);
	}

	.section-card.pending {
		opacity: 0.6;
	}

	.section-title {
		font-size: 0.95rem;
		font-weight: 600;
		margin-bottom: 6px;
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.section-kebab {
		margin-left: auto;
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: 1rem;
		line-height: 1;
		padding: 0 6px;
		border-radius: var(--radius);
		cursor: pointer;
	}
	.section-kebab:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours);
	}

	.section-index {
		color: var(--fg-muted);
		margin-right: 4px;
	}

	.status-indicator {
		font-size: 0.75rem;
		margin-left: 6px;
	}

	.loading-dots {
		color: var(--accent);
		animation: pulse 1s ease-in-out infinite;
	}

	.status-error {
		color: #ef4444;
		font-weight: 700;
	}

	@keyframes pulse {
		0%, 100% { opacity: 0.4; }
		50% { opacity: 1; }
	}

	.section-content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: 0.85rem;
		line-height: 1.5;
		color: var(--fg);
		margin: 0;
	}

	.section-content.muted {
		color: var(--fg-muted);
	}

	.skeleton {
		height: 40px;
		background: var(--border);
		border-radius: 4px;
		animation: pulse 1.5s ease-in-out infinite;
	}

	.section-pending {
		color: var(--fg-muted);
		font-size: 0.8rem;
		font-style: italic;
	}

	.section-error {
		color: #ef4444;
		font-size: 0.8rem;
	}
</style>
