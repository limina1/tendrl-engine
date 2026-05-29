<script lang="ts">
	// Shown when the engine (in Confirm mode) emits a fetch `intent` that
	// needs approval. Phase 6 wiring: when the intent carries a
	// structured `summary` (RequestSummary), render the canonical DSL
	// sentence, the filter clauses, and the per-phase composition so the
	// user sees EXACTLY what's about to be requested (and from where)
	// rather than a flat relay list. Falls back to the legacy flat view
	// when summary is absent (older engine intents).

	import type { FetchEvent, Phase } from '$lib/types';
	import { resolveConfirm } from '$lib/network/fetch-events.svelte';

	type IntentEvent = Extract<FetchEvent, { type: 'intent' }>;
	let { intent }: { intent: IntentEvent } = $props();

	// Every proposed relay starts selected; the user can drop any for
	// this one operation, or append an extra.
	let deselected = $state<Set<string>>(new Set());
	let extras = $state<string[]>([]);
	let appendInput = $state('');
	let appendError = $state<string | null>(null);
	let stepsOpen = $state(false);

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
		if (!intent.summary?.dsl) return;
		navigator.clipboard?.writeText(intent.summary.dsl).catch(() => {});
	}

	function confirm() {
		if (selectedRelays.length === 0) return;
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

		<!-- DSL sentence — the canonical formal-language form. Always
		     shown when the intent carries a summary. Click → copy. -->
		{#if intent.summary?.dsl}
			<div class="rf-section">
				<div class="rf-section-head-row">
					<span class="rf-section-head">Query</span>
					<button class="rf-copy" onclick={copyDsl} title="Copy DSL sentence">copy</button>
				</div>
				<code class="rf-dsl">{intent.summary.dsl}</code>
			</div>
		{/if}

		<!-- Filters — one block per NIP-01 filter, clauses inline. -->
		{#if intent.summary?.filters?.length}
			<div class="rf-section">
				<div class="rf-section-head">Filters</div>
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

		<!-- Composition — per-stage, per-phase, per-relay. Same checkbox
		     interaction as before but grouped so the user can see which
		     relay belongs to which phase of the fan-out. -->
		{#if intent.summary?.composition?.phases?.length}
			<div class="rf-section">
				<div class="rf-section-head">Composition</div>
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
								<div class="rf-phase-label">{relayLabel(phase)}</div>
								{#if urls.length === 0}
									<p class="rf-empty rf-empty--small">No relays in this class.</p>
								{:else}
									<ul class="rf-list">
										{#each urls as url (url)}
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
						{/each}
					</div>
				{/each}
			</div>
		{:else}
			<!-- Legacy fallback: flat relay list when no summary. -->
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
		{/if}

		<!-- Per-user extras — additional relays to fetch from beyond
		     what the engine proposed. Always available. -->
		{#if extras.length > 0}
			<div class="rf-section">
				<div class="rf-section-head">Extras</div>
				<ul class="rf-list">
					{#each extras as url (url)}
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
			</div>
		{/if}

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
		{#if appendError}
			<p class="rf-error">{appendError}</p>
		{/if}

		{#if intent.steps.length > 0}
			<div class="rf-section">
				<button class="rf-steps-head" onclick={() => (stepsOpen = !stepsOpen)} aria-expanded={stepsOpen}>
					<span class="rf-caret">{stepsOpen ? '▾' : '▸'}</span>
					Steps ({intent.steps.length})
				</button>
				{#if stepsOpen}
					<ol class="rf-steps">
						{#each intent.steps as step, i (i)}
							<li>{step}</li>
						{/each}
					</ol>
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
