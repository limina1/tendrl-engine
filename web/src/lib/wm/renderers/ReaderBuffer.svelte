<script lang="ts">
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import type { LazySection, PublicationDetail, TagEntry, ViewMode } from '$lib/types';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

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

	function extractPublicationTags(pub: PublicationDetail | null): TagEntry[] {
		if (!pub) return [];
		// pub.index is the raw 30040 event; tags include `t` (topic), `title`,
		// `summary`, `image`, `d`, `a`. Drop the structural ones (a-tag list
		// is regenerated on publish from the section list; d-tag is set by
		// the publish step) and keep user-facing metadata.
		const skip = new Set(['d', 'a', 'alt', 'e', 'p']);
		const rawTags =
			(pub.index as { data?: { tags?: string[][] } } | null)?.data?.tags ?? [];
		return rawTags
			.filter((t) => !skip.has(t[0]))
			.map((t) => ({ name: t[0], value: t.slice(1).join(', ') }));
	}

	async function ensureAllSectionsLoaded() {
		for (let i = 0; i < sections.length; i++) {
			if (sections[i].status === 'pending') handleLoadSection(i);
		}
		// loadingPromises is mutated by handleLoadSection — snapshot then await.
		const inflight = Array.from(loadingPromises.values());
		if (inflight.length) await Promise.all(inflight);
	}

	async function editInComposer() {
		// Replace whatever is in the compose pool with this publication's
		// sections, then jump to the composer buffer. Force-load any
		// pending sections first so we don't lose content.
		await ensureAllSectionsLoaded();
		app.clearComposePool();
		app.seedDraftMetadata(publication?.title ?? null, extractPublicationTags(publication));
		for (const s of sections) {
			if (s.status !== 'loaded' || s.content == null) continue;
			app.importSectionToCompose(s.addr, s.title, s.content);
		}
		app.navigateToCompose();
	}

	async function editFocusedSection() {
		// Just the currently-paginated section. Replaces whatever is in the
		// pool so "Edit §" never leaks the rest of the document. Skips
		// publication-level title/tag seeding since we're scoped to a
		// single section.
		const s = sections[currentSection];
		if (!s) return;
		if (s.status !== 'loaded' || s.content == null) {
			handleLoadSection(currentSection);
			const inflight = Array.from(loadingPromises.values());
			if (inflight.length) await Promise.all(inflight);
		}
		const reloaded = sections[currentSection];
		if (!reloaded || reloaded.status !== 'loaded' || reloaded.content == null) return;
		app.clearComposePool();
		app.seedDraftMetadata(null, []);
		app.importSectionToCompose(reloaded.addr, reloaded.title, reloaded.content);
		app.navigateToCompose();
	}
</script>

<div class="reader-wrap">
	<div class="toolbar">
		<button class:active={viewMode === 'outline'} onclick={() => (viewMode = 'outline')}>Outline</button>
		<button class:active={viewMode === 'continuous'} onclick={() => (viewMode = 'continuous')}>Continuous</button>
		<button class:active={viewMode === 'paginated'} onclick={() => (viewMode = 'paginated')}>Paginated</button>
		<span class="sp"></span>
		{#if viewMode === 'paginated'}
			<button class="edit" onclick={editFocusedSection} disabled={!publication} title="Send focused section to composer">Edit §</button>
		{/if}
		<button class="edit" onclick={editInComposer} disabled={!publication} title="Send all loaded sections to composer">Edit</button>
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
	.toolbar .sp { flex: 1; }
	.toolbar .edit {
		color: var(--id-draft);
		border-color: var(--id-draft);
	}
	.toolbar .edit:hover:not(:disabled) {
		background: var(--id-draft);
		color: var(--bg);
	}
	.toolbar .edit:disabled { opacity: 0.5; cursor: not-allowed; }
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
