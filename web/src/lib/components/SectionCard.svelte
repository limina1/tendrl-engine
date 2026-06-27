<script lang="ts">
	import type { LazySection, SectionStatus } from '$lib/types';
	import * as api from '$lib/api';
	import { type Highlight, type HighlightSpan } from '$lib/discussions/highlights';
	import type { ResolvedRef } from '$lib/nostr/nostrdown';
	import RichContent from './RichContent.svelte';

	let {
		section,
		truncate = false,
		index = undefined,
		preview = false,
		onclick = undefined,
		onviewjson = undefined,
		highlights = [],
		focusedHighlightId = null,
		publicationAtag = undefined
	}: {
		section: LazySection;
		truncate?: boolean;
		index?: number | undefined;
		preview?: boolean;
		onclick?: (() => void) | undefined;
		/** When provided, renders a kebab `⋮` in the top-right that opens
		 *  this section's underlying event in the structured JSON modal. */
		onviewjson?: ((section: LazySection) => void) | undefined;
		/** All NIP-84 highlights to overlay on this section's content.
		 *  Each highlight whose `content` matches a substring lands as
		 *  its own <mark> in the author's hue. */
		highlights?: Highlight[];
		/** Id of the highlight to emphasize (from ?highlight= marker). */
		focusedHighlightId?: string | null;
		/** Containing publication coordinate ("30040:pubkey:dtag") — context for
		 *  resolving nostrdown `{{ref:…}}` sibling references. */
		publicationAtag?: string | undefined;
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

	// Multi-highlight overlay: spans are resolved engine-side (POST
	// /highlights/resolve) against the exact content this card renders, async
	// into state. In preview mode (single-line truncated cards) we skip overlays
	// — the preview text is too short and shifted to bear them usefully.
	let highlightSpans = $state<HighlightSpan[]>([]);
	$effect(() => {
		const text = displayContent;
		const hls = highlights;
		if (!text || preview || hls.length === 0) {
			highlightSpans = [];
			return;
		}
		let cancelled = false;
		api.resolveHighlights([{ key: 'section', content: text, highlights: hls }])
			.then((m) => {
				if (!cancelled) highlightSpans = m['section'] ?? [];
			})
			.catch(() => {
				if (!cancelled) highlightSpans = [];
			});
		return () => {
			cancelled = true;
		};
	});

	// Nostrdown `{{ }}` references, resolved engine-side (POST
	// /nostrdown/resolve) against the same content this card renders. Skipped in
	// preview/truncate (shifted/short text can't bear them) and when the content
	// carries no `{{` token (avoids a needless round trip). `RichContent` merges
	// these with the highlight spans onto one segmentation.
	let nostrdownRefs = $state<ResolvedRef[]>([]);
	$effect(() => {
		const text = displayContent;
		if (!text || preview || !text.includes('{{')) {
			nostrdownRefs = [];
			return;
		}
		let cancelled = false;
		api.resolveNostrdown([
			{ key: 'section', content: text, publication: publicationAtag, author: section.addr?.pubkey }
		])
			.then((m) => {
				if (!cancelled) nostrdownRefs = m['section'] ?? [];
			})
			.catch(() => {
				if (!cancelled) nostrdownRefs = [];
			});
		return () => {
			cancelled = true;
		};
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
	data-section-addr={section.addr ? `${section.addr.kind}:${section.addr.pubkey}:${section.addr.d_tag}` : undefined}
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
		<RichContent
			content={displayContent}
			spans={highlightSpans}
			refs={nostrdownRefs}
			{focusedHighlightId}
			muted={preview}
		/>
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

	/* Section body (content + its highlight/nostrdown overlays) now renders via
	   RichContent, which owns the `.section-content` <pre> and overlay styles. */

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
