<script lang="ts">
	// Embedding-index controls: index status, the auto-embed toggle,
	// which kinds get embedded (canonical menu + custom kinds), and the
	// embed-missing / full-reembed actions. Shared between the search panel's
	// bottom "Embedding settings" bar and the search-config modal's "Embedding"
	// section so the two never drift. Presentational — status and callbacks are
	// passed in by the host (both pull from app state).
	//
	// Unlike the modal's search defaults (committed on Save), these act
	// immediately: kinds and the auto-embed flag persist engine-side the moment
	// they change, and the buttons kick off engine passes right away.

	import type { EmbeddingStatusResponse } from '$lib/types';
	import { kindLabel } from '$lib/search/search-config.svelte';

	let {
		status = null,
		syncing = false,
		onembedmissing,
		onembedreindex,
		onsetembedkinds,
		onsetautoembed
	}: {
		status?: EmbeddingStatusResponse | null;
		syncing?: boolean;
		onembedmissing?: () => void;
		onembedreindex?: () => void;
		onsetembedkinds?: (kinds: number[]) => void;
		onsetautoembed?: (enabled: boolean) => void;
	} = $props();

	// The full rendered kind list: the canonical menu first, then any custom
	// kinds the user has added (those in the active set but not in the menu).
	const menu = $derived(status?.available_kinds ?? []);
	const active = $derived(status?.embed_kinds ?? []);
	const customKinds = $derived(active.filter((k) => !menu.includes(k)));

	let newKind = $state('');
	let addError = $state<string | null>(null);

	// Toggle one kind in/out of the active set and persist via the host.
	function toggleKind(kind: number) {
		const next = new Set(active);
		if (next.has(kind)) next.delete(kind);
		else next.add(kind);
		onsetembedkinds?.([...next]);
	}

	// Add a custom kind typed into the "Add kind" box.
	function addKind() {
		addError = null;
		const k = Number(newKind.trim());
		if (!Number.isInteger(k) || k < 0 || k > 65535) {
			addError = 'Enter a kind number (0–65535)';
			return;
		}
		if (active.includes(k)) {
			addError = `k:${k} is already embedded`;
			return;
		}
		onsetembedkinds?.([...active, k]);
		newKind = '';
	}

	function fullReembed() {
		if (confirm('Clear the embedding index and re-embed everything? This can take a while.')) {
			onembedreindex?.();
		}
	}
</script>

