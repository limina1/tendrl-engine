<script lang="ts">
	// Shown when the engine (in Confirm mode) emits a fetch `intent` that
	// needs approval. Phase 6 wiring: when the intent carries a
	// structured `summary` (RequestSummary), render the canonical DSL
	// sentence, the filter clauses, and the per-phase composition so the
	// user sees EXACTLY what's about to be requested (and from where)
	// rather than a flat relay list. Falls back to the legacy flat view
	// when summary is absent (older engine intents).

	import type { FetchEvent, NipFilter, CompositionShape, Phase } from '$lib/types';
	import { resolveConfirm, reissueConfirm } from '$lib/network/fetch-events.svelte';

	type IntentEvent = Extract<FetchEvent, { type: 'intent' }>;
	let {
		intent,
		general = false,
		onToggleGeneral
	}: {
		intent: IntentEvent;
		/** Live "general feed" preference (app.feedGeneral). */
		general?: boolean;
		/** Flip the preference + re-run the feed sync (re-composes the query). */
		onToggleGeneral?: () => void;
	} = $props();

	/** Render a single NipFilter as a space-separated clause string.
	 *  Mirrors the engine's filter_to_dsl_clauses ordering — event-shape
	 *  fields first (k, by, id, tags), then query controls (search,
	 *  limit, time bounds). */
	function renderFilter(f: NipFilter): string {
		const parts: string[] = [];
		if (f.kinds?.length) parts.push(`k:${f.kinds.join(',')}`);
		if (f.authors?.length) parts.push(`by:${f.authors.join(',')}`);
		if (f.ids?.length) for (const id of f.ids) parts.push(`id:${id}`);
		if (f.tags)
			for (const [tag, vals] of Object.entries(f.tags))
				for (const v of vals) parts.push(`#${tag}:${v}`);
		if (f.search) parts.push(`~:"${f.search}"`);
		if (f.limit !== undefined) parts.push(`limit:${f.limit}`);
		if (f.since !== undefined) parts.push(`since:${f.since}`);
		if (f.until !== undefined) parts.push(`until:${f.until}`);
		return parts.join(' ');
	}

	/** Render the composition shape as the DSL trailing string —
	 *  `via:read then:indexer.fallback` etc. */
	function renderComposition(comp: CompositionShape | undefined): string {
		if (!comp?.phases?.length) return '';
		const parts: string[] = [];
		for (let i = 0; i < comp.phases.length; i++) {
			const stage = comp.phases[i];
			const keyword = i === 0 ? 'via' : stage.start_delay_ms > 0 ? 'also' : 'then';
			const phases = stage.members.map(([p]) => p as string).join(',');
			if (!phases) continue;
			if (keyword === 'also') parts.push(`also:${phases} Δ${stage.start_delay_ms}`);
			else parts.push(`${keyword}:${phases}`);
		}
		return parts.join(' ');
	}

	// Every proposed relay starts selected; the user can drop any for
	// this one operation, or append an extra.
	let deselected = $state<Set<string>>(new Set());
	let extras = $state<string[]>([]);
	let appendInput = $state('');
	let appendError = $state<string | null>(null);
	let detailsOpen = $state(false);
	// "General feed" — the broad, un-author-scoped pull is now part of the
	// engine-composed query (the `general` flag threads through listPublications
	// → list_root_publications). This toggle just reflects the live preference
	// and re-requests on change:
	//   - logged out → the query has no author scope, so it's already broad:
	//     toggle forced on (green) + disabled (nothing to narrow to).
	//   - logged in → reflects app.feedGeneral; toggling re-runs the sync, which
	//     re-composes the query (scoped ± broad) into a fresh confirm intent.
	const isScopedQuery = $derived(
		(intent.summary?.filters ?? []).some((f) => (f.authors?.length ?? 0) > 0)
	);
	// Only a feed-list intent (kind 30040) has a meaningful general toggle.
	const isFeedIntent = $derived(
		(intent.summary?.filters ?? []).some((f) => f.kinds?.includes(30040))
	);
	const generalOn = $derived(isScopedQuery ? general : true);

	function toggleGeneral() {
		// Cancel this intent IN PLACE — reissueConfirm keeps the modal mounted
		// (pendingReplace stops the cancel's `failed` event from nulling the
		// slot), so when the app flips feedGeneral and re-runs the sync the
		// re-composed intent REPLACES the open modal instead of closing +
		// reopening it. Using resolveConfirm(false) here unmounts the modal
		// (visible flicker / "the component closes").
		reissueConfirm();
		onToggleGeneral?.();
	}
	// Split mode: when true, render each filter as its own
	// standalone single-filter request (each with the same composition
	// appended) so the user can see + copy them individually. The
	// engine still fires one multi-filter REQ either way — this is
	// purely a display toggle.
	let querySplit = $state(false);

	// Compute the filter-by-filter forms so split mode and individual
	// copy buttons share the same source.
	const compositionDsl = $derived(
		intent.summary ? renderComposition(intent.summary.composition) : ''
	);
	const filterDsls = $derived.by(() => {
		if (!intent.summary?.filters?.length) return [] as string[];
		return intent.summary.filters.map((f) => {
			const clauses = renderFilter(f);
			return compositionDsl ? `${clauses} ${compositionDsl}` : clauses;
		});
	});
	// Wrapped joined form: filters on their own lines with the `|`
	// separator visible, then composition on its own trailing line.
	const joinedWrapped = $derived.by(() => {
		if (!intent.summary?.filters?.length) return intent.summary?.dsl ?? '';
		const filterClauses = intent.summary.filters.map(renderFilter);
		const filterPart = filterClauses.join(' |\n');
		return compositionDsl ? `${filterPart}\n${compositionDsl}` : filterPart;
	});

	// Flat union of every relay listed across all composition stages
	// (or `intent.relays` when there's no summary). Drives the
	// resolve-confirm payload — the engine ultimately wants a single
	// list of approved URLs.
	const proposedRelays = $derived.by(() => {
		const summary = intent.summary;
		if (summary?.composition?.phases?.length) {
			const seen = new Set<string>();
			const out: string[] = [];
			for (const stage of summary.composition.phases) {
				for (const [, urls] of stage.members) {
					for (const u of urls) {
						if (!seen.has(u)) {
							seen.add(u);
							out.push(u);
						}
					}
				}
			}
			return out;
		}
		return intent.relays;
	});

	const allRelays = $derived([...proposedRelays, ...extras]);
	const selectedRelays = $derived(allRelays.filter((r) => !deselected.has(r)));

	const PATTERN_LABEL: Record<string, string> = {
		event: 'event',
		publication: 'publication',
		thread: 'thread',
		search: 'search',
		profile: 'profile',
		custom: 'fetch'
	};

	function relayLabel(phase: Phase): string {
		switch (phase) {
			case 'read': return 'Read';
			case 'write': return 'Write';
			case 'publish': return 'Publish';
			case 'broadcast': return 'Broadcast';
			case 'search.default': return 'Search · default';
			case 'search.fallback': return 'Search · fallback';
			case 'indexer.default': return 'Indexer · default';
			case 'indexer.fallback': return 'Indexer · fallback';
		}
	}

	function toggle(url: string) {
		if (deselected.has(url)) deselected.delete(url);
		else deselected.add(url);
		deselected = new Set(deselected);
	}

	function addExtra() {
		const v = appendInput.trim();
		if (!v) return;
		if (!/^wss?:\/\//i.test(v)) {
			appendError = 'Relay URL must start with ws:// or wss://';
			return;
		}
		if (allRelays.includes(v)) {
			appendError = 'Already in the list';
			return;
		}
		extras = [...extras, v];
		appendInput = '';
		appendError = null;
	}

	function copyDsl() {
		// Copy-all is always the canonical single-line form (the same
		// string from_dsl round-trips with), not the wrapped display.
		if (!intent.summary?.dsl) return;
		navigator.clipboard?.writeText(intent.summary.dsl).catch(() => {});
	}
	function copyOne(s: string) {
		navigator.clipboard?.writeText(s).catch(() => {});
	}

	function confirm() {
		if (selectedRelays.length === 0) return;
		// The broad "general feed" pull (when on) is already part of this
		// intent's composed query, so a plain confirm covers it.
		resolveConfirm(true, selectedRelays);
	}
	function cancel() {
		resolveConfirm(false);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') cancel();
		if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'INPUT') confirm();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="rf-backdrop" onclick={cancel} role="presentation">
	<div
		class="rf-modal"
		data-tour="feed-sync"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		tabindex="-1"
	>
		<header class="rf-header">
			<h3 class="rf-title">
				<span class="rf-pattern">{PATTERN_LABEL[intent.pattern] ?? intent.pattern}</span>
				{intent.label}
			</h3>
			<button class="rf-close" onclick={cancel} aria-label="Close">×</button>
		</header>

		<!-- Primary action: flat relay list with per-relay
		     deselect. This is what the user is actually deciding —
		     "fire from these relays or not." Structured details
		     (DSL / filters / composition) are in a collapsed
		     section below for users who want to inspect them. -->
		<div class="rf-section">
			<div class="rf-section-head">Relays</div>
			{#if allRelays.length === 0}
				<p class="rf-empty">No relays proposed — add one below.</p>
			{:else}
				<ul class="rf-list">
					{#each allRelays as url (url)}
						<li>
							<label class="rf-row">
								<input
									type="checkbox"
									checked={!deselected.has(url)}
									onchange={() => toggle(url)}
								/>
								<code class="rf-url">{url}</code>
							</label>
						</li>
					{/each}
				</ul>
			{/if}
		</div>

		<div class="rf-append">
			<input
				class="rf-input"
				placeholder="wss://relay.example.com"
				bind:value={appendInput}
				onkeydown={(e) => {
					if (e.key === 'Enter') {
						e.preventDefault();
						addExtra();
					}
				}}
			/>
			<button class="rf-append-btn" onclick={addExtra}>Add relay</button>
		</div>
		{#if isFeedIntent}
			<div class="rf-general" data-tour="general-feed">
				<button
					class="rf-append-btn rf-general-btn"
					class:rf-general-btn--on={generalOn}
					onclick={toggleGeneral}
					disabled={!isScopedQuery}
					aria-pressed={generalOn}
					title={isScopedQuery
						? 'General feed: also pull recent publications from all authors, not just yours. Toggling re-runs the fetch.'
						: 'Logged out — the feed is already broad (all authors)'}
				>{generalOn ? '✓' : '○'} General feed</button>
			</div>
		{/if}
		{#if appendError}
			<p class="rf-error">{appendError}</p>
		{/if}

		<!-- Expandable details — structured summary (DSL sentence,
		     filter clauses, composition phases) for users who want
		     to see exactly what's being requested. Collapsed by
		     default since the relay picker above is the action. -->
		{#if intent.summary?.dsl || intent.summary?.filters?.length || intent.summary?.composition?.phases?.length || intent.steps.length > 0}
			<div class="rf-section">
				<button
					class="rf-steps-head"
					onclick={() => (detailsOpen = !detailsOpen)}
					aria-expanded={detailsOpen}
				>
					<span class="rf-caret">{detailsOpen ? '▾' : '▸'}</span>
					Details
				</button>

				{#if detailsOpen}
					{#if intent.summary?.dsl}
						<div class="rf-detail-block">
							<div class="rf-section-head-row">
								<span class="rf-sub-head">Query</span>
								<div class="rf-query-toggles">
									<button
										class="rf-copy"
										onclick={copyDsl}
										title="Copy the canonical single-line DSL — round-trips with the parser."
									>copy all</button>
									{#if filterDsls.length > 1}
										<button
											class="rf-copy"
											onclick={() => (querySplit = !querySplit)}
											title={querySplit
												? 'Show as a single union request (engine fires one REQ with multiple filters)'
												: 'Split into per-filter request lines, each individually copyable'}
										>{querySplit ? 'join' : 'split'}</button>
									{/if}
								</div>
							</div>
							{#if querySplit && filterDsls.length > 1}
								<!-- Split mode: each filter rendered as its
								     own standalone single-filter request,
								     individually copyable. Engine still
								     fires one multi-filter REQ — this is
								     a display + copy convenience. -->
								<ol class="rf-query-split">
									{#each filterDsls as one, i (i)}
										<li>
											<button
												class="rf-copy rf-copy--inline"
												onclick={() => copyOne(one)}
												title="Copy this filter"
											>copy</button>
											<code class="rf-dsl rf-dsl--inline">{one}</code>
										</li>
									{/each}
								</ol>
							{:else}
								<!-- Joined wrapped: canonical form but
								     displayed multi-line on `|` for
								     readability. Copy still gives the
								     single-line canonical sentence. -->
								<code class="rf-dsl rf-dsl--wrapped">{joinedWrapped}</code>
							{/if}
						</div>
					{/if}

					{#if intent.summary?.filters?.length}
						<div class="rf-detail-block">
							<div class="rf-sub-head">Filters</div>
							{#each intent.summary.filters as f, i (i)}
								<div class="rf-filter">
									<span class="rf-filter-idx">#{i + 1}</span>
									<div class="rf-filter-clauses">
										{#if f.kinds?.length}
											<span class="rf-clause">k:{f.kinds.join(',')}</span>
										{/if}
										{#if f.authors?.length}
											<span class="rf-clause" title={f.authors.join(', ')}>
												by:{f.authors.length === 1
													? `${f.authors[0].slice(0, 12)}…`
													: `${f.authors.length} authors`}
											</span>
										{/if}
										{#if f.ids?.length}
											<span class="rf-clause">ids:{f.ids.length}</span>
										{/if}
										{#if f.since != null}<span class="rf-clause">since:{f.since}</span>{/if}
										{#if f.until != null}<span class="rf-clause">until:{f.until}</span>{/if}
										{#if f.limit != null}<span class="rf-clause">limit:{f.limit}</span>{/if}
										{#if f.search}<span class="rf-clause">~:"{f.search}"</span>{/if}
										{#if f.tags}
											{#each Object.entries(f.tags) as [tag, vals]}
												<span class="rf-clause">{tag}:{vals.join(',')}</span>
											{/each}
										{/if}
									</div>
								</div>
							{/each}
						</div>
					{/if}

					{#if intent.summary?.composition?.phases?.length}
						<div class="rf-detail-block">
							<div class="rf-sub-head">Composition</div>
							{#each intent.summary.composition.phases as stage, i (i)}
								<div class="rf-stage">
									<div class="rf-stage-head">
										<span class="rf-stage-num">{i + 1}.</span>
										<span class="rf-stage-label">{stage.label}</span>
										{#if stage.start_delay_ms > 0}
											<span class="rf-stage-delay">Δ{stage.start_delay_ms}ms</span>
										{/if}
									</div>
									{#each stage.members as [phase, urls]}
										<div class="rf-phase">
											<div class="rf-phase-label">
												{relayLabel(phase)} · {urls.length} relay{urls.length === 1 ? '' : 's'}
											</div>
										</div>
									{/each}
								</div>
							{/each}
						</div>
					{/if}

					{#if intent.steps.length > 0}
						<div class="rf-detail-block">
							<div class="rf-sub-head">Steps</div>
							<ol class="rf-steps">
								{#each intent.steps as step, i (i)}
									<li>{step}</li>
								{/each}
							</ol>
						</div>
					{/if}
				{/if}
			</div>
		{/if}

		<footer class="rf-footer">
			<button class="rf-action rf-action--ghost" onclick={cancel}>Cancel</button>
			<button
				class="rf-action rf-action--primary"
				onclick={confirm}
				disabled={selectedRelays.length === 0}
			>
				Fetch from {selectedRelays.length} relay{selectedRelays.length === 1 ? '' : 's'}
			</button>
		</footer>
	</div>
</div>

<style>
	.rf-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.55);
		z-index: 250;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.rf-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 90vw;
		max-width: 560px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		overflow-y: auto;
	}
	.rf-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.rf-title {
		margin: 0;
		font-size: var(--t-sm);
		color: var(--base7);
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.rf-pattern {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 1px);
		color: var(--id-yours);
		border: 1px solid color-mix(in srgb, var(--id-yours) 40%, transparent);
		border-radius: var(--r-sm);
		padding: 1px 6px;
	}
	.rf-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
	}
	.rf-close:hover {
		color: var(--fg);
	}

	.rf-section {
		padding: 8px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.rf-section-head {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin-bottom: 6px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-section-head-row {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		margin-bottom: 6px;
	}
	.rf-sub-head {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 2px);
	}
	.rf-detail-block {
		padding: 6px 0;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 50%, transparent);
	}
	.rf-detail-block:first-of-type {
		border-top: none;
		padding-top: 8px;
	}
	.rf-copy {
		appearance: none;
		background: none;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--base5);
		font: inherit;
		font-size: calc(var(--t-xs) - 1px);
		padding: 1px 6px;
		cursor: pointer;
	}
	.rf-copy:hover {
		color: var(--fg);
	}
	.rf-dsl {
		display: block;
		background: var(--bg-surface);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		overflow-x: auto;
		white-space: nowrap;
	}
	.rf-dsl--wrapped {
		white-space: pre-wrap;
		word-break: break-word;
	}
	.rf-dsl--inline {
		display: inline;
		padding: 2px 6px;
		white-space: normal;
		word-break: break-word;
	}
	.rf-query-toggles {
		display: flex;
		gap: 4px;
	}
	.rf-query-split {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.rf-query-split li {
		display: flex;
		gap: 8px;
		align-items: baseline;
		padding: 4px 0;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 30%, transparent);
	}
	.rf-query-split li:first-child {
		border-top: none;
	}
	.rf-copy--inline {
		flex-shrink: 0;
	}
	.rf-filter {
		display: flex;
		gap: 8px;
		align-items: baseline;
		padding: 3px 0;
	}
	.rf-filter-idx {
		color: var(--base5);
		min-width: 22px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-filter-clauses {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 8px;
	}
	.rf-clause {
		font-family: var(--font-mono);
		color: var(--base7);
	}
	.rf-stage {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 6px 0;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 60%, transparent);
	}
	.rf-stage:first-child {
		border-top: none;
		padding-top: 0;
	}
	.rf-stage-head {
		display: flex;
		gap: 6px;
		align-items: baseline;
	}
	.rf-stage-num {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-stage-label {
		font-weight: 500;
		color: var(--base7);
	}
	.rf-stage-delay {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-phase {
		display: flex;
		flex-direction: column;
		gap: 2px;
		padding-left: 22px;
	}
	.rf-phase-label {
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-top: 4px;
	}
	.rf-steps-head {
		background: transparent;
		border: none;
		color: var(--id-yours);
		font: inherit;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 1px);
		cursor: pointer;
		padding: 0;
		display: inline-flex;
		align-items: center;
		gap: 4px;
	}
	.rf-steps-head:hover {
		color: var(--fg);
	}
	.rf-caret {
		min-width: 1ch;
	}
	.rf-steps {
		margin: 8px 0 0;
		padding-left: 22px;
		color: var(--base6);
		display: flex;
		flex-direction: column;
		gap: 3px;
	}
	.rf-list {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 22vh;
		overflow-y: auto;
	}
	.rf-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 3px 6px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.rf-row:hover {
		background: var(--bg-surface);
	}
	.rf-row input[type='checkbox'] {
		accent-color: var(--state-online);
	}
	.rf-url {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		background: transparent;
		color: var(--base6);
	}
	.rf-empty {
		margin: 0;
		color: var(--base5);
		font-style: italic;
	}
	.rf-empty--small {
		font-size: calc(var(--t-xs) - 1px);
		padding-left: 4px;
	}

	.rf-append {
		display: flex;
		gap: 6px;
		padding: 6px 14px 10px;
	}
	.rf-input {
		flex: 1;
		font: inherit;
		padding: 4px 8px;
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg);
		border-radius: var(--r-sm);
	}
	.rf-append-btn {
		font: inherit;
		padding: 4px 10px;
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base6);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	/* "General feed" toggle — styled like Add relay, sits beneath it, and
	   clicks green when on (confirming then also pulls a broad feed). */
	.rf-general {
		display: flex;
		justify-content: flex-end;
		padding: 0 14px 10px;
	}
	.rf-general-btn--on {
		background: rgba(180, 190, 130, 0.14);
		color: var(--state-online);
		border-color: color-mix(in srgb, var(--state-online) 50%, transparent);
	}
	.rf-general-btn--on:hover {
		background: rgba(180, 190, 130, 0.22);
	}
	/* Logged out → the toggle is forced on + disabled, but should still read as
	   on (green), not dimmed like a normal disabled button. */
	.rf-general-btn--on:disabled {
		opacity: 1;
		cursor: default;
	}
	.rf-append-btn:hover {
		border-color: var(--state-online);
		color: var(--state-online);
	}
	.rf-error {
		margin: 0 14px 8px;
		color: var(--id-draft);
		font-size: calc(var(--t-xs) - 1px);
	}

	.rf-footer {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.rf-action {
		font: inherit;
		padding: 5px 14px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.rf-action--ghost {
		color: var(--base5);
	}
	.rf-action--ghost:hover {
		color: var(--fg);
	}
	.rf-action--primary {
		border-color: var(--state-online);
		color: var(--state-online);
	}
	.rf-action--primary:hover:not(:disabled) {
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
	}
	.rf-action--primary:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
