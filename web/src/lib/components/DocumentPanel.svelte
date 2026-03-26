<script lang="ts">
	import type { LazySection, ViewMode, DocMode, PublicationSummary, PublicationDetail, ComposeState, ContextItem, SyncMode } from '$lib/types';
	import DocumentToolbar from './DocumentToolbar.svelte';
	import OutlineView from './OutlineView.svelte';
	import ContinuousView from './ContinuousView.svelte';
	import PaginatedView from './PaginatedView.svelte';
	import JsonPreview from './JsonPreview.svelte';
	import ComposeView from './ComposeView.svelte';

	let {
		docMode,
		publication,
		sections,
		viewMode,
		currentSection,
		previewVisible,
		compose,
		loading,
		feed = [],
		feedLoading = false,
		feedSyncing = false,
		feedLoadingMore = false,
		feedHasMore = true,
		onviewmode,
		ontogglepreview,
		oncompose,
		onnavigate,
		oncomposeupdate,
		oncancelcompose,
		onsendtochat,
		onpublishcompose,
		ondeletecompose,
		ondeletepermanentcompose,
		ondoctochat,
		ondocpublish,
		onopenpub,
		onfeedsync,
		onfeedloadmore,
		onloadsection,
		onignoreevent,
		onignorepubkey,
		syncMode,
		onsenditemtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy
	}: {
		docMode: DocMode;
		publication: PublicationDetail | null;
		sections: LazySection[];
		viewMode: ViewMode;
		currentSection: number;
		previewVisible: boolean;
		compose: ComposeState;
		loading: boolean;
		feed?: PublicationSummary[];
		feedLoading?: boolean;
		feedSyncing?: boolean;
		feedLoadingMore?: boolean;
		feedHasMore?: boolean;
		onviewmode: (mode: ViewMode) => void;
		ontogglepreview: () => void;
		oncompose: () => void;
		onnavigate: (index: number) => void;
		oncomposeupdate: (state: ComposeState) => void;
		oncancelcompose: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublishcompose: (items: ContextItem[]) => void;
		ondeletecompose: (items: ContextItem[]) => void;
		ondeletepermanentcompose: (items: ContextItem[]) => void;
		ondoctochat: () => void;
		ondocpublish: () => void;
		onopenpub?: (pub_summary: PublicationSummary) => void;
		onfeedsync?: () => void;
		onfeedloadmore?: () => void;
		onloadsection?: (index: number) => void;
		onignoreevent?: (event_id: string) => void;
		onignorepubkey?: (pubkey: string) => void;
		syncMode: SyncMode;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
	} = $props();

	let feedMenuOpen: string | null = $state(null);

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}
</script>

