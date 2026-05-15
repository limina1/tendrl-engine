<script lang="ts">
	// Search-defaults editor. Opened by the gear button on the search
	// panel. Two modes — Form (collapsible widget sections) and Query
	// (the raw search-syntax equivalent) — are two views of the same
	// config. Kinds + Relays open by default; Author, Time window and
	// NIP-50 collapse so power users aren't slowed by knobs they rarely
	// touch, while still being one click (or one typed token) away.

	import * as api from '$lib/api';
	import {
		KNOWN_KINDS,
		searchConfig,
		searchConfigUI,
		closeSearchConfig,
		saveSearchConfig,
		resetSearchConfig,
		type AuthorMode
	} from '$lib/search/search-config.svelte';

	type Mode = 'form' | 'query';
	let mode = $state<Mode>('form');

	// Which sections are expanded. Kinds + Relays open; the rest fold.
	let open = $state<Record<string, boolean>>({
		kinds: true,
		author: false,
		time: false,
		relays: true,
		nip50: false
	});

	// Local draft — committed to `searchConfig` only on Save, so Cancel
	// leaves the live defaults untouched.
	let draftKinds = $state<number[]>([]);
	let draftLimit = $state<number>(searchConfig.limit);
	let draftRelays = $state<string[]>([]);
	let draftAddedRelays = $state<string[]>([]);
	let draftCustomKinds = $state<number[]>([]);
	let draftAuthor = $state<{ mode: AuthorMode; pubkey: string }>({ mode: 'me', pubkey: '' });
	let draftSince = $state<number | null>(null);
	let draftUntil = $state<number | null>(null);
	let draftNip50 = $state({ enabled: false, language: '', nsfw: true, includeSpam: false });

	let customKind = $state('');
	let queryText = $state('');
	let parseNote = $state<string | null>(null);
	let showNames = $state(true);
	let relayInput = $state('');
	let relayError = $state<string | null>(null);

	let configRelays = $state<string[]>([]);

	const knownKindNums = KNOWN_KINDS.map((k) => k.kind);

	// Re-seed the draft and reload the config relays each time it opens.
	$effect(() => {
		if (searchConfigUI.open) {
			draftKinds = [...searchConfig.kinds];
			draftLimit = searchConfig.limit;
			draftRelays = [...searchConfig.relays];
			draftAddedRelays = [...new Set([...searchConfig.addedRelays, ...searchConfig.relays])];
			draftCustomKinds = [
				...new Set([
					...searchConfig.customKinds,
					...searchConfig.kinds.filter((k) => !knownKindNums.includes(k))
				])
			].sort((a, b) => a - b);
			draftAuthor = { mode: searchConfig.author.mode, pubkey: searchConfig.author.pubkey };
			draftSince = searchConfig.since;
			draftUntil = searchConfig.until;
			draftNip50 = { ...searchConfig.nip50 };
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

	const relayOptions = $derived([
		...configRelays,
		...draftAddedRelays.filter((r) => !configRelays.includes(r))
	]);

	// ---- date <-> unix helpers (Time window) -------------------------
	function unixToDate(u: number | null): string {
		if (u == null) return '';
		return new Date(u * 1000).toISOString().slice(0, 10);
	}
	function dateToUnix(s: string, endOfDay: boolean): number | null {
		if (!s) return null;
		const ms = Date.parse(`${s}T${endOfDay ? '23:59:59' : '00:00:00'}Z`);
		return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
	}

	// ---- kinds -------------------------------------------------------
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
		if (!knownKindNums.includes(n) && !draftCustomKinds.includes(n)) {
			draftCustomKinds = [...draftCustomKinds, n].sort((a, b) => a - b);
		}
		if (!draftKinds.includes(n)) draftKinds = [...draftKinds, n].sort((a, b) => a - b);
		customKind = '';
		parseNote = null;
	}

	// ---- relays ------------------------------------------------------
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
		if (!draftAddedRelays.includes(v)) draftAddedRelays = [...draftAddedRelays, v];
		if (!draftRelays.includes(v)) draftRelays = [...draftRelays, v];
		relayInput = '';
		relayError = null;
	}

	// ---- form <-> query round-trip -----------------------------------
	// Query mode carries the dimensions that have a query token: kinds,
	// author, time. NIP-50 has no token (it's a relay mode), so it stays
	// a form widget regardless of mode.
	function syncQueryFromForm() {
		const parts = draftKinds.map((k) => `k:${k}`);
		if (draftAuthor.mode === 'me') parts.push('by:me');
		else if (draftAuthor.mode === 'pubkey' && draftAuthor.pubkey) {
			parts.push(`by:${draftAuthor.pubkey}`);
		}
		if (draftSince != null) parts.push(`since:${draftSince}`);
		if (draftUntil != null) parts.push(`until:${draftUntil}`);
		queryText = parts.join(' ');
	}

	function syncFormFromQuery() {
		const tokens = queryText.split(/\s+/).filter(Boolean);
		const kinds: number[] = [];
		const ignored: string[] = [];
		let author: { mode: AuthorMode; pubkey: string } = { mode: 'anyone', pubkey: '' };
		let since: number | null = null;
		let until: number | null = null;
		for (const t of tokens) {
			const km = t.match(/^k(?:ind)?:(\d+)$/i);
			const sm = t.match(/^since:(\d+)$/i);
			const um = t.match(/^until:(\d+)$/i);
			if (km) {
				const n = Number(km[1]);
				if (!kinds.includes(n)) kinds.push(n);
			} else if (sm) {
				since = Number(sm[1]);
			} else if (um) {
				until = Number(um[1]);
			} else if (/^by:/i.test(t)) {
				const v = t.slice(3);
				author = v.toLowerCase() === 'me' ? { mode: 'me', pubkey: '' } : { mode: 'pubkey', pubkey: v };
			} else {
				ignored.push(t);
			}
		}
		draftKinds = kinds.sort((a, b) => a - b);
		const newCustom = kinds.filter((k) => !knownKindNums.includes(k));
		draftCustomKinds = [...new Set([...draftCustomKinds, ...newCustom])].sort((a, b) => a - b);
		draftAuthor = author;
		draftSince = since;
		draftUntil = until;
		parseNote = ignored.length ? `Ignored (not a scope token): ${ignored.join(' ')}` : null;
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
		searchConfig.addedRelays = draftAddedRelays.filter((r) => !configRelays.includes(r));
		searchConfig.customKinds = [...draftCustomKinds];
		searchConfig.author = { mode: draftAuthor.mode, pubkey: draftAuthor.pubkey.trim() };
		searchConfig.since = draftSince;
		searchConfig.until = draftUntil;
		searchConfig.nip50 = { ...draftNip50, language: draftNip50.language.trim() };
		saveSearchConfig();
		closeSearchConfig();
	}

	function handleReset() {
		resetSearchConfig();
		draftKinds = [...searchConfig.kinds];
		draftLimit = searchConfig.limit;
		draftRelays = [...searchConfig.relays];
		draftAddedRelays = [...searchConfig.addedRelays];
		draftCustomKinds = [...searchConfig.customKinds];
		draftAuthor = { mode: searchConfig.author.mode, pubkey: searchConfig.author.pubkey };
		draftSince = searchConfig.since;
		draftUntil = searchConfig.until;
		draftNip50 = { ...searchConfig.nip50 };
		if (mode === 'query') syncQueryFromForm();
		parseNote = null;
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeSearchConfig();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet sectionHead(key: string, title: string, summary: string)}
	<button
		type="button"
		class="sc-sec-head"
		onclick={() => (open[key] = !open[key])}
		aria-expanded={open[key]}
	>
		<span class="sc-sec-arrow">{open[key] ? '▾' : '▸'}</span>
		<span class="sc-sec-title">{title}</span>
		<span class="sc-sec-summary">{summary}</span>
	</button>
{/snippet}

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
				token — including the offline <em>Search relays</em> fallback.
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

			<div class="sc-scroll">
				{#if mode === 'form'}
					<!-- Kinds -->
					<section class="sc-sec">
						{@render sectionHead(
							'kinds',
							'Kinds to search',
							`${draftKinds.length} selected`
						)}
						{#if open.kinds}
							<div class="sc-sec-body">
								<label class="sc-toggle sc-toggle--right">
									<input type="checkbox" bind:checked={showNames} />
									<span>names</span>
								</label>
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
											><span class="sc-pill-txt"
												>{showNames ? k.label : k.kind}</span
											></button
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
											onclick={() => toggleKind(k)}
											><span class="sc-pill-txt">{k}</span></button
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
						{/if}
					</section>

					<!-- Author scope -->
					<section class="sc-sec">
						{@render sectionHead(
							'author',
							'Author scope',
							draftAuthor.mode === 'me'
								? 'me'
								: draftAuthor.mode === 'anyone'
									? 'anyone'
									: 'specific'
						)}
						{#if open.author}
							<div class="sc-sec-body">
								<label class="sc-radio">
									<input type="radio" value="me" bind:group={draftAuthor.mode} />
									<span>Me — scope to my own events (<code>by:me</code>)</span>
								</label>
								<label class="sc-radio">
									<input type="radio" value="anyone" bind:group={draftAuthor.mode} />
									<span>Anyone — no author constraint</span>
								</label>
								<label class="sc-radio">
									<input type="radio" value="pubkey" bind:group={draftAuthor.mode} />
									<span>Specific — <code>by:&lt;npub&gt;</code></span>
								</label>
								{#if draftAuthor.mode === 'pubkey'}
									<input
										class="sc-input"
										placeholder="npub1… or 64-hex pubkey"
										bind:value={draftAuthor.pubkey}
									/>
								{/if}
							</div>
						{/if}
					</section>

					<!-- Time window -->
					<section class="sc-sec">
						{@render sectionHead(
							'time',
							'Time window',
							draftSince == null && draftUntil == null ? 'any time' : 'bounded'
						)}
						{#if open.time}
							<div class="sc-sec-body">
								<div class="sc-dates">
									<label class="sc-date">
										<span>From</span>
										<input
											type="date"
											class="sc-input"
											value={unixToDate(draftSince)}
											onchange={(e) =>
												(draftSince = dateToUnix(e.currentTarget.value, false))}
										/>
									</label>
									<label class="sc-date">
										<span>To</span>
										<input
											type="date"
											class="sc-input"
											value={unixToDate(draftUntil)}
											onchange={(e) =>
												(draftUntil = dateToUnix(e.currentTarget.value, true))}
										/>
									</label>
									<button
										class="sc-append-btn"
										onclick={() => {
											draftSince = null;
											draftUntil = null;
										}}>Clear</button
									>
								</div>
								<p class="sc-hint">
									NIP-01 <code>since</code> / <code>until</code> bounds. Empty =
									unbounded.
								</p>
							</div>
						{/if}
					</section>
				{:else}
					<section class="sc-sec sc-sec--open">
						<div class="sc-sec-body">
							<div class="sc-group-head"><span>Scope syntax</span></div>
							<textarea
								class="sc-query"
								rows="3"
								bind:value={queryText}
								spellcheck="false"
								placeholder="k:30040 k:30041 by:me since:1700000000"
							></textarea>
							<p class="sc-hint">
								<code>k:</code>/<code>kind:</code>, <code>by:</code>,
								<code>since:</code>, <code>until:</code> tokens are read here.
								Result limit and NIP-50 have their own controls.
							</p>
						</div>
					</section>
				{/if}

				<!-- Relays — mode-independent -->
				<section class="sc-sec">
					{@render sectionHead(
						'relays',
						'Relays to search from',
						draftRelays.length === 0 ? 'config default' : `${draftRelays.length} chosen`
					)}
					{#if open.relays}
						<div class="sc-sec-body">
							{#if relayOptions.length === 0}
								<p class="sc-empty">
									No relays in <code>[relay.fetch]</code> — add one below.
								</p>
							{:else}
								<!-- Click toggles; colour carries state — green when
								     on, purple for a user-added relay left off,
								     muted for an unselected config relay. -->
								<ul class="sc-relays">
									{#each relayOptions as url (url)}
										{@const on = draftRelays.includes(url)}
										{@const added = !configRelays.includes(url)}
										<li>
											<button
												type="button"
												class="sc-relay"
												class:sc-relay--on={on}
												class:sc-relay--added-off={added && !on}
												aria-pressed={on}
												onclick={() => toggleRelay(url)}>{url}</button
											>
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
								Leave all unchecked to fall back to the engine's configured fetch
								relays.
							</p>
						</div>
					{/if}
				</section>

				<!-- NIP-50 — mode-independent -->
				<section class="sc-sec">
					{@render sectionHead(
						'nip50',
						'NIP-50 relay search',
						draftNip50.enabled ? 'on' : 'off'
					)}
					{#if open.nip50}
						<div class="sc-sec-body">
							<label class="sc-radio">
								<input type="checkbox" bind:checked={draftNip50.enabled} />
								<span>Ask relays to full-text match the query (NIP-50)</span>
							</label>
							<label class="sc-field">
								<span>Language</span>
								<input
									class="sc-input sc-input--sm"
									placeholder="ISO 639-1, e.g. en"
									bind:value={draftNip50.language}
									disabled={!draftNip50.enabled}
								/>
							</label>
							<label class="sc-radio">
								<input
									type="checkbox"
									bind:checked={draftNip50.nsfw}
									disabled={!draftNip50.enabled}
								/>
								<span>Include NSFW results</span>
							</label>
							<label class="sc-radio">
								<input
									type="checkbox"
									bind:checked={draftNip50.includeSpam}
									disabled={!draftNip50.enabled}
								/>
								<span>Include spam (<code>include:spam</code>)</span>
							</label>
							<p class="sc-hint">
								Extensions are advisory — relays without NIP-50 support ignore
								them.
							</p>
						</div>
					{/if}
				</section>
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

	.sc-scroll {
		overflow-y: auto;
		border-top: 1px solid var(--panel-border);
	}

	/* Collapsible section */
	.sc-sec {
		border-bottom: 1px solid var(--panel-border);
	}
	.sc-sec-head {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 8px 14px;
		background: transparent;
		border: none;
		cursor: pointer;
		font: inherit;
		text-align: left;
		color: var(--id-yours);
	}
	.sc-sec-head:hover {
		background: var(--bg-surface);
	}
	.sc-sec-arrow {
		color: var(--base5);
		width: 10px;
		flex-shrink: 0;
	}
	.sc-sec-title {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 1px);
	}
	.sc-sec-summary {
		margin-left: auto;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.sc-sec-body {
		padding: 4px 14px 12px;
	}

	.sc-group-head {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin: 2px 0 6px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.sc-toggle {
		display: flex;
		align-items: center;
		gap: 4px;
		color: var(--base5);
		cursor: pointer;
	}
	.sc-toggle--right {
		justify-content: flex-end;
		margin-bottom: 6px;
	}
	.sc-toggle input {
		accent-color: var(--state-online);
	}

	/* Pill cloud — slanted parallelogram chips that pack horizontally.
	   The button is skewed; an inner span counter-skews so the label
	   itself stays upright. */
	.sc-cloud {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 5px;
		margin-bottom: 8px;
	}
	.sc-pill {
		font: inherit;
		padding: 2px 9px;
		border-radius: 2px;
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base5);
		cursor: pointer;
		transform: skewX(12deg);
	}
	.sc-pill-txt {
		display: inline-block;
		transform: skewX(-12deg);
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
	.sc-input:disabled {
		opacity: 0.45;
	}
	.sc-input--limit {
		flex: none;
		width: 72px;
	}
	.sc-input--sm {
		flex: none;
		width: 150px;
	}
	.sc-append-btn {
		font: inherit;
		padding: 4px 10px;
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base6);
		border-radius: var(--r-sm);
		cursor: pointer;
		white-space: nowrap;
	}
	.sc-append-btn:hover {
		border-color: var(--state-online);
		color: var(--state-online);
	}

	/* Author / NIP-50 rows */
	.sc-radio {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 3px 0;
		cursor: pointer;
		color: var(--fg);
	}
	.sc-radio input {
		accent-color: var(--state-online);
	}
	.sc-radio code,
	.sc-hint code,
	.sc-empty code {
		background: transparent;
		color: var(--id-yours);
	}
	.sc-field {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 0;
		color: var(--base5);
	}

	/* Time window */
	.sc-dates {
		display: flex;
		align-items: flex-end;
		gap: 8px;
		flex-wrap: wrap;
	}
	.sc-date {
		display: flex;
		flex-direction: column;
		gap: 3px;
		color: var(--base5);
	}
	.sc-date .sc-input {
		flex: none;
		width: 140px;
	}

	/* Relays — colour-coded toggle rows, no checkbox. */
	.sc-relays {
		list-style: none;
		margin: 0 0 6px;
		padding: 0;
		max-height: 20vh;
		overflow-y: auto;
	}
	.sc-relay {
		display: block;
		width: 100%;
		text-align: left;
		font: inherit;
		background: none;
		border: none;
		cursor: pointer;
		padding: 3px 6px;
		border-radius: var(--r-sm);
		color: var(--base5);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sc-relay:hover {
		background: var(--bg-surface);
	}
	/* on = green (any origin); user-added but off = purple. */
	.sc-relay--on {
		color: var(--state-online);
	}
	.sc-relay--added-off {
		color: var(--id-imported);
	}
	.sc-empty {
		margin: 0 0 6px;
		color: var(--base5);
		font-style: italic;
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
		margin: 6px 0 0;
		color: var(--base5);
		line-height: 1.5;
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
