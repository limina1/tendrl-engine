<script lang="ts">
	import type { NostrEvent, PublicationSummary } from '$lib/types';
	import * as api from '$lib/api';
	import type { Profile } from '$lib/api';
	import ProfileName from './ProfileName.svelte';

	let {
		pubkey,
		onopenpub,
		onback
	}: {
		pubkey: string;
		onopenpub?: (pub_summary: PublicationSummary) => void;
		onback: () => void;
	} = $props();

	type Tab = 'publications' | 'sections' | 'comments';
	let activeTab: Tab = $state('publications');
	let profile = $state<Profile | null>(null);
	let publications = $state<PublicationSummary[]>([]);
	let sections = $state<NostrEvent[]>([]);
	let comments = $state<NostrEvent[]>([]);
	let loading = $state(true);
	let fetching = $state(false);

	function getTag(event: NostrEvent, name: string): string | null {
		const tag = event.tags.find(t => t[0] === name);
		return tag ? tag[1] : null;
	}

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	async function loadLocal(pk: string) {
		const [prof, pubResult, secResult, comResult] = await Promise.all([
			api.getProfile(pk),
			api.queryEvents([{ kinds: [30040], authors: [pk], limit: 500 }], 'local_only'),
			api.queryEvents([{ kinds: [30041], authors: [pk], limit: 200 }], 'local_only'),
			api.queryEvents([{ kinds: [1111], authors: [pk], limit: 200 }], 'local_only')
		]);
		profile = prof.found ? prof : null;
		// Convert raw 30040 events to PublicationSummary, deduplicate by d_tag (keep newest)
		const byDtag = new Map<string, PublicationSummary>();
		for (const e of (pubResult.events as NostrEvent[])) {
			const d_tag = getTag(e, 'd') || '';
			const existing = byDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			byDtag.set(d_tag, {
				addr: { kind: 30040, pubkey: e.pubkey, d_tag },
				title: getTag(e, 'title'),
				summary: getTag(e, 'summary'),
				image: getTag(e, 'image'),
				author_pubkey: e.pubkey,
				version: null,
				created_at: e.created_at,
				section_count: e.tags.filter(t => t[0] === 'a').length
			} as PublicationSummary);
		}
		publications = [...byDtag.values()].sort((a, b) => b.created_at - a.created_at);
		sections = (secResult.events as NostrEvent[]).sort((a, b) => b.created_at - a.created_at);
		comments = (comResult.events as NostrEvent[]).sort((a, b) => b.created_at - a.created_at);
	}

	async function handleFetch() {
		fetching = true;
		try {
			// Fetch this author's events from all configured fetch relays
			const rc = await api.getRelayConfig();
			const kinds = rc.fetch.kinds.length > 0 ? rc.fetch.kinds : [0, 30040, 30041, 1111];
			for (const relay of rc.fetch.urls) {
				try {
					await api.fetchFromRelay(relay, kinds, [pubkey], 500);
				} catch {}
			}
			// Also fetch profile from general relays
			await api.prefetchProfiles([pubkey]);
			// Wait for nostrdb to process ingested events
			await new Promise(r => setTimeout(r, 500));
			// Reload local data
			await loadLocal(pubkey);
		} catch (e) {
			console.error('Fetch failed:', e);
		} finally {
			fetching = false;
		}
	}

	$effect(() => {
		const pk = pubkey;
		loading = true;
		profile = null;
		publications = [];
		sections = [];
		comments = [];

		loadLocal(pk).catch(() => {}).finally(() => { loading = false; });
	});
</script>

