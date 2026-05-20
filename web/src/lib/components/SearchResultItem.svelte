<script lang="ts">
	import type { SearchResult, ContextItem } from '$lib/types';
	import ProfileName from './ProfileName.svelte';
	import ProfileResultItem from './ProfileResultItem.svelte';

	let {
		result,
		checked = false,
		ontogglecheck,
		onselect,
		onviewjson,
		onaddtocontext,
		onaddtocompose,
		onignore,
		onignorepubkey,
		items = [],
		localPubkeys = new Set<string>(),
		onviewprofile
	}: {
		result: SearchResult;
		checked?: boolean;
		ontogglecheck?: () => void;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
		onaddtocontext: (result: SearchResult) => void;
		onaddtocompose: (result: SearchResult) => void;
		onignore?: (result: SearchResult) => void;
		onignorepubkey?: (result: SearchResult) => void;
		items?: ContextItem[];
		localPubkeys?: Set<string>;
		onviewprofile?: (pubkey: string) => void;
	} = $props();

	const poolMatch = $derived(
		items.find((e) => e.source_event_id === result.event_id) ?? null
	);

	let tagsExpanded = $state(false);
	let menuOpen = $state(false);
	let menuBtn: HTMLButtonElement | undefined = $state();
	let menuDirection: 'up' | 'down' = $state('up');

	// Drop the menu down when there isn't room above the kebab inside its
	// nearest scroll container — otherwise the upward-opening dropdown gets
	// clipped by the scroll container's top edge.
	const DROPDOWN_HEIGHT_PX = 100;

	function findScrollableAncestor(el: HTMLElement): HTMLElement | null {
		let node: HTMLElement | null = el.parentElement;
		while (node) {
			const overflowY = getComputedStyle(node).overflowY;
			if (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'overlay') return node;
			node = node.parentElement;
		}
		return null;
	}

	function toggleMenu(e: MouseEvent) {
		e.stopPropagation();
		if (menuOpen) {
			menuOpen = false;
			return;
		}
		if (menuBtn) {
			const btnRect = menuBtn.getBoundingClientRect();
			const container = findScrollableAncestor(menuBtn);
			const topBound = container?.getBoundingClientRect().top ?? 0;
			const spaceAbove = btnRect.top - topBound;
			menuDirection = spaceAbove < DROPDOWN_HEIGHT_PX ? 'down' : 'up';
		}
		menuOpen = true;
	}

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
</script>

