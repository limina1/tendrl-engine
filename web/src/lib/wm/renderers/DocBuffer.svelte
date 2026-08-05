<script lang="ts">
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import type { Profile } from '$lib/api';
	import type { NAddr } from '$lib/types';
	import type { Buffer } from '../types';
	import { getAppState } from '$lib/state.svelte';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import ReplyBox from '$lib/components/ReplyBox.svelte';
	import HighlightCapture from '$lib/components/HighlightCapture.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { type ThreadNode } from '$lib/discussions/thread';
	import {
		highlightFromEvent,
		type Highlight,
		type HighlightSpan
	} from '$lib/discussions/highlights';
	import { prefetchAuthors, refreshAuthors } from '$lib/discussions/authors.svelte';
	import PoolStateBadges from '$lib/components/PoolStateBadges.svelte';
	import RichContent from '$lib/components/RichContent.svelte';
	import type { ResolvedRef, ParsedToken } from '$lib/nostr/nostrdown';

	// A slim viewer for single addressable documents — NIP-23 long-form
	// articles (kind 30023) and NKBIP-02 wiki pages (kind 30818). The
	// reader's pagination machinery (Prev/Next, "Section 1 of 1", outline,
	// view-mode toggle) is meaningless for a one-event document, so this
	// drops all of it: header + body + one comment thread at the bottom.

	let { buffer }: { buffer: Buffer } = $props();
	const app = getAppState();

	type DocEvent = {
		id: string;
		kind: number;
		pubkey: string;
		created_at: number;
		content: string;
		tags: string[][];
		relays?: string[];
	};

	const parsed = $derived(parseBufferId(buffer.id));
	// `?focus_comment=<id>` / `?highlight=<id>` suffixes arrive when a
	// discussion card or highlight sends the user here to see the source
	// with its full thread — same marker convention as ReaderBuffer.
	const focusCommentId = $derived(bufferMarker(buffer.id, 'focus_comment'));
	const focusHighlightId = $derived(bufferMarker(buffer.id, 'highlight'));

	function bufferMarker(id: string, name: string): string | null {
		const q = id.indexOf('?');
		if (q < 0) return null;
		const v = new URLSearchParams(id.slice(q + 1)).get(name);
		return v && /^[0-9a-fA-F]{64}$/.test(v) ? v.toLowerCase() : null;
	}

	function parseBufferId(id: string): { kind: number; pubkey: string; dTag: string } | null {
		const q = id.indexOf('?');
		const core = q < 0 ? id : id.slice(0, q);
		const m = core.match(/^doc:(\d+):([0-9a-fA-F]{64}):(.+)$/);
		if (!m) return null;
		const kind = parseInt(m[1], 10);
		if (!Number.isFinite(kind)) return null;
		return { kind, pubkey: m[2].toLowerCase(), dTag: m[3] };
	}

	let loading = $state(true);
	let loadingStatus = $state<string | null>(null);
	let error = $state<string | null>(null);
	let title = $state<string | null>(null);
	let docEventId = $state<string | null>(null);
	let docRelays = $state<string[]>([]);
	let summary = $state<string | null>(null);
	let body = $state('');
	let kindLabel = $state('');
	let authorPubkey = $state('');
	let authorProfile = $state<Profile | null>(null);
	let threads = $state<ThreadNode[]>([]);
	let highlights = $state<Highlight[]>([]);
	let commentsOpen = $state(true);
	let refreshing = $state(false);

	// Highlight spans resolved engine-side (POST /highlights/resolve), async
	// into state; RichContent merges them with the nostrdown refs below.
	let highlightSpans = $state<HighlightSpan[]>([]);
	$effect(() => {
		const text = body;
		const hls = highlights;
		if (!text || hls.length === 0) {
			highlightSpans = [];
			return;
		}
		let cancelled = false;
		api.resolveHighlights([{ key: 'doc', content: text, highlights: hls }])
			.then((m) => {
				if (!cancelled) highlightSpans = m['doc'] ?? [];
			})
			.catch(() => {
				if (!cancelled) highlightSpans = [];
			});
		return () => {
			cancelled = true;
		};
	});

	// Nostrdown `{{ }}`/`[[ ]]` references, resolved engine-side. An isolated
	// doc has no publication context to hand over — instead its own coordinate
	// goes along and the engine derives the containing 30040 (reverse a-tag
	// lookup) so sibling refs resolve here too. Tokens land first as
	// "resolving" chips; the resolve supersedes them.
	let docRefs = $state<ResolvedRef[]>([]);
	let docTokens = $state<ParsedToken[]>([]);
	$effect(() => {
		const text = body;
		const coord = addrStr;
		const author = authorPubkey;
		if (!text || !(text.includes('{{') || text.includes('[['))) {
			docRefs = [];
			docTokens = [];
			return;
		}
		let cancelled = false;
		api.parseNostrdown([{ key: 'doc', content: text }])
			.then((m) => {
				if (!cancelled) docTokens = m['doc'] ?? [];
			})
			.catch(() => {
				if (!cancelled) docTokens = [];
			});
		api.resolveNostrdown([
			{ key: 'doc', content: text, author: author || undefined, coord: coord ?? undefined }
		])
			.then((m) => {
				if (!cancelled) docRefs = m['doc'] ?? [];
			})
			.catch(() => {
				if (!cancelled) docRefs = [];
			});
		return () => {
			cancelled = true;
		};
	});

	// When arriving via ?highlight=<id>, scroll the focused <mark> into
	// view once the overlay has rendered (same deferred-frame trick as
	// CommentThread's focused-node scroll).
	let wrapEl = $state<HTMLElement | null>(null);
	$effect(() => {
		const id = focusHighlightId;
		if (!id || highlightSpans.length === 0 || !wrapEl) return;
		requestAnimationFrame(() => {
			const mark = wrapEl?.querySelector(`[data-hl-ids*="${id}"]`);
			mark?.scrollIntoView({ behavior: 'auto', block: 'center' });
			mark?.classList.add('hl-flash');
		});
	});

	const authorName = $derived(
		authorProfile?.display_name ||
			authorProfile?.name ||
			(authorPubkey ? authorPubkey.slice(0, 12) + '…' : '')
	);

	async function loadDoc(p: { kind: number; pubkey: string; dTag: string }) {
		loading = true;
		loadingStatus = null;
		error = null;
		title = null;
		summary = null;
		body = '';
		docEventId = null;
		docRelays = [];
		threads = [];
		highlights = [];
		authorProfile = null;
		authorPubkey = '';
		try {
			// Two-phase load (the reader's pattern): local cache first, then a
			// confirm-gated relay fetch — this buffer is the canonical target
			// for comment root/parent refs, so an uncached doc must reach
			// relays instead of dead-ending.
			let resp = await api.getAddressable(p.kind, p.pubkey, p.dTag, 'local_only');
			if (!resp.event) {
				loadingStatus = 'Not in local cache — fetching from relays…';
				resp = await api.getAddressable(p.kind, p.pubkey, p.dTag, 'fetch_always', {
					bypassOffline: true
				});
			}
			const ev = resp.event as DocEvent | null;
			if (!ev) {
				const noun =
					p.kind === 30023 ? 'Article'
					: p.kind === 30818 ? 'Wiki page'
					: p.kind === 30817 ? 'Spec'
					: 'Document';
				error = `${noun} not found locally or on your relays.`;
				return;
			}
			const tag = (n: string) => ev.tags.find((t) => t[0] === n)?.[1] ?? null;
			authorPubkey = ev.pubkey || p.pubkey;
			docEventId = ev.id ?? null;
			docRelays = ev.relays ?? [];
			kindLabel =
				p.kind === 30023 ? 'article'
				: p.kind === 30818 ? 'wiki'
				: p.kind === 30817 ? 'spec'
				: `kind ${p.kind}`;
			title = tag('title') ?? (p.kind === 30818 || p.kind === 30817 ? p.dTag : null);
			summary = tag('summary');
			body = ev.content ?? '';
			// Discussions and the author profile are secondary — let the body
			// paint immediately and fill these in as they arrive.
			const addr: NAddr = { kind: p.kind, pubkey: p.pubkey, d_tag: p.dTag };
			void loadDiscussions(addr);
			void loadAuthor(authorPubkey);
		} catch (e) {
			error = api.errorMessage(e);
		} finally {
			loading = false;
			loadingStatus = null;
		}
	}

	async function loadDiscussions(
		addr: NAddr,
		opts: { policy?: 'local_only' | 'local_first' | 'fetch_always'; bypassOffline?: boolean } = {}
	) {
		const addrStr = `${addr.kind}:${addr.pubkey}:${addr.d_tag}`;
		try {
			const resp = await api.getDiscussionList({
				addresses: [addrStr],
				kinds: [1111, 9802],
				policy: opts.policy ?? 'local_first',
				bypassOffline: opts.bypassOffline,
				limit: 500,
				threaded: true
			});
			// kind 1111 = NIP-22 comments → threaded engine-side, keyed by this
			// doc's addr; kind 9802 = NIP-84 highlights → overlaid on the body as
			// <mark>s in author hues.
			threads = resp.threads_by_address?.[addrStr] ?? [];
			highlights = resp.events.filter((e) => e.kind === 9802).map(highlightFromEvent);
			const authors = new Set(resp.events.map((e) => e.pubkey));
			if (authors.size > 0) prefetchAuthors([...authors]);
		} catch (e) {
			console.warn('[DocBuffer] discussion load failed', e);
		}
	}

	async function loadAuthor(pk: string) {
		try {
			authorProfile = await api.getProfile(pk);
		} catch {
			authorProfile = null;
		}
	}

	// After a post the engine has already ingested the new event, so a
	// local-only refetch includes it — no relay round-trip, no client-side
	// thread splicing.
	function refreshDiscussionsLocal() {
		if (!parsed) return;
		void loadDiscussions(
			{ kind: parsed.kind, pubkey: parsed.pubkey, d_tag: parsed.dTag },
			{ policy: 'local_only' }
		);
	}

	const addrStr = $derived(parsed ? `${parsed.kind}:${parsed.pubkey}:${parsed.dTag}` : null);

	const canSignNow = $derived(identityCanSign(app.identityStatus));

	// Highlight capture source: this buffer renders exactly one document, so
	// only its own address resolves. The rendered event id pins the offset
	// frame precisely.
	function highlightContentFor(addr: string): { content: string; eventId?: string } | null {
		if (!addrStr || addr !== addrStr || !body) return null;
		return { content: body, eventId: docEventId ?? undefined };
	}

	async function handleRefresh() {
		if (refreshing || !parsed) return;
		refreshing = true;
		try {
			const addr: NAddr = { kind: parsed.kind, pubkey: parsed.pubkey, d_tag: parsed.dTag };
			// Explicit user action: reach relays even when the app is offline.
			await loadDiscussions(addr, { policy: 'fetch_always', bypassOffline: true });
			const authors = new Set([authorPubkey, ...threads.flatMap(threadAuthors)]);
			authors.delete('');
			if (authors.size > 0) {
				try {
					await refreshAuthors([...authors]);
				} catch (e) {
					console.warn('[DocBuffer] author refresh failed', e);
				}
			}
			await loadAuthor(authorPubkey);
		} finally {
			refreshing = false;
		}
	}

	function threadAuthors(node: ThreadNode): string[] {
		return [node.event.pubkey, ...node.children.flatMap(threadAuthors)];
	}

	$effect(() => {
		const p = parsed;
		if (!p) {
			error = 'Buffer id does not encode a document';
			loading = false;
			return;
		}
		untrack(() => loadDoc(p));
	});
