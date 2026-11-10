<script lang="ts">
	// A kind-32176 narration opened as a book: the root's chapters (its `a`
	// children, in order — a leaf alone is one chapter) rendered like the
	// publication view, each with a player and its transcript, but with no
	// text of the narrated document (that is the reader's job — this
	// buffer opens when the document is NOT local; the header offers the
	// reader as soon as it is).
	//
	// Buffer id: `audiobook:<narrator-hex>:<d-tag>` (d-tag trailing, may
	// hold ':'). Engine owns the tree walk + ranking (`GET /narration/
	// event`); this component renders and holds view state only. No Svelte
	// lifecycle hooks (buffer renderers unmount on every switch).
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import * as nar from '$lib/narration.svelte';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import type { Buffer } from '../types';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import { getAuthorDisplayName, prefetchAuthors } from '$lib/discussions/authors.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();
	const app = getAppState();

	const coord = $derived(parseBufferId(buffer.id));
	function parseBufferId(id: string): { pubkey: string; dTag: string } | null {
		const m = id.match(/^audiobook:([0-9a-f]{64}):(.*)$/);
		return m ? { pubkey: m[1], dTag: m[2] } : null;
	}

	type ViewState = { openTranscript: string | null; scrollTop: number };
	// svelte-ignore state_referenced_locally
	const saved = store.bufferState.get(buffer.id) as ViewState | undefined;
	let openTranscript = $state<string | null>(saved?.openTranscript ?? null);
	let wrapEl: HTMLDivElement | undefined = $state();
	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, { openTranscript, scrollTop: wrapEl?.scrollTop ?? 0 });
		};
	});

	let book = $state<api.Audiobook | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let fetching = $state(false);

	async function load(policy: 'local_only' | 'local_first' | 'fetch_always') {
		if (!coord) return;
		const { pubkey, dTag } = coord;
		if (policy === 'fetch_always') fetching = true;
		else loading = book === null;
		error = null;
		try {
			const b = await api.getNarrationEvent(pubkey, dTag, policy);
			book = b;
			prefetchAuthors([b.narration.pubkey, ...b.chapters.map((c) => c.recording.pubkey)]);
			if (saved?.scrollTop && wrapEl) wrapEl.scrollTop = saved.scrollTop;
		} catch (e) {
			if (book === null) error = api.errorMessage(e, 'Recording not found');
			else app.pushToast(api.errorMessage(e, 'Fetch failed'), 'error', 5000);
		} finally {
			loading = false;
			fetching = false;
		}
	}

	$effect(() => {
		const c = coord;
		if (!c) return;
		untrack(() => void load('local_first'));
	});

	const title = $derived(book?.about_title ?? book?.narration.title ?? coord?.dTag ?? '');
	const narrator = $derived(book ? getAuthorDisplayName(book.narration.pubkey) : '');
	const totalSecs = $derived((book?.chapters ?? []).reduce((a, c) => a + (c.recording.duration_secs ?? 0), 0));
	const playable = $derived((book?.chapters ?? []).filter((c) => (c.recording.piece_urls?.length ?? c.recording.urls.length) > 0).length);
	/** The coordinate a listen is remembered under: the narrated 30040, or
	 *  a single document; a bare section / unbound recording is not kept. */
	const historyCoord = $derived.by(() => {
		if (!book) return null;
		if (book.about_publication) return book.about_publication;
		const k = book.about_kind;
		return book.about && (k === 30040 || k === 30023 || k === 30818 || k === 30817) ? book.about : null;
	});

	function items() {
		return (book?.chapters ?? []).map((c, i) => ({
			coord: c.about ?? `${coord?.pubkey}:${coord?.dTag}#${i}`,
			title: c.title,
			recording: c.recording
		}));
	}

	function playFrom(i: number) {
		if (!nar.playChapters(items(), title, i, historyCoord)) app.pushToast('No server offered this audio', 'info');
	}

	function playAll() {
		playFrom(0);
	}

	function isCurrent(c: api.AudiobookChapter, i: number): boolean {
		const cur = nar.current();
		return !!cur && cur.recording.id === c.recording.id && (cur.coord === (c.about ?? `${coord?.pubkey}:${coord?.dTag}#${i}`));
	}

	function openDocument() {
		if (!book) return;
		const target = book.about_publication ?? (book.about_local ? book.about : null);
		if (target) app.openCoord(target);
	}

	function openMenu() {
		if (!coord) return;
		void app.openAddressableInModal({ kind: 32176, pubkey: coord.pubkey, d_tag: coord.dTag });
	}

	function host(u: string): string {
		try {
			return new URL(u).host;
		} catch {
			return u;
		}
	}
</script>

