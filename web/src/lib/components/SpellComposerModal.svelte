<script lang="ts">
	// Save a search as a kind-777 spell. The engine compiles the query
	// (POST /api/v1/spell/compose) — the preview below IS that compile:
	// clause block + degradation warnings. Saving signs via the generic
	// identity path and ingests locally; broadcast is the deliberate
	// second step (signing-is-the-snapshot, publish philosophy).
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import SpellClauseBlock from './SpellClauseBlock.svelte';

	let { query, onclose }: { query: string; onclose: () => void } = $props();

	const app = getAppState();

	let name = $state('');
	let description = $state('');
	let topics = $state('');
	// Parameterization rows: promote a literal in the query to $arg.<name>.
	let params = $state<{ name: string; prompt: string; value: string }[]>([]);

	let preview = $state<api.SpellComposeResponse | null>(null);
	let composeError = $state<string | null>(null);
	let saving = $state(false);

	function composeRequest() {
		return {
			query,
			name: name.trim() || undefined,
			description: description.trim() || undefined,
			topics: topics
				.split(',')
				.map((t) => t.trim())
				.filter(Boolean),
			params: params
				.filter((p) => p.name.trim() && p.value.trim())
				.map((p) => ({
					name: p.name.trim(),
					prompt: p.prompt.trim() || undefined,
					value: p.value.trim()
				}))
		};
	}

	// Debounced live compile: any input change re-previews after a beat.
	$effect(() => {
		void name; void description; void topics;
		void params.map((p) => p.name + p.prompt + p.value).join();
		const t = setTimeout(() => {
			api.composeSpell(composeRequest())
				.then((r) => { preview = r; composeError = null; })
				.catch((e) => { preview = null; composeError = api.errorMessage(e); });
		}, 250);
		return () => clearTimeout(t);
	});

	async function save(broadcast: boolean) {
		if (!preview || saving) return;
		saving = true;
		try {
			// Recompose once more so the template reflects the latest inputs
			// (the debounce may lag a final keystroke).
			const composed = await api.composeSpell(composeRequest());
			const { signed_event } = await api.signTemplate({ template: composed.template });
			await api.ingestEvent(signed_event);
			if (broadcast) {
				const res = await api.broadcastEvent({ event: signed_event });
				app.pushToast(
					`Spell saved — accepted by ${res.successful}/${res.total} relays`,
					res.successful > 0 ? 'success' : 'error'
				);
			} else {
				app.pushToast('Spell saved locally — broadcast when ready', 'success');
			}
			onclose();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Save failed'), 'error');
		} finally {
			saving = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') onclose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="sc-backdrop" onclick={onclose} role="presentation">
	<div
		class="sc-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		tabindex="-1"
	>
		<header class="sc-header">
			<h3 class="sc-title">Save as spell</h3>
			<code class="sc-query">{query}</code>
			<button class="sc-close" onclick={onclose} aria-label="Close">×</button>
		</header>

		<div class="sc-body">
			<label class="sc-field">
				<span>Name</span>
				<input bind:value={name} placeholder="Filed under #&#123;tag&#125;" />
			</label>
			<label class="sc-field">
				<span>Description</span>
				<input bind:value={description} placeholder="What this query collects (event content)" />
			</label>
			<label class="sc-field">
				<span>Topics — label the spell itself, not a filter</span>
				<input
					bind:value={topics}
					placeholder="comma,separated (to match #hashtags in events, use t: in the query)"
				/>
			</label>

			<div class="sc-params">
				<div class="sc-params-head">
					<span>Parameters</span>
					<button
						class="sc-add"
						onclick={() => (params = [...params, { name: '', prompt: '', value: '' }])}
					>+ promote a literal to $arg</button>
				</div>
				{#each params as p, i (i)}
					<div class="sc-param-row">
						<input class="sc-param-name" bind:value={p.name} placeholder="name" />
						<input class="sc-param-value" bind:value={p.value} placeholder="literal in query" />
						<input class="sc-param-prompt" bind:value={p.prompt} placeholder="prompt shown at run time" />
						<button
							class="sc-remove"
							onclick={() => (params = params.filter((_, j) => j !== i))}
							aria-label="Remove parameter"
						>×</button>
					</div>
				{/each}
			</div>

			<div class="sc-preview">
				<span class="sc-preview-head">Preview</span>
				{#if composeError}
					<p class="sc-error">{composeError}</p>
				{:else if preview}
					<SpellClauseBlock clauses={preview.clauses} />
					{#each preview.warnings as w (w)}
						<p class="sc-warning">⚠ {w}</p>
					{/each}
				{:else}
					<p class="sc-muted">compiling…</p>
				{/if}
			</div>
		</div>

		<footer class="sc-footer">
			<button class="sc-save" disabled={!preview || saving} onclick={() => save(false)}>
				{saving ? 'Saving…' : 'Save locally'}
			</button>
			<button class="sc-save sc-save--broadcast" disabled={!preview || saving} onclick={() => save(true)}>
				Save + broadcast
			</button>
		</footer>
	</div>
</div>

<style>
	.sc-backdrop {
		position: fixed;
		inset: 0;
		background: color-mix(in srgb, black 55%, transparent);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 60;
	}
	.sc-modal {
		width: min(560px, 92vw);
		max-height: 85vh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		display: flex;
		flex-direction: column;
	}
	.sc-header {
		display: flex;
		align-items: baseline;
		gap: 10px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}
	.sc-title {
		margin: 0;
		font-size: var(--t-sm);
	}
	.sc-query {
		flex: 1;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sc-close {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: var(--t-md);
		cursor: pointer;
	}
	.sc-body {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 12px 14px;
	}
	.sc-field {
		display: flex;
		flex-direction: column;
		gap: 2px;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sc-field input,
	.sc-param-row input {
		background: var(--bg-alt, transparent);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg);
		font-size: var(--t-xs);
		padding: 4px 8px;
	}
	.sc-params-head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sc-add,
	.sc-remove {
		background: none;
		border: none;
		color: var(--accent);
		font-size: var(--t-2xs);
		cursor: pointer;
	}
	.sc-param-row {
		display: flex;
		gap: 6px;
		margin-top: 4px;
	}
	.sc-param-name { width: 90px; }
	.sc-param-value { width: 140px; }
	.sc-param-prompt { flex: 1; }
	.sc-preview {
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 8px 10px;
	}
	.sc-preview-head {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.sc-warning {
		margin: 4px 0 0;
		font-size: var(--t-2xs);
		color: var(--warning, #c90);
	}
	.sc-error {
		margin: 4px 0 0;
		font-size: var(--t-xs);
		color: var(--error, #c55);
	}
	.sc-muted {
		margin: 4px 0 0;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sc-footer {
		display: flex;
		gap: 8px;
		justify-content: flex-end;
		padding: 10px 14px;
		border-top: 1px solid var(--border);
	}
	.sc-save {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 14px;
		background: transparent;
		border: 1px solid var(--accent);
		border-radius: var(--radius);
		color: var(--accent);
		cursor: pointer;
	}
	.sc-save--broadcast {
		background: var(--accent);
		color: var(--bg);
	}
	.sc-save:disabled {
		opacity: 0.5;
		cursor: default;
	}
</style>
