<script lang="ts">
	import type { LazySection, SectionStatus } from '$lib/types';
	import * as api from '$lib/api';
	import {
		pubkeyToHighlightFill,
		pubkeyToHighlightStroke
	} from '$lib/discussions/colors';
	import {
		segmentsFromSpans,
		type Highlight,
		type HighlightSpan
	} from '$lib/discussions/highlights';

	let {
		section,
		truncate = false,
		index = undefined,
		preview = false,
		onclick = undefined,
		onviewjson = undefined,
		highlights = [],
		focusedHighlightId = null
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
	} = $props();

	function styleFor(pubkey: string, focused: boolean): string {
		const fill = pubkeyToHighlightFill(pubkey);
		const stroke = pubkeyToHighlightStroke(pubkey);
		// Inset stripe anchored to the left edge per the plan's recipe.
		// Focused (`?highlight=<id>` or drawer click-to-scroll) adds an
		// outer green ring so the user can find the scrolled-to mark
		// even when it shares a hue with neighbours.
		if (focused) {
			return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke}, 0 0 0 2px var(--state-online);`;
		}
		return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke};`;
	}

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

	const highlightSegments = $derived.by(() => {
		if (!displayContent || preview || highlightSpans.length === 0) return null;
		const segs = segmentsFromSpans(displayContent, highlightSpans, focusedHighlightId);
		return segs.some((s) => s.highlight !== null) ? segs : null;
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
		{#if highlightSegments}
			<pre class="section-content" class:muted={preview}>{#each highlightSegments as seg, i (i)}{#if seg.highlight}<mark class="hl-overlay" data-hl-ids={seg.highlight.id} style={styleFor(seg.highlight.pubkey, seg.highlight.focused)} title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…">{seg.text}</mark>{:else}{seg.text}{/if}{/each}</pre>
		{:else}
			<pre class="section-content" class:muted={preview}>{displayContent}</pre>
		{/if}
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
		font-size: var(--t-sm);
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
		font-size: var(--t-base);
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
		font-size: var(--t-2xs);
		margin-left: 6px;
	}

	.loading-dots {
		color: var(--accent);
		animation: pulse 1s ease-in-out infinite;
	}

	.status-error {
		color: var(--danger);
		font-weight: 700;
	}

	@keyframes pulse {
		0%, 100% { opacity: 0.4; }
		50% { opacity: 1; }
	}

	.section-content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
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
		font-size: var(--t-2xs);
		font-style: italic;
	}

	.section-error {
		color: var(--danger);
		font-size: var(--t-2xs);
	}
	.hl-overlay {
		color: inherit;
		padding: 1px 2px;
		border-radius: 2px;
	}
	/* Drawer's flash animation — applied imperatively when the user
	   clicks a row in the highlights drawer. Brightness/saturation
	   pulse so it works with whatever per-author hue is already
	   painted on the mark. */
	@keyframes hl-flash {
		0%, 100% { filter: brightness(1) saturate(1); }
		30%      { filter: brightness(1.5) saturate(1.6); }
	}
	.hl-overlay.hl-flash {
		animation: hl-flash 1.2s ease-in-out;
	}
</style>
