<script lang="ts">
	// "Insert reference" builder — opened from the composer mode-bar `{{ }}`
	// button (and from inline autocomplete mid-token). One tab per nostrdown form:
	//   ref     → pick a sibling section in the current draft  (instant, client-side)
	//   wiki    → pick an existing wiki/article by title       (live search)
	//   embed   → build an event coordinate, the hard case: author (by: profile
	//             autocomplete) + kind + a T|d|custom tag value → resolve to a
	//             matching event → encode its naddr.
	//   slot    → same coordinate builder, restricted to 30040/30041 (block-level
	//             transclude → an a-tag in the 30040).
	//   quote   → coordinate builder + the excerpt text that travels inline.
	//   mention → profile search (or a pasted npub/nprofile) → `{{@npub…}}`.
	// On confirm it hands a `{{…}}` token back to the composer, which inserts it
	// at the editor cursor. Reuses the existing search (`api.search` returns live
	// `profiles[]` for author lookup) and `api.encode` (coordinate → naddr).
	import * as api from '$lib/api';
	import type { SearchResult, ProfileResult } from '$lib/types';
	import { cachedSlug, ensureSlugs, slug } from '$lib/nostr/slugs';

	let {
		open = false,
		initialTab = 'ref',
		sectionTitles = [],
		oninsert,
		onclose
	}: {
		open?: boolean;
		/** Tab to show each time the modal opens. */
		initialTab?: 'ref' | 'wiki' | 'embed' | 'slot' | 'quote' | 'mention';
		/** Titles of the other sections in the current draft (for `ref:`). */
		sectionTitles?: string[];
		oninsert: (token: string) => void;
		onclose: () => void;
	} = $props();

	type Tab = 'ref' | 'wiki' | 'embed' | 'slot' | 'quote' | 'mention';
	let tab = $state<Tab>('ref');
	// Snap to the requested tab each time the modal opens.
	let wasOpen = false;
	$effect(() => {
		if (open && !wasOpen) tab = initialTab;
		wasOpen = open;
	});

	// ── ref: filter the draft's own section titles ──────────────────────────
	// Slugs come from the engine (cached): prefetch the candidate titles, and
	// normalize the live query into `refQ`; the filter then runs synchronously
	// against the cache.
	let refFilter = $state('');
	let titleSlugs = $state<Record<string, string>>({});
	let refQ = $state('');
	$effect(() => {
		const titles = sectionTitles;
		ensureSlugs(titles).then(() => {
			titleSlugs = Object.fromEntries(titles.map((t) => [t, cachedSlug(t)]));
		});
	});
	$effect(() => {
		const v = refFilter;
		slug(v).then((s) => {
			if (v === refFilter) refQ = s;
		});
	});
	const refMatches = $derived.by(() => {
		const q = refQ;
		return sectionTitles.filter((t) => !q || (titleSlugs[t] ?? '').includes(q));
	});

	function insertRef(title: string) {
		oninsert(`{{ref:${title}}}`);
		close();
	}

	// ── wiki: live title search over addressable kinds ──────────────────────
	let wikiQuery = $state('');
	let wikiResults = $state<SearchResult[]>([]);
	let wikiBusy = $state(false);
	let wikiTimer: ReturnType<typeof setTimeout> | undefined;
	// Live slug preview of the typed wiki topic (engine-normalized).
	let wikiSlug = $state('');
	$effect(() => {
		const v = wikiQuery;
		slug(v).then((s) => {
			if (v === wikiQuery) wikiSlug = s;
		});
	});

	function onWikiInput() {
		clearTimeout(wikiTimer);
		const q = wikiQuery.trim();
		if (!q) {
			wikiResults = [];
			return;
		}
		wikiTimer = setTimeout(async () => {
			wikiBusy = true;
			try {
				const resp = await api.search(`k:30818 k:30023 ${q}`, 20);
				wikiResults = (resp.results ?? []).filter((r) => r.addr && r.title);
			} catch {
				wikiResults = [];
			} finally {
				wikiBusy = false;
			}
		}, 200);
	}

	async function insertWiki(target: string) {
		const s = await slug(target);
		if (s) oninsert(`{{wiki:${s}}}`);
		close();
	}

	// ── embed / slot / quote: shared coordinate builder ─────────────────────
	// Prefix the coordinate tabs emit; quote wraps the target with its excerpt.
	const coordPrefix = $derived(tab === 'slot' ? 'slot' : tab === 'quote' ? 'quote' : 'embed');
	// The excerpt a quote carries inline (after `|`) — required: a quote with no
	// text has nothing to render.
	let quoteText = $state('');
	const quoteReady = $derived(tab !== 'quote' || quoteText.trim().length > 0);

	// Paste any entity directly. Slots take addressable events only (they become
	// index a-tags); quotes take events, not profiles.
	let pasteEntity = $state('');
	const ENTITY_RE = /^(nostr:)?(naddr1|nevent1|note1|npub1|nprofile1)[a-z0-9]+$/i;
	const SLOT_ENTITY_RE = /^(nostr:)?naddr1[a-z0-9]+$/i;
	const QUOTE_ENTITY_RE = /^(nostr:)?(naddr1|nevent1|note1)[a-z0-9]+$/i;
	const pasteValid = $derived.by(() => {
		const v = pasteEntity.trim();
		if (tab === 'slot') return SLOT_ENTITY_RE.test(v);
		if (tab === 'quote') return QUOTE_ENTITY_RE.test(v);
		return ENTITY_RE.test(v);
	});

	function emitCoord(target: string) {
		if (tab === 'quote') {
			oninsert(`{{quote:${target}|${quoteText.trim()}}}`);
		} else {
			oninsert(`{{${coordPrefix}:${target}}}`);
		}
		close();
	}

	function insertEntity() {
		if (!pasteValid || !quoteReady) return;
		emitCoord(pasteEntity.trim().replace(/^nostr:/i, ''));
	}

	// ── embed: coordinate builder ───────────────────────────────────────────
	let authorText = $state('');
	let authorPubkey = $state<string | null>(null);
	let authorLabel = $state('');
	let authorSugg = $state<ProfileResult[]>([]);
	let authorTimer: ReturnType<typeof setTimeout> | undefined;

	function onAuthorInput() {
		clearTimeout(authorTimer);
		authorPubkey = null;
		const q = authorText.trim();
		if (!q) {
			authorSugg = [];
			return;
		}
		authorTimer = setTimeout(async () => {
			try {
				const resp = await api.search(`by:name:${q}`, 8);
				authorSugg = resp.profiles ?? [];
			} catch {
				authorSugg = [];
			}
		}, 200);
	}

	function pickAuthor(p: ProfileResult) {
		authorPubkey = p.pubkey;
		authorLabel = p.display_name || p.name || p.pubkey.slice(0, 8) + '…';
		authorText = authorLabel;
		authorSugg = [];
		runEmbedSearch();
	}

	function clearAuthor() {
		authorPubkey = null;
		authorLabel = '';
		authorText = '';
		authorSugg = [];
		runEmbedSearch();
	}

	const KINDS = [
		{ v: 30041, label: '30041 · section' },
		{ v: 30040, label: '30040 · publication' },
		{ v: 30023, label: '30023 · article' },
		{ v: 30818, label: '30818 · wiki' },
		{ v: 0, label: 'any kind' }
	];
	let kindSel = $state(30041);
	// Slots reference addressable publication events only (a-tags), so the
	// coordinate builder restricts to 30040/30041 on the Slot tab.
	const kindOptions = $derived(
		tab === 'slot' ? KINDS.filter((k) => k.v === 30040 || k.v === 30041) : KINDS
	);
	$effect(() => {
		if (tab === 'slot' && kindSel !== 30040 && kindSel !== 30041) kindSel = 30041;
	});

	type TagMode = 'T' | 'd' | 'custom';
	let tagMode = $state<TagMode>('T');
	let tagCustom = $state('');
	let tagValue = $state('');
	const tagName = $derived(tagMode === 'custom' ? tagCustom.trim() : tagMode);

	let embedResults = $state<SearchResult[]>([]);
	let embedBusy = $state(false);
	let embedTimer: ReturnType<typeof setTimeout> | undefined;
	let inserting = $state(false);

	function onEmbedInput() {
		clearTimeout(embedTimer);
		embedTimer = setTimeout(runEmbedSearch, 200);
	}

	async function runEmbedSearch() {
		const parts: string[] = [];
		if (kindSel) parts.push(`k:${kindSel}`);
		if (authorPubkey) parts.push(`by:${authorPubkey}`);
		if (tagName && tagValue.trim()) parts.push(`${tagName}:${tagValue.trim()}`);
		if (parts.length === 0) {
			embedResults = [];
			return;
		}
		embedBusy = true;
		try {
			const resp = await api.search(parts.join(' '), 25);
			embedResults = (resp.results ?? []).filter((r) => r.addr);
		} catch {
			embedResults = [];
		} finally {
			embedBusy = false;
		}
	}

	async function insertEmbed(r: SearchResult) {
		if (!r.addr || inserting || !quoteReady) return;
		inserting = true;
		try {
			const naddr = await api.encode({
				kind: 'naddr',
				kind_int: r.addr.kind,
				pubkey: r.addr.pubkey,
				d_tag: r.addr.d_tag
			});
			emitCoord(naddr);
		} catch {
			// leave the modal open so the user can retry / pick another
		} finally {
			inserting = false;
		}
	}

	// ── mention: profile search → `{{@npub…}}` ──────────────────────────────
	let mentionQuery = $state('');
	let mentionSugg = $state<ProfileResult[]>([]);
	let mentionTimer: ReturnType<typeof setTimeout> | undefined;
	// Optional display text — `{{@npub…|display}}`; without it the renderer
	// falls back to the profile's handle.
	let mentionDisplay = $state('');
	const MENTION_ENTITY_RE = /^(nostr:)?(npub1|nprofile1)[a-z0-9]+$/i;
	const mentionPasteValid = $derived(MENTION_ENTITY_RE.test(mentionQuery.trim()));

	function onMentionInput() {
		clearTimeout(mentionTimer);
		const q = mentionQuery.trim();
		if (!q || MENTION_ENTITY_RE.test(q)) {
			mentionSugg = [];
			return;
		}
		mentionTimer = setTimeout(async () => {
			try {
				const resp = await api.search(`by:name:${q}`, 8);
				mentionSugg = resp.profiles ?? [];
			} catch {
				mentionSugg = [];
			}
		}, 200);
	}

	function emitMention(entity: string) {
		const d = mentionDisplay.trim();
		oninsert(`{{@${entity}${d ? `|${d}` : ''}}}`);
		close();
	}

	async function insertMentionProfile(p: ProfileResult) {
		if (inserting) return;
		inserting = true;
		try {
			emitMention(await api.encode({ kind: 'npub', pubkey: p.pubkey }));
		} catch {
			// leave the modal open so the user can retry / pick another
		} finally {
			inserting = false;
		}
	}

	function insertMentionEntity() {
		if (!mentionPasteValid) return;
		emitMention(mentionQuery.trim().replace(/^nostr:/i, ''));
	}

	function authorName(r: SearchResult): string {
		const p = api.getCachedProfile(r.author);
		return p?.display_name || p?.name || r.author.slice(0, 8) + '…';
	}

	function close() {
		onclose();
	}

	// Focus the input when its tab mounts (Svelte action — avoids the a11y
	// autofocus-attribute warning).
	function focusInput(node: HTMLInputElement | HTMLTextAreaElement) {
		node.focus();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.stopPropagation();
			close();
		}
	}
