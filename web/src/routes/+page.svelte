<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import { goto } from '$app/navigation';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import * as api from '$lib/api';
	import { untrack } from 'svelte';

	const app = getAppState();

	$effect(() => {
		untrack(() => {
			app.loadFeed();
		});
	});

	let feedMenuOpen: string | null = $state(null);
	let fetchPanelOpen = $state(false);
	let customRelayUrl = $state('');
	let customKinds = $state('30040,30041');
	let fetchingRelay: string | null = $state(null);

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	function openPub(pubkey: string, d_tag: string) {
		goto(`/p/${pubkey}/${d_tag}`);
	}
</script>

<svelte:head><title>tendrl</title></svelte:head>

<div class="document-panel">
	<div class="doc-toolbar">
		<div class="doc-toolbar-left"></div>
		<div class="doc-toolbar-right">
			<button onclick={() => app.handleCompose()}>Compose</button>
		</div>
	</div>

	<div class="doc-content">
		{#if app.feedLoading}
			<div class="doc-empty"><p>Loading publications...</p></div>
		{:else if app.feed.length > 0}
			<div class="feed-list">
				<div class="feed-header">
					<span>Publications ({app.feed.length})</span>
					<button class="feed-sync-btn" onclick={app.handleFeedSync} disabled={app.feedSyncing}>
						{app.feedSyncing ? 'Syncing...' : 'Sync all'}
					</button>
					<button class="feed-sync-btn" onclick={() => (fetchPanelOpen = !fetchPanelOpen)}>
						{fetchPanelOpen ? '✕' : 'Fetch'}
					</button>
				</div>
				{#if fetchPanelOpen}
					<div class="fetch-panel">
						{#each app.fetchRelayUrls as relay}
							<button
								class="fetch-relay-btn"
								disabled={fetchingRelay === relay}
								onclick={async () => {
									fetchingRelay = relay;
									const kinds = customKinds.split(',').map(k => parseInt(k.trim())).filter(k => !isNaN(k));
									await app.handleFetchFromRelay(relay, kinds);
									fetchingRelay = null;
								}}
							>
								{fetchingRelay === relay ? '...' : '↻'} {relay.replace('wss://', '').replace('ws://', '')}
							</button>
						{/each}
						{#if app.authorCount > 0}
							<button
								class="fetch-relay-btn fetch-authors-btn"
								disabled={fetchingRelay === '__authors__'}
								onclick={async () => {
									fetchingRelay = '__authors__';
									await app.handleFetchAuthors();
									fetchingRelay = null;
								}}
							>
								{fetchingRelay === '__authors__' ? '...' : '↻'} Fetch {app.authorCount} followed authors
							</button>
						{/if}
						<button
							class="fetch-relay-btn"
							disabled={fetchingRelay === '__sections__'}
							onclick={async () => {
								fetchingRelay = '__sections__';
								await app.handleFetchSections();
								fetchingRelay = null;
							}}
						>
							{fetchingRelay === '__sections__' ? '...' : '↻'} Fetch missing sections
						</button>
						<div class="fetch-custom">
							<input type="text" bind:value={customRelayUrl} placeholder="wss://relay.example.com" class="fetch-input" />
							<input type="text" bind:value={customKinds} placeholder="30040,30041" class="fetch-kinds-input" title="Event kinds to fetch" />
							<button
								class="fetch-go-btn"
								disabled={!customRelayUrl.trim() || fetchingRelay === customRelayUrl}
								onclick={async () => {
									const url = customRelayUrl.trim();
									if (!url) return;
									fetchingRelay = url;
									const kinds = customKinds.split(',').map(k => parseInt(k.trim())).filter(k => !isNaN(k));
									await app.handleFetchFromRelay(url, kinds);
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
				{#each app.feed as pub_item (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
					{@const feedKey = `${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="feed-item"
						onclick={() => openPub(pub_item.addr.pubkey, pub_item.addr.d_tag)}
						onkeydown={(e) => { if (e.key === 'Enter') openPub(pub_item.addr.pubkey, pub_item.addr.d_tag); }}
						role="button"
						tabindex="0"
					>
						<div class="feed-item-header">
							<span class="feed-item-title">{pub_item.title ?? '[Untitled]'}</span>
							{#if app.localPubkeys.has(pub_item.author_pubkey)}
								<span class="local-badge">local</span>
							{/if}
							<span class="feed-item-meta">{pub_item.section_count} sections</span>
							<div class="feed-menu-container">
								<button class="feed-menu-btn" onclick={(e) => { e.stopPropagation(); feedMenuOpen = feedMenuOpen === feedKey ? null : feedKey; }} title="More">⋮</button>
								{#if feedMenuOpen === feedKey}
									<!-- svelte-ignore a11y_click_events_have_key_events -->
									<div class="feed-menu-backdrop" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; }} role="presentation"></div>
									<div class="feed-menu-dropdown">
										<button class="feed-menu-item" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; app.ignoreEvent(`${pub_item.addr.kind}:${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`); }}>Hide publication</button>
										<button class="feed-menu-item feed-menu-danger" onclick={(e) => { e.stopPropagation(); feedMenuOpen = null; app.ignorePubkey(pub_item.author_pubkey); }}>Hide author</button>
									</div>
								{/if}
							</div>
						</div>
						{#if pub_item.summary}
							<p class="feed-item-summary">{pub_item.summary}</p>
						{/if}
						<div class="feed-item-footer">
							<span class="feed-item-author"><ProfileName pubkey={pub_item.author_pubkey} onviewprofile={(pk) => goto(`/profile/${pk}`)} /></span>
							<span class="feed-item-time">{formatTime(pub_item.created_at)}</span>
						</div>
					</div>
				{/each}
				{#if app.feedHasMore}
					<div class="feed-load-more">
						<button onclick={app.handleFeedLoadMore} disabled={app.feedLoadingMore}>
							{app.feedLoadingMore ? 'Loading...' : 'Load more'}
						</button>
					</div>
				{/if}
			</div>
		{:else}
			<div class="doc-empty">
				<div class="empty-actions">
					<p>No publications found locally.</p>
					<button onclick={app.handleFeedSync} disabled={app.feedSyncing}>
						{app.feedSyncing ? 'Syncing...' : 'Fetch from relays'}
					</button>
				</div>
			</div>
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

	.doc-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		gap: 8px;
	}

	.doc-toolbar-left,
	.doc-toolbar-right {
		display: flex;
		gap: 6px;
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

	.feed-list { flex: 1; overflow-y: auto; }

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

	.feed-item:hover { background: var(--bg-surface); }

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

	.fetch-relay-btn:hover { background: var(--bg); border-color: var(--accent); }
	.fetch-authors-btn { border-color: var(--accent); color: var(--accent); }

	.fetch-custom { display: flex; gap: 4px; margin-top: 2px; }

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

	.fetch-go-btn { font-size: 0.7rem; padding: 4px 10px; }
	.fetch-save-btn { font-size: 0.8rem; min-width: 24px; color: var(--accent); }
	.fetch-hint { font-size: 0.6rem; color: var(--fg-muted); white-space: nowrap; }

	.feed-menu-container { position: relative; }
	.feed-menu-btn {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: 0.9rem;
		padding: 0 4px;
		line-height: 1;
	}
	.feed-menu-btn:hover { color: var(--fg); }
	.feed-menu-backdrop { position: fixed; inset: 0; z-index: 50; }
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
	.feed-menu-item:hover { background: var(--bg-surface); }
	.feed-menu-danger { color: #ef4444; }
	.feed-menu-danger:hover { background: #ef444415; }

	.feed-load-more { padding: 12px 16px; text-align: center; }
	.feed-load-more button { font-size: 0.8rem; padding: 6px 20px; }
</style>
