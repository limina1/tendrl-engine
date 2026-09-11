<script lang="ts">
	// A single short-form event — kind 1 note, kind 1111 comment, or any
	// other non-document kind opened by id — as its own reading card.
	// Before this the reader wrapped such events as a one-section
	// publication, so a note wore the section template ("Section 1",
	// pager, Edit §). Visually it stays the section card (same column,
	// reading face, rich body); the header is what differs: a kind label
	// instead of a section title, then author · time, then the standard
	// pool/provenance pills + menu.
	//
	// The thread under it is the NIP-22 discussion rooted at this event
	// (comments carry `E`/`e` = this id): local store first, then a
	// Confirm-gated relay pull on demand — same two-phase shape as
	// DiscussionViewBuffer.pullThread.
	import * as api from '$lib/api';
	import type { NostrEvent } from '$lib/types';
	import { getAppState } from '$lib/state.svelte';
	import { kindLabel } from '$lib/search/search-config.svelte';
	import { countThread, flattenThread, type ThreadNode } from '$lib/discussions/thread';
	import { prefetchAuthors } from '$lib/discussions/authors.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import RichContent from './RichContent.svelte';
	import ProfileName from './ProfileName.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';
	import CommentThread from './CommentThread.svelte';
	import ReplyBox from './ReplyBox.svelte';

	let {
		event,
		onviewprofile = undefined
	}: {
		event: NostrEvent;
		onviewprofile?: (pubkey: string) => void;
	} = $props();

	const app = getAppState();

	const label = $derived(kindLabel(event.kind).toLowerCase());
	const poolItem = $derived(app.findPoolItem(event));
	const canSign = $derived(identityCanSign(app.identityStatus));

	let threadNodes = $state<ThreadNode[]>([]);
	let threadCount = $state(0);
	let threadOpen = $state(false);
	let threadLoading = $state(false);
	let threadError = $state<string | null>(null);
	let replyOpen = $state(false);
	const replyCtl = $state<{ openId: string | null }>({ openId: null });

	function fmtTime(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}

	function applyThreads(threads: ThreadNode[]) {
		threadNodes = threads;
		threadCount = countThread(threads);
		const authors = new Set(flattenThread(threads).map((n) => n.event.pubkey));
		authors.delete('');
		if (authors.size > 0) prefetchAuthors([...authors]);
	}

	const threadOpts = () => ({
		kinds: [1111],
		limit: 500,
		threaded: true,
		eventIds: [event.id]
	});

	// Local read on mount: whatever the store already holds renders at
	// once; the relay pull is the user's call (Confirm-gated).
	$effect(() => {
		const id = event.id;
		let cancelled = false;
		api
			.getDiscussionList({ ...threadOpts(), policy: 'local_only' })
			.then((resp) => {
				if (cancelled || event.id !== id) return;
				const threads = resp.threads ?? [];
				applyThreads(threads);
				if (threads.length > 0) threadOpen = true;
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	});

	async function pullThread() {
		if (threadLoading) return;
		threadLoading = true;
		threadError = null;
		threadOpen = true;
		try {
			const fresh = await api.getDiscussionList({
				...threadOpts(),
				policy: 'fetch_always',
				bypassOffline: true
			});
			applyThreads(fresh.threads ?? []);
		} catch (e) {
			threadError = e instanceof Error ? e.message : String(e);
		} finally {
			threadLoading = false;
		}
	}

	async function refreshThreadLocal() {
		try {
			const resp = await api.getDiscussionList({ ...threadOpts(), policy: 'local_only' });
			applyThreads(resp.threads ?? []);
			threadOpen = true;
		} catch {
			// The toast already reported the post result.
		}
	}
</script>

<article class="note-card" data-event-id={event.id}>
	<header class="note-head">
		<span class="note-kind" title="kind {event.kind}">{label}</span>
		<span class="note-meta">
			<ProfileName pubkey={event.pubkey} {onviewprofile} />
			<span class="note-dot">·</span>
			<time datetime={new Date(event.created_at * 1000).toISOString()}>{fmtTime(event.created_at)}</time>
		</span>
		<div class="note-actions">
			<PoolStateBadges
				item={poolItem}
				onpillctx={() => app.togglePoolMembership(event, 'context')}
				onpillcmp={() => app.togglePoolMembership(event, 'compose')}
				onpilldrop={poolItem ? () => app.dropPoolItem(poolItem.id) : undefined}
				signed={!!event.sig}
				relays={event.relays ?? []}
				orientation="horizontal"
			/>
			<button
				class="pill pill--menu"
				onclick={() => app.getEventForModal(event.id)}
				title="Open this event's menu (m)"
			>menu</button>
		</div>
	</header>

	<div class="note-body">
		<RichContent content={event.content} />
	</div>

	<footer class="note-thread">
		<div class="note-thread-bar">
			<button class="note-action" onclick={pullThread} disabled={threadLoading}>
				{threadLoading ? 'Pulling thread…' : 'Pull thread from relays'}
				{#if threadCount > 0}
					<span class="note-thread-count">{threadCount}</span>
				{/if}
			</button>
			{#if threadNodes.length > 0}
				<button class="note-action" onclick={() => (threadOpen = !threadOpen)}>
					{threadOpen ? 'Hide thread' : 'Show thread'}
				</button>
			{/if}
			<button
				class="note-action"
				disabled={!canSign}
				title={canSign ? 'Comment on this event (NIP-22)' : 'Sign in to comment'}
				onclick={() => (replyOpen = !replyOpen)}
			>
				{replyOpen ? 'Cancel' : 'Comment'}
			</button>
		</div>
		{#if replyOpen}
			<ReplyBox
				root={{ event_id: event.id, kind: event.kind, pubkey: event.pubkey }}
				placeholder="Comment on this {label}…"
				autofocus
				onposted={() => {
					replyOpen = false;
					refreshThreadLocal();
				}}
				oncancel={() => (replyOpen = false)}
			/>
		{/if}
		{#if threadError}
			<div class="note-error">{threadError}</div>
		{/if}
		{#if threadOpen && threadNodes.length > 0}
			<CommentThread nodes={threadNodes} replyable onposted={refreshThreadLocal} {replyCtl} />
		{/if}
	</footer>
</article>

<style>
	/* Same column as SectionCard: reading face/size/measure from the
	   enclosing surface, centred in the pane. */
	.note-card {
		padding: 12px 16px 20px;
		max-width: var(--rd-measure, none);
		margin-inline: auto;
		font-family: var(--rd-font, inherit);
		font-size: var(--rd-size, inherit);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.note-head {
		display: flex;
		align-items: baseline;
		gap: 10px;
		flex-wrap: wrap;
	}
	.note-kind {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	.note-meta {
		display: inline-flex;
		align-items: baseline;
		gap: 6px;
		color: var(--base5);
		font-size: var(--t-sm);
		font-family: var(--font-mono);
	}
	.note-dot {
		opacity: 0.6;
	}
	.note-actions {
		margin-left: auto;
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.note-body {
		line-height: 1.55;
		color: var(--fg);
	}
	.note-thread {
		display: flex;
		flex-direction: column;
		gap: 8px;
		border-top: 1px solid var(--border);
		padding-top: 8px;
	}
	.note-thread-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.note-action {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--accent);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		cursor: pointer;
	}
	.note-action:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.note-thread-count {
		margin-left: 4px;
		color: var(--base5);
	}
	.note-error {
		color: var(--error, #c66);
		font-size: var(--t-xs);
		font-family: var(--font-mono);
	}
</style>
