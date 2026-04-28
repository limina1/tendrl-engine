<script lang="ts">
	import { onMount } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	onMount(() => {
		app.handleListDocuments();
	});
</script>

<div class="kb">
	<div class="kb__header">
		<span>Knowledgebase ({app.documentFiles.length} files)</span>
	</div>
	{#if app.documentFiles.length > 0}
		<div class="kb__list">
			{#each app.documentFiles as file (file.name)}
				<button class="kb__row" onclick={() => app.handleParseDocument(file.name)}>
					<span class="kb__icon">📄</span>
					<span class="kb__name">{file.name}</span>
					<span class="kb__meta">{file.format}</span>
				</button>
			{/each}
		</div>
	{:else}
		<div class="empty"><p>No documents imported. Use M-x or the Search panel's import flow.</p></div>
	{/if}
</div>

<style>
	.kb { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.kb__header {
		padding: 8px 12px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.kb__list { flex: 1; overflow-y: auto; min-height: 0; }
	.kb__row {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 6px 12px;
		text-align: left;
		background: transparent;
		border: none;
		border-bottom: 1px solid var(--panel-border);
		color: var(--fg);
		font-size: var(--t-sm);
		cursor: pointer;
	}
	.kb__row:hover { background: var(--panel-bg-soft); }
	.kb__icon { font-size: 14px; opacity: 0.7; }
	.kb__name { font-family: var(--font-mono); font-size: var(--t-xs); flex: 1; }
	.kb__meta {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		text-transform: uppercase;
	}
	.empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
</style>