</script>

<div class="doc-wrap" bind:this={wrapEl}>
	{#if loading}
		<div class="doc-status">
			Loading…
			{#if loadingStatus}
				<div class="doc-loading-status">{loadingStatus}</div>
			{/if}
		</div>
	{:else if error}
		<div class="doc-status doc-status--error">{error}</div>
	{:else}
		<header class="doc-bar">
			<div class="doc-headings">
				<span class="doc-kind">{kindLabel}</span>
				<h1 class="doc-title">{title ?? '[Untitled]'}</h1>
				{#if summary}
					<p class="doc-summary">{summary}</p>
				{/if}
			</div>
			{#if parsed}
				{@const addr = { kind: parsed.kind, pubkey: parsed.pubkey, d_tag: parsed.dTag }}
				<!-- Action cluster, feat-ui-patterns order: provenance/pool
				     pills → menu LAST. Refresh sits past the cluster as a
				     fetch affordance, not a row action. -->
				<div class="doc-actions">
					<button
						class="hl-mode-pill"
						class:hl-mode-pill--on={app.highlightMode}
						disabled={!canSignNow}
						onclick={() => app.toggleHighlightMode()}
						title={canSignNow
							? app.highlightMode
								? 'Highlight mode is ON — select text to publish a highlight. Click to turn off.'
								: 'Turn on highlight mode: select text in the body to publish a NIP-84 highlight'
							: 'Sign in to highlight'}
					>hl{app.highlightMode ? ' ●' : ''}</button>
					<PoolStateBadges
						item={app.findPoolItemByAddr(addr)}
						onpillctx={() => app.pillActionByAddr(addr, 'context')}
						onpillcmp={() => app.pillActionByAddr(addr, 'compose')}
						onpilldrop={() => app.pillActionByAddr(addr, 'drop')}
						signed={true}
						relays={docRelays}
						orientation="horizontal"
					/>
					<button
						class="pill pill--menu"
						onclick={() => app.openAddressableInModal(addr)}
						title="Open this document's event menu (m)"
					>menu</button>
				</div>
			{/if}
			<button
				class="doc-refresh"
				class:spinning={refreshing}
				onclick={handleRefresh}
				disabled={refreshing}
				title="Fetch comments and updates from relays"
			>
				↻
			</button>
		</header>

		{#if authorPubkey}
			<!-- Author chip — avatar + name, click jumps to their profile page. -->
			<button
				class="author-chip"
				onclick={() => app.handleViewProfile(authorPubkey)}
				title="Open {authorName}'s profile"
			>
				{#if authorProfile?.picture}
					<img class="author-avatar" src={authorProfile.picture} alt="" />
				{:else}
					<span class="author-avatar author-avatar--ph" aria-hidden="true">@</span>
				{/if}
				<span class="author-name">{authorName}</span>
			</button>
		{/if}

		<div class="doc-content">
			<HighlightCapture getContent={highlightContentFor} onposted={refreshDiscussionsLocal} />
			<!-- data-section-addr marks the highlight-capture boundary; the body
			     text here is verbatim source (text + exact-text marks only), so
			     capture's plain text-walk fallback maps offsets exactly. -->
			<div class="doc-body" data-section-addr={addrStr}>
				<RichContent
					content={body}
					spans={highlightSpans}
					refs={docRefs}
					tokens={docTokens}
					focusedHighlightId={focusHighlightId}
				/>
			</div>

			<section class="doc-comments">
				<button
					class="doc-comments-head"
					onclick={() => (commentsOpen = !commentsOpen)}
					aria-expanded={commentsOpen}
				>
					<span class="ptr">{commentsOpen ? '▾' : '▸'}</span>
					Comments ({threads.length})
				</button>
				{#if commentsOpen}
					{#if threads.length > 0}
						<CommentThread
							nodes={threads}
							focusedEventId={focusCommentId}
							replyable
							onposted={refreshDiscussionsLocal}
						/>
					{:else}
						<p class="doc-comments-empty">No comments yet.</p>
					{/if}
					{#if addrStr}
						<ReplyBox
							root={{ address: addrStr }}
							placeholder="Comment on this {kindLabel}…"
							onposted={refreshDiscussionsLocal}
						/>
					{/if}
				{/if}
			</section>
		</div>
	{/if}
</div>

<style>
	.doc-wrap {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}

	.doc-status {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
		font-size: var(--t-xs);
		padding: 24px;
		text-align: center;
	}
	.doc-status--error {
		color: var(--danger);
	}
	.doc-loading-status {
		margin-top: 6px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}

	.hl-mode-pill {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		background: none;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 2px 8px;
		cursor: pointer;
	}
	.hl-mode-pill:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.hl-mode-pill--on {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.hl-mode-pill:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.doc-actions {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-shrink: 0;
		padding-top: 2px;
	}

	.doc-bar {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
	}

	.doc-headings {
		flex: 1;
		min-width: 0;
	}

	.doc-kind {
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}

	.doc-title {
		font-size: var(--t-md);
		font-weight: 700;
		margin: 2px 0 0;
		line-height: 1.3;
	}

	.doc-summary {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-style: italic;
		margin: 4px 0 0;
		line-height: 1.45;
	}

	.doc-refresh {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: var(--t-sm);
		line-height: 1;
		padding: 4px 8px;
		border-radius: var(--radius);
		cursor: pointer;
	}
	.doc-refresh:hover:not(:disabled) {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.doc-refresh:disabled {
		cursor: default;
		opacity: 0.5;
	}
	.doc-refresh.spinning {
		animation: doc-spin 0.8s linear infinite;
	}
	@keyframes doc-spin {
		to {
			transform: rotate(360deg);
		}
	}

	.author-chip {
		display: flex;
		align-items: center;
		gap: 8px;
		margin: 8px 16px 0;
		padding: 4px 10px 4px 4px;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		border-radius: 999px;
		cursor: pointer;
		align-self: flex-start;
		max-width: calc(100% - 32px);
	}
	.author-chip:hover {
		border-color: var(--id-remote);
	}

	.author-avatar {
		width: 24px;
		height: 24px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}
	.author-avatar--ph {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--border);
		color: var(--fg-muted);
		font-size: var(--t-2xs);
	}

	.author-name {
		font-size: var(--t-2xs);
		font-weight: 600;
		color: var(--fg);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.doc-content {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
	}

	.doc-body {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.6;
		color: var(--fg);
		margin: 0;
		padding: 14px 16px;
	}

	@keyframes hl-flash {
		0%,
		100% {
			filter: brightness(1) saturate(1);
		}
		30% {
			filter: brightness(1.5) saturate(1.6);
		}
	}
	:global(.doc-body .hl-overlay.hl-flash) {
		animation: hl-flash 1.2s ease-in-out;
	}

	.doc-comments {
		padding: 12px 16px 24px;
		border-top: 1px solid var(--border);
	}
	.doc-comments-head {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin-bottom: 6px;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: transparent;
		border: none;
		padding: 0;
		cursor: pointer;
	}
	.doc-comments-head:hover {
		color: var(--fg);
	}
	.doc-comments-head .ptr {
		min-width: 1ch;
		display: inline-block;
	}
	.doc-comments-empty {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-style: italic;
		margin: 4px 0 0;
	}
</style>