{#if result.kind === 0}
	<!-- A kind-0 hit is an author match — render the profile, not a
	     generic document row. -->
	<ProfileResultItem {result} {checked} {ontogglecheck} {onviewprofile} {onignorepubkey} {localPubkeys} />
{:else}
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="result-item" class:kind-index={result.kind === 30040} class:kind-section={result.kind === 30041}>
	<div class="result-header">
		{#if ontogglecheck}
			<label class="result-check" onclick={(e) => e.stopPropagation()}>
				<input type="checkbox" {checked} onchange={ontogglecheck} />
			</label>
		{/if}
		<div class="result-header-text" onclick={() => onselect(result)} onkeydown={(e) => e.key === 'Enter' && onselect(result)} role="button" tabindex="0">
			<span class="result-title">{result.title ?? '[Untitled]'}</span>
			<span class="kind-badge">{KINDS[result.kind] ?? result.kind}</span>
			{#if localPubkeys?.has(result.author)}
				<span class="local-badge">local</span>
			{/if}
			{#if result.semantic_score != null}
				<span class="score-badge">{(result.semantic_score * 100).toFixed(0)}%</span>
			{/if}
			{#if poolMatch?.in_context}
				<span class="loc-badge" class:loc-synced={!poolMatch.modified} class:loc-modified={poolMatch.modified}>context</span>
			{/if}
			{#if poolMatch?.in_compose}
				<span class="loc-badge" class:loc-synced={!poolMatch.modified} class:loc-modified={poolMatch.modified}>compose</span>
			{/if}
		</div>
	</div>

	<p class="result-preview" onclick={() => onselect(result)} role="presentation">{preview}</p>

	{#if result.tags.length > 0}
		<button class="tag-toggle" onclick={() => (tagsExpanded = !tagsExpanded)}>
			<span class="tag-arrow" class:open={tagsExpanded}>{tagsExpanded ? '▾' : '▸'}</span>
			<span class="tag-count">{result.tags.length} tags</span>
		</button>
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
		<span class="result-author"><ProfileName pubkey={result.author} {onviewprofile} /></span>
		<span class="result-time">{formatTime(result.created_at)}</span>
		<button class="action-btn icon-btn" onclick={(e) => { e.stopPropagation(); onaddtocontext(result); }} title="Send to chat">◂</button>
		<div class="menu-container">
			<button bind:this={menuBtn} class="action-btn menu-btn" onclick={toggleMenu} title="More actions">⋮</button>
			{#if menuOpen}
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<div class="menu-backdrop" onclick={() => (menuOpen = false)} role="presentation"></div>
				<div class="menu-dropdown" class:menu-dropdown--down={menuDirection === 'down'}>
					<button class="menu-item" onclick={(e) => { e.stopPropagation(); menuOpen = false; onviewjson(result); }}>Open menu</button>
					{#if onignore}
						<button class="menu-item menu-item-danger" onclick={(e) => { e.stopPropagation(); menuOpen = false; onignore(result); }}>Hide event</button>
					{/if}
					{#if onignorepubkey}
						<button class="menu-item menu-item-danger" onclick={(e) => { e.stopPropagation(); menuOpen = false; onignorepubkey(result); }}>Hide author</button>
					{/if}
				</div>
			{/if}
		</div>
	</div>
</div>
{/if}

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
		border-right: 3px solid transparent;
	}

	.result-item.kind-index {
		border-right-color: #3b82f6;
	}

	.result-item.kind-section {
		border-right-color: #22c55e;
	}

	.result-header {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-bottom: 4px;
	}

	.result-check {
		display: flex;
		align-items: center;
		flex-shrink: 0;
	}

	.result-header-text {
		flex: 1;
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 8px;
		cursor: pointer;
	}

	.result-header-text:hover {
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

	.local-badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 3px;
		background: #f9731633;
		color: #f97316;
		white-space: nowrap;
		font-weight: 600;
	}

	.score-badge {
		font-size: 0.65rem;
		padding: 1px 5px;
		border-radius: 4px;
		background: #22c55e33;
		color: #22c55e;
		font-weight: 600;
		white-space: nowrap;
	}

	.result-preview {
		font-size: 0.8rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin-bottom: 4px;
		cursor: pointer;
	}

	/* Tag disclosure toggle */

	.tag-toggle {
		display: flex;
		align-items: center;
		gap: 4px;
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: 0.7rem;
		cursor: pointer;
		padding: 2px 0;
		margin-bottom: 4px;
	}

	.tag-toggle:hover {
		color: var(--fg);
	}

	.tag-arrow {
		font-size: 0.6rem;
	}

	.tag-count {
		font-size: 0.65rem;
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

	/* Meta row with actions */

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

	/* Hamburger menu */

	.menu-container {
		position: relative;
	}

	.menu-btn {
		font-size: 0.9rem;
		min-width: 20px;
		text-align: center;
		line-height: 1;
	}

	.menu-backdrop {
		position: fixed;
		inset: 0;
		z-index: 50;
	}

	.menu-dropdown {
		position: absolute;
		right: 0;
		bottom: 100%;
		z-index: 51;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
		min-width: 120px;
		padding: 4px 0;
	}

	.menu-dropdown--down {
		bottom: auto;
		top: 100%;
	}

	.menu-item {
		display: block;
		width: 100%;
		text-align: left;
		padding: 6px 12px;
		font-size: 0.75rem;
		background: none;
		border: none;
		color: var(--fg);
		cursor: pointer;
	}

	.menu-item:hover {
		background: var(--bg-surface);
	}

	.menu-item-danger {
		color: #ef4444;
	}

	.menu-item-danger:hover {
		background: #ef444415;
	}

	/* Location badges */

	.loc-badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 3px;
		white-space: nowrap;
	}

	.loc-synced {
		background: #22c55e33;
		color: #22c55e;
	}

	.loc-modified {
		background: #eab30833;
		color: #eab308;
	}
</style>
