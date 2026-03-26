<script lang="ts">
	import type { SyncMode, ButtonLabels, EmbeddingStatusResponse } from '$lib/types';

	let {
		syncMode,
		buttonLabels,
		embeddingStatus = null,
		embeddingSyncing = false,
		ignoredCount = 0,
		onsetsyncmode,
		onsetbuttonlabels,
		onhome,
		onsyncembeddings
	}: {
		syncMode: SyncMode;
		buttonLabels: ButtonLabels;
		embeddingStatus?: EmbeddingStatusResponse | null;
		embeddingSyncing?: boolean;
		ignoredCount?: number;
		onsetsyncmode: (mode: SyncMode) => void;
		onsetbuttonlabels: (mode: ButtonLabels) => void;
		onhome?: () => void;
		onsyncembeddings?: () => void;
	} = $props();

	let settingsOpen = $state(false);
</script>

<div class="workbench-toolbar">
	<button class="workbench-title" onclick={onhome}>tendrl</button>
	{#if ignoredCount > 0}
		<span class="ignored-count" title="{ignoredCount} events/authors hidden">{ignoredCount} hidden</span>
	{/if}
	<span class="spacer"></span>
	{#if embeddingStatus?.enabled}
		{@const pct = embeddingStatus.total_events > 0 ? Math.round((embeddingStatus.indexed_count / embeddingStatus.total_events) * 100) : 0}
		<span class="embed-status" class:offline={!embeddingStatus.sidecar_available}>
			{#if !embeddingStatus.sidecar_available}
				embed offline
			{:else if embeddingSyncing}
				{embeddingStatus.indexed_count}/{embeddingStatus.total_events}
			{:else}
				{embeddingStatus.indexed_count}/{embeddingStatus.total_events}
			{/if}
		</span>
		{#if embeddingSyncing}
			<div class="embed-progress">
				<div class="embed-progress-bar" style:width="{pct}%"></div>
			</div>
		{/if}
		<button class="embed-sync-btn" onclick={onsyncembeddings} disabled={embeddingSyncing} title="Sync embeddings">
			{embeddingSyncing ? '...' : '↻'}
		</button>
	{/if}
	<button class="settings-toggle" onclick={() => (settingsOpen = !settingsOpen)} title="Settings">
		{settingsOpen ? '✕' : '⚙'}
	</button>
</div>

{#if settingsOpen}
	<div class="settings-bar">
		<span class="settings-label">Sync:</span>
		<button class="settings-btn" class:active={syncMode === 'reactive'} onclick={() => onsetsyncmode('reactive')}>reactive</button>
		<button class="settings-btn" class:active={syncMode === 'explicit'} onclick={() => onsetsyncmode('explicit')}>explicit</button>
		<span class="settings-label">Labels:</span>
		<button class="settings-btn" class:active={buttonLabels === 'icon'} onclick={() => onsetbuttonlabels('icon')}>◂ □ ▸</button>
		<button class="settings-btn" class:active={buttonLabels === 'text'} onclick={() => onsetbuttonlabels('text')}>text</button>
	</div>
{/if}

<style>
	.workbench-toolbar {
		display: flex;
		align-items: center;
		padding: 6px 16px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.workbench-title {
		font-weight: 700;
		font-size: 1rem;
		background: none !important;
		border: none !important;
		color: var(--accent);
		cursor: pointer;
		padding: 2px 4px !important;
		border-radius: 0;
		letter-spacing: 0.02em;
	}

	.workbench-title:hover {
		color: var(--fg);
		background: none !important;
	}

	.spacer {
		flex: 1;
	}

	.ignored-count {
		font-size: 0.65rem;
		color: #ef4444;
		opacity: 0.7;
		margin-left: 8px;
	}

	.embed-status {
		font-size: 0.7rem;
		color: var(--fg-muted);
		margin-right: 4px;
	}

	.embed-status.offline {
		color: #ef4444;
	}

	.embed-progress {
		width: 60px;
		height: 6px;
		background: var(--border);
		border-radius: 3px;
		overflow: hidden;
		margin-right: 4px;
	}

	.embed-progress-bar {
		height: 100%;
		background: var(--accent);
		border-radius: 3px;
		transition: width 0.3s ease;
	}

	.embed-sync-btn {
		font-size: 0.75rem;
		padding: 1px 6px;
		margin-right: 8px;
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		cursor: pointer;
		border-radius: var(--radius);
	}

	.embed-sync-btn:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}

	.settings-toggle {
		padding: 2px 8px;
		font-size: 0.9rem;
		border: none;
		background: transparent;
		color: var(--fg-muted);
		cursor: pointer;
	}

	.settings-toggle:hover {
		color: var(--fg);
	}

	.settings-bar {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 4px 16px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.settings-label {
		font-size: 0.65rem;
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
	}

	.settings-btn {
		font-size: 0.65rem;
		padding: 2px 6px;
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
