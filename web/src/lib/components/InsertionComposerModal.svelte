<script lang="ts">
	// Create annotation — the one form behind every insertion door (a
	// section's ⋮, the M menu, a capture card). An insertion is a kind-30041
	// section anchored to a place in a publication (NKBIP-01 insertions
	// extension, nips/NKBIP-01-insertions.org): the anchor tags are emitted by
	// the engine and only *shown* here; the user chooses placement (attach =
	// margin note · insert before / after = claims a slot), title, body and
	// format. The wire preview comes from `/insertions/preview` — the same
	// builder the publish path signs, so nothing here can drift from it.
	// Signs as you; assistant-authored insertions come through the tools.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));
	// svelte-ignore state_referenced_locally
	const open = app.insertionComposer;
	/** The same anchor, in the request window, with the chosen mode — the
	 *  one annotate window read as four modes across two forms. */
	function askInstead(output: 'notes' | 'kasten' | 'vocabulary') {
		if (!open) return;
		const { placement: _p, ...anchor } = open;
		app.insertionComposer = null;
		app.annotationRequest = { ...anchor, output };
	}

	type Placement = 'attach' | 'before' | 'after';
	type Format = 'markdown' | 'org' | 'asciidoc' | 'plain';
	const PLACEMENTS: { id: Placement; label: string; hint: string }[] = [
		{ id: 'attach', label: 'attach', hint: 'a margin note — no place in the reading order' },
		{ id: 'before', label: 'insert before', hint: 'claims the slot before this section' },
		{ id: 'after', label: 'insert after', hint: 'claims the slot after this section' }
	];
	const FORMATS: Format[] = ['markdown', 'org', 'asciidoc', 'plain'];

	// Prefill: the open payload wins per field; otherwise the draft from the
	// last close. Publish clears the memory; plain close keeps it.
	const draft = app.insertionDraft;
	let root = $state(open?.root ?? '');
	let placement = $state<Placement>(
		open?.placement ?? ((draft?.placement as Placement | undefined) ?? 'attach')
	);
	let title = $state(draft?.title ?? '');
	let content = $state(draft?.content ?? '');
	let format = $state<Format>((draft?.format as Format | undefined) ?? 'markdown');
	let context = $state(open?.context ?? '');
	let posting = $state(false);

	const parent = $derived(open?.parent ?? null);
	const topLevel = $derived(!parent || parent === root);
	const roots = $derived(open?.roots ?? []);
	const rootTitle = $derived(
		roots.find((r) => r.coord === root)?.title ?? open?.rootTitle ?? null
	);

	// Confirm mode: the publish intent renders PublishConfirmModal beneath
	// us; yield while it decides (hidden, inert), come back with state intact.
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');

	const canPost = $derived(
		canSign && !posting && !!root && title.trim().length > 0 && content.trim().length > 0
	);

	function short(coord: string): string {
		const [k, pk, ...d] = coord.split(':');
		return `${k}:${pk?.slice(0, 8)}…:${d.join(':')}`;
	}

	function request(broadcast: boolean): api.InsertionRequest {
		return {
			anchor: {
				root,
				parent: topLevel ? undefined : (parent ?? undefined),
				parent_event_id: open?.parentEventId,
				placement
			},
			title: title.trim(),
			content: content.trim(),
			format: format === 'markdown' ? undefined : format,
			context: context.trim() || undefined,
			broadcast
		};
	}

	// --- Event JSON preview (engine builder, debounced) ---------------------
	let jsonOpen = $state(false);
	let jsonText = $state<string | null>(null);
	let jsonErr = $state<string | null>(null);
	let jsonTimer: ReturnType<typeof setTimeout> | undefined;
	let jsonToken = 0;
	$effect(() => {
		if (!jsonOpen) return;
		const req = request(true);
		clearTimeout(jsonTimer);
		jsonTimer = setTimeout(async () => {
			const token = ++jsonToken;
			if (!req.title || !req.content) {
				jsonText = null;
				jsonErr = null;
				return;
			}
			try {
				const resp = await api.previewInsertion(req);
				if (token !== jsonToken) return;
				jsonText = JSON.stringify(resp.event, null, 2);
				jsonErr = null;
			} catch (e) {
				if (token !== jsonToken) return;
				jsonText = null;
				jsonErr = api.errorMessage(e, 'preview failed');
			}
		}, 350);
		return () => clearTimeout(jsonTimer);
	});

	function close(rememberDraft = true) {
		app.insertionDraft =
			rememberDraft && (title.trim() || content.trim())
				? { title, content, format, placement }
				: null;
		app.insertionComposer = null;
	}

	async function post(broadcast: boolean) {
		if (!canPost) return;
		posting = true;
		try {
			const resp = await api.publishInsertion(request(broadcast));
			const results = resp.broadcast_results ?? [];
			const ok = results.filter((r) => r.success).length;
			const word = placement === 'attach' ? 'Annotation' : 'Insertion';
			app.pushToast(
				!broadcast
					? `${word} signed and kept local`
					: results.length === 0
						? `${word} saved locally (no publish relays)`
						: ok === 0
							? `${word} saved locally — 0/${results.length} relays accepted`
							: `${word} published (${ok}/${results.length} relays)`,
				broadcast && ok === 0 && results.length > 0 ? 'error' : 'success'
			);
			app.bumpInsertions();
			close(false);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Annotation failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (posting) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			void post(true);
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="ins-backdrop {yielding ? 'ins-backdrop--yield' : ''}"
	onclick={() => {
		if (!posting) close();
	}}
	role="presentation"
>
	<div
		class="ins-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Create annotation"
		tabindex="-1"
	>
		<header class="ins-header">
			<h3 class="ins-title">
				<span class="ins-dot" aria-hidden="true">●</span>
				write a note
				{#if topLevel}
					<span class="ins-title__on">— on the publication</span>
				{:else if open?.parentTitle}
					<span class="ins-title__on">— on <em>{open.parentTitle}</em></span>
				{/if}
			</h3>
			<button class="ins-close" onclick={() => close()} aria-label="Close">×</button>
		</header>

		<div class="ins-label">mode <span class="ins-label__sub">— write it yourself, or ask your agent for notes, a kasten, or vocabulary</span></div>
		<div class="ins-seg" role="radiogroup" aria-label="Mode">
			<button class="ins-seg__opt on" role="radio" aria-checked="true" title="Write the note yourself (this form)">write</button>
			<button class="ins-seg__opt" role="radio" aria-checked="false" onclick={() => askInstead('notes')} title="Ask your agent for loose notes attached here">notes</button>
			<button class="ins-seg__opt" role="radio" aria-checked="false" onclick={() => askInstead('kasten')} title="Ask for several notes plus an index — promotable as a sub-chapter">kasten</button>
			<button class="ins-seg__opt" role="radio" aria-checked="false" onclick={() => askInstead('vocabulary')} title="Ask for one definition per word — highlight the words in the text">vocabulary</button>
		</div>

		<div class="ins-label">anchor</div>
		<div class="ins-anchor">
			{#if !topLevel && parent}
				<span class="ins-chip" title={parent}><b>a</b> {short(parent)}</span>
			{/if}
			{#if roots.length > 1}
				<label class="ins-chip ins-chip--select" title="Which publication this note belongs to">
					<b>A</b>
					<select bind:value={root} disabled={posting}>
						{#each roots as r (r.coord)}
							<!-- Two editions can share a title (an original and a
							     derivative); the author prefix tells them apart. -->
							<option value={r.coord}>{r.title ?? short(r.coord)} · {r.coord.split(':')[1]?.slice(0, 8)}…</option>
						{/each}
					</select>
				</label>
			{:else}
				<span class="ins-chip" title={root}><b>A</b> {rootTitle ?? short(root)}</span>
			{/if}
		</div>
		<div class="ins-help">
			Emitted by the engine — never typed. Open the menu on another section to change the target
			{#if root.startsWith('30041:')}
				· this note lives in no index, so it is its own root
			{/if}
		</div>

		<div class="ins-label">placement</div>
		<div class="ins-seg" role="radiogroup" aria-label="Placement">
			{#each PLACEMENTS as p (p.id)}
				<button
					class="ins-seg__opt"
					class:on={placement === p.id}
					role="radio"
					aria-checked={placement === p.id}
					onclick={() => (placement = p.id)}
					disabled={posting}
					title={p.hint}
				>{topLevel && p.id !== 'attach' ? (p.id === 'before' ? 'front matter' : 'appendix') : p.label}</button>
			{/each}
		</div>
		<div class="ins-help">{PLACEMENTS.find((p) => p.id === placement)?.hint}</div>

		<label class="ins-label" for="ins-title">title</label>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			id="ins-title"
			class="ins-input"
			data-entry
			autofocus
			placeholder="A modern reading of…"
			bind:value={title}
			disabled={posting}
		/>

		<div class="ins-label ins-label--row">
			<label for="ins-body">body</label>
			<span class="ins-seg ins-seg--mini" role="radiogroup" aria-label="Format">
				{#each FORMATS as f (f)}
					<button
						class="ins-seg__opt"
						class:on={format === f}
						role="radio"
						aria-checked={format === f}
						onclick={() => (format = f)}
						disabled={posting}
					>{f}</button>
				{/each}
			</span>
		</div>
		<textarea
			id="ins-body"
			class="ins-input ins-body"
			data-entry
			placeholder="The section text — this becomes a kind-30041 section, promotable into the book as-is."
			bind:value={content}
			disabled={posting}
		></textarea>

		<label class="ins-label" for="ins-context">
			context <span class="ins-label__sub">· optional excerpt this note responds to</span>
		</label>
		<textarea
			id="ins-context"
			class="ins-input ins-context"
			data-entry
			placeholder="From your selection, or paste a line of the parent"
			bind:value={context}
			disabled={posting}
		></textarea>

		<button class="ins-json-toggle" onclick={() => (jsonOpen = !jsonOpen)}>
			{jsonOpen ? '▾' : '▸'} event json
		</button>
		{#if jsonOpen}
			{#if jsonErr}
				<div class="ins-help ins-help--bad">{jsonErr}</div>
			{:else if jsonText}
				<pre class="ins-json">{jsonText}</pre>
			{:else}
				<div class="ins-help">enter a title and body to preview the event</div>
			{/if}
		{/if}

		<footer class="ins-foot">
			<span class="ins-hint">
				{#if !canSign}
					sign in to publish
				{:else}
					signs as you · Ctrl-Enter publishes
				{/if}
			</span>
			<span class="ins-spacer"></span>
			<button class="ins-btn" onclick={() => close()} disabled={posting}>Cancel</button>
			<button
				class="ins-btn"
				onclick={() => post(false)}
				disabled={!canPost}
				title="Sign the section and keep it local — broadcast later from its menu"
			>sign · keep local</button>
			<button class="ins-btn ins-btn--post" onclick={() => post(true)} disabled={!canPost}>
				{posting ? 'Publishing…' : 'sign · broadcast'}
			</button>
		</footer>
	</div>
</div>

<style>
	.ins-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 8dvh;
	}
	.ins-backdrop--yield {
		visibility: hidden;
	}
	.ins-modal {
		width: min(600px, 94vw);
		max-height: 88dvh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}
	.ins-seg {
		display: inline-flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		overflow: hidden;
		align-self: flex-start;
	}
	.ins-seg__opt {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: transparent;
		border: none;
		border-right: 1px solid var(--panel-border);
		padding: 4px 10px;
		cursor: pointer;
	}
	.ins-seg__opt:last-child {
		border-right: none;
	}
	.ins-seg__opt.on {
		background: var(--id-remote);
		color: var(--bg);
	}
	.ins-label__sub {
		text-transform: none;
		letter-spacing: 0;
		font-weight: 400;
		color: var(--base5);
	}
	.ins-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.ins-title {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--fg);
		display: flex;
		align-items: baseline;
		gap: 6px;
		flex-wrap: wrap;
	}
	.ins-dot {
		color: var(--id-local);
	}
	.ins-title__on {
		color: var(--base5);
		font-weight: 400;
	}
	.ins-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
	}
	.ins-close:hover {
		color: var(--fg);
	}
	.ins-label {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-top: 6px;
	}
	.ins-label--row {
		display: flex;
		align-items: center;
		gap: 10px;
	}
	.ins-label__sub {
		text-transform: none;
		letter-spacing: 0;
	}
	.ins-anchor {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.ins-chip {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--fg-muted);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 2px 8px;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.ins-chip b {
		color: var(--id-local);
	}
	.ins-chip--select select {
		font: inherit;
		color: var(--fg);
		background: transparent;
		border: none;
		max-width: 40ch;
	}
	.ins-help {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	.ins-help--bad {
		color: var(--error, #e06c75);
	}
	.ins-seg {
		display: inline-flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		overflow: hidden;
		align-self: flex-start;
	}
	.ins-seg__opt {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: transparent;
		border: none;
		border-right: 1px solid var(--panel-border);
		padding: 4px 10px;
		min-height: 30px;
		cursor: pointer;
	}
	.ins-seg__opt:last-child {
		border-right: none;
	}
	.ins-seg__opt.on {
		background: var(--id-local);
		color: var(--bg);
	}
	.ins-seg--mini .ins-seg__opt {
		padding: 1px 7px;
		min-height: 22px;
		font-size: calc(var(--t-xs) - 2px);
	}
	.ins-input {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 5px 8px;
	}
	.ins-input:focus {
		outline: none;
		border-color: var(--id-local);
	}
	.ins-body {
		min-height: 140px;
		resize: vertical;
		font-family: var(--rd-font, var(--font-sans));
	}
	.ins-context {
		min-height: 40px;
		resize: vertical;
	}
	.ins-json-toggle {
		align-self: flex-start;
		background: none;
		border: none;
		padding: 0;
		margin-top: 4px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		cursor: pointer;
	}
	.ins-json-toggle:hover {
		color: var(--fg);
	}
	.ins-json {
		margin: 0;
		padding: 6px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--fg-muted);
		max-height: 200px;
		overflow: auto;
		white-space: pre;
	}
	.ins-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 6px;
		flex-wrap: wrap;
	}
	.ins-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.ins-spacer {
		flex: 1;
	}
	.ins-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		min-height: 30px;
		cursor: pointer;
	}
	.ins-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.ins-btn--post {
		color: var(--id-local);
	}
	.ins-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
