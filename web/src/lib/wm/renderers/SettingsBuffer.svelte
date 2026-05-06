<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import type { Buffer } from '../types';
	import type { EditorInsertMode, SyncMode, ButtonLabels, ComposeDefaultMode } from '$lib/types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
</script>

<div class="settings-view">
	<div class="settings-header">Settings</div>

	<div class="settings-group">
		<div class="settings-group-title">Editor</div>

		<div class="settings-row">
			<label class="settings-label" for="line-numbers">Line numbers</label>
			<label class="switch">
				<input
					id="line-numbers"
					type="checkbox"
					checked={app.editorLineNumbers}
					onchange={(e) => (app.editorLineNumbers = e.currentTarget.checked)}
				/>
				<span class="switch-text">{app.editorLineNumbers ? 'on' : 'off'}</span>
			</label>
		</div>

		<div class="settings-row">
			<label class="settings-label" for="vim-mode">Vim mode</label>
			<label class="switch">
				<input
					id="vim-mode"
					type="checkbox"
					checked={app.editorVimMode}
					onchange={(e) => (app.editorVimMode = e.currentTarget.checked)}
				/>
				<span class="switch-text">{app.editorVimMode ? 'on' : 'off'}</span>
			</label>
		</div>

		<div class="settings-row">
			<span class="settings-label">Insert from search</span>
			<div class="radio-group">
				{#each ['cursor', 'append'] as opt (opt)}
					<label class="radio">
						<input
							type="radio"
							name="insert-mode"
							value={opt}
							checked={app.editorInsertMode === opt}
							onchange={() => (app.editorInsertMode = opt as EditorInsertMode)}
						/>
						<span>{opt}</span>
					</label>
				{/each}
			</div>
		</div>
		<p class="settings-hint">
			<strong>cursor</strong>: insert at the caret in the plain editor (falls back to append
			when plain mode isn't active).<br />
			<strong>append</strong>: append at the bottom of the document or as a new section block.
		</p>
	</div>

	<div class="settings-group">
		<div class="settings-group-title">Compose</div>

		<div class="settings-row">
			<span class="settings-label">Default edit mode</span>
			<div class="radio-group">
				{#each ['full', 'plain'] as opt (opt)}
					<label class="radio">
						<input
							type="radio"
							name="compose-default-mode"
							value={opt}
							checked={app.composeDefaultMode === opt}
							onchange={() => (app.composeDefaultMode = opt as ComposeDefaultMode)}
						/>
						<span>{opt}</span>
					</label>
				{/each}
			</div>
		</div>

		<div class="settings-row">
			<span class="settings-label">Sync mode</span>
			<div class="radio-group">
				{#each ['reactive', 'explicit'] as opt (opt)}
					<label class="radio">
						<input
							type="radio"
							name="sync-mode"
							value={opt}
							checked={app.syncMode === opt}
							onchange={() => (app.syncMode = opt as SyncMode)}
						/>
						<span>{opt}</span>
					</label>
				{/each}
			</div>
		</div>

		<div class="settings-row">
			<span class="settings-label">Button labels</span>
			<div class="radio-group">
				{#each ['icon', 'text'] as opt (opt)}
					<label class="radio">
						<input
							type="radio"
							name="button-labels"
							value={opt}
							checked={app.buttonLabels === opt}
							onchange={() => (app.buttonLabels = opt as ButtonLabels)}
						/>
						<span>{opt}</span>
					</label>
				{/each}
			</div>
		</div>
	</div>
</div>

<style>
	.settings-view {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 0 0 24px;
	}

	.settings-header {
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}

	.settings-group {
		padding: 14px 16px 6px;
		border-bottom: 1px solid var(--panel-border);
	}

	.settings-group-title {
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		margin-bottom: 8px;
	}

	.settings-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 6px 0;
		gap: 12px;
	}

	.settings-label {
		font-size: var(--t-sm);
		color: var(--fg);
	}

	.settings-hint {
		font-size: var(--t-xs);
		color: var(--base5);
		margin: 4px 0 8px;
		line-height: 1.5;
	}

	.switch {
		display: flex;
		align-items: center;
		gap: 6px;
		cursor: pointer;
	}

	.switch-text {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
	}

	.radio-group {
		display: flex;
		gap: 10px;
	}

	.radio {
		display: flex;
		align-items: center;
		gap: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		cursor: pointer;
		color: var(--base6);
	}

	.radio input[type='radio']:checked + span {
		color: var(--id-yours);
	}
</style>
