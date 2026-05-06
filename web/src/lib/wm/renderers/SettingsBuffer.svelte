<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import { detectNip07 } from '$lib/identity/signer';
	import type { Buffer } from '../types';
	import type { EditorInsertMode, SyncMode, ButtonLabels, ComposeDefaultMode } from '$lib/types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	let nip07Available = $state(false);
	$effect(() => {
		// Detect once on mount; window.nostr is injected by extensions
		// at document_start, so by the time SettingsBuffer renders it's
		// either there or it isn't.
		nip07Available = detectNip07();
	});

	// Inputs for engine login flow
	let ncryptsecInput = $state('');
	let passwordInput = $state('');

	const currentSource = $derived(app.identityStatus?.source ?? 'engine');
	const currentState = $derived(app.identityStatus?.state ?? 'none');

	async function pickSource(source: 'engine' | 'nip07') {
		if (source === 'engine') {
			await app.handleSelectEngineSource();
		} else if (source === 'nip07') {
			await app.handleSelectNip07Source();
		}
	}

	async function doLogin() {
		const v = ncryptsecInput.trim();
		if (!v) return;
		await app.handleIdentityLogin(v);
		ncryptsecInput = '';
	}

	async function doUnlock() {
		const v = passwordInput;
		if (!v) return;
		await app.handleIdentityUnlock(v);
		passwordInput = '';
	}
</script>

<div class="settings-view">
	<div class="settings-header">Settings</div>

	<div class="settings-group">
		<div class="settings-group-title">Identity</div>

		<div class="settings-row">
			<span class="settings-label">Status</span>
			<div class="status-row">
				<span
					class="pill {currentState === 'unlocked'
						? 'pill--online'
						: currentState === 'locked'
							? 'pill--draft'
							: 'pill--ghost'}"
				>{currentState}</span>
				<span class="pill pill--ghost source-pill">source: {currentSource}</span>
			</div>
		</div>

		<div class="settings-row">
			<span class="settings-label">Source</span>
			<div class="radio-group">
				<label class="radio">
					<input
						type="radio"
						name="identity-source"
						value="engine"
						checked={currentSource === 'engine'}
						onchange={() => pickSource('engine')}
					/>
					<span>engine</span>
				</label>
				<label class="radio" class:radio--disabled={!nip07Available}>
					<input
						type="radio"
						name="identity-source"
						value="nip07"
						disabled={!nip07Available}
						checked={currentSource === 'nip07'}
						onchange={() => pickSource('nip07')}
					/>
					<span>nip07{nip07Available ? '' : ' (no extension)'}</span>
				</label>
			</div>
		</div>

		{#if currentSource === 'engine'}
			{#if currentState === 'none'}
				<div class="settings-row settings-row--stack">
					<label class="settings-label" for="ncryptsec-input">ncryptsec</label>
					<textarea
						id="ncryptsec-input"
						class="settings-textarea"
						bind:value={ncryptsecInput}
						placeholder="ncryptsec1..."
						rows="2"
						spellcheck="false"
					></textarea>
					<button class="settings-action" onclick={doLogin} disabled={app.identityLoading}>
						{app.identityLoading ? 'Working…' : 'Login'}
					</button>
				</div>
			{:else if currentState === 'locked'}
				<div class="settings-row settings-row--stack">
					{#if app.identityStatus?.npub}
						<span class="settings-label mono">{app.identityStatus.npub}</span>
					{/if}
					<input
						id="password-input"
						class="settings-input"
						type="password"
						bind:value={passwordInput}
						placeholder="Password"
						onkeydown={(e) => e.key === 'Enter' && doUnlock()}
					/>
					<div class="action-row">
						<button class="settings-action" onclick={doUnlock} disabled={app.identityLoading}>
							{app.identityLoading ? 'Working…' : 'Unlock'}
						</button>
						<button class="settings-action settings-action--danger" onclick={app.handleIdentityLogout}
							>Logout</button
						>
					</div>
				</div>
			{:else if currentState === 'unlocked'}
				<div class="settings-row settings-row--stack">
					{#if app.identityStatus?.npub}
						<span class="settings-label mono">{app.identityStatus.npub}</span>
					{/if}
					{#if app.identityStatus?.seconds_remaining != null}
						<span class="settings-hint"
							>auto-locks in {app.identityStatus.seconds_remaining}s</span
						>
					{/if}
					<div class="action-row">
						<button class="settings-action" onclick={app.handleIdentityLock}>Lock</button>
						<button class="settings-action settings-action--danger" onclick={app.handleIdentityLogout}
							>Logout</button
						>
					</div>
				</div>
			{/if}
		{:else if currentSource === 'nip07'}
			<div class="settings-row settings-row--stack">
				<span class="settings-hint">
					Signing requests are routed to <strong>window.nostr</strong>; the engine never
					sees your secret.
				</span>
				{#if app.externalSignerPubkey}
					<span class="settings-label mono">{app.externalSignerPubkey.slice(0, 16)}…</span>
				{/if}
				<div class="action-row">
					<button class="settings-action" onclick={() => pickSource('engine')}
						>Disconnect</button
					>
				</div>
			</div>
		{/if}

		{#if app.identityError}
			<p class="settings-error">{app.identityError}</p>
		{/if}
	</div>

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

	.radio--disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}

	.status-row {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	.source-pill {
		font-family: var(--font-mono);
	}

	.settings-row--stack {
		flex-direction: column;
		align-items: stretch;
		gap: 6px;
	}

	.settings-textarea,
	.settings-input {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 6px 8px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: var(--bg);
		color: var(--fg);
		width: 100%;
		resize: vertical;
		box-sizing: border-box;
	}

	.action-row {
		display: flex;
		gap: 6px;
	}

	.settings-action {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 10px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.settings-action:hover:not(:disabled) {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.settings-action:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.settings-action--danger:hover:not(:disabled) {
		border-color: var(--id-draft);
		color: var(--id-draft);
	}

	.settings-error {
		font-size: var(--t-xs);
		color: var(--id-draft);
		margin: 6px 0 0;
		font-family: var(--font-mono);
	}

	.mono {
		font-family: var(--font-mono);
	}
</style>
