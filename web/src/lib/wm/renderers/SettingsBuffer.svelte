<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import { detectNip07 } from '$lib/identity/signer';
	import { discovery, rearmDiscovery, setWalkthroughEnabled } from '$lib/wm/discovery.svelte';
	import * as api from '$lib/api';
	import type { Buffer } from '../types';
	import type { EditorInsertMode, SyncMode, ButtonLabels, ComposeDefaultMode } from '$lib/types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	let nip07Available = $state(false);
	// Snapshot of the last-saved values from config.toml. Loaded on
	// mount + after a successful save. Compared against the live
	// app.* values to drive the dirty flag on the Save button.
	type SavedBaseline = {
		editor: { line_numbers: boolean; vim_mode: boolean; insert_mode: string };
		compose: { default_mode: string; sync_mode: string; button_labels: string };
		network: { mode: string };
		identity_source: string | null;
		identity_lock_timeout_minutes: number;
	};
	let savedBaseline = $state<SavedBaseline | null>(null);

	async function captureSavedBaseline() {
		try {
			const s = await api.getSettings();
			savedBaseline = {
				editor: s.editor,
				compose: s.compose,
				network: s.network,
				identity_source: s.identity?.source ?? null,
				identity_lock_timeout_minutes: s.identity?.lock_timeout_minutes ?? 0
			};
		} catch {
			// Endpoint unavailable — leave baseline null, dirty stays
			// false (button disabled) so we don't false-claim changes.
			savedBaseline = null;
		}
	}

	// --- AI assistant settings (provider/model/auth + tool policy) ---
	let aiSettings = $state<api.AiSettings | null>(null);
	let aiBusy = $state(false);

	async function loadAiSettings() {
		try {
			aiSettings = await api.getAiSettings();
		} catch {
			aiSettings = null;
		}
	}

	// --- Editable system prompt (prompt.md) ---
	let aiPrompt = $state('');
	let aiPromptPath = $state('');
	let aiPromptDirty = $state(false);

	async function loadAiPrompt() {
		try {
			const r = await api.getAiPrompt();
			aiPrompt = r.content;
			aiPromptPath = r.path;
			aiPromptDirty = false;
		} catch {
			/* leave blank if unavailable */
		}
	}

	async function saveAiPrompt() {
		aiBusy = true;
		try {
			await api.saveAiPrompt(aiPrompt);
			aiPromptDirty = false;
			app.pushToast('System prompt saved', 'success', 2500);
		} catch (e) {
			app.pushToast(
				`Prompt save failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		} finally {
			aiBusy = false;
		}
	}

	async function applyAiUpdate(update: api.AiSettingsUpdate) {
		aiBusy = true;
		try {
			aiSettings = await api.saveAiSettings(update);
		} catch (e) {
			app.pushToast(
				`AI settings save failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		} finally {
			aiBusy = false;
		}
	}

	function toggleAiTool(name: string, enabled: boolean) {
		if (!aiSettings) return;
		const names = aiSettings.tools
			.filter((t) => (t.name === name ? enabled : t.enabled))
			.map((t) => t.name);
		applyAiUpdate({ enabled_tools: names });
	}

	$effect(() => {
		// Detect window.nostr on mount, then keep retrying for a couple
		// of seconds. Extensions inject at document_start, but after a
		// hard reload / cache clear the buffer can render *before* the
		// extension finishes injecting — a one-shot check there latches
		// the radio disabled ("no extension") forever even once the
		// signer is live. Poll a few times (mirrors the boot-time
		// auto-reconnect detection) and stop as soon as it appears.
		nip07Available = detectNip07();
		let cancelled = false;
		if (!nip07Available) {
			(async () => {
				for (const delay of [100, 250, 500, 1000]) {
					await new Promise((r) => setTimeout(r, delay));
					if (cancelled) return;
					if (detectNip07()) {
						nip07Available = true;
						return;
					}
				}
			})();
		}
		// Force a fresh /identity + /settings fetch on mount. Without
		// this, opening Settings shortly after an engine restart shows
		// stale identityStatus from the last 30s-poll tick. Fire and
		// forget — Svelte 5 re-derives `currentSource` once the state
		// updates land.
		app.refreshIdentity();
		captureSavedBaseline();
		loadAiSettings();
		loadAiPrompt();
		// Pull a fresh embedding status so the Embeddings section
		// reflects live health + index counts, not the last 30s-poll tick.
		app.refreshEmbeddingStatus();
		return () => {
			cancelled = true;
		};
	});

	// Inputs for engine login flow
	let ncryptsecInput = $state('');
	let passwordInput = $state('');
	// Inputs for the assistant identity flow
	let assistantKeyInput = $state('');
	let assistantPasswordInput = $state('');
	const assistantState = $derived(app.assistantStatus?.state ?? 'none');

	async function doAssistantLogin() {
		const v = assistantKeyInput.trim();
		if (!v) return;
		await app.handleAssistantLogin(v);
		assistantKeyInput = '';
	}

	async function doAssistantUnlock() {
		const v = assistantPasswordInput;
		if (!v) return;
		await app.handleAssistantUnlock(v);
		assistantPasswordInput = '';
	}

	// Prefer the live session source. Fall back to the source the user
	// last persisted (config.toml [identity] source) so a fresh reload
	// doesn't flash "engine" for the ~1–2s before the NIP-07
	// auto-reconnect completes. Final fallback: 'engine'.
	const currentSource = $derived(
		app.identityStatus?.source ?? app.savedIdentitySource ?? 'engine'
	);
	const currentState = $derived(app.identityStatus?.state ?? 'none');
	const isAutoReconnecting = $derived(app.identityAutoReconnecting);

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

	const networkMode = $derived(app.networkStatus?.mode ?? 'confirm');
	async function setNetworkMode(mode: 'auto' | 'confirm') {
		if (networkMode === mode) return;
		await app.handleSetNetworkMode(mode);
	}

	let saving = $state(false);

	/** True when any live setting differs from the last-saved value in
	 *  config.toml. Drives the dirty visual + enabled state on the
	 *  Save Settings button — nothing to save means nothing to click.
	 *
	 *  Default-dirty when the baseline hasn't loaded yet: better to
	 *  let the user click (no-op save) than to stick the button in a
	 *  perma-disabled state if `/api/v1/settings` errored or hasn't
	 *  resolved yet. */
	const settingsDirty = $derived.by(() => {
		const b = savedBaseline;
		if (!b) return true;
		if (b.editor.line_numbers !== app.editorLineNumbers) return true;
		if (b.editor.vim_mode !== app.editorVimMode) return true;
		if (b.editor.insert_mode !== app.editorInsertMode) return true;
		if (b.compose.default_mode !== app.composeDefaultMode) return true;
		if (b.compose.sync_mode !== app.syncMode) return true;
		if (b.compose.button_labels !== app.buttonLabels) return true;
		// networkStatus may briefly be null before the first poll —
		// don't false-positive in that window. Only call it dirty
		// when we actually have a live mode to compare against.
		const liveMode = app.networkStatus?.mode;
		if (liveMode != null && b.network.mode !== liveMode) return true;
		const liveSource = app.identityStatus?.source ?? 'engine';
		if ((b.identity_source ?? 'engine') !== liveSource) return true;
		// Live timeout is applied immediately via the lock-timeout
		// endpoint; dirty just means it isn't persisted to config yet.
		if (b.identity_lock_timeout_minutes !== (app.identityStatus?.lock_timeout_minutes ?? 0))
			return true;
		return false;
	});

	// Purge button state. Calls the engine's /api/v1/purge which
	// deletes the LMDB files + re-execs itself in-place; the toast
	// driven by handlePurge tracks the ~1 second reconnect window.
	let purgeLoading = $state(false);
	async function requestPurge() {
		// Resolve data_dir live right before the confirm. Reading from
		// app.dataDir (populated during initialize()) was unreliable
		// when the buffer was opened before init finished, or when the
		// dataDir state field hadn't been hot-reloaded yet. A direct
		// fetch here is sub-10ms and guarantees the prompt always
		// names the actual path the engine will unlink.
		let path = '<unknown — engine config not reachable>';
		try {
			const cfg = await api.getConfig();
			if (cfg.data_dir) path = cfg.data_dir;
		} catch {
			/* fall back to the unknown placeholder */
		}
		const msg =
			'Purge the local nostrdb cache and restart the engine?\n\n' +
			`Files to delete:\n  ${path}/data.mdb\n  ${path}/lock.mdb\n\n` +
			'Preserved: relays.json (in same dir), config.toml, identity ncryptsec.';
		if (!confirm(msg)) return;
		purgeLoading = true;
		try {
			await app.handlePurge();
		} finally {
			purgeLoading = false;
		}
	}

	async function saveSettings() {
		saving = true;
		try {
			const sourceToPersist = app.identityStatus?.source ?? 'engine';
			const resp = await api.snapshotConfig({
				include_relays: true,
				editor: {
					line_numbers: app.editorLineNumbers,
					vim_mode: app.editorVimMode,
					insert_mode: app.editorInsertMode
				},
				compose: {
					default_mode: app.composeDefaultMode,
					sync_mode: app.syncMode,
					button_labels: app.buttonLabels
				},
				network_mode: app.networkStatus?.mode ?? 'confirm',
				// Persist the current signing source so reload reconnects
				// to the same extension/key without manual re-select.
				identity_source: sourceToPersist,
				// Persist the live auto-lock timeout (0 = never) so it
				// survives a restart, matching the live session value.
				identity_lock_timeout_minutes: app.identityStatus?.lock_timeout_minutes ?? 0
			});
			// Mirror the just-persisted value into the in-memory cache
			// so `currentSource`'s fallback chain reflects the user's
			// latest choice immediately — important if they don't reload
			// right away but later look at the radio.
			app.setSavedIdentitySource(sourceToPersist);
			app.pushToast(resp.message, 'success', 3500);
			// Re-capture baseline so the dirty flag clears.
			await captureSavedBaseline();
		} catch (e) {
			app.pushToast(
				`Save failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			saving = false;
		}
	}
</script>

<div class="settings-view">
	<div class="settings-header">
		<span class="settings-header-title">Settings</span>
		<button
			class="settings-save"
			class:settings-save--dirty={settingsDirty}
			onclick={saveSettings}
			disabled={saving}
			title={!settingsDirty
				? 'Current settings already match config.toml — click to re-write them anyway and confirm.'
				: 'Write current identity source · editor · compose · network · relays into config.toml so the next boot starts here.'}
		>
			{saving ? 'Saving…' : settingsDirty ? 'Save settings *' : 'Save settings'}
		</button>
	</div>

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

		{#if currentSource === 'engine' && currentState !== 'none'}
			<div class="settings-row">
				<span class="settings-label">Lock after</span>
				<div class="radio-group">
					{#each [0, 5, 15, 30, 60] as mins (mins)}
						<label class="radio">
							<input
								type="radio"
								name="lock-timeout"
								value={mins}
								checked={(app.identityStatus?.lock_timeout_minutes ?? 0) === mins}
								onchange={() => app.handleSetLockTimeout(mins)}
							/>
							<span>{mins === 0 ? 'never' : `${mins}m`}</span>
						</label>
					{/each}
				</div>
			</div>
			<p class="settings-hint">
				Auto-locks the engine key after this much inactivity; unlocking needs the password
				again. Only applies to the engine key — a NIP-07 signer holds its own key.
			</p>
		{/if}

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
					<span class="settings-hint">
						Held for this session only — the key never rests in config.toml. Sign in
						again (or via NIP-07) after a restart.
					</span>
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
				{#if isAutoReconnecting}
					<span class="settings-hint">
						Reconnecting to <strong>window.nostr</strong>… (extension prompt may appear)
					</span>
				{:else}
					<span class="settings-hint">
						Signing requests are routed to <strong>window.nostr</strong>; the engine never
						sees your secret.
					</span>
				{/if}
				{#if app.externalSignerPubkey}
					<span class="settings-label mono">{app.externalSignerPubkey.slice(0, 16)}…</span>
				{/if}
				<div class="action-row">
					<button
						class="settings-action"
						onclick={() => pickSource('nip07')}
						disabled={app.identityLoading || isAutoReconnecting}
						title="Re-register window.nostr (useful if the auto-reconnect was missed)"
					>
						{app.identityLoading || isAutoReconnecting ? 'Working…' : 'Reconnect'}
					</button>
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
		<div class="settings-group-title">Assistant identity</div>
		{#if assistantState === 'none'}
			<div class="settings-row settings-row--stack">
				<label class="settings-label" for="assistant-key-input">nsec or ncryptsec</label>
				<textarea
					id="assistant-key-input"
					class="settings-textarea"
					bind:value={assistantKeyInput}
					placeholder="nsec1… or ncryptsec1…"
					rows="2"
					spellcheck="false"
				></textarea>
				<button class="settings-action" onclick={doAssistantLogin} disabled={app.assistantLoading}>
					{app.assistantLoading ? 'Working…' : 'Set assistant'}
				</button>
				<span class="settings-hint">
					Establishes the <strong>by:assistant</strong> identity. An nsec is live
					immediately; an ncryptsec loads locked and needs a password. Stored in your OS
					keyring (never config); a raw nsec is never written to disk.
				</span>
				{#if app.assistantStatus && app.assistantStatus.keyring_available === false}
					<span class="settings-error">
						OS keyring unavailable — the assistant won't persist across a restart.
					</span>
				{/if}
			</div>
		{:else if assistantState === 'locked'}
			<div class="settings-row settings-row--stack">
				{#if app.assistantStatus?.npub}
					<span class="settings-label mono">{app.assistantStatus.npub}</span>
				{/if}
				<input
					id="assistant-password-input"
					class="settings-input"
					type="password"
					bind:value={assistantPasswordInput}
					placeholder="Password"
					onkeydown={(e) => e.key === 'Enter' && doAssistantUnlock()}
				/>
				<div class="action-row">
					<button
						class="settings-action"
						onclick={doAssistantUnlock}
						disabled={app.assistantLoading}
					>
						{app.assistantLoading ? 'Working…' : 'Unlock'}
					</button>
					<button class="settings-action settings-action--danger" onclick={app.handleAssistantLogout}
						>Remove</button
					>
				</div>
				<span class="settings-hint">
					Scoping (<strong>by:assistant</strong>) works while locked; unlock to publish as
					the assistant.
				</span>
			</div>
		{:else}
			<div class="settings-row settings-row--stack">
				{#if app.assistantStatus?.npub}
					<span class="settings-label mono">{app.assistantStatus.npub}</span>
				{/if}
				<div class="action-row">
					<button class="settings-action settings-action--danger" onclick={app.handleAssistantLogout}
						>Remove</button
					>
				</div>
			</div>
		{/if}
		{#if app.assistantError}
			<p class="settings-error">{app.assistantError}</p>
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

	<div class="settings-group">
		<div class="settings-group-title">Network</div>

		<div class="settings-row">
			<span class="settings-label">Default mode</span>
			<div class="radio-group">
				{#each ['auto', 'confirm'] as opt (opt)}
					<label class="radio">
						<input
							type="radio"
							name="network-mode"
							value={opt}
							checked={networkMode === opt}
							onchange={() => setNetworkMode(opt as 'auto' | 'confirm')}
						/>
						<span>{opt}</span>
					</label>
				{/each}
			</div>
		</div>
		<p class="settings-hint">
			<strong>auto</strong>: relay fetches run without confirmation.<br />
			<strong>confirm</strong>: every relay fetch raises a confirm modal — useful when bandwidth, privacy, or rate-limits matter.
		</p>

		<div class="settings-row">
			<label class="settings-label" for="walkthrough-toggle">Walkthrough</label>
			<label class="switch">
				<input
					id="walkthrough-toggle"
					type="checkbox"
					checked={discovery.enabled}
					onchange={(e) =>
						e.currentTarget.checked ? rearmDiscovery() : setWalkthroughEnabled(false)}
				/>
				<span class="switch-text">{discovery.enabled ? 'on' : 'off'}</span>
			</label>
		</div>

		<div class="settings-row">
			<span class="settings-label">Reset first-run setup</span>
			<button
				class="settings-action"
				onclick={() => app.resetNetworkModeChoice()}
				title="Re-arm the mode-choice modal + walkthrough for the next load (no data is touched)"
			>
				Reset…
			</button>
		</div>
		<p class="settings-hint">
			<strong>Walkthrough</strong>: contextual tips that point out features the first
			time you reach them. Toggling on (re)arms them; off silences them. The mode-line
			<strong>W</strong> button replays them any time.<br />
			<strong>Reset first-run setup</strong>: re-arms the mode-choice modal + walkthrough so they reappear on next load, as if freshly installed (no data is touched).
		</p>
	</div>

	<!-- AI assistant: provider/model/auth channel + per-tool policy. Tool
	     toggles apply live (next agent turn); provider/model/auth persist to
	     config.toml and take effect on the next engine restart. -->
	<div class="settings-group">
		<div class="settings-group-title">AI assistant</div>

		{#if !aiSettings}
			<p class="settings-hint">AI settings unavailable (engine not reachable).</p>
		{:else}
			<div class="settings-row">
				<span class="settings-label">Model</span>
				<input
					class="settings-input"
					type="text"
					value={aiSettings.model}
					disabled={aiBusy}
					onchange={(e) => applyAiUpdate({ model: e.currentTarget.value.trim() })}
				/>
			</div>

			<p class="settings-hint">
				Auth uses <code>ANTHROPIC_API_KEY</code> from the engine's environment. Model changes
				apply on engine restart.
			</p>

			<div class="settings-subtitle">System prompt</div>
			<p class="settings-hint">
				Prepended to every agent turn, re-read each message. Editable here or on disk
				{#if aiPromptPath}at <code>{aiPromptPath}</code>{/if}.
			</p>
			<textarea
				class="settings-textarea"
				rows="8"
				bind:value={aiPrompt}
				disabled={aiBusy}
				oninput={() => (aiPromptDirty = true)}
			></textarea>
			<div class="settings-row">
				<button
					class="settings-action"
					onclick={saveAiPrompt}
					disabled={aiBusy || !aiPromptDirty}>Save prompt</button
				>
			</div>

			<div class="settings-subtitle">Tools the assistant may use</div>
			{#each aiSettings.tools as tool (tool.name)}
				<div class="settings-row">
					<label class="settings-label" for={`ai-tool-${tool.name}`} title={tool.description}>
						{tool.name}
						<span class="ai-tool-cat">{tool.category}</span>
					</label>
					<label class="switch">
						<input
							id={`ai-tool-${tool.name}`}
							type="checkbox"
							checked={tool.enabled}
							disabled={aiBusy}
							onchange={(e) => toggleAiTool(tool.name, e.currentTarget.checked)}
						/>
						<span class="switch-text">{tool.enabled ? 'on' : 'off'}</span>
					</label>
				</div>
			{/each}
			<p class="settings-hint">
				Tool changes apply immediately to the next message. <code>publish</code>-category tools
				are off by default and broadcast signed events when enabled.
			</p>
		{/if}
	</div>

	<!-- AI assistant: provider/model/auth channel + per-tool policy. Tool
	     toggles apply live (next agent turn); provider/model/auth persist to
	     config.toml and take effect on the next engine restart. -->
	<div class="settings-group">
		<div class="settings-group-title">AI assistant</div>

		{#if !aiSettings}
			<p class="settings-hint">AI settings unavailable (engine not reachable).</p>
		{:else}
			<div class="settings-row">
				<span class="settings-label">Model</span>
				<input
					class="settings-input"
					type="text"
					value={aiSettings.model}
					disabled={aiBusy}
					onchange={(e) => applyAiUpdate({ model: e.currentTarget.value.trim() })}
				/>
			</div>

			<p class="settings-hint">
				Auth uses <code>ANTHROPIC_API_KEY</code> from the engine's environment. Model changes
				apply on engine restart.
			</p>

			<div class="settings-subtitle">System prompt</div>
			<p class="settings-hint">
				Prepended to every agent turn, re-read each message. Editable here or on disk
				{#if aiPromptPath}at <code>{aiPromptPath}</code>{/if}.
			</p>
			<textarea
				class="settings-textarea"
				rows="8"
				bind:value={aiPrompt}
				disabled={aiBusy}
				oninput={() => (aiPromptDirty = true)}
			></textarea>
			<div class="settings-row">
				<button
					class="settings-action"
					onclick={saveAiPrompt}
					disabled={aiBusy || !aiPromptDirty}>Save prompt</button
				>
			</div>

			<div class="settings-subtitle">Tools the assistant may use</div>
			{#each aiSettings.tools as tool (tool.name)}
				<div class="settings-row">
					<label class="settings-label" for={`ai-tool-${tool.name}`} title={tool.description}>
						{tool.name}
						<span class="ai-tool-cat">{tool.category}</span>
					</label>
					<label class="switch">
						<input
							id={`ai-tool-${tool.name}`}
							type="checkbox"
							checked={tool.enabled}
							disabled={aiBusy}
							onchange={(e) => toggleAiTool(tool.name, e.currentTarget.checked)}
						/>
						<span class="switch-text">{tool.enabled ? 'on' : 'off'}</span>
					</label>
				</div>
			{/each}
			<p class="settings-hint">
				Tool changes apply immediately to the next message. <code>publish</code>-category tools
				are off by default and broadcast signed events when enabled.
			</p>
		{/if}
	</div>

	<!-- Embeddings / semantic search. Status + manual sync/reindex for
	     the HNSW vector index, embedded in-process via ONNX. Counts and model
	     health come from /api/v1/embed/status; the buttons drive /embed/sync
	     and /embed/reindex. -->
	<div class="settings-group">
		<div class="settings-group-title">Embeddings</div>

		{#if !app.embeddingStatus?.enabled}
			<div class="settings-row">
				<span class="settings-label">Status</span>
				<span class="pill pill--ghost">disabled</span>
			</div>
			<p class="settings-hint">
				Semantic search (<code>~:query</code>) is off. Set
				<code>enabled = true</code> under <code>[embedding]</code> in
				<code>config.toml</code> and restart — embeddings run in-process (ONNX),
				no extra services required.
			</p>
		{:else}
			{@const e = app.embeddingStatus}
			<div class="settings-row">
				<span class="settings-label">Embeddings</span>
				<div class="status-row">
					<span class="pill {e.embedding_available ? 'pill--online' : 'pill--ghost'}">
						{e.embedding_available ? 'ready' : 'unavailable'}
					</span>
					{#if e.model}
						<span class="pill pill--ghost source-pill">{e.model}</span>
					{/if}
				</div>
			</div>

			<div class="settings-row">
				<span class="settings-label">Index</span>
				<span class="settings-value mono">
					{e.indexed_count} / {e.total_events} embedded
				</span>
			</div>

			{#if (e.stale_count ?? 0) > 0 || (e.missing_sections ?? 0) > 0}
				<div class="settings-row">
					<span class="settings-label">Pending</span>
					<span class="settings-value mono">
						{#if (e.missing_sections ?? 0) > 0}{e.missing_sections} unfetched{/if}{#if (e.missing_sections ?? 0) > 0 && (e.stale_count ?? 0) > 0}, {/if}{#if (e.stale_count ?? 0) > 0}{e.stale_count} stale{/if}
					</span>
				</div>
			{/if}

			<div class="settings-row">
				<span class="settings-label">Actions</span>
				<div class="action-row">
					<button
						class="settings-action"
						onclick={app.handleSyncEmbeddings}
						disabled={app.embeddingSyncing}
						title="Embed any events not yet in the vector index."
					>
						{app.embeddingSyncing ? 'Working…' : 'Sync'}
					</button>
					<button
						class="settings-action"
						onclick={app.handleReindexEmbeddings}
						disabled={app.embeddingSyncing}
						title="Rebuild the entire vector index from scratch."
					>
						Reindex
					</button>
				</div>
			</div>
			<p class="settings-hint">
				<strong>Sync</strong>: embed events missing from the index (also runs
				automatically every 60s).<br />
				<strong>Reindex</strong>: drop and rebuild the whole index — use after
				changing the embedding model.
			</p>
		{/if}
	</div>

	<!-- Save Settings moved to the top header. Hint kept here so
	     the explanation stays visible alongside the field group it
	     describes. -->
	<p class="settings-hint settings-hint--footer">
		Save Settings writes editor / compose / network mode / current relay set into <code>config.toml</code>. Survives restarts and is portable to another machine. <code>relays.json</code> + in-memory state stay authoritative at runtime.
	</p>

	<!-- Data / maintenance. Purge wipes the local LMDB cache and
	     re-execs the engine in place (~1 second of unavailability).
	     Relays.json, config.toml, identity ncryptsec are preserved. -->
	<div class="settings-group">
		<div class="settings-group-title">Data</div>
		<div class="settings-row">
			<span class="settings-label">Purge local cache</span>
			<button
				class="settings-action settings-action--danger"
				onclick={requestPurge}
				disabled={purgeLoading}
				title="Delete cached events, profiles, and ingest queue; engine restarts in ~1s. Useful for testing the indexer-fallback flow from a cold cache."
			>
				{purgeLoading ? 'Purging…' : 'Purge…'}
			</button>
		</div>
		<p class="settings-hint">
			Deletes the local <code>nostrdb</code> cache (events, profiles, ingest
			queue) and re-execs the engine in place. The next "Pull from your
			profile" walks the full read → indexer.default → indexer.fallback
			chain from a cold cache. <code>relays.json</code>, <code>config.toml</code>,
			and the identity ncryptsec are preserved.
		</p>
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
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.settings-header-title {
		flex: 1;
		min-width: 0;
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

	.settings-subtitle {
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		margin: 12px 0 4px;
	}

	.settings-textarea {
		width: 100%;
		box-sizing: border-box;
		font-family: var(--font-mono, monospace);
		font-size: var(--t-xs);
		line-height: 1.4;
		padding: 8px;
		border: 1px solid var(--base3);
		border-radius: 4px;
		background: var(--base0);
		color: var(--base7);
		resize: vertical;
	}

	.settings-input {
		flex: 1;
		min-width: 0;
		margin-left: 12px;
		font-family: var(--font-mono, monospace);
		font-size: var(--t-xs);
		padding: 4px 8px;
		border: 1px solid var(--base3);
		border-radius: 4px;
		background: var(--base0);
		color: var(--base7);
	}

	.ai-tool-cat {
		font-size: var(--t-2xs, 0.7rem);
		color: var(--base5);
		margin-left: 6px;
		text-transform: uppercase;
		letter-spacing: 0.04em;
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

	.settings-value {
		font-size: var(--t-xs);
		color: var(--base6);
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

	.settings-hint--footer {
		padding: 12px 16px 0;
		margin: 0;
		font-size: var(--t-xs);
		color: var(--muted);
	}

	/* Data / purge */
	.settings-action--danger {
		border-color: color-mix(in srgb, var(--state-error, var(--red)) 50%, var(--panel-border));
		color: var(--state-error, var(--red));
	}
	.settings-action--danger:hover:not([disabled]) {
		background: color-mix(in srgb, var(--state-error, var(--red)) 14%, transparent);
	}
	.settings-save {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--muted);
		cursor: pointer;
		border-radius: var(--r-sm);
		font-weight: 500;
		text-transform: none;
		letter-spacing: 0;
	}
	/* Dirty state — current in-memory settings differ from config.toml.
	   Warm tint signals "there's something to save." */
	.settings-save--dirty:not([disabled]) {
		background: color-mix(in srgb, var(--id-forked) 22%, transparent);
		border-color: var(--id-forked);
		color: var(--id-forked);
	}
	.settings-save--dirty:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-forked) 32%, transparent);
	}
	.settings-save[disabled] {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