<div class="document-panel">
	<DocumentToolbar
		{viewMode}
		{docMode}
		{previewVisible}
		{onviewmode}
		{ontogglepreview}
		{oncompose}
		onsendtochat={ondoctochat}
		onpublish={ondocpublish}
	/>

	{#if docMode === 'reading' && publication?.title}
		<div class="doc-title">{publication.title}</div>
	{/if}

	<div class="doc-content">
		{#if docMode === 'empty'}
			{#if feedLoading}
				<div class="doc-empty"><p>Loading publications...</p></div>
			{:else if feed.length > 0}
				<div class="feed-list">
					<div class="feed-header">
						<span>Publications ({feed.length})</span>
						<button class="feed-sync-btn" onclick={onfeedsync} disabled={feedSyncing}>
							{feedSyncing ? 'Syncing...' : 'Sync from relays'}
						</button>
					</div>
					{#each feed as pub_item (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
						{@const feedKey = `${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="feed-item"
							onclick={() => onopenpub?.(pub_item)}
							onkeydown={(e) => { if (e.key === 'Enter') onopenpub?.(pub_item); }}
							role="button"
							tabindex="0"
						>
							<div class="feed-item-header">
								<span class="feed-item-title">{pub_item.title ?? '[Untitled]'}</span>
								<span class="feed-item-meta">{pub_item.section_count} sections</span>
								<div class="feed-menu-container">
									<button class="feed-menu-btn" onclick={(e) => { e.stopPropagation(); feedMenuOpen = feedMenuOpen === feedKey ? null : feedKey; }} title="More">⋮</button>
									{#if feedMenuOpen === feedKey}
										<!-- svelte-ignore a11y_click_events_have_key_events -->
										<div class="feed-menu-backdrop" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; }} role="presentation"></div>
										<div class="feed-menu-dropdown">
											<button class="feed-menu-item" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; onignoreevent?.(pub_item.addr.d_tag); }}>Hide publication</button>
											<button class="feed-menu-item feed-menu-danger" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; onignorepubkey?.(pub_item.author_pubkey); }}>Hide author</button>
										</div>
									{/if}
								</div>
							</div>
							{#if pub_item.summary}
								<p class="feed-item-summary">{pub_item.summary}</p>
							{/if}
							<div class="feed-item-footer">
								<span class="feed-item-author">{pub_item.author_pubkey.slice(0, 12)}...</span>
								<span class="feed-item-time">{formatTime(pub_item.created_at)}</span>
							</div>
						</div>
					{/each}
					{#if feedHasMore}
						<div class="feed-load-more">
							<button onclick={onfeedloadmore} disabled={feedLoadingMore}>
								{feedLoadingMore ? 'Loading...' : 'Load more'}
							</button>
						</div>
					{/if}
				</div>
			{:else}
				<div class="doc-empty">
					<div class="empty-actions">
						<p>No publications found locally.</p>
						<button onclick={onfeedsync} disabled={feedSyncing}>
							{feedSyncing ? 'Syncing...' : 'Fetch from relays'}
						</button>
					</div>
				</div>
			{/if}
		{:else if docMode === 'reading'}
			{#if loading}
				<div class="doc-empty"><p>Loading...</p></div>
			{:else if viewMode === 'outline'}
				<OutlineView {sections} onload={onloadsection} onselect={(index) => { onloadsection?.(index); onviewmode('paginated'); onnavigate(index); }} />
			{:else if viewMode === 'continuous'}
				<ContinuousView {sections} publication={publication ? { title: publication.title, summary: publication.summary } : null} onload={onloadsection} />
			{:else}
				<PaginatedView {sections} {currentSection} {onnavigate} onload={onloadsection} />
			{/if}
		{:else if docMode === 'compose'}
			<ComposeView
				{compose}
				onupdate={oncomposeupdate}
				oncancel={oncancelcompose}
				{onsendtochat}
				onpublish={onpublishcompose}
				ondelete={ondeletecompose}
				ondeletepermanent={ondeletepermanentcompose}
				{syncMode}
				{onsenditemtochat}
				{ontogglereadonly}
				{onlocksource}
				{oncrosspanelcopy}
			/>
		{/if}

		{#if previewVisible && publication}
			<JsonPreview data={publication} onclose={ontogglepreview} />
		{/if}
	</div>
</div>

<style>
	.document-panel {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.doc-title {
		padding: 10px 16px;
		font-size: 1.1rem;
		font-weight: 700;
		border-bottom: 1px solid var(--border);
	}

	.doc-content {
		flex: 1;
		position: relative;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.doc-empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
		font-size: 0.85rem;
	}

	.empty-actions {
		text-align: center;
		display: flex;
		flex-direction: column;
		gap: 12px;
		align-items: center;
	}

	/* Feed list */

	.feed-list {
		flex: 1;
		overflow-y: auto;
	}

	.feed-header {
		padding: 10px 16px;
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--border);
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.feed-sync-btn {
		font-size: 0.65rem;
		padding: 2px 8px;
		text-transform: none;
		letter-spacing: normal;
		font-weight: 400;
	}

	.feed-item {
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
		cursor: pointer;
		border-left: 3px solid #3b82f6;
	}

	.feed-item:hover {
		background: var(--bg-surface);
	}

	.feed-item-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin-bottom: 2px;
	}

	.feed-item-title {
		font-size: 0.9rem;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		flex: 1;
	}

	.feed-item-meta {
		font-size: 0.7rem;
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.feed-item-summary {
		font-size: 0.8rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		-webkit-box-orient: vertical;
	}

	.feed-item-footer {
		display: flex;
		gap: 8px;
		font-size: 0.7rem;
		color: var(--fg-muted);
		margin-top: 4px;
	}

	.feed-menu-container {
		position: relative;
	}

	.feed-menu-btn {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: 0.9rem;
		padding: 0 4px;
		line-height: 1;
	}

	.feed-menu-btn:hover {
		color: var(--fg);
	}

	.feed-menu-backdrop {
		position: fixed;
		inset: 0;
		z-index: 50;
	}

	.feed-menu-dropdown {
		position: absolute;
		right: 0;
		top: 100%;
		z-index: 51;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
		min-width: 130px;
		padding: 4px 0;
	}

	.feed-menu-item {
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

	.feed-menu-item:hover {
		background: var(--bg-surface);
	}

	.feed-menu-danger {
		color: #ef4444;
	}

	.feed-menu-danger:hover {
		background: #ef444415;
	}

	.feed-load-more {
		padding: 12px 16px;
		text-align: center;
	}

	.feed-load-more button {
		font-size: 0.8rem;
		padding: 6px 20px;
	}
</style>