<div class="profile-view">
	<div class="profile-bar">
		<button class="back-btn" onclick={onback}>&larr;</button>
		{#if profile?.picture}
			<img class="avatar" src={profile.picture} alt="" />
		{:else}
			<div class="avatar placeholder">?</div>
		{/if}
		<div class="identity">
			<span class="name">{profile?.display_name || profile?.name || pubkey.slice(0, 12) + '...'}</span>
			{#if profile?.about}
				<span class="about">{profile.about}</span>
			{/if}
		</div>
		<span class="bar-spacer"></span>
		<button class="fetch-btn" onclick={handleFetch} disabled={fetching} title="Fetch this author's events from relays">
			{fetching ? 'Fetching...' : '↻ Fetch'}
		</button>
	</div>

	<div class="tabs">
		<button class="tab" class:active={activeTab === 'publications'} onclick={() => activeTab = 'publications'}>
			Publications ({publications.length})
		</button>
		<button class="tab" class:active={activeTab === 'sections'} onclick={() => activeTab = 'sections'}>
			Sections ({sections.length})
		</button>
		<button class="tab" class:active={activeTab === 'comments'} onclick={() => activeTab = 'comments'}>
			Comments ({comments.length})
		</button>
	</div>

	<div class="tab-content">
		{#if loading}
			<div class="empty">Loading...</div>
		{:else if activeTab === 'publications'}
			{#if publications.length === 0}
				<div class="empty">No publications</div>
			{:else}
				{#each publications as pub_item (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						onclick={() => onopenpub?.(pub_item)}
						onkeydown={(e) => { if (e.key === 'Enter') onopenpub?.(pub_item); }}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							<span class="item-title">{pub_item.title ?? '[Untitled]'}</span>
							<span class="item-meta">{pub_item.section_count} sections</span>
						</div>
						{#if pub_item.summary}
							<p class="item-preview">{pub_item.summary}</p>
						{/if}
						<span class="item-time">{formatTime(pub_item.created_at)}</span>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'sections'}
			{#if sections.length === 0}
				<div class="empty">No sections</div>
			{:else}
				{#each sections as sec (sec.id)}
					{@const title = getTag(sec, 'title') || getTag(sec, 'd') || '[Untitled]'}
					{@const parentAddr = getTag(sec, 'a')}
					<div class="item">
						<div class="item-header">
							<span class="item-title">{title}</span>
						</div>
						{#if sec.content}
							<p class="item-preview">{sec.content.slice(0, 200)}</p>
						{/if}
						<div class="item-footer">
							{#if parentAddr}
								<span class="item-ref">{parentAddr.split(':').pop()}</span>
							{/if}
							<span class="item-time">{formatTime(sec.created_at)}</span>
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'comments'}
			{#if comments.length === 0}
				<div class="empty">No comments</div>
			{:else}
				{#each comments as comment (comment.id)}
					{@const rootAddr = getTag(comment, 'A') || getTag(comment, 'E') || getTag(comment, 'I')}
					{@const rootKind = getTag(comment, 'K')}
					<div class="item">
						<div class="item-header">
							{#if rootAddr}
								<span class="item-ref">on {rootKind ? `k:${rootKind}` : ''} {rootAddr.split(':').pop()}</span>
							{/if}
						</div>
						<p class="item-content">{comment.content}</p>
						<span class="item-time">{formatTime(comment.created_at)}</span>
					</div>
				{/each}
			{/if}
		{/if}
	</div>
</div>

<style>
	.profile-view {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.profile-bar {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
	}

	.back-btn {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: 1.1rem;
		cursor: pointer;
		padding: 2px 6px;
	}

	.back-btn:hover {
		color: var(--fg);
	}

	.avatar {
		width: 36px;
		height: 36px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.avatar.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: 1rem;
	}

	.identity {
		min-width: 0;
		display: flex;
		flex-direction: column;
	}

	.name {
		font-weight: 600;
		font-size: 0.95rem;
	}

	.about {
		font-size: 0.75rem;
		color: var(--fg-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.bar-spacer {
		flex: 1;
	}

	.fetch-btn {
		font-size: 0.7rem;
		padding: 4px 10px;
		background: none;
		border: 1px solid var(--accent);
		color: var(--accent);
		border-radius: var(--radius);
		cursor: pointer;
		white-space: nowrap;
	}

	.fetch-btn:hover:not(:disabled) {
		background: var(--accent);
		color: white;
	}

	.fetch-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}

	.tabs {
		display: flex;
		border-bottom: 1px solid var(--border);
	}

	.tab {
		flex: 1;
		padding: 8px 12px;
		font-size: 0.75rem;
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--fg-muted);
		cursor: pointer;
		text-align: center;
	}

	.tab:hover {
		color: var(--fg);
	}

	.tab.active {
		color: var(--fg);
		border-bottom-color: var(--accent);
	}

	.tab-content {
		flex: 1;
		overflow-y: auto;
	}

	.empty {
		padding: 24px;
		text-align: center;
		color: var(--fg-muted);
		font-size: 0.85rem;
	}

	.item {
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
	}

	.pub-item {
		cursor: pointer;
		border-left: 3px solid #3b82f6;
	}

	.pub-item:hover {
		background: var(--bg-surface);
	}

	.item-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin-bottom: 2px;
	}

	.item-title {
		font-size: 0.9rem;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		flex: 1;
	}

	.item-meta {
		font-size: 0.7rem;
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.item-preview {
		font-size: 0.8rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		-webkit-box-orient: vertical;
	}

	.item-content {
		font-size: 0.85rem;
		line-height: 1.5;
		margin: 4px 0;
		white-space: pre-wrap;
		word-break: break-word;
	}

	.item-footer {
		display: flex;
		gap: 8px;
		align-items: center;
	}

	.item-ref {
		font-size: 0.7rem;
		color: var(--accent);
		font-family: var(--font-mono);
	}

	.item-time {
		font-size: 0.7rem;
		color: var(--fg-muted);
	}
</style>
