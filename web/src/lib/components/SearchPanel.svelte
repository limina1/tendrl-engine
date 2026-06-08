<script lang="ts">
	import type {
		SearchResult,
		ProfileResult,
		ContextItem,
		DocumentFile,
		ImportPage,
		TagValueCount,
		EmbeddingStatusResponse
	} from '$lib/types';
	import { onMount } from 'svelte';
	import SearchInput from './SearchInput.svelte';
	import SearchResultItem from './SearchResultItem.svelte';
	import PersonResultItem from './PersonResultItem.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';
	import EmbeddingSettings from './EmbeddingSettings.svelte';
	import {
		searchConfig,
		openSearchConfig,
		kindLabel
	} from '$lib/search/search-config.svelte';

	let {
		results,
		profiles = [],
		tagCounts = {},
		count = 0,
		localCount = 0,
		relayCount = 0,
		loading = false,
		searchContext = 'knowledge base',
		onsearch,
		onselect,
		onviewjson,
		onaddtocontext,
		onaddtocompose,
		onaddmanytocontext,
		onaddmanytocompose,
		onignore,
		onignorepubkey,
		// Import props
		documentFiles = [],
		importPages = [],
		importFilename = '',
		importLoading = false,
		onlistdocuments,
		onimportfile,
		onparsedocument,
		onimportpagetocontext,
		onimportpagetocompose,
		onimportpagestocontext,
		onimportpagestocompose,
		items = [],
		localPubkeys = new Set<string>(),
		onviewprofile,
		cursor = -1,
		listEl = $bindable<HTMLDivElement | undefined>(undefined),
		canPromptRelays = false,
		onsearchrelays,
		hasSearched = false,
		networkMode = 'auto',
		relaySearchLoading = false,
		// Refs tab — held items in the reference pool. Cursor + open/release
		// are routed through the host (SearchBuffer) so the same nav handler
		// drives them as the Search/KB tabs.  `heldItems` is the filtered
		// list (post-refsQuery); `heldTotal` is the unfiltered count, so the
		// tab badge shows the underlying pool size even while the user types.
		heldItems = [],
		heldTotal = 0,
		refsCursor = -1,
		importCursor = -1,
		onopenheld,
		onreleaseheld,
		onrouterefcontext,
		onrouterefcompose,
		onrouterefsearch,
		onresultpillaction,
		refsQuery = $bindable<string>(''),
		activeTab = $bindable<'search' | 'refs' | 'import'>('search'),
		searchValue = $bindable<string>(''),
		// Embedding settings — the index that powers `~:` semantic search.
		// Status + actions come from the host (SearchBuffer → app state).
		embeddingStatus = null,
		embeddingSyncing = false,
		onembedmissing,
		onembedreindex,
		onsetembedkinds,
		onsetautoembed,
		onrefreshembedstatus
	}: {
		results: SearchResult[];
		profiles?: ProfileResult[];
		tagCounts?: Record<string, TagValueCount[]>;
		count?: number;
		localCount?: number;
		relayCount?: number;
		loading?: boolean;
		searchContext?: string;
		onsearch: (query: string) => void;
		onselect: (result: SearchResult) => void;
		onviewjson: (result: SearchResult) => void;
		onaddtocontext: (result: SearchResult) => void;
		onaddtocompose: (result: SearchResult) => void;
		onaddmanytocontext: (results: SearchResult[]) => void;
		onaddmanytocompose: (results: SearchResult[]) => void;
		onignore?: (result: SearchResult) => void;
		onignorepubkey?: (result: SearchResult) => void;
		documentFiles?: DocumentFile[];
		importPages?: ImportPage[];
		importFilename?: string;
		importLoading?: boolean;
		onlistdocuments?: () => void;
		onimportfile?: (file: File) => void;
		onparsedocument?: (filename: string) => void;
		onimportpagetocontext?: (page: ImportPage) => void;
		onimportpagetocompose?: (page: ImportPage) => void;
		onimportpagestocontext?: (pages: ImportPage[]) => void;
		onimportpagestocompose?: (pages: ImportPage[]) => void;
		items?: ContextItem[];
		localPubkeys?: Set<string>;
		onviewprofile?: (pubkey: string) => void;
		cursor?: number;
		listEl?: HTMLDivElement;
		/** When true, the offline-empty CTA appears so the user can
		 *  prompt for relays and re-run the same query against the
		 *  network. The host (state.svelte.ts) decides when this is
		 *  meaningful — typically offline mode + a non-empty query
		 *  that returned zero local hits. */
		canPromptRelays?: boolean;
		onsearchrelays?: () => void;
		/** Whether the user has submitted a query yet this session.
		 *  Distinguishes "no results because nothing was searched"
		 *  from "no results found in local DB". */
		hasSearched?: boolean;
		/** Current engine network mode — drives the copy in the
		 *  empty-result state. In `auto` we tell the user the relay
		 *  fan-out already happened (or is in flight); in `confirm`
		 *  we surface the explicit CTA. */
		networkMode?: 'auto' | 'confirm';
		/** True while the relay fan-out (handleSearchViaRelays) is in
		 *  flight, distinct from the initial local search loading. */
		relaySearchLoading?: boolean;
		/** Held items (the reference pool). Rendered in the Refs tab.
		 *  Already filtered by `refsQuery` upstream — render as-is. */
		heldItems?: ContextItem[];
		/** Unfiltered pool size — drives the tab badge so the user sees
		 *  total held even while typing in the refs filter. */
		heldTotal?: number;
		/** Cursor index within the Refs / Import tab — passed in so the
		 *  host (SearchBuffer) owns the per-tab cursor state and the global
		 *  nav handler can move it alongside the existing search cursor. */
		refsCursor?: number;
		importCursor?: number;
		onopenheld?: (item: ContextItem) => void;
		onreleaseheld?: (id: string) => void;
		/** Refs row route actions — host (SearchBuffer) decides what to
		 *  do (call into app state, navigate, etc). All three operate on
		 *  the held ContextItem; the host knows whether to fire a toast,
		 *  flip tabs, run anything async, etc. */
		onrouterefcontext?: (item: ContextItem) => void;
		onrouterefcompose?: (item: ContextItem) => void;
		onrouterefsearch?: (item: ContextItem) => void;
		/** Pill click on a search-tab row. The host (SearchBuffer) decides
		 *  whether to add to pool (fresh result), toggle the membership
		 *  (existing pool item), or drop. */
		onresultpillaction?: (result: SearchResult, kind: 'context' | 'compose' | 'drop') => void;
		/** Local case-insensitive substring filter over heldItems. Bindable
		 *  so the host can clear it when needed and observe edits. */
		refsQuery?: string;
		/** The engine-side search query string (the input's value).
		 *  Bindable so the Refs tab's "→ search" route can append a coord
		 *  token without running the search. */
		searchValue?: string;
		/** Active tab — bindable so the host's nav handler can cycle on h/l.
		 *  Order is internal → external: Search and Refs work over events
		 *  already known to the engine; KB last because its pages come
		 *  from outside the Nostr graph. */
		activeTab?: 'search' | 'refs' | 'import';
		/** Embedding index status (sidecar health, counts, active/available
		 *  kinds). Null until the first status fetch resolves. */
		embeddingStatus?: EmbeddingStatusResponse | null;
		/** True while an embed/reindex pass is running — disables actions. */
		embeddingSyncing?: boolean;
		/** Embed events not yet in the index (incremental sync). */
		onembedmissing?: () => void;
		/** Clear the index and re-embed everything from scratch. */
		onembedreindex?: () => void;
		/** Persist a new set of embeddable kinds (engine-side). */
		onsetembedkinds?: (kinds: number[]) => void;
		/** Toggle auto-embed on retrieval + publishing (engine-side). */
		onsetautoembed?: (enabled: boolean) => void;
		/** Fetch current embedding status (called on mount if not yet loaded). */
		onrefreshembedstatus?: () => void;
	} = $props();

	// Self-fetch status on mount when the host hasn't populated it yet, so the
	// footer fills in regardless of buffer/mount timing (WM buffers persist, so
	// relying solely on the host's mount hook is fragile).
	onMount(() => { if (!embeddingStatus) onrefreshembedstatus?.(); });

	let checkedIds: Set<string> = $state(new Set());

	// Embedding-settings footer: collapsed by default (ephemeral view state —
	// the frontend owns expansion). Expands to reveal the shared
	// EmbeddingSettings controls (status + kinds + actions).
	let embedPanelOpen = $state(false);

	// Grouped mode: when the query had `count:NAME`, the response includes
	// histogram buckets. We switch the panel to a folded view where the
	// top level is bucket headers (value + count) and each expands to
	// reveal the contributing events (looked up from `results` by id).
	const groupedNames = $derived(Object.keys(tagCounts ?? {}));
	const isGrouped = $derived(groupedNames.length > 0);
	const resultsById = $derived(new Map(results.map((r) => [r.event_id, r])));
	let expandedBuckets: Set<string> = $state(new Set());
	function bucketKey(tagName: string, value: string): string {
		return `${tagName}::${value}`;
	}
	function toggleBucket(tagName: string, value: string) {
		const key = bucketKey(tagName, value);
		const next = new Set(expandedBuckets);
		if (next.has(key)) next.delete(key);
		else next.add(key);
		expandedBuckets = next;
	}

	function toggleCheck(id: string) {
		const next = new Set(checkedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedIds = next;
	}

	function selectAll() {
		checkedIds = new Set(results.map((r) => r.event_id));
	}

	function invertSelection() {
		const next = new Set<string>();
		for (const r of results) {
			if (!checkedIds.has(r.event_id)) next.add(r.event_id);
		}
		checkedIds = next;
	}

	function sendCheckedToContext() {
		const checked = results.filter((r) => checkedIds.has(r.event_id));
		if (checked.length > 0) {
			onaddmanytocontext(checked);
			checkedIds = new Set();
		}
	}

	function sendCheckedToCompose() {
		const checked = results.filter((r) => checkedIds.has(r.event_id));
		if (checked.length > 0) {
			onaddmanytocompose(checked);
			checkedIds = new Set();
		}
	}

	const hasChecked = $derived(checkedIds.size > 0);

	// Import state
	let importChecked: Set<number> = $state(new Set());
	let pageRange = $state('');
	let fileInput: HTMLInputElement | undefined = $state();

	function parseRange(range: string, max: number): number[] {
		const nums = new Set<number>();
		for (const part of range.split(',')) {
			const trimmed = part.trim();
			if (trimmed.includes('-')) {
				const [a, b] = trimmed.split('-').map(Number);
				if (!isNaN(a) && !isNaN(b)) {
					for (let i = Math.max(1, a); i <= Math.min(max, b); i++) nums.add(i);
				}
			} else {
				const n = Number(trimmed);
				if (!isNaN(n) && n >= 1 && n <= max) nums.add(n);
			}
		}
		return [...nums];
	}

	function applyRange() {
		const selected = parseRange(pageRange, importPages.length);
		importChecked = new Set(selected);
	}

	function importSelectAll() {
		importChecked = new Set(importPages.map(p => p.page_num));
	}

	function importInvert() {
		const next = new Set<number>();
		for (const p of importPages) {
			if (!importChecked.has(p.page_num)) next.add(p.page_num);
		}
		importChecked = next;
	}

	const hasImportChecked = $derived(importChecked.size > 0);
	const checkedImportPages = $derived(importPages.filter(p => importChecked.has(p.page_num)));
	const hasDocResults = $derived(importPages.length > 0 && importFilename !== '');
	const semanticTotal = $derived(
		(results.filter(r => r.semantic_score != null).length) +
		(importPages.length > 0 && importFilename ? importPages.length : 0)
	);
</script>

<div class="search-panel">
	{#if semanticTotal > 0}
		<div class="semantic-summary">
			{semanticTotal} semantic {semanticTotal === 1 ? 'match' : 'matches'}
			{#if results.some(r => r.semantic_score != null) && hasDocResults}
				({results.filter(r => r.semantic_score != null).length} events, {importPages.length} doc pages)
			{/if}
		</div>
	{/if}
	<div class="tab-bar">
		<button class="tab" class:active={activeTab === 'search'} onclick={() => (activeTab = 'search')}>
			Search
			{#if results.some(r => r.semantic_score != null)}
				<span class="tab-badge">{results.filter(r => r.semantic_score != null).length}</span>
			{/if}
		</button>
		<button class="tab" class:active={activeTab === 'refs'} onclick={() => (activeTab = 'refs')}>
			Refs
			{#if heldTotal > 0}
				<span class="tab-badge">{heldTotal}</span>
			{/if}
		</button>
		<button class="tab" class:active={activeTab === 'import'} onclick={() => { activeTab = 'import'; if (importPages.length === 0 && documentFiles.length === 0) onlistdocuments?.(); }}>
			KB
			{#if hasDocResults}
				<span class="tab-badge">{importPages.length}</span>
			{/if}
		</button>
	</div>

	{#if activeTab === 'search'}
		<SearchInput {onsearch} bind:value={searchValue} />

		<!-- Scope strip: the kinds a search runs against when the query
		     itself has no `k:` token. Makes the otherwise-invisible
		     default scope explicit, and the gear edits it. -->
		<div class="search-scope">
			<span class="scope-label">scope</span>
			{#if searchConfig.kinds.length > 0}
				{#each searchConfig.kinds as k (k)}
					<span class="scope-chip" title={kindLabel(k)}>k:{k}</span>
				{/each}
			{:else}
				<span class="scope-chip scope-chip--all" title="No kind filter — every kind matches">all kinds</span>
			{/if}
			<span class="scope-spacer"></span>
			<button
				class="scope-gear"
				onclick={openSearchConfig}
				title="Knowledge base — search defaults (kinds, limit, relays) and embedding settings"
				aria-label="Knowledge base settings"
			>⚙</button>
		</div>

		{#if count > 0}
			<div class="search-bar">
				<span class="search-summary">
					{count} results ({localCount} local, {relayCount} relay)
				</span>
				<div class="search-actions">
					<button class="sel-btn" onclick={selectAll} disabled={results.length === 0} title="Select all">All</button>
					<button class="sel-btn" onclick={invertSelection} disabled={results.length === 0} title="Invert selection">Inv</button>
					<button class="icon-btn" onclick={sendCheckedToContext} disabled={!hasChecked} title="Send to chat">◂</button>
					<button class="icon-btn" onclick={sendCheckedToCompose} disabled={!hasChecked} title="Send to compose">□</button>
				</div>
			</div>
		{/if}

		<div class="search-results" bind:this={listEl}>
			<!-- People category: kind-0 author matches, surfaced above
			     content results — search's people/notes fan-out. -->
			{#if profiles.length > 0}
				<div class="people-section">
					<div class="people-header">
						<span class="people-header__label">People</span>
						<span class="people-header__count">{profiles.length}</span>
					</div>
					{#each profiles as profile (profile.pubkey)}
						<PersonResultItem {profile} {onviewprofile} {localPubkeys} />
					{/each}
				</div>
			{/if}

			{#if isGrouped}
				<!-- Grouped view: top-level rows are histogram buckets from
				     `count:NAME`. Click a bucket to expand it into its
				     contributing events. -->
				{#each groupedNames as tagName}
					<div class="bucket-group">
						<div class="bucket-group__header">
							<span class="bucket-group__name">{tagName}</span>
							<span class="bucket-group__total">{tagCounts[tagName].length} values</span>
						</div>
						{#each tagCounts[tagName] as bucket (tagName + ':' + bucket.value)}
							{@const expanded = expandedBuckets.has(bucketKey(tagName, bucket.value))}
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<button
								class="bucket"
								class:bucket--open={expanded}
								onclick={() => toggleBucket(tagName, bucket.value)}
								title="{bucket.count} events with {tagName}={bucket.value}"
							>
								<span class="bucket__arrow" class:open={expanded}>{expanded ? '▾' : '▸'}</span>
								<span class="bucket__value">{bucket.value || '(empty)'}</span>
								<span class="bucket__count">{bucket.count}</span>
							</button>
							{#if expanded}
								<div class="bucket__events">
									{#each bucket.event_ids as id (id)}
										{@const r = resultsById.get(id)}
										{#if r}
											<div class="result-row result-row--nested" data-cursor={results.indexOf(r)}>
												<SearchResultItem
													result={r}
													checked={checkedIds.has(r.event_id)}
													ontogglecheck={() => toggleCheck(r.event_id)}
													{onselect}
													{onviewjson}
													{onaddtocontext}
													{onaddtocompose}
													{onignore}
													{onignorepubkey}
													{items}
													{localPubkeys}
													{onviewprofile}
													onpillaction={onresultpillaction}
												/>
											</div>
										{:else}
											<div class="bucket__event-missing">
												<span class="evid">{id.slice(0, 12)}…</span>
												<span class="hint">(event not in results — likely beyond fetch limit)</span>
											</div>
										{/if}
									{/each}
								</div>
							{/if}
						{/each}
					</div>
				{/each}
			{:else}
				{#each results as result, i (result.event_id)}
					<div class="result-row" class:result-row--cursor={i === cursor} data-cursor={i}>
						<SearchResultItem
							{result}
							checked={checkedIds.has(result.event_id)}
							ontogglecheck={() => toggleCheck(result.event_id)}
							{onselect}
							{onviewjson}
							{onaddtocontext}
							{onaddtocompose}
							{onignore}
							{onignorepubkey}
							{items}
							{localPubkeys}
							{onviewprofile}
							onpillaction={onresultpillaction}
						/>
					</div>
				{/each}
			{/if}

			{#if !loading && !relaySearchLoading && results.length === 0 && profiles.length === 0 && !isGrouped}
				{#if !hasSearched}
					<p class="empty">Search {searchContext}</p>
				{:else if canPromptRelays}
					<!-- Confirm mode: the modal will gate the fan-out;
					     keep the explicit CTA as a re-trigger so the
					     user can re-run if they declined the modal. -->
					<div class="empty empty-cta">
						<p>No events found in local DB.</p>
						<button class="empty-cta__btn" onclick={() => onsearchrelays?.()}>
							Search relays →
						</button>
					</div>
				{:else if networkMode === 'auto'}
					<!-- Auto mode: the fan-out ran silently and also
					     returned zero. Tell the user explicitly so the
					     panel doesn't read as "nothing happened". A
					     manual retry button is still useful in case
					     relay state has changed since. -->
					<div class="empty empty-cta">
						<p>Not found locally or on the connected relays.</p>
						<button class="empty-cta__btn" onclick={() => onsearchrelays?.()}>
							Retry relay search →
						</button>
					</div>
				{:else}
					<p class="empty">Search {searchContext}</p>
				{/if}
			{/if}

			{#if loading || relaySearchLoading}
				{@const isRelayPhase = !loading && relaySearchLoading}
				<div
					class="empty search-loading"
					class:search-loading--relay={isRelayPhase}
				>
					<p class="search-loading__label">
						{isRelayPhase ? 'Searching relays…' : 'Searching database…'}
					</p>
					<div
						class="search-loading__bar"
						role="progressbar"
						aria-label={isRelayPhase ? 'Searching relays' : 'Searching database'}
					>
						<div class="search-loading__fill"></div>
					</div>
					<p class="search-loading__hint">
						{#if isRelayPhase}
							Asking the configured relays for events matching this query.
							The activity toast tracks per-relay progress.
						{:else}
							Scanning every event — unindexed filters have no shortcut.
							Add a kind or a single-letter tag (e.g. <code>C:bible</code>)
							to narrow the scan.
						{/if}
					</p>
				</div>
			{/if}
		</div>
	{:else if activeTab === 'import'}
		<!-- KB / Import tab -->
		{#if importPages.length > 0}
			<!-- Page view -->
			<div class="import-header">
				<button class="import-back" onclick={() => onlistdocuments?.()}>← Back</button>
				<span class="import-file-name">{importFilename}</span>
				<span class="import-page-count">{importPages.length} pages</span>
			</div>
			<div class="search-bar">
				<input
					type="text"
					class="range-input"
					bind:value={pageRange}
					placeholder="1,3-7,10"
					onkeydown={(e) => { if (e.key === 'Enter') applyRange(); }}
					title="Page range"
				/>
				<div class="search-actions">
					<button class="sel-btn" onclick={importSelectAll}>All</button>
					<button class="sel-btn" onclick={importInvert}>Inv</button>
					<button class="icon-btn" onclick={() => onimportpagestocontext?.(checkedImportPages)} disabled={!hasImportChecked} title="Send to chat">◂</button>
					<button class="icon-btn" onclick={() => onimportpagestocompose?.(checkedImportPages)} disabled={!hasImportChecked} title="Send to compose">□</button>
				</div>
			</div>
			<div class="search-results">
				{#each importPages as page, i (page.page_num)}
					<div
						class="import-page"
						class:checked={importChecked.has(page.page_num)}
						class:import-page--cursor={i === importCursor}
						data-cursor={i}
					>
						<label class="import-check">
							<input type="checkbox" checked={importChecked.has(page.page_num)} onchange={() => {
								const next = new Set(importChecked);
								if (next.has(page.page_num)) next.delete(page.page_num); else next.add(page.page_num);
								importChecked = next;
							}} />
						</label>
						<div class="import-page-content">
							<div class="import-page-header">
								<span class="import-page-num">p.{page.page_num}</span>
								{#if page.title}
									<span class="import-page-title">{page.title}</span>
								{/if}
							</div>
							<p class="import-page-preview">{page.content.slice(0, 150)}{page.content.length > 150 ? '...' : ''}</p>
						</div>
						<div class="import-page-actions">
							<button class="icon-btn" onclick={() => onimportpagetocontext?.(page)} title="Send to chat">◂</button>
							<button class="icon-btn" onclick={() => onimportpagetocompose?.(page)} title="Send to compose">□</button>
						</div>
					</div>
				{/each}
			</div>
		{:else}
			<!-- File list view -->
			<div class="search-results">
				{#if importLoading}
					<p class="empty">Parsing document...</p>
				{:else}
					{#each documentFiles as file}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div class="import-file" onclick={() => onparsedocument?.(file.name)} onkeydown={(e) => { if (e.key === 'Enter') onparsedocument?.(file.name); }} role="button" tabindex="0">
							<span class="import-file-badge">{file.format}</span>
							<span class="import-file-name">{file.name}</span>
							<span class="import-file-size">{(file.size / 1024).toFixed(0)}KB</span>
						</div>
					{/each}
					{#if documentFiles.length === 0}
						<p class="empty">No documents in folder</p>
					{/if}
				{/if}
				<div class="import-upload">
					<input type="file" accept=".pdf,.docx,.epub,.html,.htm,.txt,.md,.org,.adoc" bind:this={fileInput} onchange={() => {
						const f = fileInput?.files?.[0];
						if (f) onimportfile?.(f);
					}} hidden />
					<button class="import-upload-btn" onclick={() => fileInput?.click()}>Upload file</button>
				</div>
			</div>
		{/if}
	{:else}
		<!-- Refs tab — held items from the reference pool, embedded so
		     research-style use (search ↔ refs ↔ kb) cycles through the
		     same panel via h/l. -->
		{#if heldTotal > 0}
			<!-- Local substring filter over title + content. Pure client
			     side; the held set is small enough that this is instant. -->
			<div class="refs-filter">
				<input
					type="text"
					class="refs-filter__input"
					placeholder="filter refs…"
					bind:value={refsQuery}
					data-entry
				/>
				{#if refsQuery}
					<button
						class="refs-filter__clear"
						onclick={() => (refsQuery = '')}
						title="Clear filter"
						aria-label="Clear filter"
					>×</button>
					<span class="refs-filter__count">
						{heldItems.length} / {heldTotal}
					</span>
				{/if}
			</div>
		{/if}
		<div class="search-results">
			{#if heldTotal === 0}
				<p class="empty">
					Nothing here yet. Anything you send to <strong>context</strong>
					or <strong>compose</strong> shows up automatically — refs is
					the recency history of pool activity. Drop a row to remove
					it from everywhere.
				</p>
			{:else if heldItems.length === 0}
				<p class="empty">
					No held items match <code>{refsQuery}</code>. Clear the filter
					or try a different substring.
				</p>
			{:else}
				{#each heldItems as item, i (item.id)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="held-row"
						class:held-row--cursor={i === refsCursor}
						data-cursor={i}
						onclick={() => onopenheld?.(item)}
						onkeydown={(e) => { if (e.key === 'Enter') onopenheld?.(item); }}
						role="button"
						tabindex="0"
					>
						<div class="held-row__body">
							<div class="held-row__head">
								<span class="held-row__title">{item.title}</span>
								{#if item.source_addr?.kind != null}
									<span class="held-row__kind">k:{item.source_addr.kind}</span>
								{/if}
							</div>
						</div>
						<!-- One unified pill stack — actions + state. ctx/cmp toggle,
						     drop drops, srch appends coord to search input. -->
						<div class="held-row__rail">
							<PoolStateBadges
								{item}
								onpillctx={() => onrouterefcontext?.(item)}
								onpillcmp={() => onrouterefcompose?.(item)}
								onpilldrop={() => onreleaseheld?.(item.id)}
							/>
							{#if onrouterefsearch}
								<button
									class="held-row__srch"
									onclick={(e) => {
										e.stopPropagation();
										onrouterefsearch?.(item);
									}}
									title="Append coordinate token to the search query"
								>search</button>
							{/if}
						</div>
					</div>
				{/each}
			{/if}
		</div>
	{/if}

	<!-- Embedding settings — persistent footer at the bottom of the search
	     buffer. Always rendered (even before the status fetch resolves) so it's
	     discoverable regardless of mount timing; collapsed by default. Expands
	     to show sidecar status, index counts, the embeddable-kind checkboxes,
	     and the embed actions. This is the home of "what gets embedded": the
	     engine persists the kind set. -->
	<div class="embed-panel" class:embed-panel--open={embedPanelOpen}>
		<button
			class="embed-panel__bar"
			onclick={() => (embedPanelOpen = !embedPanelOpen)}
			aria-expanded={embedPanelOpen}
			title="Embedding settings — the index that powers ~: semantic search"
		>
			<span class="embed-panel__arrow">{embedPanelOpen ? '▾' : '▸'}</span>
			<span class="embed-panel__title">Embedding settings</span>
			{#if embeddingStatus?.enabled}
				<span class="embed-panel__stat">embed {embeddingStatus.indexed_count}/{embeddingStatus.total_events}</span>
				<span
					class="embed-panel__dot"
					class:embed-panel__dot--ok={embeddingStatus.sidecar_available}
					class:embed-panel__dot--off={!embeddingStatus.sidecar_available}
					title={embeddingStatus.sidecar_available ? 'Sidecar connected' : 'Sidecar unreachable'}
				></span>
			{:else if embeddingStatus}
				<span class="embed-panel__stat embed-panel__stat--off">off</span>
			{:else}
				<span class="embed-panel__stat embed-panel__stat--off">…</span>
			{/if}
		</button>

		{#if embedPanelOpen}
			<div class="embed-panel__body">
				<EmbeddingSettings
					status={embeddingStatus}
					syncing={embeddingSyncing}
					{onembedmissing}
					{onembedreindex}
					{onsetembedkinds}
					{onsetautoembed}
				/>
			</div>
		{/if}
	</div>
</div>

<style>
	.search-panel {
		display: flex;
		flex-direction: column;
		flex: 1;
		min-height: 0;
		overflow: hidden;
	}

	.search-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 12px 8px;
		gap: 8px;
	}

	.search-summary {
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	.search-actions {
		display: flex;
		gap: 4px;
		align-items: center;
	}

	.sel-btn {
		font-size: 0.65rem;
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		padding: 4px 8px;
		font-size: 0.85rem;
		min-width: 28px;
	}

	/* Scope strip — sits directly under the search input. */
	.search-scope {
		display: flex;
		align-items: center;
		gap: 5px;
		padding: 0 12px 6px;
		flex-wrap: wrap;
	}
	.scope-label {
		font-size: 0.6rem;
		text-transform: uppercase;
		letter-spacing: 0.07em;
		color: var(--fg-muted);
	}
	.scope-chip {
		font-family: var(--font-mono);
		font-size: 0.62rem;
		padding: 1px 6px;
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--accent) 14%, transparent);
		color: var(--accent);
		cursor: default;
	}
	.scope-chip--all {
		background: color-mix(in srgb, var(--fg-muted) 16%, transparent);
		color: var(--fg-muted);
	}
	.scope-spacer {
		flex: 1;
	}
	.scope-gear {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: 0.8rem;
		padding: 0 2px;
		line-height: 1;
	}
	.scope-gear:hover {
		color: var(--accent);
	}

	.search-results {
		flex: 1;
		overflow-y: auto;
	}

	/* Ranger-style cursor: bright bar + tinted background, mirrors
	   FeedBuffer / ReaderBuffer outline cursor. Same treatment for the
	   import-page rows and held-item rows so j/k reads the same across
	   the three tabs. */
	.result-row--cursor,
	.import-page--cursor,
	.held-row--cursor {
		box-shadow: inset 4px 0 0 var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.result-row--nested {
		padding-left: 18px;
		border-left: 2px solid var(--border);
	}

	/* Refs filter — small inline substring search over the held set.
	   Compact bar at the top of the Refs tab; the engine-side search
	   input is irrelevant here since refs is already local. */
	.refs-filter {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--border);
	}
	.refs-filter__input {
		flex: 1;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		color: var(--fg);
	}
	.refs-filter__input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.refs-filter__clear {
		font-family: var(--font-mono);
		font-size: 0.9rem;
		line-height: 1;
		padding: 0 8px;
		background: transparent;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		color: var(--fg-muted);
		cursor: pointer;
	}
	.refs-filter__clear:hover { color: var(--fg); border-color: var(--id-yours); }
	.refs-filter__count {
		font-family: var(--font-mono);
		font-size: 0.65rem;
		color: var(--fg-muted);
		white-space: nowrap;
	}

	/* Refs tab rows — pool's held items. Imported-accent border-left so
	   the row reads as a reference. */
	.held-row {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		border-left: 3px solid var(--id-imported);
		cursor: pointer;
	}
	.held-row:hover { background: var(--bg-surface); }
	.held-row__body { flex: 1; min-width: 0; }
	.held-row__head {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.held-row__title {
		font-size: 0.85rem;
		font-weight: 600;
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.held-row__kind {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		white-space: nowrap;
	}
	/* The held-row rail is the right-side strip — PoolStateBadges plus
	   the srch route (which is refs-specific and not part of the shared
	   pill set). Wrapping them in a flex column keeps the visual rhythm
	   consistent across rows even though srch lives outside the
	   component. */
	.held-row__rail {
		display: flex;
		flex-direction: column;
		gap: 3px;
		flex-shrink: 0;
		align-items: flex-start;
	}
	.held-row__srch {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		line-height: 1.4;
		padding: 0 6px;
		background: transparent;
		border: none;
		border-radius: 3px;
		color: var(--base5);
		cursor: pointer;
		white-space: nowrap;
		font-weight: 600;
	}
	.held-row__srch:hover {
		color: var(--fg);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}

	/* People category header — author matches above content results. */
	.people-section {
		margin-bottom: 4px;
	}
	.people-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		background: color-mix(in srgb, #a093c7 12%, transparent);
		border-bottom: 1px solid var(--border);
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.people-header__label {
		flex: 1;
		color: #a093c7;
		font-weight: 600;
	}
	.people-header__count {
		font-family: var(--font-mono);
		color: var(--fg-muted);
	}

	/* Grouped view for `count:NAME` queries. */
	.bucket-group {
		margin-bottom: 8px;
	}
	.bucket-group__header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		background: var(--panel-bg-soft, var(--border));
		border-bottom: 1px solid var(--border);
		font-family: var(--font-mono);
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
	}
	.bucket-group__name {
		flex: 1;
		color: var(--id-yours);
		font-weight: 600;
	}
	.bucket-group__total {
		font-family: var(--font-mono);
	}
	.bucket {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 5px 12px;
		background: none;
		border: none;
		border-bottom: 1px solid var(--border);
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-family: var(--font-mono);
		font-size: 0.78rem;
	}
	.bucket:hover {
		background: color-mix(in srgb, var(--id-yours) 8%, transparent);
	}
	.bucket--open {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.bucket__arrow {
		color: var(--fg-muted);
		font-size: 0.7rem;
		width: 12px;
		flex-shrink: 0;
	}
	.bucket__arrow.open {
		color: var(--id-yours);
	}
	.bucket__value {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.bucket__count {
		color: var(--fg-muted);
		font-size: 0.7rem;
		padding: 1px 6px;
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		flex-shrink: 0;
	}
	.bucket__events {
		background: color-mix(in srgb, var(--id-yours) 4%, transparent);
	}
	.bucket__event-missing {
		padding: 4px 12px 4px 30px;
		font-size: 0.7rem;
		color: var(--fg-muted);
		font-family: var(--font-mono);
	}
	.bucket__event-missing .hint {
		font-style: italic;
		margin-left: 8px;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: 0.85rem;
		padding: 0 12px;
	}
	.empty-cta {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
	}

	/* Search loading — an exhaustive scan (multi-char tag / keyword) can
	   take a few seconds, so show an indeterminate bar rather than a
	   bare "Searching..." that looks hung. */
	.search-loading {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
	}
	.search-loading__label {
		margin: 0;
		font-family: var(--font-mono);
	}
	.search-loading__bar {
		width: 180px;
		height: 4px;
		border-radius: var(--r-sm, 3px);
		background: color-mix(in srgb, var(--fg-muted) 20%, transparent);
		overflow: hidden;
	}
	.search-loading__fill {
		height: 100%;
		width: 40%;
		border-radius: inherit;
		background: var(--accent);
		animation: search-scan 1.1s ease-in-out infinite;
	}
	/* Relay phase uses the "network" accent so the bar visually
	 * distinguishes "scanning local DB" from "asking relays" — same
	 * animation, different colour, matches the activity-toast palette
	 * the user sees in confirm mode. */
	.search-loading--relay .search-loading__fill {
		background: var(--id-yours);
	}
	.search-loading--relay .search-loading__bar {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	@keyframes search-scan {
		from { transform: translateX(-110%); }
		to { transform: translateX(260%); }
	}
	.search-loading__hint {
		margin: 0;
		max-width: 240px;
		font-size: 0.7rem;
		line-height: 1.4;
	}
	.search-loading__hint code {
		font-family: var(--font-mono);
		color: var(--accent);
	}
	.empty-cta p { margin: 0; }
	.empty-cta__btn {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 6px 16px;
		background: transparent;
		border: 1px solid var(--state-online);
		color: var(--state-online);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.empty-cta__btn:hover {
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
	}

	/* Semantic summary */
	.semantic-summary {
		padding: 4px 12px;
		font-size: 0.7rem;
		color: #22c55e;
		background: #22c55e10;
		text-align: center;
		border-bottom: 1px solid var(--border);
	}

	/* Tab bar */
	.tab-bar {
		display: flex;
		border-bottom: 1px solid var(--border);
	}
	.tab {
		flex: 1;
		padding: 6px 0;
		font-size: 0.75rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.tab.active {
		color: var(--accent);
		border-bottom-color: var(--accent);
	}
	.tab-badge {
		font-size: 0.6rem;
		background: var(--accent);
		color: white;
		padding: 0 4px;
		border-radius: 8px;
		margin-left: 4px;
	}

	/* Import file list */
	.import-file {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		cursor: pointer;
	}
	.import-file:hover { background: var(--bg-surface); }
	.import-file-badge {
		font-size: 0.6rem;
		padding: 1px 5px;
		border-radius: 3px;
		background: var(--border);
		color: var(--fg-muted);
		text-transform: uppercase;
		font-weight: 600;
	}
	.import-file-name {
		flex: 1;
		font-size: 0.8rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.import-file-size {
		font-size: 0.7rem;
		color: var(--fg-muted);
	}

	/* Import upload */
	.import-upload {
		padding: 12px;
		text-align: center;
	}
	.import-upload-btn {
		font-size: 0.8rem;
		padding: 6px 16px;
	}

	/* Import header */
	.import-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--border);
		font-size: 0.8rem;
	}
	.import-back {
		background: none;
		border: none;
		color: var(--accent);
		cursor: pointer;
		font-size: 0.75rem;
		padding: 2px 4px;
	}
	.import-page-count {
		color: var(--fg-muted);
		font-size: 0.7rem;
		margin-left: auto;
	}

	/* Range input */
	.range-input {
		width: 80px;
		font-size: 0.7rem;
		padding: 3px 6px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		font-family: var(--font-mono);
	}

	/* Import page cards */
	.import-page {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
	}
	.import-page.checked { background: var(--bg-surface); }
	.import-check { flex-shrink: 0; display: flex; align-items: center; }
	.import-page-content { flex: 1; min-width: 0; }
	.import-page-header {
		display: flex;
		gap: 6px;
		align-items: center;
		margin-bottom: 2px;
	}
	.import-page-num {
		font-size: 0.65rem;
		color: var(--fg-muted);
		font-family: var(--font-mono);
	}
	.import-page-title {
		font-size: 0.8rem;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.import-page-preview {
		font-size: 0.75rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 0;
	}
	.import-page-actions {
		display: flex;
		gap: 2px;
		flex-shrink: 0;
	}

	/* Embedding settings — collapsible footer pinned to the bottom of the
	   search buffer. The bar stays a thin glanceable strip when collapsed. */
	.embed-panel {
		flex-shrink: 0;
		border-top: 1px solid var(--panel-border);
		background: var(--panel-bg-soft, var(--bg-surface));
	}
	.embed-panel__bar {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 6px 12px;
		background: none;
		border: none;
		cursor: pointer;
		color: var(--fg-muted);
		font-size: 0.68rem;
		text-align: left;
	}
	.embed-panel__bar:hover { color: var(--fg); }
	.embed-panel__arrow { width: 0.8em; flex-shrink: 0; }
	.embed-panel__title {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-weight: 600;
	}
	.embed-panel__stat {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: 0.62rem;
		color: var(--fg-muted);
	}
	.embed-panel__stat--off { color: var(--fg-muted); opacity: 0.7; }
	.embed-panel__dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.embed-panel__dot--ok { background: var(--green); }
	.embed-panel__dot--off { background: var(--red); }

	.embed-panel__body {
		padding: 4px 12px 12px;
	}
</style>
