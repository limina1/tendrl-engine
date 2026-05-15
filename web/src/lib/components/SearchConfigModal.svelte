<script lang="ts">
	// Search-defaults editor. Opened by the gear button on the search
	// panel. Two modes — Form (a checkbox board) and Query (the raw
	// search-syntax equivalent) — are two views of the same config:
	// `{ kinds, limit, relays }`. The kinds chosen here scope every
	// search that doesn't write its own `k:` token; the relays are what
	// the offline "Search relays" fallback queries.

	import * as api from '$lib/api';
	import {
		KNOWN_KINDS,
		searchConfig,
		searchConfigUI,
		closeSearchConfig,
		saveSearchConfig,
		resetSearchConfig
	} from '$lib/search/search-config.svelte';

	type Mode = 'form' | 'query';
	let mode = $state<Mode>('form');

	// Local draft — committed to `searchConfig` only on Save, so Cancel
	// leaves the live defaults untouched.
	let draftKinds = $state<number[]>([...searchConfig.kinds]);
	let draftLimit = $state<number>(searchConfig.limit);
	let draftRelays = $state<string[]>([...searchConfig.relays]);
	let customKind = $state('');
	let queryText = $state('');
	let parseNote = $state<string | null>(null);
	let relayInput = $state('');
	let relayError = $state<string | null>(null);
	// Known-kind pills toggle between their number and their functional
	// name. Default to names — the whole point of the table is that a
	// reader shouldn't have to memorise kind numbers.
	let showNames = $state(true);
	// Custom kinds the user added. Tracked separately from `draftKinds`
	// so one toggled *off* still renders (purple-outlined) instead of
	// disappearing the moment it leaves the selected set.
	let draftCustomKinds = $state<number[]>([]);

	// The engine's configured [relay.fetch] relays — the pickable set.
	let configRelays = $state<string[]>([]);

	const knownKindNums = KNOWN_KINDS.map((k) => k.kind);

	// Re-seed the draft and reload the config relays each time the modal
	// opens.
	$effect(() => {
		if (searchConfigUI.open) {
			draftKinds = [...searchConfig.kinds];
			draftLimit = searchConfig.limit;
			draftRelays = [...searchConfig.relays];
			// Custom registry ∪ any selected kind that isn't a known one
			// (covers configs saved before the registry existed).
			draftCustomKinds = [
				...new Set([
					...searchConfig.customKinds,
					...searchConfig.kinds.filter((k) => !knownKindNums.includes(k))
				])
			].sort((a, b) => a - b);
			customKind = '';
			relayInput = '';
			relayError = null;
			parseNote = null;
			showNames = true;
			mode = 'form';
			loadConfigRelays();
		}
	});

	async function loadConfigRelays() {
		try {
			const cfg = await api.getRelayConfig();
			configRelays = cfg.fetch.urls;
		} catch {
			configRelays = [];
		}
	}

	// Relays to list: configured fetch relays + any the user typed in.
	const relayOptions = $derived([
		...configRelays,
		...draftRelays.filter((r) => !configRelays.includes(r))
	]);

	function toggleKind(kind: number) {
		if (draftKinds.includes(kind)) draftKinds = draftKinds.filter((k) => k !== kind);
		else draftKinds = [...draftKinds, kind].sort((a, b) => a - b);
	}

	function addCustomKind() {
		const n = Number(customKind.trim());
		if (!Number.isInteger(n) || n < 0) {
			parseNote = 'Kind must be a non-negative integer.';
			return;
		}
		// A known kind typed here is just selected, not made "custom".
		if (!knownKindNums.includes(n) && !draftCustomKinds.includes(n)) {
			draftCustomKinds = [...draftCustomKinds, n].sort((a, b) => a - b);
		}
		if (!draftKinds.includes(n)) draftKinds = [...draftKinds, n].sort((a, b) => a - b);
		customKind = '';
		parseNote = null;
	}

	function toggleRelay(url: string) {
		if (draftRelays.includes(url)) draftRelays = draftRelays.filter((r) => r !== url);
		else draftRelays = [...draftRelays, url];
	}

	function addRelay() {
		const v = relayInput.trim();
		if (!v) return;
		if (!/^wss?:\/\//i.test(v)) {
			relayError = 'Relay URL must start with ws:// or wss://';
			return;
		}
		if (!draftRelays.includes(v)) draftRelays = [...draftRelays, v];
		relayInput = '';
		relayError = null;
	}

	// Form → Query: render the chosen kinds as the `k:` tokens an engine
	// query would carry. Limit lives in its own field, not the textarea.
	function syncQueryFromForm() {
		queryText = draftKinds.map((k) => `k:${k}`).join(' ');
	}

	// Query → Form: pull `k:`/`kind:` tokens back out. Anything else is
	// ignored (with a note) — this editor sets the default kind scope.
	function syncFormFromQuery() {
		const tokens = queryText.split(/\s+/).filter(Boolean);
		const kinds: number[] = [];
		const ignored: string[] = [];
		for (const t of tokens) {
			const km = t.match(/^k(?:ind)?:(\d+)$/i);
			if (km) {
				const n = Number(km[1]);
				if (!kinds.includes(n)) kinds.push(n);
			} else {
				ignored.push(t);
			}
		}
		draftKinds = kinds.sort((a, b) => a - b);
		// Register any non-standard kind so it keeps a board pill.
		const newCustom = kinds.filter((k) => !knownKindNums.includes(k));
		draftCustomKinds = [...new Set([...draftCustomKinds, ...newCustom])].sort((a, b) => a - b);
		parseNote = ignored.length
			? `Ignored (not a kind token): ${ignored.join(' ')}`
			: null;
	}

	function switchMode(next: Mode) {
		if (next === mode) return;
		if (next === 'query') syncQueryFromForm();
		else syncFormFromQuery();
		mode = next;
	}

	function handleSave() {
		if (mode === 'query') syncFormFromQuery();
		searchConfig.kinds = [...draftKinds];
		searchConfig.limit = Math.max(1, Math.min(1000, Math.floor(draftLimit) || 1));
		searchConfig.relays = [...draftRelays];
		searchConfig.customKinds = [...draftCustomKinds];
		saveSearchConfig();
		closeSearchConfig();
	}

	function handleReset() {
		resetSearchConfig();
		draftKinds = [...searchConfig.kinds];
		draftLimit = searchConfig.limit;
		draftRelays = [...searchConfig.relays];
		draftCustomKinds = [...searchConfig.customKinds];
		if (mode === 'query') syncQueryFromForm();
		parseNote = null;
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeSearchConfig();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#if searchConfigUI.open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="sc-backdrop" onclick={closeSearchConfig} role="presentation">
		<div
			class="sc-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			tabindex="-1"
		>
			<header class="sc-header">
				<h3 class="sc-title">Search defaults</h3>
				<button class="sc-close" onclick={closeSearchConfig} aria-label="Close">×</button>
			</header>

			<p class="sc-blurb">
				These defaults scope every search that doesn't write its own
				<code>k:</code> filter — including the offline
				<em>Search relays</em> fallback.
			</p>

			<div class="sc-tabs">
				<button
					class="sc-tab"
					class:sc-tab--active={mode === 'form'}
					onclick={() => switchMode('form')}>Form</button
				>
				<button
					class="sc-tab"
					class:sc-tab--active={mode === 'query'}
					onclick={() => switchMode('query')}>Query</button
				>
				<span class="sc-tabs-spacer"></span>
				<label class="sc-limit">
					<span class="sc-limit-label">Result limit</span>
					<input
						class="sc-input sc-input--limit"
						type="number"
						min="1"
						max="1000"
						bind:value={draftLimit}
					/>
				</label>
			</div>

			{#if mode === 'form'}
				<div class="sc-body">
					<div class="sc-group-head">
						<span>Kinds to search</span>
						<label class="sc-toggle">
							<input type="checkbox" bind:checked={showNames} />
							<span>names</span>
						</label>
					</div>
					<!-- Wrapping pill cloud. Known kinds show their functional
					     name or number (the `names` toggle); the other form
					     plus the spec live in the hover tooltip. Custom kinds
					     stay numeric and outline purple while excluded. -->
					<div class="sc-cloud">
						{#each KNOWN_KINDS as k (k.kind)}
							<button
								type="button"
								class="sc-pill"
								class:sc-pill--on={draftKinds.includes(k.kind)}
								aria-pressed={draftKinds.includes(k.kind)}
								title={showNames
									? `kind ${k.kind} · ${k.note}`
									: `${k.label} · ${k.note}`}
								onclick={() => toggleKind(k.kind)}
								>{showNames ? k.label : k.kind}</button
							>
						{/each}
						{#each draftCustomKinds as k (k)}
							<button
								type="button"
								class="sc-pill"
								class:sc-pill--on={draftKinds.includes(k)}
								class:sc-pill--custom-off={!draftKinds.includes(k)}
								aria-pressed={draftKinds.includes(k)}
								title="custom kind {k}"
								onclick={() => toggleKind(k)}>{k}</button
							>
						{/each}
					</div>

					<div class="sc-append">
						<input
							class="sc-input"
							placeholder="custom kind, e.g. 30817"
							bind:value={customKind}
							onkeydown={(e) => {
								if (e.key === 'Enter') {
									e.preventDefault();
									addCustomKind();
								}
							}}
						/>
						<button class="sc-append-btn" onclick={addCustomKind}>Add kind</button>
					</div>
				</div>
			{:else}
				<div class="sc-body">
					<div class="sc-group-head"><span>Kind-scope syntax</span></div>
					<textarea
						class="sc-query"
						rows="2"
						bind:value={queryText}
						spellcheck="false"
						placeholder="k:30040 k:30041 k:30818 k:30023"
					></textarea>
					<p class="sc-hint">
						Only <code>k:</code>/<code>kind:</code> tokens are read here.
						The result limit has its own field above.
					</p>
				</div>
			{/if}

			<div class="sc-body sc-body--relays">
				<div class="sc-group-head"><span>Relays to search from</span></div>
				{#if relayOptions.length === 0}
					<p class="sc-empty">No relays in <code>[relay.fetch]</code> — add one below.</p>
				{:else}
					<ul class="sc-relays">
						{#each relayOptions as url (url)}
							<li>
								<label class="sc-relay-row">
									<input
										type="checkbox"
										checked={draftRelays.includes(url)}
										onchange={() => toggleRelay(url)}
									/>
									<code class="sc-relay-url">{url}</code>
								</label>
							</li>
						{/each}
					</ul>
				{/if}
				<div class="sc-append">
					<input
						class="sc-input"
						placeholder="wss://relay.example.com"
						bind:value={relayInput}
						onkeydown={(e) => {
							if (e.key === 'Enter') {
								e.preventDefault();
								addRelay();
							}
						}}
					/>
					<button class="sc-append-btn" onclick={addRelay}>Add relay</button>
				</div>
				{#if relayError}
					<p class="sc-note">{relayError}</p>
				{/if}
				<p class="sc-hint">
					Leave all unchecked to fall back to the engine's configured
					fetch relays.
				</p>
			</div>

			{#if parseNote}
				<p class="sc-note">{parseNote}</p>
			{/if}

			<footer class="sc-footer">
				<button class="sc-action sc-action--ghost" onclick={handleReset}>Reset</button>
				<span class="sc-spacer"></span>
				<button class="sc-action sc-action--ghost" onclick={closeSearchConfig}>Cancel</button>
				<button
					class="sc-action sc-action--primary"
					onclick={handleSave}
					disabled={draftKinds.length === 0}
					title={draftKinds.length === 0 ? 'Pick at least one kind' : ''}
				>
					Save defaults
				</button>
			</footer>
		</div>
	</div>
{/if}

<style>
	.sc-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.55);
		z-index: 250;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.sc-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 90vw;
		max-width: 540px;
		max-height: 88vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.sc-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.sc-title {
		margin: 0;
		font-size: var(--t-sm);
		color: var(--base7);
	}
	.sc-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
	}
	.sc-close:hover {
		color: var(--fg);
	}
	.sc-blurb {
		margin: 0;
		padding: 8px 14px;
		color: var(--base5);
		line-height: 1.5;
		border-bottom: 1px solid var(--panel-border);
	}
	.sc-blurb code {
		background: transparent;
		color: var(--id-yours);
	}

	.sc-tabs {
		display: flex;
		align-items: center;
		padding: 8px 14px 0;
		gap: 4px;
	}
	.sc-tab {
		font: inherit;
		padding: 4px 14px;
		background: transparent;
		border: 1px solid var(--panel-border);
		border-bottom: none;
		border-radius: var(--r-sm) var(--r-sm) 0 0;
		color: var(--base5);
		cursor: pointer;
	}
	.sc-tab--active {
		color: var(--id-yours);
		border-color: var(--id-yours);
		background: var(--bg-surface);
	}
	.sc-tabs-spacer {
		flex: 1;
	}
	.sc-limit {
		display: flex;
		align-items: center;
		gap: 6px;
		padding-bottom: 4px;
	}
	.sc-limit-label {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}

	.sc-body {
		padding: 10px 14px;
		overflow-y: auto;
	}
	.sc-body--relays {
		border-top: 1px solid var(--panel-border);
		padding-top: 8px;
	}
	.sc-group-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin: 4px 0 6px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.sc-toggle {
		display: flex;
		align-items: center;
		gap: 4px;
		text-transform: none;
		letter-spacing: 0;
		color: var(--base5);
		cursor: pointer;
	}
	.sc-toggle input {
		accent-color: var(--state-online);
	}
	/* Wrapping pill cloud — one toggle button per kind. */
	.sc-cloud {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin-bottom: 8px;
	}
	.sc-pill {
		font: inherit;
		padding: 4px 12px;
		border-radius: 999px;
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base5);
		cursor: pointer;
	}
	.sc-pill:hover {
		border-color: var(--state-online);
		color: var(--fg);
	}
	.sc-pill--on {
		border-color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
		color: var(--state-online);
	}
	/* Excluded custom kind — purple outline marks it as user-added and
	   currently out of scope (vs the plain grey of an off known kind). */
	.sc-pill--custom-off {
		border-color: var(--id-imported);
		color: var(--id-imported);
	}
	.sc-pill--custom-off:hover {
		border-color: var(--id-imported);
		color: var(--id-imported);
	}

	.sc-append {
		display: flex;
		gap: 6px;
		margin-bottom: 4px;
	}
	.sc-input {
		flex: 1;
		font: inherit;
		padding: 4px 8px;
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg);
		border-radius: var(--r-sm);
	}
	.sc-input--limit {
		flex: none;
		width: 72px;
	}
	.sc-append-btn {
		font: inherit;
		padding: 4px 10px;
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base6);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.sc-append-btn:hover {
		border-color: var(--state-online);
		color: var(--state-online);
	}

	.sc-relays {
		list-style: none;
		margin: 0 0 6px;
		padding: 0;
		max-height: 20vh;
		overflow-y: auto;
	}
	.sc-relay-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 3px 6px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.sc-relay-row:hover {
		background: var(--bg-surface);
	}
	.sc-relay-row input[type='checkbox'] {
		accent-color: var(--state-online);
	}
	.sc-relay-url {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		background: transparent;
		color: var(--base6);
	}
	.sc-empty {
		margin: 0 0 6px;
		color: var(--base5);
		font-style: italic;
	}
	.sc-empty code {
		background: transparent;
		color: var(--id-yours);
	}

	.sc-query {
		width: 100%;
		font: inherit;
		padding: 6px 8px;
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg);
		border-radius: var(--r-sm);
		resize: vertical;
		box-sizing: border-box;
	}
	.sc-hint {
		margin: 4px 0 0;
		color: var(--base5);
		line-height: 1.5;
	}
	.sc-hint code {
		background: transparent;
		color: var(--id-yours);
	}
	.sc-note {
		margin: 6px 14px 4px;
		color: var(--id-draft);
		line-height: 1.5;
	}

	.sc-footer {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.sc-spacer {
		flex: 1;
	}
	.sc-action {
		font: inherit;
		padding: 5px 14px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.sc-action--ghost {
		color: var(--base5);
	}
	.sc-action--ghost:hover {
		color: var(--fg);
	}
	.sc-action--primary {
		border-color: var(--state-online);
		color: var(--state-online);
	}
	.sc-action--primary:hover:not(:disabled) {
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
	}
	.sc-action--primary:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
