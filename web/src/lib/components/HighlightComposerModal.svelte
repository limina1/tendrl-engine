<script lang="ts">
	// The general highlighter (M-x tendrl-highlight): compose a NIP-84
	// highlight from PASTED text against any citable source — the reverse of
	// highlight mode's in-reader selection capture, sharing the same publish
	// endpoint. The source field accepts a nostr ref (naddr/nevent/note,
	// `kind:pubkey:d` coordinate, 64-hex id), a web URL, or an external id
	// (isbn:/doi:); classification is local string shape + the engine's
	// `/decode` for bech32, shown live as a badge so the user sees what the
	// event will cite before publishing. No offset/context tags — pasted text
	// has no pinned reference frame.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { stripNostrPrefix, isHex64 } from '$lib/nostr/nip19';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));

	// Prefill precedence, per field: the explicit open payload (e.g. the
	// M-x selection grab) wins; otherwise the draft remembered from the
	// last close. Publish clears the memory; plain close keeps it.
	const initialSource = app.highlightComposer?.source ?? app.highlightDraft?.source ?? '';
	let source = $state(initialSource);
	let text = $state(app.highlightComposer?.text ?? app.highlightDraft?.text ?? '');
	let annotation = $state(app.highlightDraft?.annotation ?? '');
	let posting = $state(false);

	// Confirm mode: publishing raises a publish_intent that renders
	// PublishConfirmModal (z 250, below us at 300). While it's up, this
	// modal YIELDS — hidden, keys and backdrop inert — instead of fighting
	// it for clicks and Escape. Deny brings us back with state intact;
	// approve resolves the in-flight post (toast + close as usual).
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');

	type ResolvedSource =
		| { badge: string; detail: string; target: Parameters<typeof api.publishHighlight>[0]['target'] }
		| { badge: 'unsupported'; detail: string; target: null };
	let resolved = $state<ResolvedSource | null>(null);
	let resolving = $state(false);

	const BECH32_RE = /^(npub|nprofile|note|nevent|naddr)1[023456789acdefghjklmnpqrstuvwxyz]+$/i;
	const COORD_RE = /^\d{1,9}:[0-9a-f]{64}:/i;
	const DOI_RE = /^10\.\d{4,9}\/\S+$/;
	const DOMAIN_RE = /^[a-z0-9-]+(\.[a-z0-9-]+)+(\/\S*)?$/i;

	async function classify(raw: string): Promise<ResolvedSource | null> {
		const s = raw.trim();
		if (!s) return null;
		const bech = stripNostrPrefix(s);
		if (BECH32_RE.test(bech)) {
			try {
				const d = await api.decode(bech);
				if (d.kind === 'naddr') {
					const addr = `${d.kind_int}:${d.pubkey}:${d.d_tag}`;
					return { badge: 'nostr address', detail: addr, target: { address: addr } };
				}
				if (d.kind === 'nevent' || d.kind === 'note') {
					return {
						badge: 'nostr event',
						detail: d.event_id.slice(0, 16) + '…',
						target: { event_id: d.event_id }
					};
				}
				return {
					badge: 'unsupported',
					detail: 'a profile is an author, not a highlightable source',
					target: null
				};
			} catch (e) {
				return { badge: 'unsupported', detail: api.errorMessage(e, 'not decodable'), target: null };
			}
		}
		if (COORD_RE.test(s)) {
			return { badge: 'nostr address', detail: s, target: { address: s } };
		}
		if (isHex64(s)) {
			return { badge: 'nostr event', detail: s.slice(0, 16) + '…', target: { event_id: s } };
		}
		if (/^isbn[:\s]/i.test(s)) {
			return { badge: 'ISBN', detail: s, target: { external: { id: s, id_kind: 'isbn' } } };
		}
		if (/^doi:/i.test(s) || DOI_RE.test(s) || /^https?:\/\/(dx\.)?doi\.org\//i.test(s)) {
			return { badge: 'DOI', detail: s, target: { external: { id: s, id_kind: 'doi' } } };
		}
		if (/^https?:\/\//i.test(s)) {
			return { badge: 'URL', detail: s, target: { url: s } };
		}
		if (DOMAIN_RE.test(s)) {
			const url = `https://${s}`;
			return { badge: 'URL', detail: url, target: { url } };
		}
		return {
			badge: 'unsupported',
			detail: 'expects a nostr ref (naddr/nevent/note/coordinate), URL, isbn: or doi:',
			target: null
		};
	}

	// Debounced, token-guarded resolution driven by the input handler (not an
	// $effect — async $state reads from effects loop; see the effect-async
	// memory). The token drops stale decode responses on fast retyping.
	let debounceTimer: ReturnType<typeof setTimeout> | undefined;
	let classifyToken = 0;
	function onSourceInput() {
		clearTimeout(debounceTimer);
		const snapshot = source;
		debounceTimer = setTimeout(async () => {
			const token = ++classifyToken;
			resolving = true;
			const r = await classify(snapshot);
			if (token !== classifyToken) return;
			resolved = r;
			resolving = false;
		}, 250);
	}

	// A prefilled source (open payload or restored draft) classifies
	// immediately.
	if (initialSource.trim()) onSourceInput();

	const canPost = $derived(
		canSign && !posting && text.trim().length >= 3 && !!resolved?.target
	);

	// --- Event JSON preview ------------------------------------------------
	// The wire form comes from the engine's own template builder (same code
	// path as publish, /highlight/preview) so it can't drift — no TS twin.
	// The $effect reads state synchronously and hands snapshots to the
	// debounced async fetch (see the effect-async-state memory).
	let jsonOpen = $state(false);
	let jsonText = $state<string | null>(null);
	let jsonErr = $state<string | null>(null);
	let jsonTimer: ReturnType<typeof setTimeout> | undefined;
	let jsonToken = 0;

	$effect(() => {
		if (!jsonOpen) return;
		const target = resolved?.target ?? null;
		const content = text.trim();
		const comment = annotation.trim();
		clearTimeout(jsonTimer);
		jsonTimer = setTimeout(async () => {
			const token = ++jsonToken;
			if (!target || !content) {
				jsonText = null;
				jsonErr = null;
				return;
			}
			try {
				const resp = await api.previewHighlight({
					target,
					content,
					comment: comment || undefined
				});
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
		app.highlightDraft =
			rememberDraft && (source.trim() || text.trim() || annotation.trim())
				? { source, text, annotation }
				: null;
		app.highlightComposer = null;
	}

	async function post() {
		if (!canPost || !resolved?.target) return;
		posting = true;
		try {
			const resp = await api.publishHighlight({
				target: resolved.target,
				content: text.trim(),
				comment: annotation.trim() || undefined
			});
			const { successful, total } = resp.broadcast;
			app.pushToast(
				total === 0
					? 'Highlight saved locally (no publish relays)'
					: successful === 0
						? `Highlight saved locally — 0/${total} relays accepted`
						: `Highlight published (${successful}/${total} relays)`,
				successful === 0 && total > 0 ? 'error' : 'success'
			);
			close(false);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Highlight failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		// A post in flight (incl. an open publish-confirm) owns the keys:
		// Escape must not close us mid-request, Ctrl-Enter must not re-post.
		if (posting) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			post();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="ghl-backdrop {yielding ? 'ghl-backdrop--yield' : ''}"
	onclick={() => {
		if (!posting) close();
	}}
	role="presentation"
>
	<div
		class="ghl-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Compose highlight"
		tabindex="-1"
	>
		<header class="ghl-header">
			<h3 class="ghl-title">highlight</h3>
			<button class="ghl-close" onclick={() => close()} aria-label="Close">×</button>
		</header>

		<label class="ghl-label" for="ghl-source">
			source
			{#if resolving}
				<span class="ghl-badge ghl-badge--pending">resolving…</span>
			{:else if resolved}
				<span class="ghl-badge {resolved.target ? '' : 'ghl-badge--bad'}">{resolved.badge}</span>
			{/if}
		</label>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			id="ghl-source"
			class="ghl-input"
			data-entry
			autofocus={!source}
			placeholder="naddr1… / nevent1… / https://… / isbn:… / doi:…"
			bind:value={source}
			oninput={onSourceInput}
			disabled={posting}
		/>
		{#if resolved && !resolving}
			<div class="ghl-detail {resolved.target ? '' : 'ghl-detail--bad'}">{resolved.detail}</div>
		{/if}

		<label class="ghl-label" for="ghl-text">highlighted text</label>
		<textarea
			id="ghl-text"
			class="ghl-input ghl-text"
			data-entry
			placeholder="Paste the passage being highlighted…"
			bind:value={text}
			disabled={posting}
		></textarea>

		<label class="ghl-label" for="ghl-note">annotation</label>
		<input
			id="ghl-note"
			class="ghl-input"
			data-entry
			placeholder="Optional — makes it a quote highlight"
			bind:value={annotation}
			disabled={posting}
		/>

		{#if text.trim()}
			<!-- Markup kept tight: .ghl-preview is white-space:pre-wrap, so
			     template indentation would render as literal whitespace. -->
			<blockquote class="ghl-preview">{text.trim()}{#if annotation.trim()}<footer
						class="ghl-preview-note">{annotation.trim()}</footer>{/if}</blockquote>
		{/if}

		<button class="ghl-json-toggle" onclick={() => (jsonOpen = !jsonOpen)}>
			{jsonOpen ? '▾' : '▸'} event json
		</button>
		{#if jsonOpen}
			{#if jsonErr}
				<div class="ghl-detail ghl-detail--bad">{jsonErr}</div>
			{:else if jsonText}
				<pre class="ghl-json">{jsonText}</pre>
			{:else}
				<div class="ghl-detail">enter a source and text to preview the event</div>
			{/if}
		{/if}

		<footer class="ghl-foot">
			<span class="ghl-hint">
				{#if !canSign}
					sign in to publish
				{:else}
					{text.trim().length} chars · Ctrl-Enter
				{/if}
			</span>
			<span class="ghl-spacer"></span>
			<button class="ghl-btn" onclick={() => close()} disabled={posting}>Cancel</button>
			<button class="ghl-btn ghl-btn--post" onclick={post} disabled={!canPost}>
				{posting ? 'Publishing…' : 'Highlight'}
			</button>
		</footer>
	</div>
</div>

<style>
	.ghl-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 12vh;
	}
	/* Stand down while PublishConfirmModal (z 250) is deciding the post —
	   visibility (not display) keeps bound elements alive. */
	.ghl-backdrop--yield {
		visibility: hidden;
	}
	.ghl-modal {
		width: min(520px, 92vw);
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.ghl-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.ghl-title {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--fg);
	}
	.ghl-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
	}
	.ghl-close:hover {
		color: var(--fg);
	}
	.ghl-label {
		display: flex;
		align-items: center;
		gap: 8px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-top: 4px;
	}
	.ghl-badge {
		font-size: calc(var(--t-xs) - 2px);
		color: var(--id-yours);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 0 6px;
	}
	.ghl-badge--pending {
		color: var(--base5);
	}
	.ghl-badge--bad {
		color: var(--error, #e06c75);
	}
	.ghl-input {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 5px 8px;
	}
	.ghl-input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.ghl-text {
		min-height: 88px;
		resize: vertical;
	}
	.ghl-detail {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.ghl-detail--bad {
		color: var(--error, #e06c75);
		white-space: normal;
	}
	.ghl-preview {
		margin: 4px 0 0;
		padding: 4px 10px;
		border-left: 2px solid var(--id-yours);
		font-size: var(--t-xs);
		color: var(--fg-muted);
		max-height: 120px;
		overflow-y: auto;
		/* Pasted passages keep their paragraph breaks — the event content
		   carries them verbatim, so the preview must too. */
		white-space: pre-wrap;
	}
	.ghl-preview-note {
		margin-top: 4px;
		font-style: italic;
		color: var(--base5);
	}
	.ghl-json-toggle {
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
	.ghl-json-toggle:hover {
		color: var(--fg);
	}
	.ghl-json {
		margin: 0;
		padding: 6px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--fg-muted);
		max-height: 180px;
		overflow: auto;
		white-space: pre;
	}
	.ghl-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 6px;
	}
	.ghl-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.ghl-spacer {
		flex: 1;
	}
	.ghl-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.ghl-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.ghl-btn--post {
		color: var(--id-yours);
	}
	.ghl-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
