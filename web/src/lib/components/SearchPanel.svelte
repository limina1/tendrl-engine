<script lang="ts">
	import type { SearchResult } from '$lib/types';
	import SearchInput from './SearchInput.svelte';
	import SearchResultItem from './SearchResultItem.svelte';

	let {
		results,
		count = 0,
		localCount = 0,
		relayCount = 0,
		loading = false,
		onsearch,
		onselect,
		onviewjson
	}: {
		results: SearchResult[];
		count?: number;
		localCount?: number;
		relayCount?: number;
		loading?: boolean;
		onsearch: (query: string) => void;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
	} = $props();
</script>

<div class="search-panel">
	<SearchInput {onsearch} disabled={loading} />

	{#if count > 0}
		<div class="search-summary">
			{count} results ({localCount} local, {relayCount} relay)
		</div>
	{/if}

	<div class="search-results">
		{#each results as result (result.event_id)}
			<SearchResultItem {result} {onselect} {onviewjson} />
		{/each}

		{#if !loading && results.length === 0}
			<p class="empty">Search publications, sections, and notes</p>
		{/if}

		{#if loading}
			<p class="empty">Searching...</p>
		{/if}
	</div>
</div>

<style>
	.search-panel {
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.search-summary {
		padding: 4px 12px 8px;
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	.search-results {
		flex: 1;
		overflow-y: auto;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: 0.85rem;
		padding: 0 12px;
	}
</style>