<div class="ab-wrap" bind:this={wrapEl}>
	{#if loading}
		<div class="ab-status">Loading…</div>
	{:else if error}
		<div class="ab-status ab-status--error">{error}</div>
	{:else if book}
		<header class="ab-bar">
			<div class="ab-headings">
				<span class="ab-kind">{book.is_root ? 'narration index' : 'audio leaf'} · kind 32176</span>
				<h1 class="ab-title">🎧 {title}</h1>
				<div class="ab-sub">
					<span>read by</span>
					<button class="ab-who" onclick={() => app.handleViewProfile(book!.narration.pubkey)} title={book.narration.pubkey}>{book.narration.mine ? 'you' : narrator}</button>
					{#if book.narration.voice}<span class="ab-dim">· {book.narration.voice}</span>{/if}
					{#if book.narration.model}<span class="ab-dim">· {book.narration.model}</span>{/if}
					<span class="ab-dim">· {book.chapters.length} chapter{book.chapters.length === 1 ? '' : 's'}</span>
					{#if totalSecs > 0}<span class="ab-dim">· {nar.fmtTime(totalSecs)}</span>{/if}
					{#if book.narration.relays.length === 0}<span class="ab-pill" title="Signed here, not yet on any relay">local</span>{:else}<span class="ab-pill" title={book.narration.relays.join('\n')}>{book.narration.relays.length} relay{book.narration.relays.length === 1 ? '' : 's'}</span>{/if}
				</div>
			</div>
			<div class="ab-actions">
				<button class="ab-btn ab-btn--main" onclick={playAll} disabled={playable === 0} title="Play every chapter in order">▶ play all</button>
				{#if book.about_publication || (book.about_local && book.about)}
					<button class="ab-btn" onclick={openDocument} title="Open the narrated document in the reader">open publication</button>
				{:else if book.about}
					<span class="ab-dim" title={book.about}>narrates {book.about.split(':')[0]}:…:{book.about.split(':').slice(2).join(':')} — not local</span>
				{/if}
				<button class="ab-btn" onclick={openMenu} title="Open this recording's event menu (tags, A/E/K binding, blossom hints, raw JSON)">menu</button>
				<button class="ab-btn ab-refresh" class:spinning={fetching} onclick={() => load('fetch_always')} disabled={fetching} title="Fetch missing chapters and newer versions from relays (the relays this came from first, then your general set)">↻</button>
			</div>
		</header>

		{#if book.missing.length > 0}
			<div class="ab-note">{book.missing.length} chapter{book.missing.length === 1 ? '' : 's'} listed by the index {book.missing.length === 1 ? 'is' : 'are'} not local yet — ↻ fetches them.</div>
		{/if}
		{#if book.fetched_from.length > 0}
			<div class="ab-note ab-note--dim" title={book.fetched_from.join('\n')}>pulled from {book.fetched_from.length} relay{book.fetched_from.length === 1 ? '' : 's'}</div>
		{/if}

		<ol class="ab-chapters">
			{#each book.chapters as c, i (c.recording.id + ':' + i)}
				{@const current = isCurrent(c, i)}
				{@const playing = current && nar.player.playing}
				{@const urls = (c.recording.piece_urls?.length ? c.recording.piece_urls : [c.recording.urls]).flat()}
				<li class="ab-chapter" class:ab-chapter--current={current}>
					<div class="ab-row">
						<button class="ab-play" onclick={() => (playing ? nar.pause() : current ? nar.toggle() : playFrom(i))} disabled={urls.length === 0} title={playing ? 'Pause' : 'Play from this chapter'} aria-label={playing ? 'Pause' : 'Play'}>{playing ? '⏸' : '▶'}</button>
						<div class="ab-main">
							<div class="ab-line">
								<span class="ab-idx">{i + 1}.</span>
								<span class="ab-ctitle">{c.title}</span>
								{#if c.recording.duration_secs}<span class="ab-dim">· {nar.fmtTime(c.recording.duration_secs)}</span>{/if}
								{#if c.recording.pieces.length > 1}<span class="ab-dim">· {c.recording.pieces.length} blobs</span>{/if}
							</div>
							<div class="ab-line ab-line--meta">
								{#if c.recording.pubkey !== book.narration.pubkey}<span class="ab-dim"><ProfileName pubkey={c.recording.pubkey} /> ·</span>{/if}
								<span class="ab-pill ab-pill--{c.recording.version_match}" title={c.recording.version_match === 'exact' ? 'Recorded from the local version of the text' : c.recording.version_match === 'other_version' ? 'Recorded from another version of the text' : 'Not pinned to a version'}>{nar.matchLabel(c.recording.version_match)}</span>
								{#if c.about}
									{#if c.about_local}
										<button class="ab-link" onclick={() => app.openCoord(c.about!)} title={c.about}>open §</button>
									{:else}
										<span class="ab-dim" title={c.about}>§ not local</span>
									{/if}
								{/if}
								{#if urls.length > 0}
									<span class="ab-dim" title={urls.join('\n')}>{urls.length} source{urls.length === 1 ? '' : 's'} · {host(urls[0])}</span>
								{:else}
									<span class="ab-dim">no server offers this blob</span>
								{/if}
								{#if c.recording.transcript}
									<button class="ab-link" class:active={openTranscript === c.recording.id} onclick={() => (openTranscript = openTranscript === c.recording.id ? null : c.recording.id)}>{openTranscript === c.recording.id ? 'hide text' : 'text'}</button>
								{/if}
								<button class="ab-link" onclick={() => app.openAddressableInModal({ kind: 32176, pubkey: c.recording.pubkey, d_tag: c.recording.d_tag })} title="Open this leaf's event menu">⋮</button>
							</div>
						</div>
					</div>
					{#if current}
						<div class="ab-progress" aria-hidden="true"><span style="width: {nar.player.duration ? Math.min(100, (nar.player.position / nar.player.duration) * 100) : 0}%"></span></div>
					{/if}
					{#if openTranscript === c.recording.id && c.recording.transcript}
						<pre class="ab-transcript">{c.recording.transcript}</pre>
					{/if}
				</li>
			{/each}
		</ol>
		{#if book.chapters.length === 0}
			<div class="ab-status">This index lists no chapters that are local. ↻ fetches them.</div>
		{/if}
	{/if}
</div>

<style>
	.ab-wrap {
		height: 100%;
		min-height: 0;
		overflow: auto;
		padding: 12px 16px 48px;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.ab-status {
		color: var(--fg-muted);
		font-size: var(--t-sm);
		padding: 24px 0;
		text-align: center;
	}
	.ab-status--error {
		color: var(--error, #d64545);
	}
	.ab-bar {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-start;
		justify-content: space-between;
		gap: 10px 16px;
		padding-bottom: 10px;
		border-bottom: 1px solid var(--border);
	}
	.ab-headings {
		min-width: 0;
		flex: 1 1 320px;
	}
	.ab-kind {
		display: block;
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: #9b5de5;
	}
	.ab-title {
		margin: 2px 0 4px;
		font-size: var(--t-xl);
		font-weight: 600;
		line-height: 1.2;
	}
	.ab-sub {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 6px;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.ab-who {
		background: none;
		border: none;
		padding: 0;
		color: var(--fg);
		cursor: pointer;
		font: inherit;
		text-decoration: underline dotted;
	}
	.ab-dim {
		color: var(--fg-muted);
		font-size: var(--t-xs);
		white-space: nowrap;
	}
	.ab-pill {
		font-size: var(--t-3xs);
		border: 1px solid var(--border);
		border-radius: 999px;
		padding: 0 6px;
		color: var(--fg-muted);
		white-space: nowrap;
	}
	.ab-pill--exact {
		border-color: color-mix(in srgb, var(--ok, #3cab5a) 60%, var(--border));
		color: var(--ok, #3cab5a);
	}
	.ab-pill--other_version {
		border-color: color-mix(in srgb, var(--warn, orange) 60%, var(--border));
		color: var(--warn, orange);
	}
	.ab-actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
	}
	.ab-btn {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		font-size: var(--t-xs);
		color: var(--fg);
		cursor: pointer;
		font-family: inherit;
	}
	.ab-btn:hover:not(:disabled) {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.ab-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.ab-btn--main {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.ab-refresh.spinning {
		animation: ab-spin 1s linear infinite;
	}
	@keyframes ab-spin {
		to {
			transform: rotate(360deg);
		}
	}
	.ab-note {
		font-size: var(--t-xs);
		color: var(--warn, orange);
	}
	.ab-note--dim {
		color: var(--fg-muted);
	}
	.ab-chapters {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.ab-chapter {
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 8px 10px;
	}
	.ab-chapter--current {
		border-color: color-mix(in srgb, var(--id-yours) 60%, var(--border));
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
	}
	.ab-row {
		display: flex;
		align-items: flex-start;
		gap: 10px;
	}
	.ab-play {
		flex-shrink: 0;
		width: 30px;
		height: 30px;
		border-radius: 50%;
		border: 1px solid var(--border);
		background: var(--bg);
		color: var(--fg);
		cursor: pointer;
		font-size: var(--t-xs);
	}
	.ab-play:hover:not(:disabled),
	.ab-chapter--current .ab-play {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.ab-play:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.ab-main {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 3px;
	}
	.ab-line {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 4px 6px;
		min-width: 0;
	}
	.ab-line--meta {
		font-size: var(--t-xs);
	}
	.ab-idx {
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.ab-ctitle {
		font-weight: 500;
	}
	.ab-link {
		background: none;
		border: none;
		padding: 0;
		color: var(--fg-muted);
		cursor: pointer;
		font: inherit;
		font-size: var(--t-xs);
		text-decoration: underline dotted;
	}
	.ab-link:hover,
	.ab-link.active {
		color: var(--id-yours);
	}
	.ab-progress {
		margin-top: 6px;
		height: 3px;
		border-radius: 2px;
		background: var(--border);
		overflow: hidden;
	}
	.ab-progress span {
		display: block;
		height: 100%;
		background: var(--id-yours);
		transition: width 0.5s linear;
	}
	.ab-transcript {
		margin: 8px 0 0;
		padding: 8px 10px;
		border-left: 2px solid var(--border);
		font: inherit;
		font-size: var(--t-sm);
		line-height: 1.5;
		white-space: pre-wrap;
		color: var(--fg);
		max-height: 40vh;
		overflow: auto;
	}
</style>
