<script lang="ts">
	import type { SearchResult, ContextItem } from '$lib/types';
	import SearchInput from './SearchInput.svelte';
	import SearchResultItem from './SearchResultItem.svelte';

	let {
		results,
		count = 0,
		localCount = 0,
		relayCount = 0,
		loading = false,
		searchContext = 'knowledge base',
		onsearch,
		onselect,
		onviewjson,
		onaddtocontext,
		onaddtocompose,
		onaddmanytocontext,
		onaddmanytocompose,
		onignore,
		onignorepubkey,
		items = []
	}: {
		results: SearchResult[];
		count?: number;
		localCount?: number;
		relayCount?: number;
		loading?: boolean;
		searchContext?: string;
		onsearch: (query: string) => void;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
		onaddtocontext: (result: SearchResult) => void;
		onaddtocompose: (result: SearchResult) => void;
		onaddmanytocontext: (results: SearchResult[]) => void;
		onaddmanytocompose: (results: SearchResult[]) => void;
		onignore?: (result: SearchResult) => void;
		onignorepubkey?: (result: SearchResult) => void;
		items?: ContextItem[];
	} = $props();

	let checkedIds: Set<string> = $state(new Set());

	function toggleCheck(id: string) {
		const next = new Set(checkedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedIds = next;
	}

	function selectAll() {
		checkedIds = new Set(results.map((r) => r.event_id));
	}

	function invertSelection() {
		const next = new Set<string>();
		for (const r of results) {
			if (!checkedIds.has(r.event_id)) next.add(r.event_id);
		}
		checkedIds = next;
	}

	function sendCheckedToContext() {
		const checked = results.filter((r) => checkedIds.has(r.event_id));
		if (checked.length > 0) {
			onaddmanytocontext(checked);
			checkedIds = new Set();
		}
	}

	function sendCheckedToCompose() {
		const checked = results.filter((r) => checkedIds.has(r.event_id));
		if (checked.length > 0) {
			onaddmanytocompose(checked);
			checkedIds = new Set();
		}
	}

	const hasChecked = $derived(checkedIds.size > 0);
</script>

<div class="search-panel">
	<SearchInput {onsearch} />

	{#if count > 0}
		<div class="search-bar">
			<span class="search-summary">
				{count} results ({localCount} local, {relayCount} relay)
			</span>
			<div class="search-actions">
				<button class="sel-btn" onclick={selectAll} disabled={results.length === 0} title="Select all">All</button>
				<button class="sel-btn" onclick={invertSelection} disabled={results.length === 0} title="Invert selection">Inv</button>
				<button class="icon-btn" onclick={sendCheckedToContext} disabled={!hasChecked} title="Send to chat">◂</button>
				<button class="icon-btn" onclick={sendCheckedToCompose} disabled={!hasChecked} title="Send to compose">□</button>
			</div>
		</div>
	{/if}

	<div class="search-results">
		{#each results as result (result.event_id)}
			<SearchResultItem
				{result}
				checked={checkedIds.has(result.event_id)}
				ontogglecheck={() => toggleCheck(result.event_id)}
				{onselect}
				{onviewjson}
				{onaddtocontext}
				{onaddtocompose}
				{onignore}
				{onignorepubkey}
				{items}
			/>
		{/each}

		{#if !loading && results.length === 0}
			<p class="empty">Search {searchContext}</p>
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

	.search-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 12px 8px;
		gap: 8px;
	}

	.search-summary {
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	.search-actions {
		display: flex;
		gap: 4px;
		align-items: center;
	}

	.sel-btn {
		font-size: 0.65rem;
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		padding: 4px 8px;
		font-size: 0.85rem;
		min-width: 28px;
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
