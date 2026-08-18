<script lang="ts">
	// Shown when the engine (in Confirm mode) emits a `publish_intent`
	// that needs approval — the publish counterpart to FetchConfirmModal.
	// Every relay-writing path (/publish, /publish/blocks, /broadcast)
	// routes through `begin_publish_operation`, so wiring this one modal
	// gates them ALL.
	//
	// Per the design: we do NOT dump event JSON here (the user already
	// has that across other components). We render the *function* — what
	// is being replicated (index/section shape, kinds) — and the
	// *procedure* (broadcast to which relays), with the per-event list
	// one click away. The relay picker mirrors FetchConfirmModal so the
	// user can drop or add a relay for this one operation.

	import type { FetchEvent } from '$lib/types';
	import { resolveConfirm } from '$lib/network/fetch-events.svelte';
	import { addRelay, getRelayConfig } from '$lib/api';
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();

	type PublishIntentEvent = Extract<FetchEvent, { type: 'publish_intent' }>;
	let { intent }: { intent: PublishIntentEvent } = $props();

	const manifest = $derived(intent.manifest);

	// A publication carries kind-30040 indices and/or kind-30041
	// sections; anything else is a bare event broadcast.
	const isPublication = $derived(
		(manifest?.index_count ?? 0) > 0 || (manifest?.section_count ?? 0) > 0
	);

	function plural(n: number): string {
		return n === 1 ? '' : 's';
	}

	// The "function" line — what this publish replicates, in words.
	const functionLine = $derived.by(() => {
		if (!manifest) return intent.label;
		if (isPublication) {
			const parts: string[] = [];
			if (manifest.index_count > 0)
				parts.push(`${manifest.index_count} index${manifest.index_count === 1 ? '' : 'es'}`);
			if (manifest.section_count > 0)
				parts.push(`${manifest.section_count} section${plural(manifest.section_count)}`);
			const other = manifest.total - manifest.index_count - manifest.section_count;
			if (other > 0) parts.push(`${other} other`);
			const shape = parts.join(' + ');
			return `Replicate ${shape}${manifest.nested ? ' · nested tree' : ''}`;
		}
		return `Broadcast ${manifest.total} event${plural(manifest.total)}`;
	});

	const kindsLine = $derived(
		manifest?.kind_counts.length
			? 'kinds ' + manifest.kind_counts.map(([k]) => k).join(', ')
			: ''
	);

	// Relay selection — every proposed relay starts selected; the user
	// can drop any for this one operation or append an extra. Mirrors
	// FetchConfirmModal.
	let deselected = $state<Set<string>>(new Set());
	let extras = $state<string[]>([]);
	let appendInput = $state('');
	let appendError = $state<string | null>(null);
	let eventsOpen = $state(false);

	const allRelays = $derived([...intent.relays, ...extras]);

	// Parked (inactive) relays — offered unchecked, per-op opt-in only.
	// Mirrors FetchConfirmModal; persistence lives in relay management.
	let inactiveUrls = $state<string[]>([]);
	let optedIn = $state<Set<string>>(new Set());
	$effect(() => {
		getRelayConfig()
			.then((cfg) => {
				inactiveUrls = Object.keys(cfg.inactive ?? {});
			})
			.catch(() => {});
	});
	const inactiveOffered = $derived(inactiveUrls.filter((u) => !allRelays.includes(u)));
	function toggleOptIn(url: string) {
		const next = new Set(optedIn);
		if (!next.delete(url)) next.add(url);
		optedIn = next;
	}

	const selectedRelays = $derived([
		...allRelays.filter((r) => !deselected.has(r)),
		...inactiveOffered.filter((u) => optedIn.has(u))
	]);

	// The procedure — what happens on confirm, in order.
	const procedure = $derived.by(() => {
		const n = selectedRelays.length;
		const evs = manifest?.total ?? intent.event_ids.length;
		const steps = [
			`Broadcast ${evs} event${plural(evs)} → ${n} relay${plural(n)}`,
			'Record relay provenance for accepted events'
		];
		return steps;
	});

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
		// A publish confirm's target set is unambiguous, so "Add relay" is a
		// persistent edit to the publish set — mirroring the feed-sync
		// modal's fetch-set persistence. Without this, additions here were
		// one-shot and the next broadcast proposed the same old defaults.
		// (Unchecking a proposed relay stays per-operation on purpose.)
		addRelay('publish', v).catch(() => {});
		app.pushToast(`Saved ${v.replace(/^wss?:\/\//, '')} to your publish set`, 'success', 2500);
	}

	function copyOne(s: string) {
		navigator.clipboard?.writeText(s).catch(() => {});
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

	function entryTitle(e: { title?: string; d_tag?: string; event_id: string }): string {
		return e.title || e.d_tag || `${e.event_id.slice(0, 12)}…`;
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
				<span class="rf-pattern">{isPublication ? 'publication' : 'broadcast'}</span>
				{intent.label}
			</h3>
			<button class="rf-close" onclick={cancel} aria-label="Close">×</button>
		</header>

		<!-- Function: what's being replicated, in words (not JSON). -->
		<div class="rf-section">
			<div class="rf-section-head">What</div>
			<p class="rf-function">{functionLine}</p>
			{#if kindsLine}
				<p class="rf-kinds">{kindsLine}</p>
			{/if}
		</div>

		<!-- Procedure: the ordered steps that run on confirm. -->
		<div class="rf-section">
			<div class="rf-section-head">Procedure</div>
			<ol class="rf-steps">
				{#each procedure as step, i (i)}
					<li>{step}</li>
				{/each}
			</ol>
		</div>

		<!-- Relays: the actual decision — fire to these or not. -->
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
			{#if inactiveOffered.length > 0}
				<div class="rf-section-head rf-section-head--inactive">Inactive relays</div>
				<ul class="rf-list rf-list--inactive">
					{#each inactiveOffered as url (url)}
						<li class="rf-item">
							<label class="rf-row">
								<input
									type="checkbox"
									checked={optedIn.has(url)}
									onchange={() => toggleOptIn(url)}
								/>
								<code class="rf-url">{url}</code>
								<span class="rf-inactive-tag" title="Deactivated in relay management — check to include it for this broadcast only.">inactive</span>
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

		<!-- Per-event list — collapsed by default. The full JSON lives
		     elsewhere; this is just the roster of titles/ids being sent. -->
		{#if manifest?.entries?.length}
			<div class="rf-section">
				<button
					class="rf-steps-head"
					onclick={() => (eventsOpen = !eventsOpen)}
					aria-expanded={eventsOpen}
				>
					<span class="rf-caret">{eventsOpen ? '▾' : '▸'}</span>
					Events ({manifest.entries.length})
				</button>
				{#if eventsOpen}
					<ul class="rf-events">
						{#each manifest.entries as e (e.event_id)}
							<li class="rf-event">
								<span class="rf-event-kind">k:{e.kind}</span>
								<span class="rf-event-title" title={entryTitle(e)}>{entryTitle(e)}</span>
								<button
									class="rf-copy rf-copy--inline"
									onclick={() => copyOne(e.event_id)}
									title="Copy event id"
								>copy id</button>
							</li>
						{/each}
					</ul>
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
				{isPublication ? 'Publish' : 'Broadcast'} to {selectedRelays.length} relay{plural(
					selectedRelays.length
				)}
			</button>
		</footer>
	</div>
</div>

<style>
	.rf-backdrop {
		position: fixed;
		inset: 0;
		background: var(--scrim);
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
		max-height: 80dvh;
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
	.rf-section-head--inactive {
		margin-top: 10px;
		color: var(--base5);
	}
	.rf-list--inactive .rf-url {
		opacity: 0.6;
	}
	.rf-list--inactive input:checked ~ .rf-url {
		opacity: 1;
	}
	.rf-inactive-tag {
		font-size: calc(var(--t-xs) - 1px);
		color: var(--orange, #cb4b16);
		border: 1px solid currentColor;
		padding: 0 5px;
		border-radius: 2px;
		margin-left: 6px;
	}
	.rf-function {
		margin: 0;
		color: var(--base7);
		font-size: var(--t-sm);
	}
	.rf-kinds {
		margin: 4px 0 0;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-steps {
		margin: 0;
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
		max-height: 22dvh;
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
	.rf-events {
		list-style: none;
		margin: 8px 0 0;
		padding: 0;
		max-height: 28dvh;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.rf-event {
		display: flex;
		align-items: baseline;
		gap: 8px;
		padding: 3px 0;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 40%, transparent);
	}
	.rf-event:first-child {
		border-top: none;
	}
	.rf-event-kind {
		color: var(--base5);
		flex-shrink: 0;
		font-size: calc(var(--t-xs) - 1px);
	}
	.rf-event-title {
		flex: 1;
		color: var(--base7);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
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
	.rf-copy--inline {
		flex-shrink: 0;
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
