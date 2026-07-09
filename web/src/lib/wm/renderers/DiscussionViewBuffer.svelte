<script lang="ts">
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import type { Buffer } from '../types';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import { countThread, flattenThread, type ThreadNode } from '$lib/discussions/thread';
	import { prefetchAuthors } from '$lib/discussions/authors.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();
	const app = getAppState();

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

	// "Pull thread" state — the thread tree built from every kind-1111
	// comment sharing this comment's root scope, plus the resolved root
	// event the conversation hangs off.
	let threadNodes = $state<ThreadNode[]>([]);
	let threadCount = $state(0);
	let rootEvent = $state<NostrEvent | null>(null);
	let threadOpen = $state(false);
	let threadLoading = $state(false);
	let threadError = $state<string | null>(null);

	const ROOT_KIND_LABEL: Record<number, string> = {
		1: 'note',
		1111: 'comment',
		30023: 'article',
		30040: 'publication',
		30041: 'section',
		30818: 'wiki'
	};

	const eventId = $derived(parseBufferId(buffer.id));

	function parseBufferId(id: string): string | null {
		const m = id.match(/^discussion:([0-9a-fA-F]{64})$/);
		return m ? m[1].toLowerCase() : null;
	}

	// A bare 32-byte event id: 64 hex chars, nothing else.
	function isHexId(v: string): boolean {
		return /^[0-9a-f]{64}$/i.test(v);
	}
	// An addressable coordinate: `kind:pubkey:d_tag` (d_tag may be empty).
	function isAddr(v: string): boolean {
		const p = v.split(':');
		return p.length >= 3 && /^\d+$/.test(p[0]) && isHexId(p[1]);
	}

	// NIP-22 + NIP-84 tag conventions:
	//   uppercase A/E = root scope (the top of the thread)
	//   lowercase a/e = parent (immediate ancestor)
	// For a top-level comment they're identical. For a nested reply they
	// diverge — root is the article, parent is the comment being replied to.
	//
	// The ref value is validated before use: some clients write malformed
	// tags (a relay URL in the value slot). An unvalidated value would flow
	// into the thread-pull filter as a bogus `#e`/`#a`, degenerate it into
	// an unconstrained `kinds:[1111]` query, and dump 500 unrelated
	// comments. Skipping a bad tag lets a later well-formed one still match.
	function extractRefs(ev: NostrEvent): { root: ParentRef | null; parent: ParentRef | null } {
		let root: ParentRef | null = null;
		let parent: ParentRef | null = null;
		for (const tag of ev.tags) {
			if (!tag || tag.length < 2) continue;
			const [name, value, relay, pk] = tag as [string, string, string?, string?];
			if (name === 'A' && !root && isAddr(value)) root = { type: 'a', value, relay };
			else if (name === 'E' && !root && isHexId(value)) root = { type: 'e', value, relay, pubkey: pk };
			else if (name === 'a' && !parent && isAddr(value)) parent = { type: 'a', value, relay };
			else if (name === 'e' && !parent && isHexId(value)) parent = { type: 'e', value, relay, pubkey: pk };
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
		// Drop any thread pulled for a previously-viewed comment.
		threadNodes = [];
		threadCount = 0;
		rootEvent = null;
		threadOpen = false;
		threadError = null;
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

	// Apply an engine-built thread forest. The forest already includes the
	// comment being viewed — it tags the root we queried by and is locally
	// cached (the user is reading it), so it's in the result set without a
	// client-side inject. Count every node and warm the authors.
	function applyThreads(threads: ThreadNode[]) {
		threadNodes = threads;
		threadCount = countThread(threads);
		const authors = new Set(flattenThread(threads).map((n) => n.event.pubkey));
		authors.delete('');
		if (authors.size > 0) prefetchAuthors([...authors]);
	}

	// Pick this root's forest from a threaded discussions response: address
	// roots are grouped under their coordinate; an event-id root has no
	// address key, so it gets the flat forest.
	function threadsFromResp(resp: api.DiscussionsListResponse, root: ParentRef): ThreadNode[] {
		return root.type === 'a'
			? resp.threads_by_address?.[root.value] ?? []
			: resp.threads ?? [];
	}

	// Pull the whole thread in two phases:
	//   1. local_only — render whatever's already in nostrdb, instantly.
	//   2. fetch_always — refresh from relays. `bypassOffline` routes the
	//      call through the engine's confirm gate: a modal in Confirm
	//      mode, automatic in Auto. A declined modal degrades to a local
	//      read, so the worst case is the tree staying at the phase-1
	//      result — never an error, never a blank screen.
	async function pullThread() {
		if (!event || !isComment) return;
		// Already pulled — the button is just a visibility toggle now.
		if (threadOpen && threadNodes.length > 0) {
			threadOpen = false;
			return;
		}
		threadOpen = true;
		threadLoading = true;
		threadError = null;
		try {
			const root = refs.root ?? refs.parent;
			if (!root) {
				threadError = 'No root reference on this comment';
				return;
			}
			const baseOpts: Parameters<typeof api.getDiscussionList>[0] = {
				kinds: [1111],
				limit: 500,
				threaded: true
			};
			if (root.type === 'a') baseOpts.addresses = [root.value];
			else baseOpts.eventIds = [root.value];

			// Phase 1 — instant local render.
			const local = await api.getDiscussionList({ ...baseOpts, policy: 'local_only' });
			applyThreads(threadsFromResp(local, root));
			// Resolve the root header from cache while we're here.
			await loadRootEvent(root);

			// Phase 2 — relay refresh, gated by the engine.
			const fresh = await api.getDiscussionList({
				...baseOpts,
				policy: 'fetch_always',
				bypassOffline: true
			});
			applyThreads(threadsFromResp(fresh, root));
		} catch (e) {
			threadError = e instanceof Error ? e.message : String(e);
		} finally {
			threadLoading = false;
		}
	}

	async function loadRootEvent(root: ParentRef) {
		try {
			if (root.type === 'a') {
				const parts = root.value.split(':');
				if (parts.length < 3) return;
				const kind = parseInt(parts[0], 10);
				const pubkey = parts[1];
				const d_tag = parts.slice(2).join(':');
				const resp = await api.getAddressable(kind, pubkey, d_tag);
				rootEvent = (resp.event as NostrEvent | null) ?? null;
			} else {
				const resp = await api.getEvent(root.value);
				rootEvent = (resp.event as NostrEvent | null) ?? null;
			}
		} catch {
			rootEvent = null;
		}
	}

	function tagValue(ev: NostrEvent, name: string): string | null {
		return ev.tags.find((t) => t[0] === name)?.[1] ?? null;
	}

	const rootKindLabel = $derived(
		rootEvent ? (ROOT_KIND_LABEL[rootEvent.kind] ?? `kind ${rootEvent.kind}`) : ''
	);
	const rootTitle = $derived(
		rootEvent
			? tagValue(rootEvent, 'title') ??
					tagValue(rootEvent, 'd') ??
					(rootEvent.content ? rootEvent.content.slice(0, 80) : short(rootEvent.id, 16))
			: ''
	);

	function copyRaw(): void {
		if (!event) return;
		try {
			navigator.clipboard?.writeText(JSON.stringify(event, null, 2));
			app.pushToast('Raw event copied', 'success');
		} catch {
			app.pushToast("Couldn't copy raw event", 'error');
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
				<button class="dv-action" onclick={pullThread} disabled={threadLoading}>
					{threadLoading
						? 'Pulling thread…'
						: threadOpen && threadNodes.length > 0
							? 'Hide thread'
							: 'Pull thread'}
					{#if threadCount > 0}
						<span class="dv-thread-count">{threadCount}</span>
					{/if}
				</button>
				{#if threadOpen}
					<div class="dv-thread-wrap">
						{#if threadError}
							<div class="dv-error">{threadError}</div>
						{/if}
						{#if rootEvent}
							<div class="dv-root">
								<span class="dv-root-label">thread root</span>
								<button
									class="dv-root-card"
									onclick={() => {
										const r = refs.root ?? refs.parent;
										if (r) openInReader(r);
									}}
									title="Open the root in the reader"
								>
									<span class="dv-root-kind">{rootKindLabel}</span>
									<span class="dv-root-title">{rootTitle}</span>
								</button>
							</div>
						{/if}
						{#if threadNodes.length > 0}
							<CommentThread nodes={threadNodes} focusedEventId={event.id} />
						{:else if !threadLoading && !threadError}
							<div class="dv-empty-thread">No comments found in this thread.</div>
						{/if}
					</div>
				{/if}
			{:else}
				<div class="dv-comment">{event.content}</div>
			{/if}
		</div>

		<details class="dv-raw">
			<summary>
				Raw event
				<button
					class="dv-raw-copy"
					title="Copy raw event JSON"
					onclick={(e) => {
						e.preventDefault();
						e.stopPropagation();
						copyRaw();
					}}>copy</button
				>
			</summary>
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

	.dv-thread-wrap {
		display: flex;
		flex-direction: column;
		gap: 10px;
		margin-top: 4px;
	}
	.dv-root {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.dv-root-label {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
	}
	.dv-root-card {
		display: flex;
		align-items: baseline;
		gap: 8px;
		padding: 8px 10px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		text-align: left;
		cursor: pointer;
	}
	.dv-root-card:hover {
		border-color: var(--id-yours);
	}
	.dv-root-kind {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		flex-shrink: 0;
	}
	.dv-root-title {
		color: var(--fg);
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
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
	.dv-raw-copy {
		margin-left: 8px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		padding: 1px 8px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: transparent;
		color: var(--base5);
		cursor: pointer;
	}
	.dv-raw-copy:hover {
		border-color: var(--id-yours);
		color: var(--id-yours);
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
