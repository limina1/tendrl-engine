<script lang="ts">
	import type {
		SearchResult,
		ProfileResult,
		ContextItem,
		DocumentFile,
		ImportPage,
		TagValueCount
	} from '$lib/types';
	import SearchInput from './SearchInput.svelte';
	import SearchResultItem from './SearchResultItem.svelte';
	import PersonResultItem from './PersonResultItem.svelte';
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
		onsearchrelays
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
	} = $props();

	let activeTab: 'search' | 'import' = $state('search');

	let checkedIds: Set<string> = $state(new Set());

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
		<button class="tab" class:active={activeTab === 'import'} onclick={() => { activeTab = 'import'; if (importPages.length === 0 && documentFiles.length === 0) onlistdocuments?.(); }}>
			Import
			{#if hasDocResults}
				<span class="tab-badge">{importPages.length}</span>
			{/if}
		</button>
	</div>

	{#if activeTab === 'search'}
		<SearchInput {onsearch} />

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
				title="Configure search defaults — kinds, limit (explicit k: in a query overrides this)"
				aria-label="Configure search defaults"
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
						/>
					</div>
				{/each}
			{/if}

			{#if !loading && results.length === 0 && profiles.length === 0 && !isGrouped}
				{#if canPromptRelays}
					<div class="empty empty-cta">
						<p>No events found in local DB.</p>
						<button class="empty-cta__btn" onclick={() => onsearchrelays?.()}>
							Search relays →
						</button>
					</div>
				{:else}
					<p class="empty">Search {searchContext}</p>
				{/if}
			{/if}

			{#if loading}
				<div class="empty search-loading">
					<p class="search-loading__label">Searching database…</p>
					<div
						class="search-loading__bar"
						role="progressbar"
						aria-label="Searching database"
					>
						<div class="search-loading__fill"></div>
					</div>
					<p class="search-loading__hint">
						Scanning every event — unindexed filters have no shortcut.
						Add a kind or a single-letter tag (e.g. <code>C:bible</code>)
						to narrow the scan.
					</p>
				</div>
			{/if}
		</div>
	{:else}
		<!-- Import tab -->
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
				{#each importPages as page (page.page_num)}
					<div class="import-page" class:checked={importChecked.has(page.page_num)}>
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
	{/if}
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
	   FeedBuffer / ReaderBuffer outline cursor. */
	.result-row--cursor {
		box-shadow: inset 4px 0 0 var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.result-row--nested {
		padding-left: 18px;
		border-left: 2px solid var(--border);
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
</style>
