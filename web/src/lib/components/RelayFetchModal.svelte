<script lang="ts">
	// Modal shown by fetchFromRelaysWithPrompt when offline (or when the
	// caller passes forcePrompt). Lets the user pick from the engine's
	// configured fetch relays + append session-only relays, then
	// confirms with the union as the relay set for this fetch.
	//
	// Session relays live in the relay-fetch module, not the modal's
	// local state. That way a user who appended `wss://relay.foo` once
	// sees it pre-checked the next time *any* fetch surface opens this
	// modal — they only have to type the URL once per browser session.

	import {
		type RelayFetchOpts,
		fetchModal,
		addSessionRelay,
		removeSessionRelay
	} from '$lib/fetch/relay-fetch.svelte';

	let {
		opts,
		configRelays,
		onconfirm,
		oncancel
	}: {
		opts: RelayFetchOpts;
		configRelays: string[];
		onconfirm: (relays: string[]) => void;
		oncancel: () => void;
	} = $props();

	// Configured relays + previously-added session relays default to
	// selected. The user can deselect any they don't want for this
	// single fetch (selection is per-fetch; presence in the list is
	// session-wide).
	let selected = $state<Set<string>>(
		new Set([...configRelays, ...fetchModal.sessionRelays])
	);
	let appendInput = $state('');
	let appendError = $state<string | null>(null);

	function toggle(url: string) {
		if (selected.has(url)) selected.delete(url);
		else selected.add(url);
		selected = new Set(selected);
	}

	function addExtra() {
		const v = appendInput.trim();
		if (!v) return;
		if (!/^wss?:\/\//i.test(v)) {
			appendError = 'Relay URL must start with ws:// or wss://';
			return;
		}
		if (
			configRelays.includes(v) ||
			fetchModal.sessionRelays.includes(v)
		) {
			appendError = 'Already in the list';
			return;
		}
		addSessionRelay(v);
		selected.add(v);
		selected = new Set(selected);
		appendInput = '';
		appendError = null;
	}

	function removeExtra(url: string) {
		removeSessionRelay(url);
		selected.delete(url);
		selected = new Set(selected);
	}

	function handleConfirm() {
		const list = [...selected];
		if (list.length === 0) return;
		onconfirm(list);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') oncancel();
		if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'INPUT') {
			handleConfirm();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="rf-backdrop" onclick={oncancel} role="presentation">
	<div class="rf-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
		<header class="rf-header">
			<h3 class="rf-title">{opts.title}</h3>
			<button class="rf-close" onclick={oncancel} aria-label="Close">×</button>
		</header>
		{#if opts.query || opts.kinds.length > 0 || opts.authors.length > 0 || opts.search || opts.limit}
			<div class="rf-meta">
				<div class="rf-meta-head">Searching for</div>
				{#if opts.query}
					<span class="rf-meta-row">
						<span class="rf-label">query</span>
						<code class="rf-query">{opts.query}</code>
					</span>
				{/if}
				{#if opts.kinds.length > 0}
					<span class="rf-meta-row">
						<span class="rf-label">kinds</span>
						<code>{opts.kinds.join(', ')}</code>
					</span>
				{/if}
				{#if opts.authors.length > 0}
					<span class="rf-meta-row">
						<span class="rf-label">authors</span>
						<code>{opts.authors.length} pubkey{opts.authors.length === 1 ? '' : 's'}</code>
					</span>
				{/if}
				{#if opts.search}
					<span class="rf-meta-row">
						<span class="rf-label">nip-50</span>
						<code>"{opts.search}"</code>
					</span>
				{/if}
				{#if opts.limit}
					<span class="rf-meta-row">
						<span class="rf-label">limit</span>
						<code>{opts.limit}</code>
					</span>
				{/if}
			</div>
		{/if}

		<div class="rf-section">
			<div class="rf-section-head">Configured relays</div>
			{#if configRelays.length === 0}
				<p class="rf-empty">No relays in [relay.fetch] config — add one below.</p>
			{:else}
				<ul class="rf-list">
					{#each configRelays as url (url)}
						<li>
							<label class="rf-row">
								<input
									type="checkbox"
									checked={selected.has(url)}
									onchange={() => toggle(url)}
								/>
								<code class="rf-url">{url}</code>
							</label>
						</li>
					{/each}
				</ul>
			{/if}
		</div>

		{#if fetchModal.sessionRelays.length > 0}
			<div class="rf-section">
				<div class="rf-section-head">This session</div>
				<ul class="rf-list">
					{#each fetchModal.sessionRelays as url (url)}
						<li>
							<label class="rf-row">
								<input
									type="checkbox"
									checked={selected.has(url)}
									onchange={() => toggle(url)}
								/>
								<code class="rf-url">{url}</code>
								<button
									class="rf-row-remove"
									onclick={(e) => {
										e.preventDefault();
										removeExtra(url);
									}}
									title="Drop this relay from the session"
								>×</button>
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

		<footer class="rf-footer">
			<button class="rf-action rf-action--ghost" onclick={oncancel}>Cancel</button>
			<button
				class="rf-action rf-action--primary"
				onclick={handleConfirm}
				disabled={selected.size === 0}
			>
				Fetch from {selected.size} relay{selected.size === 1 ? '' : 's'}
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
		max-width: 540px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
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

	.rf-meta {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 8px 14px;
		border-bottom: 1px solid var(--panel-border);
		color: var(--base5);
	}
	.rf-meta-head {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		font-size: calc(var(--t-xs) - 1px);
		margin-bottom: 2px;
	}
	.rf-meta-row {
		display: flex;
		gap: 8px;
		align-items: baseline;
	}
	.rf-label {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		min-width: 56px;
		flex-shrink: 0;
	}
	.rf-query {
		white-space: pre-wrap;
		word-break: break-word;
		color: var(--fg);
	}
	.rf-meta code,
	.rf-url {
		background: transparent;
		color: var(--base6);
	}

	.rf-section {
		padding: 8px 14px;
	}
	.rf-section-head {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin-bottom: 6px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-list {
		list-style: none;
		margin: 0;
		padding: 0;
		max-height: 24vh;
		overflow-y: auto;
	}
	.rf-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 6px;
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
	}
	.rf-row-remove {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
		line-height: 1;
	}
	.rf-row-remove:hover {
		color: var(--id-draft);
	}
	.rf-empty {
		margin: 0;
		color: var(--base5);
		font-style: italic;
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
