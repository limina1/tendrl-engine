<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	$effect(() => {
		untrack(() => {
			app.loadFeed();
		});
	});

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	function openPub(pub: { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null; section_count: number }) {
		const id = `reader:${pub.addr.kind}:${pub.addr.pubkey}:${pub.addr.d_tag}`;
		// Switch to read layout first so the reader lands in a wide-center work slot,
		// then open the buffer (setLayout resets leaf overrides, so order matters).
		store.setLayout('read');
		store.openBuffer({
			className: 'work',
			buffer: {
				id,
				kind: 'reader',
				label: 'reader',
				kicker: pub.title ?? '[Untitled]'
			}
		});
	}
</script>

<div class="feed-wrap">
	{#if app.feedLoading}
		<div class="empty"><p>Loading publications…</p></div>
	{:else if app.feed.length > 0}
		<div class="feed-list">
			<div class="feed-header">
				<span>Publications ({app.feed.length})</span>
				<button class="sync" onclick={app.handleFeedSync} disabled={app.feedSyncing}>
					{app.feedSyncing ? 'Syncing…' : 'Sync all'}
				</button>
			</div>
			{#each app.feed as pub_item (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div
					class="row"
					onclick={() => openPub(pub_item)}
					onkeydown={(e) => { if (e.key === 'Enter') openPub(pub_item); }}
					role="button"
					tabindex="0"
				>
					<div class="row-head">
						<span class="title">{pub_item.title ?? '[Untitled]'}</span>
						{#if app.localPubkeys.has(pub_item.author_pubkey)}
							<span class="pill pill--local">local</span>
						{/if}
						<span class="meta">{pub_item.section_count} sections</span>
					</div>
					{#if pub_item.summary}
						<p class="summary">{pub_item.summary}</p>
					{/if}
					<div class="row-foot">
						<span class="author"><ProfileName pubkey={pub_item.author_pubkey} onviewprofile={(_pk) => {}} /></span>
						<span class="time">{formatTime(pub_item.created_at)}</span>
					</div>
				</div>
			{/each}
			{#if app.feedHasMore}
				<div class="more">
					<button onclick={app.handleFeedLoadMore} disabled={app.feedLoadingMore}>
						{app.feedLoadingMore ? 'Loading…' : 'Load more'}
					</button>
				</div>
			{/if}
		</div>
	{:else}
		<div class="empty">
			<p>No publications found locally.</p>
			<button onclick={app.handleFeedSync} disabled={app.feedSyncing}>
				{app.feedSyncing ? 'Syncing…' : 'Fetch from relays'}
			</button>
		</div>
	{/if}
</div>

<style>
	.feed-wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.feed-list { flex: 1; overflow-y: auto; }
	.feed-header {
		padding: 8px 12px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.sync {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.sync:hover:not(:disabled) { color: var(--fg); border-color: var(--base4); }
	.row {
		padding: 8px 12px;
		border-bottom: 1px solid var(--panel-border);
		cursor: pointer;
		border-left: 3px solid var(--id-remote);
	}
	.row:hover { background: var(--panel-bg-soft); }
	.row-head { display: flex; align-items: center; gap: 8px; margin-bottom: 2px; }
	.title { font-size: var(--t-sm); font-weight: 600; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.meta { font-size: var(--t-xs); color: var(--base5); white-space: nowrap; }
	.summary {
		font-size: var(--t-xs);
		color: var(--base6);
		line-height: var(--lh-snug);
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		-webkit-box-orient: vertical;
	}
	.row-foot {
		display: flex;
		gap: 8px;
		font-size: var(--t-xs);
		color: var(--base5);
		margin-top: 4px;
	}
	.empty {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		color: var(--base5);
		font-size: var(--t-sm);
		gap: 8px;
	}
	.empty button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 12px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
	.more { padding: 12px; text-align: center; }
	.more button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 16px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
</style>
