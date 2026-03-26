<script lang="ts">
	import type { SyncMode, ButtonLabels } from '$lib/types';

	let {
		syncMode,
		buttonLabels,
		onsetsyncmode,
		onsetbuttonlabels,
		onhome
	}: {
		syncMode: SyncMode;
		buttonLabels: ButtonLabels;
		onsetsyncmode: (mode: SyncMode) => void;
		onsetbuttonlabels: (mode: ButtonLabels) => void;
		onhome?: () => void;
	} = $props();

	let settingsOpen = $state(false);
</script>

<div class="workbench-toolbar">
	<button class="workbench-title" onclick={onhome}>tendrl</button>
	<span class="spacer"></span>
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
