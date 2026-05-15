<script lang="ts">
	// Shown when the engine (in Confirm mode) emits a fetch `intent` that
	// needs approval. Renders exactly what the engine reported it will
	// request — the label, the step sequence, and the relay set — and
	// posts the user's decision back via resolveConfirm.

	import type { FetchEvent } from '$lib/types';
	import { resolveConfirm } from '$lib/network/fetch-events.svelte';

	type IntentEvent = Extract<FetchEvent, { type: 'intent' }>;
	let { intent }: { intent: IntentEvent } = $props();

	// Every proposed relay starts selected; the user can drop any for
	// this one operation, or append an extra. We track *deselected* so no
	// $state has to snapshot the `intent` prop.
	let deselected = $state<Set<string>>(new Set());
	let extras = $state<string[]>([]);
	let appendInput = $state('');
	let appendError = $state<string | null>(null);
	let stepsOpen = $state(false);

	const allRelays = $derived([...intent.relays, ...extras]);
	const selectedRelays = $derived(allRelays.filter((r) => !deselected.has(r)));

	const PATTERN_LABEL: Record<string, string> = {
		event: 'event',
		publication: 'publication',
		thread: 'thread',
		search: 'search',
		profile: 'profile',
		custom: 'fetch'
	};

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

		{#if intent.steps.length > 0}
			<div class="rf-section">
				<button class="rf-steps-head" onclick={() => (stepsOpen = !stepsOpen)} aria-expanded={stepsOpen}>
					<span class="rf-caret">{stepsOpen ? '▾' : '▸'}</span>
					What this fetches ({intent.steps.length} step{intent.steps.length === 1 ? '' : 's'})
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
		{#if appendError}
			<p class="rf-error">{appendError}</p>
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
		max-width: 540px;
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
		max-height: 28vh;
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
		background: transparent;
		color: var(--base6);
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
