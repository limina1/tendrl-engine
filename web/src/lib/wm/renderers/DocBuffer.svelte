<script lang="ts">
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import type { Profile } from '$lib/api';
	import type { NAddr } from '$lib/types';
	import type { Buffer } from '../types';
	import { getAppState } from '$lib/state.svelte';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import { type ThreadNode } from '$lib/discussions/thread';
	import {
		segmentsFromSpans,
		type Highlight,
		type HighlightSpan
	} from '$lib/discussions/highlights';
	import { pubkeyToHighlightFill, pubkeyToHighlightStroke } from '$lib/discussions/colors';
	import { prefetchAuthors, refreshAuthors } from '$lib/discussions/authors.svelte';

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
	};

	const parsed = $derived(parseBufferId(buffer.id));

	function parseBufferId(id: string): { kind: number; pubkey: string; dTag: string } | null {
		const m = id.match(/^doc:(\d+):([0-9a-fA-F]{64}):(.+)$/);
		if (!m) return null;
		const kind = parseInt(m[1], 10);
		if (!Number.isFinite(kind)) return null;
		return { kind, pubkey: m[2].toLowerCase(), dTag: m[3] };
	}

	let loading = $state(true);
	let error = $state<string | null>(null);
	let title = $state<string | null>(null);
	let summary = $state<string | null>(null);
	let body = $state('');
	let kindLabel = $state('');
	let authorPubkey = $state('');
	let authorProfile = $state<Profile | null>(null);
	let threads = $state<ThreadNode[]>([]);
	let highlights = $state<Highlight[]>([]);
	let commentsOpen = $state(true);
	let refreshing = $state(false);

	function styleFor(pubkey: string, focused: boolean): string {
		const fill = pubkeyToHighlightFill(pubkey);
		const stroke = pubkeyToHighlightStroke(pubkey);
		if (focused) {
			return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke}, 0 0 0 2px var(--state-online);`;
		}
		return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke};`;
	}

	// Highlight spans resolved engine-side (POST /highlights/resolve), async
	// into state, then sliced into render segments by `segmentsFromSpans`.
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

	const segments = $derived(
		highlightSpans.length > 0 && body ? segmentsFromSpans(body, highlightSpans, null) : null
	);
	const hasOverlay = $derived(!!segments && segments.some((s) => s.highlight !== null));

	const authorName = $derived(
		authorProfile?.display_name ||
			authorProfile?.name ||
			(authorPubkey ? authorPubkey.slice(0, 12) + '…' : '')
	);

	async function loadDoc(p: { kind: number; pubkey: string; dTag: string }) {
		loading = true;
		error = null;
		title = null;
		summary = null;
		body = '';
		threads = [];
		highlights = [];
		authorProfile = null;
		authorPubkey = '';
		try {
			const resp = await api.getAddressable(p.kind, p.pubkey, p.dTag);
			const ev = resp.event as DocEvent | null;
			if (!ev) {
				const noun = p.kind === 30023 ? 'Article' : p.kind === 30818 ? 'Wiki page' : 'Document';
				error = `${noun} not found locally — fetch the author from their profile (↻ Fetch).`;
				return;
			}
			const tag = (n: string) => ev.tags.find((t) => t[0] === n)?.[1] ?? null;
			authorPubkey = ev.pubkey || p.pubkey;
			kindLabel = p.kind === 30023 ? 'article' : p.kind === 30818 ? 'wiki' : `kind ${p.kind}`;
			title = tag('title') ?? (p.kind === 30818 ? p.dTag : null);
			summary = tag('summary');
			body = ev.content ?? '';
			// Discussions and the author profile are secondary — let the body
			// paint immediately and fill these in as they arrive.
			const addr: NAddr = { kind: p.kind, pubkey: p.pubkey, d_tag: p.dTag };
			void loadDiscussions(addr);
			void loadAuthor(authorPubkey);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
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
			highlights = resp.events
				.filter((e) => e.kind === 9802)
				.map((e) => ({ id: e.id, content: e.content ?? '', pubkey: e.pubkey }));
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

<div class="doc-wrap">
	{#if loading}
		<div class="doc-status">Loading…</div>
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
			{#if hasOverlay && segments}
				<pre class="doc-body">{#each segments as seg, i (i)}{#if seg.highlight}<mark
								class="hl-overlay"
								data-hl-ids={seg.highlight.id}
								style={styleFor(seg.highlight.pubkey, seg.highlight.focused)}
								title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…"
							>{seg.text}</mark>{:else}{seg.text}{/if}{/each}</pre>
			{:else}
				<pre class="doc-body">{body}</pre>
			{/if}

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
						<CommentThread nodes={threads} focusedEventId={null} />
					{:else}
						<p class="doc-comments-empty">No comments yet.</p>
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
		font-size: 0.85rem;
		padding: 24px;
		text-align: center;
	}
	.doc-status--error {
		color: #ef4444;
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
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}

	.doc-title {
		font-size: 1.05rem;
		font-weight: 700;
		margin: 2px 0 0;
		line-height: 1.3;
	}

	.doc-summary {
		font-size: 0.8rem;
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
		font-size: 0.9rem;
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
		font-size: 0.8rem;
	}

	.author-name {
		font-size: 0.8rem;
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
		font-size: 0.88rem;
		line-height: 1.6;
		color: var(--fg);
		margin: 0;
		padding: 14px 16px;
	}

	.hl-overlay {
		color: inherit;
		padding: 1px 2px;
		border-radius: 2px;
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
		font-size: 0.8rem;
		color: var(--fg-muted);
		font-style: italic;
		margin: 4px 0 0;
	}
</style>
