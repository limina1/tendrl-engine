<script lang="ts">
	import type { ThreadNode } from '$lib/discussions/thread';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { getAuthorDisplayName, hasAuthorName } from '$lib/discussions/authors.svelte';

	let {
		nodes,
		focusedEventId = null,
		depth = 0,
		maxDepth = 6
	}: {
		nodes: ThreadNode[];
		/** When set, the comment with this id gets a visual ring and the
		 *  node is scrolled into view on first mount. Used by the reader's
		 *  ?focus_comment=<id> routing. */
		focusedEventId?: string | null;
		depth?: number;
		/** Past this depth, deeper replies collapse into "show N more
		 *  replies" rather than indenting further. Prevents runaway
		 *  horizontal drift on long chains. */
		maxDepth?: number;
	} = $props();

	function short(s: string, n: number): string {
		return s.length > n ? `${s.slice(0, n)}…` : s;
	}
	function fmtTime(ts: number): string {
		// Compact relative-ish format. Falls back to absolute for old events.
		const now = Date.now() / 1000;
		const diff = now - ts;
		if (diff < 60) return 'just now';
		if (diff < 3600) return `${Math.floor(diff / 60)}m`;
		if (diff < 86400) return `${Math.floor(diff / 3600)}h`;
		if (diff < 2592000) return `${Math.floor(diff / 86400)}d`;
		return new Date(ts * 1000).toLocaleDateString();
	}

	function bindRef(node: HTMLElement | undefined, eventId: string) {
		if (!node) return;
		if (focusedEventId && focusedEventId.toLowerCase() === eventId.toLowerCase()) {
			// Defer one frame so the surrounding paginated/continuous view
			// has positioned itself first.
			requestAnimationFrame(() => node.scrollIntoView({ behavior: 'auto', block: 'center' }));
		}
	}
</script>

{#each nodes as node (node.event.id)}
	{@const isFocused =
		focusedEventId !== null &&
		node.event.id.toLowerCase() === focusedEventId.toLowerCase()}
	{@const isReply = depth > 0}
	{@const authorColor = pubkeyToColor(node.event.pubkey)}
	<div
		class="ct-node"
		class:ct-node--focused={isFocused}
		class:ct-node--reply={isReply}
		style="--ct-author-color: {authorColor};"
		use:bindRef={node.event.id}
	>
		<div class="ct-meta">
			<span class="ct-marker" aria-hidden="true">{isReply ? '↳' : '▾'}</span>
			<span class="ct-author-dot" style="background: {authorColor};" aria-hidden="true"></span>
			{#if hasAuthorName(node.event.pubkey)}
				<span class="ct-author-name" title={node.event.pubkey}>{getAuthorDisplayName(node.event.pubkey)}</span>
			{:else}
				<code class="ct-author">{short(node.event.pubkey, 12)}</code>
			{/if}
			<span class="ct-sep">·</span>
			<span class="ct-time" title={new Date(node.event.created_at * 1000).toLocaleString()}>
				{fmtTime(node.event.created_at)}
			</span>
		</div>
		<div class="ct-body">{node.event.content}</div>
	</div>
	{#if node.children.length > 0}
		{#if depth + 1 >= maxDepth}
			<details class="ct-collapse">
				<summary>Show {node.children.length} more {node.children.length === 1 ? 'reply' : 'replies'}</summary>
				<div class="ct-children">
					<svelte:self
						nodes={node.children}
						{focusedEventId}
						depth={depth + 1}
						{maxDepth}
					/>
				</div>
			</details>
		{:else}
			<div class="ct-children">
				<svelte:self
					nodes={node.children}
					{focusedEventId}
					depth={depth + 1}
					{maxDepth}
				/>
			</div>
		{/if}
	{/if}
{/each}

<style>
	.ct-node {
		padding: 6px 10px 6px 8px;
		margin-bottom: 4px;
		border-left: 2px solid var(--ct-author-color, var(--panel-border));
		background: var(--bg-surface);
		border-radius: 0 var(--r-sm) var(--r-sm) 0;
	}
	.ct-node--focused {
		box-shadow: 0 0 0 1px var(--ct-author-color, var(--id-yours));
	}
	.ct-meta {
		display: flex;
		gap: 6px;
		align-items: center;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-bottom: 3px;
	}
	.ct-marker {
		color: var(--ct-author-color, var(--base5));
		font-size: var(--t-xs);
		line-height: 1;
		min-width: 1ch;
	}
	.ct-author-dot {
		display: inline-block;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.ct-author {
		background: transparent;
		color: var(--base6);
	}
	.ct-author-name {
		color: var(--base7);
		font-family: var(--font-sans, inherit);
	}
	.ct-sep { color: var(--base4); }
	.ct-body {
		font-size: var(--t-xs);
		color: var(--fg);
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-word;
		padding-left: calc(1ch + 6px + 8px + 6px);
	}

	.ct-children {
		margin-left: 14px;
		margin-top: 4px;
		padding-left: 0;
	}

	.ct-collapse {
		margin-left: 14px;
		margin-top: 4px;
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.ct-collapse > summary {
		cursor: pointer;
		padding: 2px 6px;
		font-family: var(--font-mono);
	}
	.ct-collapse > summary:hover { color: var(--id-yours); }
</style>
