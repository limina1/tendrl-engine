<script lang="ts">
	// Embedding-index controls: sidecar/index status, which kinds get embedded,
	// and the embed-missing / full-reembed actions. Shared between the search
	// panel's bottom "Embedding settings" bar and the search-config modal's
	// "Embedding" section so the two never drift. Presentational — status and
	// callbacks are passed in by the host (both pull from app state).
	//
	// Unlike the modal's search defaults (committed on Save), these act
	// immediately: the kind set is persisted engine-side the moment a box is
	// toggled, and the buttons kick off engine passes right away.

	import type { EmbeddingStatusResponse } from '$lib/types';
	import { kindLabel } from '$lib/search/search-config.svelte';

	let {
		status = null,
		syncing = false,
		onembedmissing,
		onembedreindex,
		onsetembedkinds
	}: {
		status?: EmbeddingStatusResponse | null;
		syncing?: boolean;
		onembedmissing?: () => void;
		onembedreindex?: () => void;
		onsetembedkinds?: (kinds: number[]) => void;
	} = $props();

	// Toggle one kind in/out of the embeddable set and persist via the host,
	// keeping the response's `available_kinds` order.
	function toggleKind(kind: number) {
		const active = new Set(status?.embed_kinds ?? []);
		if (active.has(kind)) active.delete(kind);
		else active.add(kind);
		const menu = status?.available_kinds ?? [];
		onsetembedkinds?.(menu.filter((k) => active.has(k)));
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
		<code>config.toml</code> (or build with <code>--features onnx</code>) to enable
		semantic search (<code>~:query</code>).
	</p>
{:else}
	<div class="es-status">
		<span
			class="es-pill"
			class:es-pill--ok={status.sidecar_available}
			class:es-pill--off={!status.sidecar_available}
		>{status.sidecar_available ? 'connected' : 'unreachable'}</span>
		{#if status.model}
			<span class="es-pill es-pill--ghost">{status.model}</span>
		{/if}
		<span class="es-counts">
			{status.indexed_count}/{status.total_events} embedded{#if status.stale_count > 0} · {status.stale_count} stale{/if}{#if status.missing_sections > 0} · {status.missing_sections} missing{/if}
		</span>
	</div>

	<div class="es-kinds">
		<span class="es-kinds-label">Kinds to embed</span>
		{#each status.available_kinds ?? [] as k (k)}
			<label class="es-kind" title={kindLabel(k)}>
				<input
					type="checkbox"
					checked={(status.embed_kinds ?? []).includes(k)}
					disabled={syncing}
					onchange={() => toggleKind(k)}
				/>
				<span>{kindLabel(k)} <span class="es-kind-num">k:{k}</span></span>
			</label>
		{/each}
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
	.es-kinds {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 4px 12px;
		margin-bottom: 10px;
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
</style>
