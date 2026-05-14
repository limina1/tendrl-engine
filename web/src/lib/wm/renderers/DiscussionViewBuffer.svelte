<script lang="ts">
	import * as api from '$lib/api';
	import { getActiveStore } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();

	type NostrEvent = {
		id: string;
		kind: number;
		pubkey: string;
		created_at: number;
		content: string;
		tags: string[][];
	};

	type ParentRef =
		| { type: 'a'; value: string; relay?: string }
		| { type: 'e'; value: string; relay?: string; pubkey?: string };

	let event = $state<NostrEvent | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let thread = $state<NostrEvent[]>([]);
	let threadOpen = $state(false);
	let threadLoading = $state(false);
	let threadError = $state<string | null>(null);

	const eventId = $derived(parseBufferId(buffer.id));

	function parseBufferId(id: string): string | null {
		const m = id.match(/^discussion:([0-9a-fA-F]{64})$/);
		return m ? m[1].toLowerCase() : null;
	}

	// NIP-22 + NIP-84 tag conventions:
	//   uppercase A/E = root scope (the top of the thread)
	//   lowercase a/e = parent (immediate ancestor)
	// For a top-level comment they're identical. For a nested reply they
	// diverge — root is the article, parent is the comment being replied to.
	function extractRefs(ev: NostrEvent): { root: ParentRef | null; parent: ParentRef | null } {
		let root: ParentRef | null = null;
		let parent: ParentRef | null = null;
		for (const tag of ev.tags) {
			if (!tag || tag.length < 2) continue;
			const [name, value, relay, pk] = tag as [string, string, string?, string?];
			if (name === 'A' && !root) root = { type: 'a', value, relay };
			else if (name === 'E' && !root) root = { type: 'e', value, relay, pubkey: pk };
			else if (name === 'a' && !parent) parent = { type: 'a', value, relay };
			else if (name === 'e' && !parent) parent = { type: 'e', value, relay, pubkey: pk };
		}
		return { root, parent };
	}

	const refs = $derived(event ? extractRefs(event) : { root: null, parent: null });
	const isComment = $derived(event?.kind === 1111);
	const isHighlight = $derived(event?.kind === 9802);

	async function load() {
		if (!eventId) {
			error = 'Buffer id is not a discussion id';
			loading = false;
			return;
		}
		loading = true;
		error = null;
		try {
			const resp = await api.getEvent(eventId);
			const ev = resp.event as NostrEvent | null;
			if (!ev) {
				error = 'Event not found in local DB. Try a Refresh on the source article first.';
			} else {
				event = ev;
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		buffer.id;
		load();
	});

	function openInReader(ref: ParentRef) {
		if (ref.type === 'a') {
			// kind:pubkey:d_tag
			const parts = ref.value.split(':');
			if (parts.length < 3) return;
			const kind = parseInt(parts[0], 10);
			const pubkey = parts[1];
			const d_tag = parts.slice(2).join(':');
			if (kind === 30040) {
				store.openBuffer({
					className: 'work',
					buffer: {
						id: `reader:30040:${pubkey}:${d_tag}`,
						kind: 'reader',
						label: 'reader',
						kicker: d_tag
					}
				});
			} else {
				// Non-publication addressable: open as event reader of the
				// matching addressable note. The reader buffer's parser
				// understands the publication form; for arbitrary kinds
				// we still go through that route — the reader will fall
				// back to "no publication" and show the addressable as a
				// standalone if needed.
				store.openBuffer({
					className: 'work',
					buffer: {
						id: `reader:${kind}:${pubkey}:${d_tag}`,
						kind: 'reader',
						label: 'reader',
						kicker: d_tag.slice(0, 16)
					}
				});
			}
		} else {
			// Standalone event by id
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:event:${ref.value}`,
					kind: 'reader',
					label: 'event',
					kicker: ref.value.slice(0, 8) + '…'
				}
			});
		}
	}

	function openSourceWithHighlight() {
		if (!event || !isHighlight) return;
		const target = refs.root ?? refs.parent;
		if (!target) return;
		// Pass the highlight event id as a fragment so the reader can
		// find the matching substring and overlay it. Buffer ids must be
		// stable per target — append `?highlight=` so a different
		// highlight on the same article gets its own buffer.
		if (target.type === 'a') {
			const parts = target.value.split(':');
			if (parts.length < 3) return;
			const kind = parseInt(parts[0], 10);
			const pubkey = parts[1];
			const d_tag = parts.slice(2).join(':');
			const baseId = kind === 30040
				? `reader:30040:${pubkey}:${d_tag}`
				: `reader:${kind}:${pubkey}:${d_tag}`;
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `${baseId}?highlight=${event.id}`,
					kind: 'reader',
					label: 'reader',
					kicker: 'highlighted'
				}
			});
		} else {
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:event:${target.value}?highlight=${event.id}`,
					kind: 'reader',
					label: 'event',
					kicker: 'highlighted'
				}
			});
		}
	}

	async function loadThread() {
		if (!event || !isComment) return;
		if (threadOpen) {
			threadOpen = false;
			return;
		}
		threadOpen = true;
		if (thread.length > 0) return;
		threadLoading = true;
		threadError = null;
		try {
			// Siblings of this comment: kind 1111 sharing the same root
			// scope. We try `#A` first (addressable root) and fall back
			// to `#E` (event root) if no A tag was on the event.
			const root = refs.root ?? refs.parent;
			if (!root) {
				threadError = 'No root reference on this comment';
				return;
			}
			const filter: Record<string, unknown> = {
				kinds: [1111],
				limit: 200
			};
			if (root.type === 'a') filter['#A'] = [root.value];
			else filter['#E'] = [root.value];
			const resp = await api.queryEvents([filter], 'local_first');
			const others = (resp.events as NostrEvent[]).filter((e) => e.id !== event!.id);
			thread = others.sort((a, b) => a.created_at - b.created_at);
		} catch (e) {
			threadError = e instanceof Error ? e.message : String(e);
		} finally {
			threadLoading = false;
		}
	}

	function short(s: string, n = 12): string {
		return s.length > n ? `${s.slice(0, n)}…` : s;
	}
	function fmtTime(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}
