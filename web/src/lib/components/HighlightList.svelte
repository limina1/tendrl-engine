<script lang="ts">
	// Flat list of NIP-84 highlights for a single section. Used in the
	// outline view's expandable section row. Comments get the full
	// threaded tree (CommentThread); highlights are inherently flat —
	// they don't reply to each other — so we render them as a level-1
	// list with the same per-author color identity used by inline marks
	// and the drawer.
	import type { Highlight } from '$lib/discussions/highlights';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { getAuthorDisplayName, hasAuthorName } from '$lib/discussions/authors.svelte';

	let {
		highlights
	}: {
		highlights: (Highlight & { created_at?: number })[];
	} = $props();

	function short(s: string, n: number): string {
		return s.length > n ? `${s.slice(0, n)}…` : s;
	}
	function fmtTime(ts: number | undefined): string {
		if (!ts) return '';
		const now = Date.now() / 1000;
		const diff = now - ts;
		if (diff < 60) return 'just now';
		if (diff < 3600) return `${Math.floor(diff / 60)}m`;
		if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
		if (diff < 2592000) return `${Math.floor(diff / 86400)}d`;
		return new Date(ts * 1000).toLocaleDateString();
	}

	// Highlights newest-first — same ordering as the drawer's bucket sort.
	const sorted = $derived(
		[...highlights].sort((a, b) => (b.created_at ?? 0) - (a.created_at ?? 0))
	);
</script>

{#each sorted as h (h.id)}
	{@const authorColor = pubkeyToColor(h.pubkey)}
	<div class="hl-item" style="--hl-author-color: {authorColor};">
		<div class="hl-meta">
			<span class="hl-marker" aria-hidden="true">•</span>
			<span class="hl-author-dot" style="background: {authorColor};" aria-hidden="true"></span>
			{#if hasAuthorName(h.pubkey)}
				<span class="hl-author-name" title={h.pubkey}>{getAuthorDisplayName(h.pubkey)}</span>
			{:else}
				<code class="hl-author">{short(h.pubkey, 12)}</code>
			{/if}
			<span class="hl-sep">·</span>
			<span class="hl-time">{fmtTime(h.created_at)}</span>
		</div>
		<blockquote class="hl-body">{h.content}</blockquote>
	</div>
{/each}

<style>
	.hl-item {
		padding: 6px 10px 6px 8px;
		margin-bottom: 4px;
		border-left: 2px solid var(--hl-author-color, var(--panel-border));
		background: var(--bg-surface);
		border-radius: 0 var(--r-sm) var(--r-sm) 0;
	}
	.hl-meta {
		display: flex;
		gap: 6px;
		align-items: center;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-bottom: 3px;
	}
	.hl-marker {
		color: var(--hl-author-color, var(--base5));
		min-width: 1ch;
	}
	.hl-author-dot {
		display: inline-block;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.hl-author {
		background: transparent;
		color: var(--base6);
	}
	.hl-author-name {
		color: var(--base7);
		font-family: var(--font-sans, inherit);
	}
	.hl-sep {
		color: var(--base4);
	}
	.hl-body {
		margin: 0;
		padding: 0;
		font-size: var(--t-xs);
		color: var(--fg);
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-word;
		font-style: italic;
		padding-left: calc(1ch + 6px + 8px + 6px);
	}
</style>
