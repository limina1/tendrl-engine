<script lang="ts">
	import * as api from '$lib/api';
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import type { LazySection, PublicationDetail, ViewMode } from '$lib/types';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	let publication = $state<PublicationDetail | null>(null);
	let sections = $state<LazySection[]>([]);
	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const loadingPromises = new Map<number, Promise<void>>();

	function parseBufferId(id: string): { pubkey: string; dTag: string } | null {
		const match = id.match(/^reader:\d+:([0-9a-fA-F]{64}):(.+)$/);
		if (!match) return null;
		return { pubkey: match[1].toLowerCase(), dTag: match[2] };
	}

	async function load() {
		const parsed = parseBufferId(buffer.id);
		if (!parsed) {
			error = 'Buffer id does not encode a publication address';
			loading = false;
			return;
		}
		loading = true;
		try {
			const resp = await api.getPublication(parsed.pubkey, parsed.dTag, 'local_first');
			publication = resp.publication;
			sections = resp.toc.map((entry, i) => ({
				addr: entry.addr,
				title: entry.title,
				content: null,
				position: i,
				status: 'pending' as const
			}));
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		// Re-run when buffer.id changes; {#key buffer.id} in BufferRenderer
		// also remounts on id change so this is belt-and-braces.
		buffer.id;
		load();
	});

	function handleLoadSection(index: number) {
		if (index < 0 || index >= sections.length) return;
		const cur = sections[index];
		if (cur.status === 'loaded' || cur.status === 'loading') return;
		if (loadingPromises.has(index)) return;
		sections[index] = { ...cur, status: 'loading' };
		const parsed = parseBufferId(buffer.id);
		if (!parsed) return;
		const promise = (async () => {
			try {
				const resp = await api.getSection(parsed.pubkey, parsed.dTag, index);
				sections[index] = {
					...sections[index],
					title: resp.section.title ?? sections[index].title,
					content: resp.section.content,
					status: 'loaded'
				};
			} catch (e) {
				sections[index] = { ...sections[index], status: 'error', error: String(e) };
			} finally {
				loadingPromises.delete(index);
			}
		})();
		loadingPromises.set(index, promise);
	}

	function handleNavigate(index: number) {
		currentSection = index;
	}
</script>

<div class="reader-wrap">
	<div class="toolbar">
		<button class:active={viewMode === 'outline'} onclick={() => (viewMode = 'outline')}>Outline</button>
		<button class:active={viewMode === 'continuous'} onclick={() => (viewMode = 'continuous')}>Continuous</button>
		<button class:active={viewMode === 'paginated'} onclick={() => (viewMode = 'paginated')}>Paginated</button>
	</div>

	{#if loading}
		<div class="empty"><p>Loading…</p></div>
	{:else if error}
		<div class="empty"><p>Error: {error}</p></div>
	{:else if !publication}
		<div class="empty"><p>No publication loaded</p></div>
	{:else}
		{#if publication.title}
			<div class="title">{publication.title}</div>
		{/if}
		<div class="content">
			{#if viewMode === 'outline'}
				<OutlineView
					{sections}
					onload={handleLoadSection}
					onselect={(i) => {
						handleLoadSection(i);
						viewMode = 'paginated';
						handleNavigate(i);
					}}
				/>
			{:else if viewMode === 'continuous'}
				<ContinuousView
					{sections}
					publication={{ title: publication.title, summary: publication.summary }}
					onload={handleLoadSection}
				/>
			{:else}
				<PaginatedView
					{sections}
					{currentSection}
					onnavigate={handleNavigate}
					onload={handleLoadSection}
				/>
			{/if}
		</div>
	{/if}
</div>

<style>
	.reader-wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.toolbar {
		display: flex;
		gap: 4px;
		padding: 6px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		flex-shrink: 0;
	}
	.toolbar button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.toolbar button.active {
		background: var(--id-yours);
		color: var(--bg);
		border-color: var(--id-yours);
	}
	.title {
		padding: 8px var(--s-3);
		font-size: var(--t-md);
		font-weight: 700;
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
	}
	.content { flex: 1; overflow: auto; min-height: 0; }
	.empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
</style>
