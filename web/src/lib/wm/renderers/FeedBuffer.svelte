<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import PoolStateBadges from '$lib/components/PoolStateBadges.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type { Buffer } from '../types';
	import type { PublicationSummary } from '$lib/types';
	import {
		discovery,
		trigger as triggerTip,
		setTipVars
	} from '$lib/wm/discovery.svelte';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	/** The feed's two modes: per-relay timelines (one relay's publications
	 *  at a time, picked from the relays the local store has provenance
	 *  from) and the bookshelf (the user's saved-books list event — the
	 *  data source lands later; the mode is here so the switch exists). */
	type FeedMode = 'relays' | 'bookshelf';
	type FeedViewState = { mode: FeedMode; cursor: number };

	// View state (mode + cursor) is per-buffer and survives a buffer switch
	// via store.bufferState; the selected timeline lives in AppState because
	// every listPublications call has to carry it.
	// svelte-ignore state_referenced_locally
	const savedView = store.bufferState.get(buffer.id) as FeedViewState | undefined;

	let mode = $state<FeedMode>(savedView?.mode ?? 'relays');
	let cursor = $state(savedView?.cursor ?? 0);
	let listEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, { mode, cursor } satisfies FeedViewState);
		};
	});

	$effect(() => {
		untrack(() => {
			app.loadFeed();
			void app.loadFeedRelays();
		});
	});

	// Bookshelf loads when the mode flips to it (and on a remount into it).
	// `mode` is local view state that loadBookshelf never writes, so this
	// can't self-trigger; the async fn's own reads are plain latches.
	$effect(() => {
		if (mode !== 'bookshelf') return;
		untrack(() => void app.loadBookshelf());
	});

	const DEFAULT_SHELF = 'my-book-collection';
	/** Shelf picker rows: every 30045 the pubkey publishes, default first.
	 *  The default shelf is always offered (even before any event is held)
	 *  and the selected one is never dropped, so the select can't go blank. */
	const shelfRows = $derived.by(() => {
		const rows: { value: string; label: string }[] = [];
		const seen = new Set<string>();
		const add = (d: string, title?: string, count?: number) => {
			if (seen.has(d)) return;
			seen.add(d);
			const name = d === DEFAULT_SHELF ? 'my books' : (title ?? d);
			rows.push({ value: d, label: count === undefined ? name : `${name} (${count})` });
		};
		add(DEFAULT_SHELF, undefined, app.bookshelf?.shelves.find((s) => s.d_tag === DEFAULT_SHELF)?.count);
		for (const sh of app.bookshelf?.shelves ?? []) add(sh.d_tag, sh.title, sh.count);
		const cur = app.bookshelfShelf ?? DEFAULT_SHELF;
		add(cur);
		return rows;
	});

	function onPickShelf(e: Event) {
		const v = (e.currentTarget as HTMLSelectElement).value;
		cursor = 0;
		void app.selectBookshelfShelf(v === DEFAULT_SHELF ? null : v);
	}

	/** Bookshelf rows the store holds, in shelf order — the navigable list. */
	const shelfPubs = $derived(
		(app.bookshelf?.books ?? []).filter((b): b is PublicationSummary & { missing: false } => !b.missing)
	);
	const shelfMissing = $derived((app.bookshelf?.books ?? []).filter((b) => b.missing));
	/** The list j/k/Enter act on in the current mode. */
	const activeList = $derived(mode === 'relays' ? app.feed : shelfPubs);

	/** Picker value ↔ timeline: the composite is the empty string (a
	 *  `<select>` can't hold null), everything else passes through. */
	const ALL = '';
	const pickerValue = $derived(app.feedTimeline ?? ALL);

	/** Rows for the picker, in the engine's order (most-indexed relay
	 *  first). Labels carry no counts: the engine tallies every index a
	 *  relay holds, nested and empty ones included, while the feed lists
	 *  roots — the header's count is the honest number. The selected
	 *  timeline is always present even when the scan no longer lists it
	 *  (e.g. its last publication was ignored) so the select never goes
	 *  blank. */
	const pickerRows = $derived.by(() => {
		const rows: { value: string; label: string }[] = [];
		const rel = app.feedRelays;
		rows.push({ value: ALL, label: 'all relays' });
		if (rel && rel.local > 0) rows.push({ value: 'local', label: 'local only' });
		for (const r of rel?.relays ?? []) {
			rows.push({ value: r.relay, label: relayHost(r.relay) });
		}
		const cur = app.feedTimeline;
		if (cur && !rows.some((r) => r.value === cur)) {
			rows.push({ value: cur, label: cur === 'local' ? 'local only' : relayHost(cur) });
		}
		return rows;
	});

	/** `wss://relay.damus.io` → `relay.damus.io` for labels. */
	function relayHost(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}

	const timelineLabel = $derived(
		app.feedTimeline === null
			? 'all relays'
			: app.feedTimeline === 'local'
				? 'local only'
				: relayHost(app.feedTimeline)
	);

	/** On a relay timeline the provenance pill should name THAT relay, not
	 *  whichever the engine listed first — so lead with it. Pure ordering
	 *  for display; the set is unchanged. */
	function relaysForPill(relays: string[]): string[] {
		const cur = app.feedTimeline;
		if (!cur || cur === 'local') return relays;
		const host = relayHost(cur);
		const i = relays.findIndex((r) => relayHost(r) === host);
		if (i <= 0) return relays;
		return [relays[i], ...relays.slice(0, i), ...relays.slice(i + 1)];
	}

	function onPickTimeline(e: Event) {
		const v = (e.currentTarget as HTMLSelectElement).value;
		cursor = 0;
		void app.selectFeedTimeline(v === ALL ? null : v);
	}

	function setMode(m: FeedMode) {
		if (m === mode) return;
		mode = m;
		cursor = 0;
	}

	$effect(() => {
		// Clamp cursor when the active list's length changes.
		if (cursor >= activeList.length) cursor = Math.max(0, activeList.length - 1);
	});

	// Walkthrough: once the feed has events (the user fetched), introduce the
	// top publication + its provenance pills. Gated on `general-feed` already
	// seen so these fire *after* the login walk's fetch beat, not on an initial
	// local load. Fires once; trigger() itself no-ops when not armed.
	let feedTipFired = false;
	$effect(() => {
		const top = app.feed[0];
		if (!top) return;
		untrack(() => {
			if (feedTipFired) return;
			if (!discovery.enabled || discovery.seen.includes('feed-first-pub')) return;
			if (!discovery.seen.includes('general-feed')) return;
			feedTipFired = true;
			const s = top.section_count;
			const r = top.relays.length;
			triggerTip('feed-first-pub', {
				title: top.title ?? '[Untitled]',
				sections: `${s} section${s === 1 ? '' : 's'}`
			});
			// Pre-stash the chained badges tip's relay count (it surfaces via `next`).
			setTipVars('feed-first-badges', {
				relays: r === 0 ? 'no relays yet' : `${r} relay${r === 1 ? '' : 's'}`
			});
		});
	});

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
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

	/** "part of N" badge → find the publications that contain this one. Reveals
	 *  the search buffer and runs a reverse a-tag query (`k:30040 a:<coord>`),
	 *  which lists every 30040 index referencing this publication as a child. */
	function findContainers(addr: { kind: number; pubkey: string; d_tag: string }) {
		store.openBuffer({
			className: 'research',
			buffer: { id: 'search', kind: 'search', label: 'search', kicker: 'containing' }
		});
		app.searchFor(`k:30040 a:${addr.kind}:${addr.pubkey}:${addr.d_tag}`);
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
		const list = activeList;
		const total = list.length;
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
			openPub(list[cursor]);
			return true;
		}
		if (action === 'menu') {
			const cur = list[cursor];
			if (cur) app.openAddressableInModal(cur.addr);
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

{#snippet pubRow(pub_item: PublicationSummary, i: number)}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="row"
		class:row--cursor={i === cursor}
		data-cursor={i}
		data-tour={i === 0 ? 'feed-first-pub' : undefined}
		onclick={() => { cursor = i; openPub(pub_item); }}
		onkeydown={(e) => {
			if (e.key === 'Enter') openPub(pub_item);
		}}
		role="button"
		tabindex="0"
	>
		<span class="cursor-marker" aria-hidden="true">{i === cursor ? '›' : ' '}</span>
		<div class="row-body">
		<!-- Two columns: text (title/summary/footer, truncating) on
		     the left, the controls rail on the right. The rail is a
		     fixed-width column so preview text can never run under
		     the pills/menu, whatever the pane width. -->
		<div class="row-main">
			<span class="title">{pub_item.title ?? '[Untitled]'}</span>
			{#if pub_item.summary}
				<p class="summary">{pub_item.summary}</p>
			{/if}
			<div class="row-foot">
				<span class="author"><ProfileName pubkey={pub_item.author_pubkey} onviewprofile={app.handleViewProfile} /></span>
				<span class="time">{formatTime(pub_item.created_at)}</span>
			</div>
		</div>
		<!-- Rail reads in one fixed order everywhere (feed + reader
		     outline): provenance/pool pills, counts, menu last — so
		     the menu pill lines up row to row. -->
		<div class="row-rail">
			{#if pub_item.local}
				<button
					class="pill pill--broadcast"
					onclick={(e) => {
						e.stopPropagation();
						app.handleBroadcastPublication(pub_item.addr);
					}}
					title="Broadcast this signed local snapshot to your publish relays"
				>broadcast</button>
			{/if}
			<!-- Provenance (local / relay / remote) lives inside the
			     unified pool-state stack so the row reads in one column.
			     "local" = signed but not broadcast (LocalPublicationTracker). -->
			<PoolStateBadges
				anchor={i === 0 ? 'feed-first-badges' : undefined}
				item={app.findPoolItemByAddr(pub_item.addr)}
				onpillctx={() => app.pillActionByAddr(pub_item.addr, 'context')}
				onpillcmp={() => app.pillActionByAddr(pub_item.addr, 'compose')}
				onpilldrop={() => app.pillActionByAddr(pub_item.addr, 'drop')}
				signed={pub_item.signed}
				relays={relaysForPill(pub_item.relays)}
				local={pub_item.local}
				forked={pub_item.forked}
				containedIn={pub_item.contained_in?.length ?? 0}
				onpartof={() => findContainers(pub_item.addr)}
			/>
			<span class="meta">{pub_item.section_count} sections</span>
			<button
				class="pill pill--menu"
				data-tour={i === 0 ? 'menu-pill' : undefined}
				onclick={(e) => {
					e.stopPropagation();
					app.openAddressableInModal(pub_item.addr);
				}}
				title="Open the event menu (m)"
			>menu</button>
		</div>
		</div>
	</div>
{/snippet}

<div class="feed-wrap" data-tour="feed">
	<div class="feed-header">
		<div class="modes" role="tablist" aria-label="Feed mode">
			<button
				class="mode"
				class:mode--active={mode === 'relays'}
				role="tab"
				aria-selected={mode === 'relays'}
				onclick={() => setMode('relays')}
			>relays</button>
			<button
				class="mode"
				class:mode--active={mode === 'bookshelf'}
				role="tab"
				aria-selected={mode === 'bookshelf'}
				onclick={() => setMode('bookshelf')}
			>bookshelf</button>
		</div>
		{#if mode === 'relays'}
			<!-- One timeline per relay the local store has provenance from.
			     Native select: the rows are data, not actions, and it keeps
			     the keyboard/mobile picker for free (see feat-ui-patterns,
			     menu idioms — this is a listbox, not a fifth menu). -->
			<select
				class="timeline-select"
				value={pickerValue}
				onchange={onPickTimeline}
				aria-label="Relay timeline"
				title="Which relay's publications to list"
			>
				{#each pickerRows as row (row.value)}
					<option value={row.value}>{row.label}</option>
				{/each}
			</select>
			<span class="count">{app.feed.length}</span>
			<button
				class="sync"
				onclick={app.handleFeedSync}
				disabled={app.feedSyncing}
				title={app.feedTimeline && app.feedTimeline !== 'local'
					? `Fetch this timeline from ${relayHost(app.feedTimeline)}`
					: 'Fetch publications from your read relays'}
			>
				{app.feedSyncing ? 'Syncing…' : 'Sync'}
			</button>
		{:else}
			<!-- Same listbox idiom as the relay picker: one 30045 shelf per row. -->
			<select
				class="timeline-select"
				value={app.bookshelfShelf ?? DEFAULT_SHELF}
				onchange={onPickShelf}
				aria-label="Bookshelf"
				disabled={app.bookshelfError === 'none'}
				title={app.bookshelf?.bookshelf ? `kind 30045 · d ${app.bookshelf.bookshelf.d_tag} · ${app.bookshelf.bookshelf.event_id.slice(0, 12)}…` : 'Which shelf (kind 30045 d-tag) to list'}
			>
				{#each shelfRows as row (row.value)}
					<option value={row.value}>{row.label}</option>
				{/each}
			</select>
			<span class="count">{shelfPubs.length}{shelfMissing.length ? ` +${shelfMissing.length} missing` : ''}</span>
			<button
				class="sync"
				onclick={app.handleBookshelfSync}
				disabled={app.bookshelfSyncing || app.bookshelfError === 'none'}
				title="Fetch your bookshelf from your read relays and backfill the books it lists"
			>
				{app.bookshelfSyncing ? 'Syncing…' : 'Sync'}
			</button>
		{/if}
	</div>
	{#if mode === 'bookshelf'}
		{#if app.bookshelfLoading && !app.bookshelf}
			<div class="empty"><p>Loading bookshelf…</p></div>
		{:else if app.bookshelfError === 'none'}
			<div class="empty">
				<p>No identity.</p>
				<p class="hint">Sign in to see the books saved on your bookshelf (kind 30045).</p>
			</div>
		{:else if !app.bookshelf?.bookshelf}
			<div class="empty">
				<p>No {app.bookshelfShelf ? `“${app.bookshelfShelf}” shelf` : 'bookshelf'} found locally.</p>
				<button onclick={app.handleBookshelfSync} disabled={app.bookshelfSyncing}>
					{app.bookshelfSyncing ? 'Syncing…' : 'Fetch from relays'}
				</button>
			</div>
		{:else if shelfPubs.length === 0 && shelfMissing.length === 0}
			<div class="empty">
				<p>Your bookshelf is empty.</p>
				<p class="hint">Books saved with the Bookshelf app will list here.</p>
			</div>
		{:else}
			<div class="feed-list" bind:this={listEl}>
				{#each shelfPubs as pub_item, i (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
					{@render pubRow(pub_item, i)}
				{/each}
				{#if shelfMissing.length}
					<div class="missing">
						<div class="missing-head">
							<span>{shelfMissing.length} not in your library</span>
							<button class="sync" onclick={app.handleBookshelfSync} disabled={app.bookshelfSyncing}>
								{app.bookshelfSyncing ? 'Fetching…' : 'Fetch'}
							</button>
						</div>
						{#each shelfMissing as m (`${m.addr.pubkey}:${m.addr.d_tag}`)}
							<div class="missing-row" title={`30040:${m.addr.pubkey}:${m.addr.d_tag}`}>
								<span class="missing-d">{m.addr.d_tag}</span>
								<span class="missing-by"><ProfileName pubkey={m.addr.pubkey} onviewprofile={app.handleViewProfile} /></span>
								{#if m.relay_hint}<span class="missing-hint">{relayHost(m.relay_hint)}</span>{/if}
							</div>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
	{:else if app.feedLoading}
		<div class="empty"><p>Loading publications…</p></div>
	{:else if app.feed.length > 0}
		<div class="feed-list" bind:this={listEl}>
			{#each app.feed as pub_item, i (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
				{@render pubRow(pub_item, i)}
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
			<p>No publications from {timelineLabel} locally.</p>
			{#if app.feedTimeline === 'local'}
				<p class="hint">Signed snapshots that no relay has accepted yet list here.</p>
			{:else}
				<button onclick={app.handleFeedSync} disabled={app.feedSyncing}>
					{app.feedSyncing ? 'Syncing…' : app.feedTimeline ? `Fetch from ${timelineLabel}` : 'Fetch from relays'}
				</button>
			{/if}
		</div>
	{/if}
</div>

<style>
	.feed-wrap { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.feed-list { flex: 1; overflow-y: auto; }
	.feed-header {
		flex-shrink: 0;
		background: var(--panel-bg);
		padding: 6px 12px;
		font-size: var(--t-xs);
		color: var(--base6);
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	/* relays | bookshelf — a two-tab segment in the mode-line pill idiom. */
	.modes { display: inline-flex; border: 1px solid var(--base3); border-radius: var(--r-sm); overflow: hidden; }
	.mode {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 2px 8px;
		background: transparent;
		border: 0;
		color: var(--base6);
		cursor: pointer;
	}
	.mode + .mode { border-left: 1px solid var(--base3); }
	.mode:hover { color: var(--fg); }
	.mode--active { background: color-mix(in srgb, var(--id-yours) 22%, transparent); color: var(--fg); }
	.timeline-select {
		flex: 1;
		min-width: 12ch;
		max-width: 36ch;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 6px;
		background: var(--panel-bg-soft);
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
	.count { font-variant-numeric: tabular-nums; color: var(--base5); margin-left: auto; white-space: nowrap; }
	/* Books the shelf lists but the store doesn't hold — bare coordinates
	   until a fetch lands them, kept below the real rows. */
	.missing { border-top: 1px dashed var(--panel-border); padding: 8px 12px; }
	.missing-head {
		display: flex; align-items: center; justify-content: space-between;
		font-size: var(--t-xs); color: var(--base5); text-transform: uppercase; letter-spacing: 0.05em;
		margin-bottom: 4px;
	}
	.missing-row { display: flex; gap: 8px; align-items: baseline; font-size: var(--t-xs); color: var(--base6); padding: 2px 0; min-width: 0; }
	.missing-d { font-family: var(--font-mono); color: var(--fg); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.missing-by { color: var(--base5); white-space: nowrap; }
	.missing-hint { margin-left: auto; color: var(--base5); font-family: var(--font-mono); white-space: nowrap; }
	.hint { font-size: var(--t-xs); color: var(--base5); margin: 0; max-width: 40ch; text-align: center; }
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
	.row-body { flex: 1; min-width: 0; display: flex; align-items: flex-start; gap: 8px; }
	.row-main { flex: 1; min-width: 0; }
	/* Controls rail — pills, counts, menu. Fixed (non-shrinking) column,
	   right-aligned, bounded so a long relay label can't widen it. */
	.row-rail {
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 3px;
		max-width: 22ch;
	}
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
	.title { display: block; font-size: var(--t-sm); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-bottom: 2px; }
	/* Fixed-width, right-aligned count so "N sections" forms a column
	   across rows regardless of how many pills precede it. */
	.meta {
		font-size: var(--t-xs);
		color: var(--base5);
		white-space: nowrap;
		min-width: 9ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.summary {
		font-size: var(--t-xs);
		color: var(--base6);
		line-height: var(--lh-snug);
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		/* Unbroken runs (URLs, naddrs) wrap instead of clipping wide. */
		overflow-wrap: anywhere;
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
