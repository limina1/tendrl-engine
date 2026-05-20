<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	let cursor = $state(0);
	let listEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		untrack(() => {
			app.loadFeed();
		});
	});

	$effect(() => {
		// Clamp cursor when feed length changes.
		if (cursor >= app.feed.length) cursor = Math.max(0, app.feed.length - 1);
	});

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	/** Strip the scheme and trailing slash from a relay URL for compact display. */
	function relayHost(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/+$/, '');
	}

	/** Compact label for a relay set: "relay.damus.io +2". */
	function relayLabel(relays: string[]): string {
		const first = relayHost(relays[0]);
		return relays.length > 1 ? `${first} +${relays.length - 1}` : first;
	}

	function openPub(pub: { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null; section_count: number }) {
		const id = `reader:${pub.addr.kind}:${pub.addr.pubkey}:${pub.addr.d_tag}`;
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

	function scrollCursorIntoView() {
		if (!listEl) return;
		const row = listEl.querySelector<HTMLDivElement>(`.row[data-cursor="${cursor}"]`);
		if (!row) return;
		// Bounds-check rather than scrollIntoView({block:'nearest'}) — that
		// API tends to nudge the viewport on every keystroke. Here the
		// scrollbar only moves when the cursor would actually leave the
		// visible area, so j/k moves the selection within the visible
		// list and only scrolls at the edges.
		const listRect = listEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			listEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			listEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function handleNav(action: NavAction): boolean {
		const total = app.feed.length;
		if (total === 0) return false;
		if (action === 'down') {
			cursor = Math.min(total - 1, cursor + 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'up') {
			cursor = Math.max(0, cursor - 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'top') {
			cursor = 0;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'bottom') {
			cursor = total - 1;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'select' || action === 'right') {
			openPub(app.feed[cursor]);
			return true;
		}
		return false;
	}

	// $effect rather than onMount: with our `{#key buffer.id}{#if kind}`
	// dispatch in BufferRenderer, onMount didn't fire reliably. $effect
	// always fires during reactive setup. _navHandlers is non-reactive
	// so this can't loop.
	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});
</script>

<div class="feed-wrap">
	{#if app.feedLoading}
		<div class="empty"><p>Loading publications…</p></div>
	{:else if app.feed.length > 0}
		<div class="feed-list" bind:this={listEl}>
			<div class="feed-header">
				<span>Publications ({app.feed.length})</span>
				<button class="sync" onclick={app.handleFeedSync} disabled={app.feedSyncing}>
					{app.feedSyncing ? 'Syncing…' : 'Sync all'}
				</button>
			</div>
			{#each app.feed as pub_item, i (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div
					class="row"
					class:row--cursor={i === cursor}
					data-cursor={i}
					onclick={() => { cursor = i; openPub(pub_item); }}
					onkeydown={(e) => {
						if (e.key === 'Enter') openPub(pub_item);
						else if (e.key === 'm') app.openAddressableInModal(pub_item.addr);
					}}
					role="button"
					tabindex="0"
				>
					<span class="cursor-marker" aria-hidden="true">{i === cursor ? '›' : ' '}</span>
					<div class="row-body">
					<div class="row-head">
						<span class="title">{pub_item.title ?? '[Untitled]'}</span>
						<button
							class="pill pill--menu"
							onclick={(e) => {
								e.stopPropagation();
								app.openAddressableInModal(pub_item.addr);
							}}
							title="Open the event menu (m)"
						>menu</button>
						<!-- Provenance badge. The current corpus is an initial bulk
						     import from relays, so an event with no recorded relay
						     metadata is treated as remote, not local. A genuine
						     "local" state (written here, not yet published) needs
						     local-write tracking — deferred until local authoring
						     is actually a feature. -->
						{#if !pub_item.signed}
							<span class="pill pill--draft" title="Unsigned draft — not yet signed">draft</span>
						{:else if pub_item.relays.length > 0}
							<span class="pill pill--remote" title={`On ${pub_item.relays.length} relay(s):\n${pub_item.relays.join('\n')}`}>{relayLabel(pub_item.relays)}</span>
						{:else}
							<span class="pill pill--remote" title="From relays — origin relay not recorded">remote</span>
						{/if}
						<span class="meta">{pub_item.section_count} sections</span>
					</div>
					{#if pub_item.summary}
						<p class="summary">{pub_item.summary}</p>
					{/if}
					<div class="row-foot">
						<span class="author"><ProfileName pubkey={pub_item.author_pubkey} onviewprofile={app.handleViewProfile} /></span>
						<span class="time">{formatTime(pub_item.created_at)}</span>
					</div>
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
		display: flex;
		align-items: flex-start;
		gap: 6px;
	}
	.row:hover { background: var(--panel-bg-soft); }
	.row-body { flex: 1; min-width: 0; }
	/* ranger-style selection: high-contrast bar + leading caret. The
	   highlight is bright enough to read from a glance even with the
	   list scrolling. */
	.row--cursor {
		background: color-mix(in srgb, var(--id-yours) 28%, transparent);
		border-left-color: var(--id-yours);
		border-left-width: 5px;
		padding-left: 10px;
	}
	.row--cursor .title { color: var(--fg); font-weight: 700; }
	.cursor-marker {
		font-family: var(--font-mono);
		font-weight: 700;
		color: var(--id-yours);
		min-width: 10px;
		line-height: 1.2;
		font-size: var(--t-sm);
	}
	.row:not(.row--cursor) .cursor-marker { color: transparent; }
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
