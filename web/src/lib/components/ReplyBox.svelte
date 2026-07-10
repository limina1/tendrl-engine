<script lang="ts">
	// Comment composer — the one write surface for kind-1111 comments. Plain
	// textarea by design (NIP-22 content is plaintext; worksheet C6). Tag
	// construction, signing, local ingest, and broadcast all happen engine-side
	// behind api.publishComment; this component only collects text and reports.
	//
	// Exactly one of `root` / `parent`:
	// - `root` → top-level comment on that scope (address / event / external);
	// - `parent` → threaded reply to that comment (full event passed along as
	//   the engine's fallback copy — worksheet B1 hybrid).
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';

	let {
		root = undefined,
		parent = undefined,
		placeholder = 'Write a comment…',
		autofocus = false,
		compact = false,
		onposted = undefined,
		oncancel = undefined
	}: {
		root?: api.CommentRootRef;
		parent?: api.DiscussionEvent;
		placeholder?: string;
		autofocus?: boolean;
		/** Reply-under-a-node styling: tighter, with a cancel affordance. */
		compact?: boolean;
		/** Fired after a successful post — parents refresh their thread via
		 *  a local_only getDiscussionList (the engine ingested before
		 *  responding, so the refetch already includes the new comment). */
		onposted?: (resp: api.DiscussionPublishResponse) => void;
		oncancel?: () => void;
	} = $props();

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));

	let text = $state('');
	let posting = $state(false);

	async function post() {
		const content = text.trim();
		if (!content || posting || !canSign) return;
		posting = true;
		try {
			const resp = await api.publishComment({
				root: parent ? undefined : root,
				parent: parent ? { event_id: parent.id, event: parent } : undefined,
				content
			});
			text = '';
			const { successful, total } = resp.broadcast;
			if (total === 0) {
				app.pushToast('Comment saved locally (no publish relays)', 'info');
			} else if (successful === 0) {
				app.pushToast(`Comment saved locally — 0/${total} relays accepted`, 'error', 5000);
			} else {
				app.pushToast(`Comment published (${successful}/${total} relays)`, 'success');
			}
			onposted?.(resp);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Comment failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	function onkeydown(e: KeyboardEvent) {
		if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			post();
		} else if (e.key === 'Escape' && oncancel) {
			e.preventDefault();
			oncancel();
		}
	}

	function focusOnMount(node: HTMLTextAreaElement) {
		if (autofocus) requestAnimationFrame(() => node.focus());
	}
</script>

<div class="rb" class:rb--compact={compact}>
	<textarea
		class="rb-input"
		bind:value={text}
		{placeholder}
		rows={compact ? 2 : 3}
		disabled={!canSign || posting}
		onkeydown={onkeydown}
		use:focusOnMount
	></textarea>
	<div class="rb-foot">
		{#if !canSign}
			<!-- Persistent affordances teach (worksheet C7): visible, disabled, honest. -->
			<span class="rb-hint">Sign in to comment</span>
		{:else}
			<span class="rb-hint">Ctrl-Enter to post</span>
		{/if}
		<span class="rb-spacer"></span>
		{#if oncancel}
			<button class="rb-btn" onclick={oncancel} disabled={posting}>Cancel</button>
		{/if}
		<button
			class="rb-btn rb-btn--post"
			onclick={post}
			disabled={!canSign || posting || !text.trim()}
		>
			{posting ? 'Posting…' : parent ? 'Reply' : 'Comment'}
		</button>
	</div>
</div>

<style>
	.rb {
		margin-top: 6px;
	}
	.rb--compact {
		margin: 4px 0 4px calc(1ch + 6px + 8px + 6px);
	}
	.rb-input {
		width: 100%;
		box-sizing: border-box;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.5;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 6px 8px;
		resize: vertical;
	}
	.rb-input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.rb-input:disabled {
		color: var(--fg-muted);
		cursor: not-allowed;
	}
	.rb-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 4px;
	}
	.rb-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.rb-spacer {
		flex: 1;
	}
	.rb-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.rb-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rb-btn--post {
		color: var(--id-yours);
	}
	.rb-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