</script>

<div class="dv">
	{#if loading}
		<div class="dv-empty">Loading…</div>
	{:else if error}
		<div class="dv-empty dv-error">{error}</div>
	{:else if event}
		<div class="dv-header">
			<span class="dv-kind-badge" class:dv-kind-badge--c={isComment} class:dv-kind-badge--h={isHighlight}>
				{isHighlight ? 'highlight' : isComment ? 'comment' : `kind ${event.kind}`}
			</span>
			<span class="dv-meta">
				by <code>{short(event.pubkey, 12)}</code> · {fmtTime(event.created_at)}
			</span>
		</div>

		{#if refs.root || refs.parent}
			<div class="dv-refs">
				{#if refs.root}
					<button class="dv-ref" onclick={() => openInReader(refs.root!)} title="Open root in reader">
						<span class="dv-ref-label">root</span>
						<code class="dv-ref-value">{short(refs.root.value, 48)}</code>
					</button>
				{/if}
				{#if refs.parent && (!refs.root || refs.parent.value !== refs.root.value)}
					<button class="dv-ref" onclick={() => openInReader(refs.parent!)} title="Open parent (immediate ancestor) in reader">
						<span class="dv-ref-label">parent</span>
						<code class="dv-ref-value">{short(refs.parent.value, 48)}</code>
					</button>
				{/if}
			</div>
		{/if}

		<div class="dv-body">
			{#if isHighlight}
				<blockquote class="dv-highlight">{event.content || '(empty highlight — non-text source)'}</blockquote>
				{#if (refs.root || refs.parent)}
					<button class="dv-action" onclick={openSourceWithHighlight}>
						Show in source
					</button>
				{/if}
			{:else if isComment}
				<div class="dv-comment">{event.content}</div>
				<button class="dv-action" onclick={loadThread} disabled={threadLoading}>
					{threadOpen ? 'Hide thread' : threadLoading ? 'Loading thread…' : `Show thread`}
					{#if thread.length > 0}
						<span class="dv-thread-count">{thread.length}</span>
					{/if}
				</button>
				{#if threadOpen}
					<div class="dv-thread">
						{#if threadError}
							<div class="dv-error">{threadError}</div>
						{:else if thread.length === 0 && !threadLoading}
							<div class="dv-empty-thread">No siblings found locally.</div>
						{:else}
							{#each thread as t (t.id)}
								<div class="dv-thread-item" class:dv-thread-item--self={t.pubkey === event.pubkey}>
									<div class="dv-thread-meta">
										<code>{short(t.pubkey, 12)}</code> · {fmtTime(t.created_at)}
									</div>
									<div class="dv-thread-body">{t.content}</div>
								</div>
							{/each}
						{/if}
					</div>
				{/if}
			{:else}
				<div class="dv-comment">{event.content}</div>
			{/if}
		</div>

		<details class="dv-raw">
			<summary>Raw event</summary>
			<pre class="dv-raw-pre">{JSON.stringify(event, null, 2)}</pre>
		</details>
	{/if}
</div>

<style>
	.dv {
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 14px 16px;
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}
	.dv-empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.dv-error {
		color: var(--id-draft);
		font-family: var(--font-mono);
	}

	.dv-header {
		display: flex;
		align-items: baseline;
		gap: 10px;
		flex-wrap: wrap;
	}
	.dv-kind-badge {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.dv-kind-badge--c { border-color: var(--id-yours); color: var(--id-yours); }
	.dv-kind-badge--h { border-color: var(--id-draft); color: var(--id-draft); }
	.dv-meta {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.dv-meta code { background: var(--bg-surface); padding: 0 4px; border-radius: var(--r-sm); }

	.dv-refs {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.dv-ref {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 10px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		text-align: left;
		cursor: pointer;
	}
	.dv-ref:hover {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.dv-ref-label {
		color: var(--base5);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		min-width: 42px;
	}
	.dv-ref-value {
		color: inherit;
		background: transparent;
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.dv-body {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.dv-highlight {
		margin: 0;
		padding: 12px 14px;
		border-left: 3px solid var(--id-draft);
		background: color-mix(in srgb, var(--id-draft) 6%, transparent);
		color: var(--fg);
		font-style: italic;
		white-space: pre-wrap;
	}
	.dv-comment {
		padding: 8px 0;
		color: var(--fg);
		white-space: pre-wrap;
		line-height: 1.55;
	}

	.dv-action {
		align-self: flex-start;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 5px 12px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.dv-action:hover:not(:disabled) {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.dv-action:disabled { opacity: 0.5; cursor: not-allowed; }
	.dv-thread-count {
		margin-left: 6px;
		padding: 0 5px;
		border-radius: var(--r-sm);
		background: var(--bg-surface);
		color: var(--base5);
	}

	.dv-thread {
		display: flex;
		flex-direction: column;
		gap: 8px;
		margin-top: 4px;
		padding-left: 10px;
		border-left: 2px solid var(--panel-border);
	}
	.dv-thread-item {
		padding: 8px 10px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
	}
	.dv-thread-item--self {
		border-color: var(--id-yours);
	}
	.dv-thread-meta {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-bottom: 4px;
	}
	.dv-thread-meta code { background: transparent; }
	.dv-thread-body {
		color: var(--fg);
		font-size: var(--t-xs);
		white-space: pre-wrap;
		line-height: 1.5;
	}
	.dv-empty-thread {
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
	}

	.dv-raw {
		font-size: var(--t-xs);
		color: var(--base5);
		margin-top: 8px;
	}
	.dv-raw-pre {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-surface);
		padding: 8px;
		border-radius: var(--r-sm);
		overflow-x: auto;
		white-space: pre-wrap;
		word-break: break-all;
		max-height: 240px;
	}
</style>
