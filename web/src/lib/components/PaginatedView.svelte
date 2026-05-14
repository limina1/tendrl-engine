<script lang="ts">
	import type { LazySection } from '$lib/types';
	import { threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import type { Highlight } from '$lib/discussions/highlights';
	import SectionCard from './SectionCard.svelte';
	import CommentThread from './CommentThread.svelte';

	let {
		sections,
		currentSection = 0,
		onnavigate,
		onload,
		onsectionjson,
		highlightsFor = null,
		focusedHighlightId = null,
		threadsFor = null,
		focusedCommentId = null
	}: {
		sections: LazySection[];
		currentSection?: number;
		onnavigate: (index: number) => void;
		onload?: (index: number) => void;
		/** Open the section's underlying event in the structured JSON modal.
		 *  Surfaces on the right margin of the pager. */
		onsectionjson?: (index: number) => void;
		/** Lookup: section addr → all highlights to overlay on its content. */
		highlightsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => Highlight[]) | null;
		/** Id of the highlight to emphasize (from ?highlight= marker). */
		focusedHighlightId?: string | null;
		/** Lookup: given a section addr, return the NIP-22 thread tree to
		 *  render beneath it. Pass null to suppress inline threads. */
		threadsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => ThreadNode[]) | null;
		focusedCommentId?: string | null;
	} = $props();

	const section = $derived(sections[currentSection]);
	const total = $derived(sections.length);
	const threads = $derived(threadsFor && section ? threadsFor(section.addr) : []);
	const highlights = $derived(highlightsFor && section ? highlightsFor(section.addr) : []);

	// Per-section thread toggle. Closed by default — same posture as the
	// highlights drawer. Resets each time the user pages, with one
	// exception: when the focused comment (`?focus_comment=<id>`) lives
	// in this section's thread, snap open so the targeted reply is
	// actually visible.
	let threadsOpen = $state(false);
	$effect(() => {
		currentSection;
		threadsOpen = threadContainsId(threads, focusedCommentId);
	});

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
			<SectionCard {section} {highlights} {focusedHighlightId} />
			{#if threads.length > 0}
				<div class="paginated-threads">
					<button
						class="paginated-threads-head"
						onclick={() => (threadsOpen = !threadsOpen)}
						aria-expanded={threadsOpen}
					>
						<span class="ptr">{threadsOpen ? '▾' : '▸'}</span>
						Comments on this section ({threads.length})
					</button>
					{#if threadsOpen}
						<CommentThread nodes={threads} focusedEventId={focusedCommentId} />
					{/if}
				</div>
			{/if}
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

	.paginated-threads {
		padding: 12px 16px;
		border-top: 1px solid var(--border);
		background: var(--bg);
	}
	.paginated-threads-head {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin-bottom: 6px;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: transparent;
		border: none;
		padding: 0;
		cursor: pointer;
	}
	.paginated-threads-head:hover { color: var(--fg); }
	.paginated-threads-head .ptr {
		min-width: 1ch;
		display: inline-block;
	}
</style>