{#if !status}
	<p class="es-hint">Loading embedding status…</p>
{:else if !status.enabled}
	<p class="es-hint">
		Embedding is disabled. Set <code>[embedding] enabled = true</code> in
		<code>config.toml</code> to enable semantic search (<code>~:query</code>).
		Embeddings run in-process — no extra services.
	</p>
{:else}
	<div class="es-status">
		<span
			class="es-pill"
			class:es-pill--ok={status.embedding_available}
			class:es-pill--off={!status.embedding_available}
		>{status.embedding_available ? 'connected' : 'unreachable'}</span>
		{#if status.model}
			<span class="es-pill es-pill--ghost">{status.model}</span>
		{/if}
		<span class="es-counts">
			{status.indexed_count}/{status.total_events} embedded{#if status.stale_count > 0} · {status.stale_count} stale{/if}{#if status.missing_sections > 0} · {status.missing_sections} missing{/if}
		</span>
	</div>

	<label class="es-toggle" title="Embed new events of the configured kinds as they're retrieved from relays or published">
		<input
			type="checkbox"
			checked={status.auto_embed}
			onchange={(e) => onsetautoembed?.(e.currentTarget.checked)}
		/>
		<span>Auto-embed on retrieval &amp; publishing</span>
	</label>

	<div class="es-kinds">
		<span class="es-kinds-label">Kinds to embed</span>
		{#each menu as k (k)}
			<label class="es-kind" title={kindLabel(k)}>
				<input
					type="checkbox"
					checked={active.includes(k)}
					disabled={syncing}
					onchange={() => toggleKind(k)}
				/>
				<span>{kindLabel(k)} <span class="es-kind-num">k:{k}</span></span>
			</label>
		{/each}
		{#each customKinds as k (k)}
			<label class="es-kind es-kind--custom" title={kindLabel(k)}>
				<input
					type="checkbox"
					checked={true}
					disabled={syncing}
					onchange={() => toggleKind(k)}
				/>
				<span>{kindLabel(k)} <span class="es-kind-num">k:{k}</span></span>
			</label>
		{/each}
	</div>

	<div class="es-addkind">
		<input
			class="es-addkind-input"
			type="text"
			inputmode="numeric"
			placeholder="custom kind, e.g. 30817"
			bind:value={newKind}
			disabled={syncing}
			onkeydown={(e) => { if (e.key === 'Enter') addKind(); }}
		/>
		<button class="es-btn es-addkind-btn" onclick={addKind} disabled={syncing}>Add kind</button>
		{#if addError}<span class="es-addkind-err">{addError}</span>{/if}
	</div>

	<div class="es-actions">
		<button
			class="es-btn"
			onclick={() => onembedmissing?.()}
			disabled={syncing}
			title="Embed events that aren't in the index yet"
		>{syncing ? 'Embedding…' : 'Embed missing'}</button>
		<button
			class="es-btn es-btn--danger"
			onclick={fullReembed}
			disabled={syncing}
			title="Clear the index and re-embed every eligible event"
		>Full re-embed</button>
	</div>
{/if}

<style>
	.es-hint {
		font-size: 0.7rem;
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 0;
	}
	.es-status {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
		margin-bottom: 10px;
	}
	.es-pill {
		font-size: 0.6rem;
		padding: 1px 7px;
		border-radius: var(--radius);
		text-transform: lowercase;
	}
	.es-pill--ok {
		background: color-mix(in srgb, var(--green) 18%, transparent);
		color: var(--green);
	}
	.es-pill--off {
		background: color-mix(in srgb, var(--red) 18%, transparent);
		color: var(--red);
	}
	.es-pill--ghost {
		background: color-mix(in srgb, var(--fg-muted) 14%, transparent);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		text-transform: none;
	}
	.es-counts {
		font-size: 0.65rem;
		color: var(--fg-muted);
	}
	.es-toggle {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.7rem;
		color: var(--fg);
		cursor: pointer;
		margin-bottom: 10px;
	}
	.es-toggle input { cursor: pointer; }
	.es-kinds {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 4px 12px;
		margin-bottom: 8px;
	}
	.es-kinds-label {
		font-size: 0.6rem;
		text-transform: uppercase;
		letter-spacing: 0.07em;
		color: var(--fg-muted);
		width: 100%;
	}
	.es-kind {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 0.7rem;
		color: var(--fg);
		cursor: pointer;
	}
	.es-kind input { cursor: pointer; }
	.es-kind input:disabled { cursor: default; }
	.es-kind-num {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		color: var(--fg-muted);
	}
	.es-addkind {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
		margin-bottom: 10px;
	}
	.es-addkind-input {
		font-size: 0.7rem;
		padding: 3px 8px;
		border-radius: var(--radius);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg);
		width: 150px;
	}
	.es-addkind-err {
		font-size: 0.62rem;
		color: var(--red);
	}
	.es-actions {
		display: flex;
		gap: 8px;
	}
	.es-btn {
		font-size: 0.68rem;
		padding: 4px 12px;
		border-radius: var(--radius);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--fg);
		cursor: pointer;
	}
	.es-btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
	.es-btn:disabled { opacity: 0.5; cursor: default; }
	.es-btn--danger:hover:not(:disabled) { border-color: var(--red); color: var(--red); }
	.es-addkind-btn { padding: 3px 10px; }
</style>
