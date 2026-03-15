<script lang="ts">
	import type { SearchResult } from '$lib/types';

	let {
		result,
		onselect,
		onviewjson
	}: {
		result: SearchResult;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
	} = $props();

	const KINDS: Record<number, string> = {
		30040: 'index',
		30041: 'section',
		1: 'note'
	};

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	const preview = $derived(
		result.preview.length > 100 ? result.preview.slice(0, 100) + '...' : result.preview
	);
</script>

<div class="result-item" onclick={() => onselect(result)} onkeydown={(e) => e.key === 'Enter' && onselect(result)} role="button" tabindex="0">
	<div class="result-header">
		<span class="result-title">{result.title ?? '[Untitled]'}</span>
		<span class="kind-badge">{KINDS[result.kind] ?? result.kind}</span>
	</div>
	<p class="result-preview">{preview}</p>
	<div class="result-meta">
		<span class="result-author">{result.author.slice(0, 12)}...</span>
		<span class="result-time">{formatTime(result.created_at)}</span>
		<button
			class="json-btn"
			onclick={(e) => { e.stopPropagation(); onviewjson(result); }}
		>JSON</button>
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
		cursor: pointer;
		transition: background 0.1s;
	}

	.result-item:hover {
		background: var(--bg-surface);
	}

	.result-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
		margin-bottom: 4px;
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
	}

	.result-meta {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 0.7rem;
		color: var(--fg-muted);
	}

	.result-author {
		flex: 1;
	}

	.json-btn {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		border: none;
		cursor: pointer;
	}

	.json-btn:hover {
		color: var(--fg);
	}
</style>
