<script lang="ts">
	import type { NostrEvent, SearchResult } from '$lib/types';
	import ProfileName from './ProfileName.svelte';
	import { getAppState } from '$lib/state.svelte';
	import {
		encodeNpub,
		encodeNevent,
		encodeNaddr,
		isHex64,
		stripNostrPrefix
	} from '$lib/nostr/nip19';
	import * as api from '$lib/api';

	let {
		event,
		onclose,
		onspawnreader,
		onfindcontaining
	}: {
		event: NostrEvent | SearchResult;
		onclose: () => void;
		onspawnreader?: (pubkey: string, d_tag: string, label: string | null) => void;
		onfindcontaining?: (kind: number, pubkey: string, d_tag: string) => void;
	} = $props();

	const app = getAppState();

	// Breadcrumb of events visited via chained nav within this modal session.
	// Reset when the displayed event id doesn't match the most recent nav
	// target (i.e. the event was set externally — e.g. history popover replay
	// or a fresh "View JSON" click). Each entry stores the original event so
	// breadcrumb-back can restore it without a refetch.
	type Crumb = { event: NostrEvent | SearchResult; label: string; id: string };
	let breadcrumb: Crumb[] = $state([]);
	let pendingNavTarget: string | null = null;

	type Normalized = {
		id: string;
		pubkey: string;
		kind: number;
		tags: string[][];
		content: string;
		created_at: number;
		title: string | null;
	};

	function normalize(input: NostrEvent | SearchResult): Normalized {
		if ('event_id' in input) {
			return {
				id: input.event_id,
				pubkey: input.author,
				kind: input.kind,
				tags: input.tags,
				content: input.preview,
				created_at: input.created_at,
				title: input.title
			};
		}
		const titleTag = input.tags.find((t) => t[0] === 'title');
		return {
			id: input.id,
			pubkey: input.pubkey,
			kind: input.kind,
			tags: input.tags,
			content: input.content,
			created_at: input.created_at,
			title: titleTag?.[1] ?? null
		};
	}

	const n = $derived(normalize(event));
	const dTag = $derived(n.tags.find((t) => t[0] === 'd')?.[1] ?? null);
	const addrRef = $derived(dTag ? `${n.kind}:${n.pubkey}:${dTag}` : null);
	let rawOpen = $state(false);

	// Containing publications — kind-30040 indexes that reference the
	// currently-displayed event by `#a` (preferred, for replaceable kinds)
	// or `#e`. Refetched on every event swap; the app-level cache makes
	// repeat visits cheap. Hidden entirely for kinds where the lookup
	// isn't meaningful (anything outside 30041/30818/30040/30023).
	const CONTAINING_KINDS = new Set([30041, 30818, 30040, 30023]);
	const containingApplicable = $derived(CONTAINING_KINDS.has(n.kind));
	let containingStatus: 'idle' | 'loading' | 'loaded' | 'failed' = $state('idle');
	let containingIndexes: { id: string; pubkey: string; d_tag: string; title: string }[] = $state([]);

	$effect(() => {
		// Re-run when the event id changes. `event` is read inside the
		// async call below — capture it locally so the closure sees the
		// version that was current when this effect fired (avoids races
		// when the user clicks chips quickly).
		const currentEvent = event;
		const currentId = n.id.toLowerCase();
		if (!containingApplicable) {
			containingStatus = 'loaded';
			containingIndexes = [];
			return;
		}
		containingStatus = 'loading';
		containingIndexes = [];
		app.findContainingIndexes(currentEvent).then((r) => {
			// Drop stale results if the user navigated away in the meantime.
			if (n.id.toLowerCase() !== currentId) return;
			containingStatus = r.status;
			containingIndexes = r.indexes;
		});
	});

	function onClickContaining(idx: { pubkey: string; d_tag: string; title: string }) {
		onclose();
		onspawnreader?.(idx.pubkey, idx.d_tag, idx.title);
	}

	function onClickShowAllRefs() {
		if (!dTag) return;
		onclose();
		onfindcontaining?.(n.kind, n.pubkey, dTag);
	}

	// Breadcrumb reset: when n.id changes, if it isn't the expected chained
	// target, the user came in via external nav (popover replay, fresh open)
	// and the breadcrumb is stale. The check runs after each prop change.
	$effect(() => {
		const id = n.id.toLowerCase();
		if (pendingNavTarget !== null && id === pendingNavTarget) {
			pendingNavTarget = null;
		} else if (pendingNavTarget === null && breadcrumb.length > 0) {
			const top = breadcrumb[breadcrumb.length - 1];
			// Back-step landed: do nothing (breadcrumb already trimmed).
			if (top.id === id) return;
			// External nav — chain is broken.
			breadcrumb = [];
		}
	});

	function pushBreadcrumb() {
		breadcrumb = [
			...breadcrumb,
			{ event, label: n.title ?? shortHex(n.id, 6, 4), id: n.id.toLowerCase() }
		];
	}

	function gotoBreadcrumb(idx: number) {
		if (idx < 0 || idx >= breadcrumb.length) return;
		const target = breadcrumb[idx];
		breadcrumb = breadcrumb.slice(0, idx);
		pendingNavTarget = target.id;
		app.eventModalData = target.event;
	}

	const KIND_LABEL: Record<number, string> = {
		0: 'profile',
		1: 'note',
		3: 'contacts',
		10002: 'relay list',
		30023: 'long-form',
		30040: 'publication index',
		30041: 'publication section',
		9802: 'highlight',
		30817: 'wiki',
		30818: 'wiki page'
	};

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}

	function copyText(s: string) {
		try { navigator.clipboard?.writeText(s); } catch { /* no-op */ }
	}

	function shortHex(s: string, head = 8, tail = 6): string {
		if (s.length <= head + tail + 1) return s;
		return `${s.slice(0, head)}…${s.slice(-tail)}`;
	}

	// ===== Tag click dispatch =====

	type TagAction =
		| { kind: 'none' }
		| { kind: 'event'; eventId: string }
		| { kind: 'reader'; pubkey: string; d_tag: string; label: string | null }
		| { kind: 'addr-nonindex'; addr: string }
		| { kind: 'search'; query: string };

	function tagAction(tag: string[]): TagAction {
		if (!Array.isArray(tag) || tag.length < 1) return { kind: 'none' };
		const key = tag[0];
		const rawValue = tag[1] ?? '';
		const value = stripNostrPrefix(rawValue).trim();
		if (!value) return { kind: 'none' };

		if (key === 'e' || key === 'q' || key === 'note') {
			if (isHex64(value)) return { kind: 'event', eventId: value.toLowerCase() };
			return { kind: 'none' };
		}
		if (key === 'p') {
			if (!isHex64(value)) return { kind: 'none' };
			try {
				const npub = encodeNpub(value.toLowerCase());
				return { kind: 'search', query: `by:${npub}` };
			} catch { return { kind: 'none' }; }
		}
		if (key === 'a') {
			const parts = value.split(':');
			if (parts.length < 3) return { kind: 'none' };
			const kind = Number(parts[0]);
			const pubkey = parts[1];
			const d_tag = parts.slice(2).join(':');
			if (!Number.isFinite(kind) || !isHex64(pubkey)) return { kind: 'none' };
			if (kind === 30040) {
				return { kind: 'reader', pubkey, d_tag, label: null };
			}
			return { kind: 'addr-nonindex', addr: value };
		}
		if (key === 'd') return { kind: 'search', query: `d:${value}` };
		if (key === 't') return { kind: 'search', query: `t:${value}` };
		// Generic tag filter — both single-char (NIP-01 short tags) and
		// multi-char names (author, client, imeta, alt, …) are accepted by
		// the parser (src/search.rs:506). The value must be whitespace-free
		// and not start with `/`; otherwise fall through to a plain chip.
		const validName = /^[A-Za-z][A-Za-z0-9_]*$/.test(key);
		const validValue = !value.startsWith('/') && /^[^\s]+$/.test(value);
		if (validName && validValue) {
			return { kind: 'search', query: `${key}:${value}` };
		}
		return { kind: 'none' };
	}

	async function onTagClick(tag: string[]) {
		const action = tagAction(tag);
		if (action.kind === 'none') return;
		if (action.kind === 'event') {
			// Chained nav — keep modal open, swap content via getEventForModal.
			pushBreadcrumb();
			pendingNavTarget = action.eventId;
			app.getEventForModal(action.eventId);
			return;
		}
		if (action.kind === 'reader') {
			onclose();
			onspawnreader?.(action.pubkey, action.d_tag, action.label);
			return;
		}
		if (action.kind === 'addr-nonindex') {
			// Version-aware: query all kinds-by-pubkey-by-d, show newest in modal.
			const parts = action.addr.split(':');
			const k = Number(parts[0]);
			const pk = parts[1];
			const d = parts.slice(2).join(':');
			try {
				const resp = await api.queryEvents(
					[{ kinds: [k], authors: [pk], '#d': [d] }],
					'local_only'
				);
				const evts = (resp?.events ?? []) as NostrEvent[];
				evts.sort((a, b) => b.created_at - a.created_at);
				if (evts[0]) {
					pushBreadcrumb();
					pendingNavTarget = evts[0].id.toLowerCase();
					app.eventModalData = evts[0];
					app.pushHistoryEntry({
						kind: 'naddr',
						coord: { kind: k, pubkey: pk, d_tag: d },
						title: evts[0].tags.find((t) => t[0] === 'title')?.[1],
						lastRunAt: Date.now()
					});
				}
			} catch (e) {
				console.error('a-tag non-index lookup failed:', e);
			}
			return;
		}
		// search
		onclose();
		app.handleSearch(action.query, { scopeToMe: false });
	}

	// Tags shown in the chip block — exclude the title tag (already in the
	// header) and the d tag (rendered as the addr identifier above).
	const tagChips = $derived(
		n.tags.filter((t) => t[0] !== 'title' && t[0] !== 'd')
	);
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="evm-backdrop" onclick={onclose} role="presentation">
	<div class="evm" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
		<header class="evm__header">
			<div class="evm__title-row">
				<span class="evm__title">{n.title ?? '[Untitled]'}</span>
				<span class="evm__kind">{KIND_LABEL[n.kind] ?? `kind ${n.kind}`}</span>
				<button class="evm__close" onclick={onclose} title="Close (Esc)">✕</button>
			</div>
			<div class="evm__meta">
				<ProfileName pubkey={n.pubkey} />
				<span class="evm__time">{formatTime(n.created_at)}</span>
			</div>
		</header>

		{#if breadcrumb.length > 0}
			<nav class="evm__crumbs" aria-label="In-modal navigation">
				{#each breadcrumb as crumb, i (crumb.id + ':' + i)}
					<button class="evm__crumb" onclick={() => gotoBreadcrumb(i)} title="Back to {crumb.label}">
						{crumb.label}
					</button>
					<span class="evm__crumb-sep">›</span>
				{/each}
				<span class="evm__crumb evm__crumb--current">{n.title ?? shortHex(n.id, 6, 4)}</span>
			</nav>
		{/if}

		<section class="evm__section">
			<h3 class="evm__heading">Copy as</h3>
			<div class="evm__copy-bar">
				<button
					class="evm__copy-pill"
					onclick={() => copyText(n.id)}
					title="Copy hex id ({n.id})"
				>
					<span class="evm__copy-icon" aria-hidden="true">📋</span>
					<span class="evm__copy-label">id</span>
				</button>
				<button
					class="evm__copy-pill"
					onclick={() => { try { copyText(encodeNevent(n.id)); } catch { /* */ } }}
					title="Copy as nevent1… (bech32m event id)"
				>
					<span class="evm__copy-icon" aria-hidden="true">📋</span>
					<span class="evm__copy-label">nevent</span>
				</button>
				{#if addrRef && dTag}
					<button
						class="evm__copy-pill"
						onclick={() => {
							try {
								copyText(encodeNaddr({ kind: n.kind, pubkey: n.pubkey, dTag }));
							} catch { /* */ }
						}}
						title="Copy as naddr1… (bech32m {n.kind}:{shortHex(n.pubkey, 6, 4)}:{dTag})"
					>
						<span class="evm__copy-icon" aria-hidden="true">📋</span>
						<span class="evm__copy-label">naddr</span>
					</button>
				{/if}
				<button
					class="evm__copy-pill"
					onclick={() => { try { copyText(encodeNpub(n.pubkey)); } catch { /* */ } }}
					title="Copy author npub1… (bech32 pubkey)"
				>
					<span class="evm__copy-icon" aria-hidden="true">📋</span>
					<span class="evm__copy-label">npub</span>
				</button>
			</div>
		</section>

		<section class="evm__section">
			<h3 class="evm__heading">Tags <span class="evm__heading-meta">({tagChips.length})</span></h3>
			{#if tagChips.length === 0}
				<div class="evm__placeholder">No tags.</div>
			{:else}
				<div class="evm__chips">
					{#each tagChips as tag, i (i)}
						{@const action = tagAction(tag)}
						{@const clickable = action.kind !== 'none'}
						{#if clickable}
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<button
								class="evm__chip evm__chip--{tag[0]} evm__chip--clickable"
								onclick={() => onTagClick(tag)}
								title="{tag[0]}: {tag[1] ?? ''}"
							>
								<span class="evm__chip-key">{tag[0]}</span>
								<span class="evm__chip-val">{tag[1] ?? ''}</span>
							</button>
						{:else}
							<span
								class="evm__chip evm__chip--{tag[0]}"
								title="{tag[0]}: {tag[1] ?? ''}"
							>
								<span class="evm__chip-key">{tag[0]}</span>
								<span class="evm__chip-val">{tag[1] ?? ''}</span>
							</span>
						{/if}
					{/each}
				</div>
			{/if}
		</section>

		{#if containingApplicable}
			<section class="evm__section">
				<h3 class="evm__heading">
					Containing publications
					{#if containingStatus === 'loaded' && containingIndexes.length > 0}
						<span class="evm__heading-meta">({containingIndexes.length})</span>
					{/if}
				</h3>
				{#if containingStatus === 'loading'}
					<div class="evm__placeholder">Searching…</div>
				{:else if containingStatus === 'failed'}
					<div class="evm__placeholder">Lookup failed.</div>
				{:else if containingIndexes.length === 0}
					<div class="evm__placeholder">No publications reference this event locally.</div>
				{:else}
					<div class="evm__containing">
						{#each containingIndexes as idx (idx.id)}
							<button
								class="evm__containing-btn"
								onclick={() => onClickContaining(idx)}
								title="Open publication: {idx.title}"
							>
								<span class="evm__containing-title">{idx.title}</span>
								<span class="evm__containing-dtag">{idx.d_tag}</span>
							</button>
						{/each}
					</div>
				{/if}
				{#if dTag}
					<button class="evm__show-all" onclick={onClickShowAllRefs}>
						Show all references →
					</button>
				{/if}
			</section>
		{/if}

		<section class="evm__section evm__section--raw">
			<button class="evm__raw-toggle" onclick={() => (rawOpen = !rawOpen)}>
				<span class="evm__raw-arrow" class:open={rawOpen}>{rawOpen ? '▾' : '▸'}</span>
				Raw JSON
			</button>
			{#if rawOpen}
				<pre class="evm__raw">{JSON.stringify(event, null, 2)}</pre>
			{/if}
		</section>
	</div>
</div>

<style>
	.evm-backdrop {
		position: fixed;
		/* Stop short of the modeline so the search-history pill stays
		   clickable while the modal is open. */
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 100;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.evm {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: min(720px, 90vw);
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.evm__header {
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.evm__title-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.evm__title {
		flex: 1;
		font-weight: 600;
		font-size: 0.95rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.evm__kind {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		text-transform: lowercase;
	}

	.evm__close {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: 0.9rem;
		padding: 2px 6px;
	}

	.evm__close:hover {
		color: var(--fg);
	}

	.evm__meta {
		display: flex;
		align-items: center;
		gap: 10px;
		margin-top: 4px;
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	/* Chained-nav breadcrumb. Each crumb is the title (or shortened id) of an
	   event we navigated away from via an e/q/a click. Click to pop back. */
	.evm__crumbs {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 6px 14px;
		border-bottom: 1px solid var(--border);
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
		font-size: 0.72rem;
		flex-wrap: wrap;
	}
	.evm__crumb {
		background: none;
		border: none;
		color: var(--id-yours);
		font-family: inherit;
		font-size: inherit;
		padding: 1px 4px;
		border-radius: var(--r-sm);
		cursor: pointer;
		max-width: 200px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evm__crumb:hover {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.evm__crumb--current {
		color: var(--fg);
		cursor: default;
	}
	.evm__crumb--current:hover {
		background: none;
	}
	.evm__crumb-sep {
		color: var(--fg-muted);
		font-size: 0.85rem;
	}

	.evm__section {
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.evm__section:last-child {
		border-bottom: none;
	}

	.evm__section--raw {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}

	.evm__heading {
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
		margin-bottom: 6px;
		font-weight: 600;
	}
	.evm__heading-meta {
		color: var(--fg-muted);
		font-weight: 400;
	}

	.evm__placeholder {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-style: italic;
	}

	/* Containing publications block */
	.evm__containing {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-bottom: 6px;
	}
	.evm__containing-btn {
		display: flex;
		align-items: center;
		gap: 10px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-size: 0.78rem;
	}
	.evm__containing-btn:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-color: var(--id-yours);
	}
	.evm__containing-title {
		flex: 1;
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evm__containing-dtag {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: var(--fg-muted);
		max-width: 200px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evm__show-all {
		background: none;
		border: none;
		color: var(--id-yours);
		font-family: inherit;
		font-size: 0.72rem;
		padding: 2px 0;
		cursor: pointer;
		text-align: left;
	}
	.evm__show-all:hover {
		text-decoration: underline;
	}

	/* Identifiers block — compact "Copy as" pill bar. Each pill is a
	   clipboard icon + format label; click copies that encoding to the
	   clipboard. Replaces the older per-row layout (id / addr / author
	   each on its own row with separate copy buttons). */
	.evm__copy-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.evm__copy-pill {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-yours) 30%, transparent);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: 0.72rem;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.evm__copy-pill:hover {
		background: color-mix(in srgb, var(--id-yours) 20%, transparent);
		border-color: var(--id-yours);
	}
	.evm__copy-pill:active {
		background: color-mix(in srgb, var(--id-yours) 30%, transparent);
	}
	.evm__copy-icon {
		font-size: 0.85rem;
		line-height: 1;
	}
	.evm__copy-label {
		font-weight: 500;
	}

	/* Tag chips */
	.evm__chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.evm__chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: var(--border);
		color: var(--fg);
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		padding: 2px 6px;
		font-family: var(--font-mono);
		font-size: 0.7rem;
		max-width: 100%;
	}
	.evm__chip--clickable {
		cursor: pointer;
	}
	.evm__chip--clickable:hover {
		background: color-mix(in srgb, var(--id-yours) 18%, var(--border));
		border-color: var(--id-yours);
	}
	.evm__chip-key {
		color: var(--fg-muted);
		font-size: 0.65rem;
		font-weight: 600;
	}
	.evm__chip-val {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 240px;
	}
	/* Per-kind tints for the most common tags */
	.evm__chip--e .evm__chip-key,
	.evm__chip--q .evm__chip-key,
	.evm__chip--note .evm__chip-key { color: var(--id-yours); }
	.evm__chip--a .evm__chip-key { color: var(--id-imported); }
	.evm__chip--p .evm__chip-key { color: var(--id-remote); }
	.evm__chip--t .evm__chip-key,
	.evm__chip--d .evm__chip-key { color: var(--cyan); }

	.evm__raw-toggle {
		background: none;
		border: none;
		color: var(--fg);
		cursor: pointer;
		font-size: 0.8rem;
		font-weight: 500;
		padding: 0;
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.evm__raw-arrow {
		display: inline-block;
		font-size: 0.7rem;
	}

	.evm__raw {
		margin-top: 8px;
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--fg-muted);
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