</script>

{#if open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="rb-backdrop" onclick={close} role="presentation">
		<div
			class="rb-modal"
			onclick={(e) => e.stopPropagation()}
			onkeydown={handleKeydown}
			role="dialog"
			aria-modal="true"
			aria-label="Insert reference"
			tabindex="-1"
		>
			<header class="rb-head">
				<div class="rb-tabs" role="tablist">
					<button class="rb-tab" class:active={tab === 'ref'} role="tab" aria-selected={tab === 'ref'} onclick={() => (tab = 'ref')}>Ref</button>
					<button class="rb-tab" class:active={tab === 'wiki'} role="tab" aria-selected={tab === 'wiki'} onclick={() => (tab = 'wiki')}>Wiki</button>
					<button class="rb-tab" class:active={tab === 'embed'} role="tab" aria-selected={tab === 'embed'} onclick={() => (tab = 'embed')}>Embed</button>
					<button class="rb-tab" class:active={tab === 'slot'} role="tab" aria-selected={tab === 'slot'} onclick={() => (tab = 'slot')}>Slot</button>
					<button class="rb-tab" class:active={tab === 'quote'} role="tab" aria-selected={tab === 'quote'} onclick={() => (tab = 'quote')}>Quote</button>
					<button class="rb-tab" class:active={tab === 'mention'} role="tab" aria-selected={tab === 'mention'} onclick={() => (tab = 'mention')}>@</button>
				</div>
				<button class="rb-close" onclick={close} aria-label="Close">×</button>
			</header>

			{#if tab === 'ref'}
				<div class="rb-body">
					<input
						class="rb-input"
						placeholder="filter sections in this draft…"
						bind:value={refFilter}
						use:focusInput
					/>
					<div class="rb-list">
						{#each refMatches as title, i (i)}
							<button class="rb-row" onclick={() => insertRef(title)}>
								<span class="rb-badge">section</span>
								<span class="rb-row__title">{title}</span>
							</button>
						{:else}
							<div class="rb-empty">
								{sectionTitles.length === 0
									? 'No other sections in this draft yet.'
									: 'No section matches.'}
							</div>
						{/each}
					</div>
				</div>
			{:else if tab === 'wiki'}
				<div class="rb-body">
					<input
						class="rb-input"
						placeholder="search wiki / article titles…"
						bind:value={wikiQuery}
						oninput={onWikiInput}
						use:focusInput
					/>
					<div class="rb-list">
						{#if wikiBusy}
							<div class="rb-empty">searching…</div>
						{/if}
						{#each wikiResults as r (r.event_id)}
							<button class="rb-row" onclick={() => insertWiki(r.addr?.d_tag ?? r.title ?? '')}>
								<span class="rb-badge">{r.kind === 30818 ? 'wiki' : 'article'}</span>
								<span class="rb-row__title">{r.title}</span>
								<span class="rb-row__meta">{authorName(r)}</span>
							</button>
						{/each}
						{#if wikiQuery.trim() && !wikiBusy}
							<button class="rb-row rb-row--free" onclick={() => insertWiki(wikiQuery)}>
								<span class="rb-badge">topic</span>
								<span class="rb-row__title">use “{wikiSlug}”</span>
							</button>
						{/if}
					</div>
				</div>
			{:else if tab === 'mention'}
				<div class="rb-body">
					<input
						class="rb-input"
						placeholder="profile name — or paste npub / nprofile…"
						bind:value={mentionQuery}
						oninput={onMentionInput}
						onkeydown={(e) => e.key === 'Enter' && insertMentionEntity()}
						use:focusInput
					/>
					<label class="rb-field">
						<span class="rb-label">as:</span>
						<input
							class="rb-input rb-input--inline"
							placeholder="display text (optional — defaults to the profile's handle)"
							bind:value={mentionDisplay}
						/>
					</label>
					<div class="rb-list">
						{#if mentionPasteValid}
							<button class="rb-row" onclick={insertMentionEntity}>
								<span class="rb-badge">@</span>
								<span class="rb-row__title">mention {mentionQuery.trim().replace(/^nostr:/i, '').slice(0, 20)}…</span>
							</button>
						{/if}
						{#each mentionSugg as p (p.pubkey)}
							<button class="rb-row" onclick={() => insertMentionProfile(p)} disabled={inserting}>
								<span class="rb-badge">@</span>
								<span class="rb-row__title">{p.display_name || p.name || 'unnamed'}</span>
								<span class="rb-row__meta">{p.pubkey.slice(0, 10)}…</span>
							</button>
						{:else}
							{#if !mentionPasteValid}
								<div class="rb-empty">Type a name to search profiles.</div>
							{/if}
						{/each}
					</div>
				</div>
			{:else}
				<div class="rb-body">
					{#if tab === 'quote'}
						<!-- the excerpt is the quote's payload — it travels inline in the token -->
						<label class="rb-field rb-field--col">
							<span class="rb-label">text:</span>
							<textarea
								class="rb-input rb-textarea"
								placeholder="the quoted passage (required — travels inline in the token)…"
								bind:value={quoteText}
								use:focusInput
							></textarea>
						</label>
					{/if}
					<!-- paste any entity directly (slot: naddr only; quote: events only) -->
					<div class="rb-field">
						<span class="rb-label">paste:</span>
						<input
							class="rb-input rb-input--inline"
							placeholder={tab === 'slot'
								? 'naddr …'
								: tab === 'quote'
									? 'naddr / nevent / note …'
									: 'naddr / nevent / note / npub …'}
							bind:value={pasteEntity}
							onkeydown={(e) => e.key === 'Enter' && insertEntity()}
						/>
						<button class="rb-go" disabled={!pasteValid || !quoteReady} onclick={insertEntity}>{coordPrefix}</button>
					</div>
					<div class="rb-or">or build a coordinate by search:</div>
					<!-- author -->
					<label class="rb-field">
						<span class="rb-label">by:</span>
						{#if authorPubkey}
							<span class="rb-chip">{authorLabel}<button class="rb-chip__x" onclick={clearAuthor} aria-label="Clear author">×</button></span>
						{:else}
							<input
								class="rb-input rb-input--inline"
								placeholder="author name (optional)…"
								bind:value={authorText}
								oninput={onAuthorInput}
							/>
						{/if}
					</label>
					{#if authorSugg.length > 0}
						<div class="rb-sugg">
							{#each authorSugg as p (p.pubkey)}
								<button class="rb-row" onclick={() => pickAuthor(p)}>
									<span class="rb-row__title">{p.display_name || p.name || 'unnamed'}</span>
									<span class="rb-row__meta">{p.pubkey.slice(0, 10)}…</span>
								</button>
							{/each}
						</div>
					{/if}

					<!-- kind + tag -->
					<div class="rb-row-fields">
						<label class="rb-field">
							<span class="rb-label">kind:</span>
							<select class="rb-select" bind:value={kindSel} onchange={runEmbedSearch}>
								{#each kindOptions as k (k.v)}
									<option value={k.v}>{k.label}</option>
								{/each}
							</select>
						</label>
						<label class="rb-field">
							<span class="rb-label">match:</span>
							<select class="rb-select" bind:value={tagMode} onchange={runEmbedSearch}>
								<option value="T">T · title-slug</option>
								<option value="d">d · identifier</option>
								<option value="custom">custom tag</option>
							</select>
						</label>
						{#if tagMode === 'custom'}
							<input class="rb-input rb-input--tag" placeholder="tag" bind:value={tagCustom} oninput={onEmbedInput} />
						{/if}
						<input
							class="rb-input rb-input--value"
							placeholder="{tagName || 'tag'} value…"
							bind:value={tagValue}
							oninput={onEmbedInput}
						/>
					</div>

					<div class="rb-list">
						{#if embedBusy}
							<div class="rb-empty">searching…</div>
						{/if}
						{#each embedResults as r (r.event_id)}
							<button
								class="rb-row"
								onclick={() => insertEmbed(r)}
								disabled={inserting || !quoteReady}
								title={quoteReady ? undefined : 'Enter the quoted text first'}
							>
								<span class="rb-badge">{r.addr?.kind}</span>
								<span class="rb-row__title">{r.title ?? r.addr?.d_tag}</span>
								<span class="rb-row__meta">{authorName(r)}</span>
							</button>
						{:else}
							{#if !embedBusy}
								<div class="rb-empty">Pick an author, kind, and/or a tag value to find an event to {coordPrefix}.</div>
							{/if}
						{/each}
					</div>
				</div>
			{/if}
		</div>
	</div>
{/if}

<style>
	.rb-backdrop {
		position: fixed;
		inset: 0;
		z-index: 320;
		background: var(--scrim, rgba(0, 0, 0, 0.5));
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding: 8vh 24px 24px;
	}
	.rb-modal {
		width: min(560px, 100%);
		max-height: 78dvh;
		display: flex;
		flex-direction: column;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-md);
		box-shadow: var(--shadow-lg, 0 12px 40px rgba(0, 0, 0, 0.5));
		font-family: var(--font-sans);
	}
	.rb-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 10px;
		border-bottom: 1px solid var(--panel-border);
	}
	.rb-tabs {
		display: flex;
		gap: 4px;
		flex: 1;
	}
	.rb-tab {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 3px 10px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: var(--bg-surface);
		color: var(--base6);
		cursor: pointer;
	}
	.rb-tab.active {
		border-color: var(--id-yours);
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
	}
	.rb-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-lg);
		line-height: 1;
		cursor: pointer;
		padding: 0 4px;
	}
	.rb-close:hover {
		color: var(--fg);
	}
	.rb-body {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 10px;
		overflow-y: auto;
	}
	.rb-input {
		width: 100%;
		padding: 6px 8px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg);
	}
	.rb-input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.rb-input--inline {
		flex: 1;
	}
	.rb-input--tag {
		width: 90px;
		flex: 0 0 auto;
	}
	.rb-input--value {
		flex: 1;
		min-width: 100px;
	}
	.rb-field {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.rb-field--col {
		align-items: flex-start;
	}
	.rb-textarea {
		min-height: 64px;
		resize: vertical;
	}
	.rb-go {
		flex: 0 0 auto;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: 4px 10px;
		border: 1px solid var(--id-yours);
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours);
		cursor: pointer;
	}
	.rb-go:disabled {
		opacity: 0.5;
		cursor: default;
		border-color: var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg-muted);
	}
	.rb-or {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-style: italic;
		margin: 2px 0;
	}
	.rb-label {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--id-yours);
		flex: 0 0 auto;
	}
	.rb-row-fields {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.rb-select {
		padding: 5px 6px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
	.rb-chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 3px 4px 3px 8px;
		font-size: var(--t-2xs);
		border: 1px solid var(--id-yours);
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
		color: var(--fg);
	}
	.rb-chip__x {
		background: none;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: var(--t-sm);
		line-height: 1;
		padding: 0 2px;
	}
	.rb-list,
	.rb-sugg {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.rb-sugg {
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 2px;
		background: var(--bg-surface);
	}
	.rb-row {
		display: flex;
		align-items: baseline;
		gap: 8px;
		width: 100%;
		text-align: left;
		padding: 6px 8px;
		background: none;
		border: none;
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
	.rb-row:hover:not(:disabled) {
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
	}
	.rb-row:disabled {
		opacity: 0.6;
		cursor: default;
	}
	.rb-row--free {
		font-style: italic;
		color: var(--fg-muted);
	}
	.rb-badge {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--id-yours);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 0 4px;
		flex: 0 0 auto;
	}
	.rb-row__title {
		flex: 1;
		font-size: var(--t-xs);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.rb-row__meta {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		flex: 0 0 auto;
	}
	.rb-empty {
		padding: 10px 8px;
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-style: italic;
	}
</style>
