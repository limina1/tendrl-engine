<script lang="ts">
	import { untrack } from 'svelte';
	import { isEventSigned, type NostrEvent, type PublicationSummary } from '$lib/types';
	import * as api from '$lib/api';
	import type { Profile } from '$lib/api';
	import { fetchFromRelaysWithPrompt } from '$lib/fetch/relay-fetch.svelte';
	import { getAppState } from '$lib/state.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';
	import { getActiveStore, type NavAction } from '$lib/wm/buffer-store.svelte';

	const app = getAppState();
	const store = getActiveStore();

	let {
		pubkey,
		bufferId,
		onopenpub,
		onopenaddr,
		oncomment,
		onback
	}: {
		pubkey: string;
		/** Owning buffer id — used to register a nav handler so j/k/Enter/m
		 *  work in this view via the global keymap. Optional so direct
		 *  embeddings (outside a WM buffer) still work without nav. */
		bufferId?: string;
		onopenpub?: (pub_summary: PublicationSummary) => void;
		/** Open any non-30040 addressable (article, wiki, section, etc.) in
		 *  the reader. The buffer-id pattern reader:&lt;kind&gt;:&lt;pk&gt;:&lt;dtag&gt; works
		 *  for these uniformly so the host can route them without caring
		 *  about kind-specific layout. */
		onopenaddr?: (addr: { kind: number; pubkey: string; d_tag: string }, title: string | null) => void;
		/** Open a NIP-22 comment (kind 1111) in its discussion view — the
		 *  comment isn't a standalone reader destination, so the host routes
		 *  it to a DiscussionViewBuffer that resolves the thread context. */
		oncomment?: (event: NostrEvent) => void;
		onback: () => void;
	} = $props();

	type Tab = 'publications' | 'articles' | 'wikis' | 'sections' | 'comments';
	let activeTab: Tab = $state('publications');
	let profile = $state<Profile | null>(null);
	let publications = $state<PublicationSummary[]>([]);
	// NIP-23 long-form articles (kind 30023) and NKBIP-02 wikis (kind
	// 30818). Both are addressable replaceable events, deduped by d_tag
	// keeping the newest version.
	type AddressableSummary = {
		addr: { kind: number; pubkey: string; d_tag: string };
		title: string | null;
		summary: string | null;
		image: string | null;
		created_at: number;
		/** Provenance — same fields PublicationSummary already carries.
		 *  Threaded through so the draft / remote / relay-label pill lights
		 *  up on articles + wikis the same way it does on publications. */
		signed: boolean;
		relays: string[];
	};
	let articles = $state<AddressableSummary[]>([]);
	let wikis = $state<AddressableSummary[]>([]);
	let sections = $state<NostrEvent[]>([]);
	let comments = $state<NostrEvent[]>([]);
	let loading = $state(true);
	let fetching = $state(false);

	function getTag(event: NostrEvent, name: string): string | null {
		const tag = event.tags.find(t => t[0] === name);
		return tag ? tag[1] : null;
	}

	/** Detect a NIP-54 fork marker on a kind-30040 index event: an `a` or
	 *  `e` tag whose 4th element is the literal "fork". Mirrors the engine
	 *  detection in `Publication::from_event` so client-built summaries
	 *  agree with server-derived ones. */
	function hasForkMarker(event: NostrEvent): boolean {
		return event.tags.some(t => (t[0] === 'a' || t[0] === 'e') && t[3] === 'fork');
	}

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	function dedupAddressable(events: NostrEvent[], kind: number): AddressableSummary[] {
		// Replaceable events: same (kind, pubkey, d_tag) → latest version
		// (highest created_at) wins. Same dedup the publication path uses,
		// generalized so 30023 and 30818 reuse it.
		const byDtag = new Map<string, AddressableSummary>();
		for (const e of events) {
			const d_tag = getTag(e, 'd') || '';
			const existing = byDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			byDtag.set(d_tag, {
				addr: { kind, pubkey: e.pubkey, d_tag },
				title: getTag(e, 'title'),
				summary: getTag(e, 'summary'),
				image: getTag(e, 'image'),
				created_at: e.created_at,
				signed: isEventSigned(e.sig),
				relays: e.relays ?? []
			});
		}
		return [...byDtag.values()].sort((a, b) => b.created_at - a.created_at);
	}

	async function loadLocal(pk: string) {
		const [prof, pubResult, artResult, wikiResult, secResult, comResult] = await Promise.all([
			api.getProfile(pk),
			api.queryEvents([{ kinds: [30040], authors: [pk], limit: 500 }], 'local_only'),
			api.queryEvents([{ kinds: [30023], authors: [pk], limit: 200 }], 'local_only'),
			api.queryEvents([{ kinds: [30818], authors: [pk], limit: 200 }], 'local_only'),
			api.queryEvents([{ kinds: [30041], authors: [pk], limit: 200 }], 'local_only'),
			api.queryEvents([{ kinds: [1111], authors: [pk], limit: 200 }], 'local_only')
		]);
		profile = prof.found ? prof : null;
		// 30040 publications: same dedup, but kept as the existing
		// PublicationSummary shape so the openpub callback contract
		// downstream (and the section_count display) is preserved.
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
				// A fork-marker `a` tag is not a content reference — strip
				// it out of section_count so the displayed count matches
				// the engine's (Publication::from_event applies the same
				// filter via the fork-marker branch in its tag loop).
				section_count: e.tags.filter(t => t[0] === 'a' && t[3] !== 'fork').length,
				relays: e.relays ?? [],
				signed: isEventSigned(e.sig),
				forked: hasForkMarker(e)
			} as PublicationSummary);
		}
		publications = [...byDtag.values()].sort((a, b) => b.created_at - a.created_at);
		articles = dedupAddressable(artResult.events as NostrEvent[], 30023);
		wikis = dedupAddressable(wikiResult.events as NostrEvent[], 30818);
		// Sections are replaceable (kind 30041) — collapse versions by
		// d-tag, newest wins, so a section isn't listed once per edit.
		const secByDtag = new Map<string, NostrEvent>();
		for (const e of (secResult.events as NostrEvent[])) {
			const d_tag = getTag(e, 'd') || '';
			const existing = secByDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			secByDtag.set(d_tag, e);
		}
		sections = [...secByDtag.values()].sort((a, b) => b.created_at - a.created_at);
		comments = (comResult.events as NostrEvent[]).sort((a, b) => b.created_at - a.created_at);
	}

	// Tab → which event kinds to pull. The top-bar Fetch button pulls
	// the union; per-tab refresh buttons scope to a single kind so the
	// user can do targeted refreshes without hammering relays for
	// everything every time.
	const TAB_KINDS: Record<Tab, number[]> = {
		publications: [30040],
		articles: [30023],
		wikis: [30818],
		sections: [30041],
		comments: [1111]
	};
	const TAB_LABEL: Record<Tab, string> = {
		publications: 'publications',
		articles: 'articles',
		wikis: 'wikis',
		sections: 'sections',
		comments: 'comments'
	};

	let tabFetchingKinds = $state<number | null>(null);

	const isOnline = $derived(app.networkStatus?.mode === 'auto');

	async function runFetch(opts: { title: string; kinds: number[] }) {
		console.debug('[ProfileView] fetch start', {
			title: opts.title,
			kinds: opts.kinds,
			pubkey,
			isOnline
		});
		const result = await fetchFromRelaysWithPrompt(
			{ title: opts.title, kinds: opts.kinds, authors: [pubkey], limit: 500 },
			{ isOnline }
		);
		console.debug('[ProfileView] fetch result', result);
		if (!result) return null;
		// nostrdb ingest is async on the engine side — give it a beat
		// before re-reading locally so the new events show up.
		await new Promise((r) => setTimeout(r, 400));
		await loadLocal(pubkey);
		return result;
	}

	async function handleFetch() {
		fetching = true;
		try {
			await runFetch({
				title: `Fetch all events for ${profile?.display_name || profile?.name || pubkey.slice(0, 12) + '…'}`,
				kinds: [0, 30040, 30023, 30818, 30041, 1111]
			});
			// Profile prefetch hits general relays unconditionally — names
			// don't go through the prompted flow because they're a side
			// effect of any fetch, not the primary target.
			await api.prefetchProfiles([pubkey]);
		} catch (e) {
			console.error('Fetch failed:', e);
		} finally {
			fetching = false;
		}
	}

	async function handleTabFetch(tab: Tab) {
		const kinds = TAB_KINDS[tab];
		tabFetchingKinds = kinds[0];
		try {
			await runFetch({
				title: `Fetch ${TAB_LABEL[tab]} for ${profile?.display_name || profile?.name || pubkey.slice(0, 12) + '…'}`,
				kinds
			});
		} catch (e) {
			console.error('Tab fetch failed:', e);
		} finally {
			tabFetchingKinds = null;
		}
	}

	$effect(() => {
		const pk = pubkey;
		loading = true;
		profile = null;
		publications = [];
		articles = [];
		wikis = [];
		sections = [];
		comments = [];

		loadLocal(pk).catch(() => {}).finally(() => { loading = false; });
	});

	// ----- Cursor + nav handler -----
	// One cursor index keyed by the active tab. j/k walk the active tab's
	// list, Enter / l opens the cursored item, m opens it in the event
	// menu. Resets when the tab changes so the cursor doesn't point past
	// the new list's end.

	let cursor = $state(0);
	let listEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		// Reset cursor when the active tab swaps.
		void activeTab;
		untrack(() => { cursor = 0; });
	});

	function activeList(): Array<unknown> {
		if (activeTab === 'publications') return publications;
		if (activeTab === 'articles') return articles;
		if (activeTab === 'wikis') return wikis;
		if (activeTab === 'sections') return sections;
		return comments;
	}

	function scrollCursorIntoView() {
		if (!listEl) return;
		const row = listEl.querySelector<HTMLDivElement>(`[data-cursor="${cursor}"]`);
		if (!row) return;
		const listRect = listEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			listEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			listEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function openCursorItem() {
		const list = activeList();
		const item = list[cursor];
		if (!item) return;
		if (activeTab === 'publications') {
			onopenpub?.(item as PublicationSummary);
		} else if (activeTab === 'articles' || activeTab === 'wikis') {
			const x = item as { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null };
			onopenaddr?.(x.addr, x.title);
		} else if (activeTab === 'sections') {
			const sec = item as NostrEvent;
			const dTag = getTag(sec, 'd') || '';
			const title = getTag(sec, 'title') || dTag || '[Untitled]';
			onopenaddr?.({ kind: 30041, pubkey: sec.pubkey, d_tag: dTag }, title);
		} else {
			oncomment?.(item as NostrEvent);
		}
	}

	function openCursorMenu() {
		const list = activeList();
		const item = list[cursor];
		if (!item) return;
		if (activeTab === 'comments') {
			// Comments aren't addressable — feed the modal the raw event.
			app.eventModalData = item as NostrEvent;
		} else if (activeTab === 'sections') {
			const sec = item as NostrEvent;
			const dTag = getTag(sec, 'd') || '';
			app.openAddressableInModal({ kind: 30041, pubkey: sec.pubkey, d_tag: dTag });
		} else {
			const addr = (item as { addr: { kind: number; pubkey: string; d_tag: string } }).addr;
			app.openAddressableInModal(addr);
		}
	}

	function handleNav(action: NavAction): boolean {
		const total = activeList().length;
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
			openCursorItem();
			return true;
		}
		if (action === 'menu') {
			openCursorMenu();
			return true;
		}
		return false;
	}

	$effect(() => {
		if (!bufferId) return;
		const id = bufferId;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
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

	{#snippet tabCell(t: Tab, label: string, count: number)}
		<div class="tab" class:active={activeTab === t}>
			<button class="tab-label" onclick={() => (activeTab = t)}>
				{label} ({count})
			</button>
			<button
				class="tab-refresh"
				onclick={() => handleTabFetch(t)}
				disabled={tabFetchingKinds === TAB_KINDS[t][0]}
				title={isOnline
					? `Fetch ${label.toLowerCase()} from configured relays`
					: `Choose relays and fetch ${label.toLowerCase()}`}
			>
				{tabFetchingKinds === TAB_KINDS[t][0] ? '…' : '↻'}
			</button>
		</div>
	{/snippet}

	{#snippet menuBtn(open: () => void)}
		<button
			class="item-menu"
			onclick={(e) => { e.stopPropagation(); open(); }}
			onkeydown={(e) => e.stopPropagation()}
			title="Open menu (m)"
			aria-label="Open event menu"
		>menu</button>
	{/snippet}

	<div class="tabs">
		{@render tabCell('publications', 'Publications', publications.length)}
		{@render tabCell('articles', 'Articles', articles.length)}
		{@render tabCell('wikis', 'Wikis', wikis.length)}
		{@render tabCell('sections', 'Sections', sections.length)}
		{@render tabCell('comments', 'Comments', comments.length)}
	</div>

	<div class="tab-content" bind:this={listEl}>
		{#if loading}
			<div class="empty">Loading...</div>
		{:else if activeTab === 'publications'}
			{#if publications.length === 0}
				<div class="empty">No publications</div>
			{:else}
				{#each publications as pub_item, i (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenpub?.(pub_item); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenpub?.(pub_item); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							<span class="item-title">{pub_item.title ?? '[Untitled]'}</span>
							<PoolStateBadges
								item={app.findPoolItemByAddr(pub_item.addr)}
								onpillctx={() => app.pillActionByAddr(pub_item.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(pub_item.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(pub_item.addr, 'drop')}
								signed={pub_item.signed}
								relays={pub_item.relays}
								forked={pub_item.forked}
							/>
							<span class="item-meta">{pub_item.section_count} sections</span>
							{@render menuBtn(() => app.openAddressableInModal(pub_item.addr))}
						</div>
						{#if pub_item.summary}
							<p class="item-preview">{pub_item.summary}</p>
						{/if}
						<span class="item-time">{formatTime(pub_item.created_at)}</span>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'articles'}
			{#if articles.length === 0}
				<div class="empty">No articles</div>
			{:else}
				{#each articles as art, i (`${art.addr.pubkey}:${art.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(art.addr, art.title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(art.addr, art.title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							<span class="item-title">{art.title ?? '[Untitled]'}</span>
							<PoolStateBadges
								item={app.findPoolItemByAddr(art.addr)}
								onpillctx={() => app.pillActionByAddr(art.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(art.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(art.addr, 'drop')}
								signed={art.signed}
								relays={art.relays}
							/>
							<span class="item-meta">long-form</span>
							{@render menuBtn(() => app.openAddressableInModal(art.addr))}
						</div>
						{#if art.summary}
							<p class="item-preview">{art.summary}</p>
						{/if}
						<span class="item-time">{formatTime(art.created_at)}</span>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'wikis'}
			{#if wikis.length === 0}
				<div class="empty">No wikis</div>
			{:else}
				{#each wikis as wiki, i (`${wiki.addr.pubkey}:${wiki.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(wiki.addr, wiki.title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(wiki.addr, wiki.title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							<span class="item-title">{wiki.title ?? wiki.addr.d_tag ?? '[Untitled]'}</span>
							<PoolStateBadges
								item={app.findPoolItemByAddr(wiki.addr)}
								onpillctx={() => app.pillActionByAddr(wiki.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(wiki.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(wiki.addr, 'drop')}
								signed={wiki.signed}
								relays={wiki.relays}
							/>
							<span class="item-meta">wiki</span>
							{@render menuBtn(() => app.openAddressableInModal(wiki.addr))}
						</div>
						{#if wiki.summary}
							<p class="item-preview">{wiki.summary}</p>
						{/if}
						<span class="item-time">{formatTime(wiki.created_at)}</span>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'sections'}
			{#if sections.length === 0}
				<div class="empty">No sections</div>
			{:else}
				{#each sections as sec, i (sec.id)}
					{@const dTag = getTag(sec, 'd') || ''}
					{@const title = getTag(sec, 'title') || dTag || '[Untitled]'}
					{@const parentAddr = getTag(sec, 'a')}
					{@const addr = { kind: 30041, pubkey: sec.pubkey, d_tag: dTag }}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(addr, title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(addr, title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							<span class="item-title">{title}</span>
							<PoolStateBadges
								item={app.findPoolItemByAddr(addr)}
								onpillctx={() => app.pillActionByAddr(addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(addr, 'drop')}
								signed={isEventSigned(sec.sig)}
								relays={sec.relays ?? []}
							/>
							{@render menuBtn(() => app.openAddressableInModal(addr))}
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
				{#each comments as comment, i (comment.id)}
					{@const rootAddr = getTag(comment, 'A') || getTag(comment, 'E') || getTag(comment, 'I')}
					{@const rootKind = getTag(comment, 'K')}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; oncomment?.(comment); }}
						onkeydown={(e) => { if (e.key === 'Enter') oncomment?.(comment); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-header">
							{#if rootAddr}
								<span class="item-ref">on {rootKind ? `k:${rootKind}` : ''} {rootAddr.split(':').pop()}</span>
							{/if}
							<PoolStateBadges
								item={app.findPoolItemByEventId(comment.id)}
								onpillctx={() => app.pillActionByEventId(comment.id, 'context')}
								onpillcmp={() => app.pillActionByEventId(comment.id, 'compose')}
								onpilldrop={() => app.pillActionByEventId(comment.id, 'drop')}
								signed={isEventSigned(comment.sig)}
								relays={comment.relays ?? []}
							/>
							{@render menuBtn(() => (app.eventModalData = comment))}
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
		display: flex;
		align-items: stretch;
		justify-content: center;
		border-bottom: 2px solid transparent;
		min-width: 0;
	}
	.tab.active {
		border-bottom-color: var(--accent);
	}
	.tab-label {
		flex: 1;
		padding: 8px 4px 8px 12px;
		font-size: 0.75rem;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		text-align: center;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tab-label:hover {
		color: var(--fg);
	}
	.tab.active .tab-label {
		color: var(--fg);
	}

	.tab-refresh {
		padding: 0 6px;
		background: none;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: 0.85rem;
		line-height: 1;
		opacity: 0.6;
		transition: opacity 100ms;
	}
	.tab-refresh:hover:not(:disabled) {
		color: var(--state-online);
		opacity: 1;
	}
	.tab-refresh:disabled {
		opacity: 0.3;
		cursor: not-allowed;
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

	/* Cursor highlight: same ranger-style bar as FeedBuffer rows so the
	   j/k cursor is unmistakable. The accent comes from --id-yours;
	   click and tab-focus both snap the cursor onto the row. */
	.item--cursor {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		border-left-color: var(--id-yours);
		border-left-width: 5px;
		padding-left: 14px;
	}
	.item--cursor .item-title { color: var(--fg); font-weight: 700; }

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

	/* Per-item "menu" affordance — opens the unified event menu modal on
	   the raw event. Also reachable via `m` on the focused card.
	   stopPropagation keeps clicks off the card, which otherwise routes
	   to the reader / discussion view. */
	.item-menu {
		margin-left: auto;
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: 0.62rem;
		padding: 1px 6px;
		cursor: pointer;
		line-height: 1.5;
	}
	.item-menu:hover {
		color: var(--accent);
		border-color: var(--accent);
	}
</style>
