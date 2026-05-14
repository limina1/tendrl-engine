<script lang="ts">
	import { onMount } from 'svelte';
	import type { LazySection } from '$lib/types';

	import CommentThread from './CommentThread.svelte';
	import { threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import {
		pubkeyToHighlightFill,
		pubkeyToHighlightStroke
	} from '$lib/discussions/colors';
	import {
		computeHighlightSegments,
		type Highlight
	} from '$lib/discussions/highlights';

	let {
		sections,
		publication = null,
		onload,
		onviewjson,
		highlightsFor = null,
		focusedHighlightId = null,
		threadsFor = null,
		focusedCommentId = null
	}: {
		sections: LazySection[];
		publication?: { title: string | null; summary: string | null } | null;
		onload?: (index: number) => void;
		/** Kebab affordance per section — opens the section's underlying
		 *  event in the structured JSON modal. */
		onviewjson?: (section: LazySection) => void;
		/** Lookup: section addr → highlights to overlay. */
		highlightsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => Highlight[]) | null;
		focusedHighlightId?: string | null;
		/** Lookup: section addr → thread tree. Pass null to suppress. */
		threadsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => ThreadNode[]) | null;
		focusedCommentId?: string | null;
	} = $props();

	function styleFor(pubkey: string, focused: boolean): string {
		const fill = pubkeyToHighlightFill(pubkey);
		const stroke = pubkeyToHighlightStroke(pubkey);
		if (focused) {
			return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke}, 0 0 0 2px var(--state-online);`;
		}
		return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke};`;
	}

	// Per-section thread toggles, keyed by addr string. Each section's
	// thread block can be collapsed independently in the continuous
	// view since they're all on screen at the same time. Closed by
	// default — auto-open below if a section contains the focused
	// comment from a `?focus_comment=<id>` marker.
	let threadOpenByAddr = $state<Record<string, boolean>>({});
	function addrKey(addr: { kind: number; pubkey: string; d_tag: string }): string {
		return `${addr.kind}:${addr.pubkey}:${addr.d_tag}`;
	}
	function isThreadOpen(addr: { kind: number; pubkey: string; d_tag: string }): boolean {
		return threadOpenByAddr[addrKey(addr)] ?? false;
	}
	function toggleThread(addr: { kind: number; pubkey: string; d_tag: string }) {
		const k = addrKey(addr);
		threadOpenByAddr[k] = !(threadOpenByAddr[k] ?? false);
	}

	$effect(() => {
		if (!focusedCommentId || !threadsFor) return;
		for (const s of sections) {
			if (!s.addr) continue;
			const t = threadsFor(s.addr);
			if (threadContainsId(t, focusedCommentId)) {
				threadOpenByAddr[addrKey(s.addr)] = true;
			}
		}
	});

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

	{#each sections as section, i (`${i}:${section.addr?.pubkey ?? ''}:${section.addr?.d_tag ?? ''}`)}
		<div
			class="continuous-section"
			data-section-index={i}
			data-section-addr={section.addr ? `${section.addr.kind}:${section.addr.pubkey}:${section.addr.d_tag}` : undefined}
		>
			{#if section.title || onviewjson}
				<h3 class="section-title">
					{section.title ?? ''}
					{#if onviewjson}
						<button
							class="section-kebab"
							onclick={() => onviewjson?.(section)}
							title="Open this section's raw event in the JSON viewer"
						>⋮</button>
					{/if}
				</h3>
			{/if}
			{#if section.status === 'loaded' && section.content}
				{@const hls = highlightsFor && section.addr ? highlightsFor(section.addr) : []}
				{@const segs = hls.length > 0
					? computeHighlightSegments(section.content, hls, focusedHighlightId)
					: null}
				{#if segs && segs.some((s) => s.highlight !== null)}
					<pre class="section-content">{#each segs as seg, si (si)}{#if seg.highlight}<mark class="hl-overlay" data-hl-ids={seg.highlight.id} style={styleFor(seg.highlight.pubkey, seg.highlight.focused)} title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…">{seg.text}</mark>{:else}{seg.text}{/if}{/each}</pre>
				{:else}
					<pre class="section-content">{section.content}</pre>
				{/if}
			{:else if section.status === 'loading'}
				<div class="skeleton"></div>
			{:else if section.status === 'error'}
				<p class="section-error">{section.error ?? 'Failed to load'}</p>
			{:else}
				<div class="skeleton pending"></div>
			{/if}
			{#if threadsFor && section.addr}
				{@const t = threadsFor(section.addr)}
				{#if t.length > 0}
					<div class="cv-threads">
						<button
							class="cv-threads-head"
							onclick={() => toggleThread(section.addr)}
							aria-expanded={isThreadOpen(section.addr)}
						>
							<span class="ptr">{isThreadOpen(section.addr) ? '▾' : '▸'}</span>
							Comments ({t.length})
						</button>
						{#if isThreadOpen(section.addr)}
							<CommentThread nodes={t} focusedEventId={focusedCommentId} />
						{/if}
					</div>
				{/if}
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
		border-radius: 4px;
		cursor: pointer;
	}
	.section-kebab:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours);
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
	.hl-overlay {
		color: inherit;
		padding: 1px 2px;
		border-radius: 2px;
	}
	@keyframes hl-flash {
		0%, 100% { filter: brightness(1) saturate(1); }
		30%      { filter: brightness(1.5) saturate(1.6); }
	}
	:global(.hl-overlay.hl-flash) {
		animation: hl-flash 1.2s ease-in-out;
	}

	.cv-threads {
		margin-top: 10px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
	}
	.cv-threads-head {
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
	.cv-threads-head:hover { color: var(--fg); }
	.cv-threads-head .ptr { min-width: 1ch; }
</style>
