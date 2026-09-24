<script lang="ts">
	// The thread under one annotation: its kind-1111 replies (NIP-22,
	// scoped to the annotation's coordinate as the root address), a reply
	// box that signs one, and "note in reply ▸" — a full 30041 anchored
	// beside the annotation with a `q` to it (the composer, with
	// `replyTo`). Contract: docs/annotations-ledger-contract.org, "Thread".
	// Mounted by the ledger's row body, the reader's cards (compact) and
	// the review buffer. Loads on first mount; the `count` prop (the
	// engine's `replies` map) stands in until then.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { getAuthorDisplayName, prefetchAuthors } from '$lib/discussions/authors.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import type { ThreadNode } from '$lib/discussions/thread';
	import { relTime } from './ledger-model';

	let {
		root,
		coordinate,
		author,
		title,
		compact = false,
		count = undefined,
		parent = undefined,
		parentTitle = undefined,
		rootTitle = undefined,
		onreplied = undefined
	}: {
		/** The publication root (30040:pk:d) the annotation belongs to. */
		root: string;
		/** The annotation's coordinate — the thread's root address. An
		 *  `818:<id>` coordinate threads under the 818 event id instead. */
		coordinate: string;
		/** The annotation's author (hex). */
		author: string;
		title: string;
		/** Reader cards: no "note in reply" button, tighter spacing. */
		compact?: boolean;
		/** The engine's reply count, shown until the thread loads. */
		count?: number | undefined;
		/** The anchor a note in reply goes to (the annotation's own). */
		parent?: string | undefined;
		parentTitle?: string | null | undefined;
		rootTitle?: string | null | undefined;
		onreplied?: (() => void) | undefined;
	} = $props();

	const app = getAppState();
	const is818 = $derived(coordinate.startsWith('818:'));
	const canSign = $derived(identityCanSign(app.identityStatus));

	let nodes = $state<ThreadNode[]>([]);
	let loaded = $state(false);
	let loading = $state(false);
	let text = $state('');
	let signing = $state(false);
	let signedLocally = $state(0);
	const book = { loading: false };

	function flatten(forest: ThreadNode[], out: ThreadNode[] = []): ThreadNode[] {
		for (const n of forest) {
			out.push(n);
			flatten(n.children, out);
		}
		return out;
	}
	const replies = $derived(flatten(nodes));
	const shown = $derived(loaded ? replies.length : (count ?? 0) + signedLocally);

	async function load() {
		if (book.loading) return;
		book.loading = true;
		loading = true;
		try {
			const resp = is818
				? await api.getDiscussionList({ eventIds: [coordinate.slice(4)], kinds: [1111], policy: 'local_only', threaded: true })
				: await api.getDiscussionList({ addresses: [coordinate], kinds: [1111], policy: 'local_only', threaded: true });
			const forest = is818 ? (resp.threads ?? []) : (resp.threads_by_address?.[coordinate] ?? []);
			nodes = forest;
			loaded = true;
			const pks = new Set(flatten(forest).map((n) => n.event.pubkey));
			if (pks.size) prefetchAuthors([...pks]);
		} catch {
			loaded = true;
		} finally {
			book.loading = false;
			loading = false;
		}
	}
	$effect(() => {
		void coordinate;
		loaded = false;
		nodes = [];
		void load();
	});

	function who(pk: string): string {
		return app.identityStatus?.pubkey === pk ? 'you' : getAuthorDisplayName(pk);
	}
	async function sendReply() {
		const content = text.trim();
		if (!content || signing) return;
		if (!canSign) {
			app.pushToast('Sign in to reply', 'info');
			return;
		}
		signing = true;
		try {
			const rootRef: api.CommentRootRef = is818
				? { event_id: coordinate.slice(4), kind: 818 }
				: { address: coordinate };
			await api.publishComment({ root: rootRef, content });
			text = '';
			signedLocally += 1;
			app.pushToast(`reply signed under “${title}” — the thread grows, the annotation is untouched`, 'success');
			await load();
			onreplied?.();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Reply failed'), 'error', 5000);
		} finally {
			signing = false;
		}
	}
	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			void sendReply();
		}
		e.stopPropagation();
	}
	function noteInReply() {
		if (is818) {
			app.pushToast('A merge request takes replies, not notes', 'info');
			return;
		}
		app.insertionComposer = {
			root,
			rootTitle: rootTitle ?? null,
			parent: parent && parent !== root ? parent : undefined,
			parentTitle: parentTitle ?? null,
			placement: 'attach',
			replyTo: coordinate,
			replyToTitle: title
		};
	}
	const pkShort = (pk: string) => (pk ? pk.slice(0, 8) + '…' : '');
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="thread" class:thread--compact={compact} onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.stopPropagation()}>
	<div class="thread__h">
		replies <span class="thread__n">· {shown} · kind 1111, threaded under this event</span>
		<span class="thread__sp"></span>
		{#if !compact}
			<button class="thread__note" onclick={noteInReply} title="A full note (30041) anchored beside this one, carrying a reference to it">note in reply ▸</button>
		{/if}
	</div>
	{#if loading && !loaded}
		<div class="thread__wait">loading…</div>
	{/if}
	{#each replies as n (n.event.id)}
		<div class="reply" data-id={n.event.id} style="--au: {pubkeyToColor(n.event.pubkey)}; --depth: {n.depth}">
			<span class="reply__rail"></span>
			<div class="reply__body">
				<div class="reply__h"><span class="reply__dot"></span>{who(n.event.pubkey)} · {relTime(n.event.created_at)}</div>
				<div class="reply__b">{n.event.content}</div>
			</div>
		</div>
	{/each}
	<div class="replybox">
		<textarea
			class="replybox__text"
			rows="1"
			placeholder="reply…"
			bind:value={text}
			{onkeydown}
			disabled={signing}
			aria-label="Reply"
		></textarea>
		<div class="replybox__col">
			<button class="replybox__sign" onclick={sendReply} disabled={signing || !text.trim()}>{signing ? 'signing…' : 'sign reply ▸'}</button>
		</div>
		<span class="replybox__wire">
			{#if is818}
				{'{'} "kind": <span class="k">1111</span>, tags: [<span class="k">E</span> {coordinate.slice(4, 12)}…] [<span class="k">K</span> 818] [<span class="k">P</span> {pkShort(author)}] {'}'} — a comment; it never changes the merge request
			{:else}
				{'{'} "kind": <span class="k">1111</span>, tags: [<span class="k">A</span> {root}] [<span class="k">a</span> {coordinate}] [<span class="k">K</span> 30040] [<span class="k">k</span> 30041] [<span class="k">p</span> {pkShort(author)}] {'}'} — a comment; it never changes the annotation
			{/if}
		</span>
	</div>
</div>

<style>
	.thread {
		margin-top: 8px;
		border-top: 1px dashed var(--panel-border);
		padding-top: 6px;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-family: var(--font-sans);
	}
	.thread--compact { margin-top: 6px; }
	.thread__h {
		display: flex;
		gap: 8px;
		align-items: center;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		color: var(--base5);
		letter-spacing: 0.08em;
		text-transform: uppercase;
	}
	.thread__n { color: var(--base4); letter-spacing: 0; font-weight: 400; text-transform: none; }
	.thread__sp { flex: 1; }
	.thread__note {
		font: var(--t-3xs) / 1.2 var(--font-mono);
		min-height: 20px;
		padding: 0 7px;
		color: var(--yellow);
		letter-spacing: 0;
		text-transform: none;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		background: transparent;
		cursor: pointer;
	}
	.thread__note:hover { border-color: var(--base5); }
	.thread__wait { font: var(--t-3xs) / 1.3 var(--font-mono); color: var(--base4); }
	.reply {
		display: grid;
		grid-template-columns: 3px minmax(0, 1fr);
		gap: 0 8px;
		margin-left: calc(var(--depth, 0) * 12px);
	}
	.reply__rail { background: var(--au); border-radius: 1px; }
	.reply__h { font: var(--t-2xs) / 1.3 var(--font-mono); color: var(--base5); display: flex; gap: 6px; align-items: center; }
	.reply__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); display: inline-block; flex: none; }
	.reply__b { font: var(--t-base) / 1.45 var(--font-serif); color: var(--fg); max-width: 62ch; white-space: pre-wrap; word-break: break-word; }
	.replybox {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		gap: 6px;
		align-items: start;
		margin-top: 4px;
	}
	.replybox__text {
		min-height: 30px;
		font: var(--t-base) / 1.4 var(--font-sans);
		padding: 5px 8px;
		resize: vertical;
		color: var(--fg);
		background: var(--base1);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 100%;
	}
	.replybox__text:focus { outline: none; border-color: var(--blue); }
	.replybox__col { display: flex; flex-direction: column; gap: 3px; align-items: flex-end; }
	.replybox__sign {
		font: var(--t-3xs) / 1.2 var(--font-mono);
		min-height: 22px;
		padding: 1px 8px;
		color: var(--cyan);
		border: 1px solid rgba(137, 184, 194, 0.4);
		border-radius: var(--r-md);
		background: transparent;
		white-space: nowrap;
		cursor: pointer;
	}
	.replybox__sign:disabled { opacity: 0.45; cursor: default; }
	.replybox__wire {
		font: var(--t-3xs) / 1.3 var(--font-mono);
		color: var(--base4);
		grid-column: 1 / -1;
		word-break: break-all;
	}
	.replybox__wire .k { color: var(--cyan); }
</style>
