<script lang="ts">
	import type { SearchResult } from '$lib/types';

	let {
		result,
		onselect,
		onviewjson,
		onaddtocontext,
		onaddtocompose
	}: {
		result: SearchResult;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
		onaddtocontext: (result: SearchResult) => void;
		onaddtocompose: (result: SearchResult) => void;
	} = $props();

	let tagsExpanded = $state(false);

	const KINDS: Record<number, string> = {
		30040: 'index',
		30041: 'section',
		1: 'note'
	};

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	function formatTag(tag: string[]): string {
		if (tag.length >= 2) return `${tag[0]}:${tag[1]}`;
		return tag[0] ?? '';
	}

	const preview = $derived(
		result.preview.length > 100 ? result.preview.slice(0, 100) + '...' : result.preview
	);

	const visibleTags = $derived(result.tags.slice(0, 3));
	const extraTagCount = $derived(Math.max(0, result.tags.length - 3));
</script>

<div class="result-item">
	<div class="result-header" onclick={() => onselect(result)} onkeydown={(e) => e.key === 'Enter' && onselect(result)} role="button" tabindex="0">
		<span class="result-title">{result.title ?? '[Untitled]'}</span>
		<span class="kind-badge">{KINDS[result.kind] ?? result.kind}</span>
	</div>
	<p class="result-preview" onclick={() => onselect(result)} role="presentation">{preview}</p>

	{#if result.tags.length > 0}
		<div class="tag-row">
			{#each visibleTags as tag}
				<span
					class="tag-pill"
					onclick={() => (tagsExpanded = !tagsExpanded)}
					role="button"
					tabindex="0"
					onkeydown={(e) => e.key === 'Enter' && (tagsExpanded = !tagsExpanded)}
				>{formatTag(tag)}</span>
			{/each}
			{#if extraTagCount > 0}
				<span
					class="tag-pill tag-more"
					onclick={() => (tagsExpanded = !tagsExpanded)}
					role="button"
					tabindex="0"
					onkeydown={(e) => e.key === 'Enter' && (tagsExpanded = !tagsExpanded)}
				>+{extraTagCount} more</span>
			{/if}
		</div>
	{/if}

	{#if tagsExpanded}
		<div class="tag-inspector">
			{#each result.tags as tag}
				<div class="tag-inspector-row">
					<span class="tag-name">{tag[0] ?? ''}</span>
					<span class="tag-value">{tag.slice(1).join(', ')}</span>
				</div>
			{/each}
		</div>
	{/if}

	<div class="result-meta">
		<span class="result-author">{result.author.slice(0, 12)}...</span>
		<span class="result-time">{formatTime(result.created_at)}</span>
		<button class="action-btn" onclick={(e) => { e.stopPropagation(); onviewjson(result); }}>JSON</button>
		<button class="action-btn icon-btn" onclick={(e) => { e.stopPropagation(); onaddtocontext(result); }} title="Send to chat">◂</button>
		<button class="action-btn icon-btn" onclick={(e) => { e.stopPropagation(); onaddtocompose(result); }} title="Send to compose">□</button>
	</div>
</div>

<style>
	.result-item {
		display: block;
		width: 100%;
		text-align: left;
		padding: 10px 12px;
		border: none;
		border-bottom: 1px solid var(--border);
		border-radius: 0;
		background: transparent;
		transition: background 0.1s;
	}

	.result-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
		margin-bottom: 4px;
		cursor: pointer;
	}

	.result-header:hover {
		color: var(--accent);
	}

	.result-title {
		font-size: 0.85rem;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		flex: 1;
	}

	.kind-badge {
		font-size: 0.7rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.result-preview {
		font-size: 0.8rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin-bottom: 4px;
		cursor: pointer;
	}

	.tag-row {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		margin-bottom: 4px;
	}

	.tag-pill {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		cursor: pointer;
	}

	.tag-pill:hover {
		color: var(--fg);
	}

	.tag-more {
		font-style: italic;
	}

	.tag-inspector {
		border: 1px solid var(--border);
		border-radius: 4px;
		padding: 6px 8px;
		margin-bottom: 4px;
		background: var(--bg-surface);
		display: flex;
		flex-direction: column;
		gap: 3px;
	}

	.tag-inspector-row {
		display: flex;
		gap: 8px;
		font-size: 0.7rem;
		font-family: var(--font-mono);
	}

	.tag-name {
		color: #22c55e;
		min-width: 40px;
	}

	.tag-value {
		color: var(--fg-muted);
		word-break: break-all;
	}

	.result-meta {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.7rem;
		color: var(--fg-muted);
	}

	.result-author {
		flex: 1;
	}

	.action-btn {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		border: none;
		cursor: pointer;
	}

	.action-btn:hover {
		color: var(--fg);
	}

	.icon-btn {
		font-size: 0.8rem;
		min-width: 22px;
		text-align: center;
	}
</style>
