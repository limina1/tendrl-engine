<script lang="ts">
	import type { LazySection, ViewMode, DocMode, PublicationSummary, PublicationDetail, ComposeState, ContextItem, SyncMode } from '$lib/types';
	import DocumentToolbar from './DocumentToolbar.svelte';
	import ProfileName from './ProfileName.svelte';
	import ProfileModal from './ProfileModal.svelte';
	import ProfileView from './ProfileView.svelte';
	import { getProfile, onProfileUpdate, type Profile } from '$lib/api';
	import * as api from '$lib/api';
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
		onfetchfromrelay,
		onfetchauthors,
		onfetchsections,
		onfeedloadmore,
		fetchRelays = [],
		authorCount = 0,
		onloadsection,
		onignoreevent,
		onignorepubkey,
		ignoredEventIds = [],
		ignoredPubkeys = [],
		onunignore,
		syncMode,
		onsenditemtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy,
		profilePubkey = null as string | null,
		onviewprofile,
		localPubkeys = new Set<string>()
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
		onfetchfromrelay?: (url: string, kinds: number[]) => void;
		onfetchauthors?: () => void;
		onfetchsections?: () => void;
		onfeedloadmore?: () => void;
		fetchRelays?: string[];
		authorCount?: number;
		onloadsection?: (index: number) => void;
		onignoreevent?: (event_id: string) => void;
		onignorepubkey?: (pubkey: string) => void;
		ignoredEventIds?: string[];
		ignoredPubkeys?: string[];
		onunignore?: (type: 'event' | 'pubkey', id: string) => void;
		syncMode: SyncMode;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
		profilePubkey?: string | null;
		onviewprofile?: (pubkey: string) => void;
		localPubkeys?: Set<string>;
	} = $props();

	let feedMenuOpen: string | null = $state(null);
	let fetchPanelOpen = $state(false);
	let authorProfile = $state<Profile | null>(null);
	let showAuthorModal = $state(false);

	// Resolve author profile when publication changes
	$effect(() => {
		const pk = publication?.author_pubkey;
		authorProfile = null;
		if (!pk) return;

		function resolve() {
			getProfile(pk!).then(p => {
				if (p.found) authorProfile = p;
			}).catch(() => {});
		}
		resolve();

		const unsub = onProfileUpdate(() => {
			if (!authorProfile) resolve();
		});

		return unsub;
	});
	let customRelayUrl = $state('');
	let customKinds = $state('30040,30041');
	let fetchingRelay: string | null = $state(null);

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

	{#if docMode === 'reading' && publication}
		{#if publication.title}
			<div class="doc-title">{publication.title}</div>
		{/if}
		<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
		<div class="doc-author-bar" onclick={() => { if (authorProfile) showAuthorModal = true; }}>
			{#if authorProfile?.picture}
				<img class="doc-author-avatar" src={authorProfile.picture} alt="" />
			{:else}
				<div class="doc-author-avatar placeholder">?</div>
			{/if}
			<span class="doc-author-name">
				<ProfileName pubkey={publication.author_pubkey} {onviewprofile} />
			</span>
			<span class="doc-author-time">{formatTime(publication.created_at)}</span>
		</div>
	{/if}

	{#if showAuthorModal && authorProfile}
		<ProfileModal profile={authorProfile} onclose={() => showAuthorModal = false} {onviewprofile} />
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
							{feedSyncing ? 'Syncing...' : 'Sync all'}
						</button>
						<button class="feed-sync-btn" onclick={() => (fetchPanelOpen = !fetchPanelOpen)}>
							{fetchPanelOpen ? '✕' : 'Fetch'}
						</button>
					</div>
					{#if fetchPanelOpen}
						<div class="fetch-panel">
							{#each fetchRelays as relay}
								<button
									class="fetch-relay-btn"
									disabled={fetchingRelay === relay}
									onclick={async () => {
										fetchingRelay = relay;
										const kinds = customKinds.split(',').map(k => parseInt(k.trim())).filter(k => !isNaN(k));
										await onfetchfromrelay?.(relay, kinds);
										fetchingRelay = null;
									}}
								>
									{fetchingRelay === relay ? '...' : '↻'} {relay.replace('wss://', '').replace('ws://', '')}
								</button>
							{/each}
							{#if authorCount > 0}
								<button
									class="fetch-relay-btn fetch-authors-btn"
									disabled={fetchingRelay === '__authors__'}
									onclick={async () => {
										fetchingRelay = '__authors__';
										await onfetchauthors?.();
										fetchingRelay = null;
									}}
								>
									{fetchingRelay === '__authors__' ? '...' : '↻'} Fetch {authorCount} followed authors
								</button>
							{/if}
							<button
								class="fetch-relay-btn"
								disabled={fetchingRelay === '__sections__'}
								onclick={async () => {
									fetchingRelay = '__sections__';
									await onfetchsections?.();
									fetchingRelay = null;
								}}
							>
								{fetchingRelay === '__sections__' ? '...' : '↻'} Fetch missing sections
							</button>
							<div class="fetch-custom">
								<input
									type="text"
									bind:value={customRelayUrl}
									placeholder="wss://relay.example.com"
									class="fetch-input"
								/>
								<input
									type="text"
									bind:value={customKinds}
									placeholder="30040,30041"
									class="fetch-kinds-input"
									title="Event kinds to fetch"
								/>
								<button
									class="fetch-go-btn"
									disabled={!customRelayUrl.trim() || fetchingRelay === customRelayUrl}
									onclick={async () => {
										const url = customRelayUrl.trim();
										if (!url) return;
										fetchingRelay = url;
										const kinds = customKinds.split(',').map(k => parseInt(k.trim())).filter(k => !isNaN(k));
										await onfetchfromrelay?.(url, kinds);
										fetchingRelay = null;
									}}
								>Fetch</button>
								<button
									class="fetch-go-btn fetch-save-btn"
									disabled={!customRelayUrl.trim()}
									onclick={async () => {
										const url = customRelayUrl.trim();
										if (!url) return;
										await api.addRelay('fetch', url);
										customRelayUrl = '';
									}}
									title="Save relay to config.toml"
								>+</button>
							</div>
							<div class="fetch-custom">
								<input
									type="text"
									placeholder="npub1... or hex pubkey"
									class="fetch-input"
									onkeydown={async (e) => {
										if (e.key !== 'Enter') return;
										const input = e.currentTarget as HTMLInputElement;
										const val = input.value.trim();
										if (!val) return;
										await api.addAuthor(val);
										input.value = '';
									}}
								/>
								<span class="fetch-hint">Enter to add author</span>
							</div>
						</div>
					{/if}
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
								{#if localPubkeys?.has(pub_item.author_pubkey)}
									<span class="local-badge">local</span>
								{/if}
								<span class="feed-item-meta">{pub_item.section_count} sections</span>
								<div class="feed-menu-container">
									<button class="feed-menu-btn" onclick={(e) => { e.stopPropagation(); feedMenuOpen = feedMenuOpen === feedKey ? null : feedKey; }} title="More">⋮</button>
									{#if feedMenuOpen === feedKey}
										<!-- svelte-ignore a11y_click_events_have_key_events -->
										<div class="feed-menu-backdrop" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; }} role="presentation"></div>
										<div class="feed-menu-dropdown">
											<button class="feed-menu-item" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; onignoreevent?.(`${pub_item.addr.kind}:${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`); }}>Hide publication</button>
											<button class="feed-menu-item feed-menu-danger" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; onignorepubkey?.(pub_item.author_pubkey); }}>Hide author</button>
										</div>
									{/if}
								</div>
							</div>
							{#if pub_item.summary}
								<p class="feed-item-summary">{pub_item.summary}</p>
							{/if}
							<div class="feed-item-footer">
								<span class="feed-item-author"><ProfileName pubkey={pub_item.author_pubkey} {onviewprofile} /></span>
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
		{:else if docMode === 'profile' && profilePubkey}
			<ProfileView
				pubkey={profilePubkey}
				{onopenpub}
				onback={() => onviewprofile?.('')}
			/>
		{:else if docMode === 'ignored'}
			<div class="ignored-view">
				<div class="ignored-header">
					<span>Hidden ({ignoredEventIds.length} events, {ignoredPubkeys.length} authors)</span>
				</div>
				{#if ignoredEventIds.length > 0}
					<div class="ignored-section-title">Events</div>
					{#each ignoredEventIds as id}
						<div class="ignored-item">
							<span class="ignored-id">{id.slice(0, 16)}...{id.slice(-8)}</span>
							<button class="unignore-btn" onclick={() => onunignore?.('event', id)}>Unblock</button>
						</div>
					{/each}
				{/if}
				{#if ignoredPubkeys.length > 0}
					<div class="ignored-section-title">Authors</div>
					{#each ignoredPubkeys as pk}
						<div class="ignored-item">
							<span class="ignored-id">{pk.slice(0, 16)}...{pk.slice(-8)}</span>
							<button class="unignore-btn" onclick={() => onunignore?.('pubkey', pk)}>Unblock</button>
						</div>
					{/each}
				{/if}
				{#if ignoredEventIds.length === 0 && ignoredPubkeys.length === 0}
					<div class="doc-empty"><p>No hidden events or authors</p></div>
				{/if}
			</div>
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

	.doc-author-bar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 16px;
		border-bottom: 1px solid var(--border);
		cursor: pointer;
	}

	.doc-author-bar:hover {
		background: var(--bg-surface);
	}

	.doc-author-avatar {
		width: 28px;
		height: 28px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.doc-author-avatar.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: 0.75rem;
	}

	.doc-author-name {
		font-size: 0.85rem;
	}

	.doc-author-time {
		font-size: 0.75rem;
		color: var(--fg-muted);
		margin-left: auto;
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

	.local-badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 3px;
		background: #f9731633;
		color: #f97316;
		white-space: nowrap;
		font-weight: 600;
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

	/* Fetch panel */

	.fetch-panel {
		padding: 6px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.fetch-relay-btn {
		text-align: left;
		font-size: 0.7rem;
		padding: 4px 8px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg);
		cursor: pointer;
	}

	.fetch-relay-btn:hover {
		background: var(--bg);
		border-color: var(--accent);
	}

	.fetch-authors-btn {
		border-color: var(--accent);
		color: var(--accent);
	}

	.fetch-custom {
		display: flex;
		gap: 4px;
		margin-top: 2px;
	}

	.fetch-input {
		flex: 1;
		font-size: 0.7rem;
		padding: 4px 6px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg);
		color: var(--fg);
		font-family: var(--font-mono);
	}

	.fetch-kinds-input {
		width: 80px;
		font-size: 0.7rem;
		padding: 4px 6px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg);
		color: var(--fg);
		font-family: var(--font-mono);
	}

	.fetch-go-btn {
		font-size: 0.7rem;
		padding: 4px 10px;
	}

	.fetch-save-btn {
		font-size: 0.8rem;
		min-width: 24px;
		color: var(--accent);
	}

	.fetch-hint {
		font-size: 0.6rem;
		color: var(--fg-muted);
		white-space: nowrap;
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

	/* Ignored view */

	.ignored-view {
		flex: 1;
		overflow-y: auto;
	}

	.ignored-header {
		padding: 10px 16px;
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--border);
	}

	.ignored-section-title {
		padding: 8px 16px 4px;
		font-size: 0.7rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
	}

	.ignored-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 6px 16px;
		border-bottom: 1px solid var(--border);
	}

	.ignored-id {
		font-size: 0.75rem;
		font-family: var(--font-mono);
		color: var(--fg-muted);
	}

	.unignore-btn {
		font-size: 0.7rem;
		padding: 2px 8px;
		color: var(--accent);
		background: none;
		border: 1px solid var(--accent);
		border-radius: var(--radius);
		cursor: pointer;
	}

	.unignore-btn:hover {
		background: var(--accent);
		color: white;
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
