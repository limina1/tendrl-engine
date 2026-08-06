<script module lang="ts">
	// Cross-instance throttle for the mount-load batch: caps the heavy
	// /settings, /ai/*, /embed and /identity loads to once/second across all
	// instances, so a remount can never storm those endpoints. Belt-and-braces
	// behind the real fix (idempotent BufferStore.openBuffer).
	let lastSettingsLoadAt = 0;
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import { nip07, startNip07Watch } from '$lib/identity/nip07.svelte';
	import { trigger as triggerTip } from '$lib/wm/discovery.svelte';
	import { discovery, rearmDiscovery, rearmFeatureTours, setWalkthroughEnabled } from '$lib/wm/discovery.svelte';
	import * as api from '$lib/api';
	import { textScale, setTextScale, TEXT_SCALE_PRESETS } from '$lib/theme/text-scale.svelte';
	import type { Buffer } from '../types';
	import { commands, SCOPE_META, SCOPE_ORDER, BASE_KEYS } from '../commands';
	import { listLeaderBindings } from '../leader';
	import {
		commandPrefs,
		effectiveKeybinding,
		isCommandHidden,
		leaderOverrides,
		setHidden,
		setBinding,
		clearBinding,
		validateBinding
	} from '../command-prefs.svelte';
	import type { EditorInsertMode, SyncMode, ButtonLabels, ComposeDefaultMode } from '$lib/types';
	import ThemePicker from '$lib/components/ThemePicker.svelte';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	// Registries for the Commands / Keybindings sections, flattened from
	// the same sources the palette and leader popup run on — with the
	// user's custom-binding overrides applied, so the listing shows the
	// EFFECTIVE tree, not the defaults.
	const leaderBindings = $derived(listLeaderBindings(leaderOverrides(() => {})));

	// Keybinding capture: one row at a time flips into a readonly input
	// that records keystrokes (data-entry, so the shell's insert-mode
	// contract routes keys here). Enter saves, Esc/blur cancels,
	// Backspace pops a token, Space records the SPC leader prefix.
	let capture = $state<{ id: string; tokens: string[]; error: string | null } | null>(null);
	let captureEl: HTMLInputElement | null = $state(null);

	function startCapture(id: string) {
		capture = { id, tokens: [], error: null };
		setTimeout(() => captureEl?.focus(), 0);
	}

	function captureKeydown(e: KeyboardEvent) {
		if (!capture) return;
		e.preventDefault();
		e.stopPropagation();
		if (e.key === 'Escape') {
			capture = null;
			(e.currentTarget as HTMLElement).blur();
			return;
		}
		if (e.key === 'Enter') {
			if (capture.tokens.length > 0 && !capture.error) {
				setBinding(capture.id, capture.tokens.join(' '));
				capture = null;
				(e.currentTarget as HTMLElement).blur();
			}
			return;
		}
		if (e.key === 'Backspace') capture.tokens.pop();
		else if (e.key === ' ') capture.tokens.push('SPC');
		else if (e.key.length === 1) capture.tokens.push(e.key);
		else return; // modifiers etc. — ignore
		capture.error = capture.tokens.length > 0 ? validateBinding(capture.tokens, capture.id) : null;
	}

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

	// Mount-once setup. Wrapped in `untrack` because the body reads reactive
	// state synchronously (triggerTip reads the discovery queue; the loaders'
	// async writes land in aiSettings/identity/embedding/baseline) — without it,
	// every load's write re-runs the effect and re-fires every fetch, a cascade
	// that storms /identity, /ai/settings, /ai/prompt, /embed/status and
	// /settings until the browser refuses new connections (ERR_INSUFFICIENT_
	// RESOURCES). untrack makes it run exactly once when the buffer mounts.
	$effect(() => {
		untrack(() => {
			// Safety net: coalesce the batch to once/second across instances. The
			// real fix for the old remount storm is `BufferStore.openBuffer` being
			// idempotent (re-opening the focused buffer is a no-op); this stays as
			// cheap insurance so a future remount regression can't storm the engine.
			const now = Date.now();
			if (now - lastSettingsLoadAt < 1000) return;
			lastSettingsLoadAt = now;
			// Keep window.nostr detection live: the watcher bursts now (covers the
			// document_start inject race) and re-checks on return-to-tab via its own
			// listeners, so enabling/unlocking the extension *after* Settings is
			// open lights up the radio on its own — no effect re-run needed.
			startNip07Watch();
			// Walkthrough: opening Settings is the "two ways in" beat of the one
			// auto walk. Precondition-gated (`relevantWhen: !hasIdentity`), so it
			// self-suppresses for anyone already signed in; seen-gating stops it
			// re-nagging. Also this buffer's opt-in registry `tour`.
			triggerTip('sign-in-methods');
			// Force a fresh /identity + /settings fetch on mount so a recent engine
			// restart doesn't leave stale status from the last 30s-poll tick.
			app.refreshIdentity();
			captureSavedBaseline();
			loadAiSettings();
			loadAiPrompt();
			// Fresh embedding status so the Embeddings section reflects live health
			// + index counts, not the last 30s-poll tick.
			app.refreshEmbeddingStatus();
		});
	});

	// Inputs for engine login flow
	let ncryptsecInput = $state('');
	let passwordInput = $state('');
	// Watch-only (npub) login — the lightest way in, listed first: on a
	// phone there's no NIP-07 extension and pasting an ncryptsec is heavy.
	let npubInput = $state('');

	async function doNpubLogin() {
		const v = npubInput.trim();
		if (!v) return;
		await app.handleIdentityNpubLogin(v);
		npubInput = '';
	}
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

	<details class="settings-group">
		<summary class="settings-group-title">Engine</summary>
		<div class="settings-group-body">

		<div class="settings-row">
			<span class="settings-label">Version</span>
			<div class="status-row">
				{#if app.engineVersion}
					<span class="pill pill--ghost source-pill">v{app.engineVersion}</span>
					<span class="pill pill--online">ok</span>
				{:else}
					<span class="pill pill--ghost source-pill">connecting…</span>
				{/if}
			</div>
		</div>
		</div>
	</details>

	<details class="settings-group" open>
		<summary class="settings-group-title">Appearance</summary>
		<div class="settings-group-body">

		<div class="appearance-controls">
			<div class="appearance-field">
				<label
					class="appearance-field__label"
					for="high-contrast"
					title="Boosts text and borders over the current theme for readability. Defaults to your OS “increase contrast” setting."
				>High contrast</label>
				<label class="switch">
					<input
						id="high-contrast"
						type="checkbox"
						checked={app.highContrast}
						onchange={(e) => app.setHighContrast(e.currentTarget.checked)}
					/>
					<span class="switch-text">{app.highContrast ? 'on' : 'off'}</span>
				</label>
			</div>

			<div class="appearance-field">
				<label
					class="appearance-field__label"
					for="theme-preview"
					title="When on, hovering a theme in the picker re-skins the app live so you can preview before choosing. Off by default — the sweep of colors can be jarring."
				>Live preview</label>
				<label class="switch">
					<input
						id="theme-preview"
						type="checkbox"
						checked={app.themePreview}
						onchange={(e) => app.setThemePreview(e.currentTarget.checked)}
					/>
					<span class="switch-text">{app.themePreview ? 'on' : 'off'}</span>
				</label>
			</div>

			<div class="appearance-field">
				<span
					class="appearance-field__label"
					title="Color scheme for the whole interface. Click a theme to keep it (turn on Live preview to preview on hover). The sun/moon button in the header toggles dark ⇄ light; your choice is remembered on this device."
				>Theme</span>
				<ThemePicker
					current={app.currentTheme}
					livePreview={app.themePreview}
					oncommit={(id) => app.setTheme(id)}
				/>
			</div>
		</div>

		<div class="settings-row">
			<span class="settings-label">Text size</span>
			<div class="radio-group">
				{#each TEXT_SCALE_PRESETS as preset (preset.id)}
					<label class="radio">
						<input
							type="radio"
							name="text-scale"
							value={preset.id}
							checked={textScale.id === preset.id}
							onchange={() => setTextScale(preset.id)}
						/>
						<span>{preset.label}</span>
					</label>
				{/each}
			</div>
		</div>
		<p class="settings-hint">
			Scales the whole interface. Applies instantly and is saved on this device —
			it isn't part of the engine config and doesn't sync across machines.
		</p>
		</div>
	</details>

	<details class="settings-group" open>
		<summary class="settings-group-title">Identity</summary>
		<div class="settings-group-body">

		<div class="settings-row">
			<span class="settings-label">Status</span>
			<div class="status-row">
				<span
					class="pill {currentState === 'unlocked'
						? 'pill--online'
						: currentState === 'locked'
							? 'pill--draft'
							: currentState === 'watching'
								? 'pill--local'
								: 'pill--ghost'}"
				>{currentState}</span>
				<span class="pill pill--ghost source-pill">source: {currentSource}</span>
			</div>
		</div>

		<div class="settings-row">
			<span class="settings-label">Source</span>
			<div class="radio-group" data-tour="identity-source">
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
				<label class="radio" class:radio--disabled={!nip07.available}>
					<input
						type="radio"
						name="identity-source"
						value="nip07"
						disabled={!nip07.available}
						checked={currentSource === 'nip07'}
						onchange={() => pickSource('nip07')}
					/>
					<span>nip07{nip07.available ? '' : ' (no extension)'}</span>
				</label>
			</div>
		</div>

		{#if currentSource === 'engine' && (currentState === 'locked' || currentState === 'unlocked')}
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
			{#if currentState === 'none' || currentState === 'watching'}
				{#if currentState === 'watching'}
					<div class="settings-row settings-row--stack">
						{#if app.identityStatus?.npub}
							<span class="settings-label mono">{app.identityStatus.npub}</span>
						{/if}
						<span class="settings-hint">
							Watch-only — your feed, profile, and <strong>by:me</strong> work from this
							npub, but nothing can be signed. Paste an ncryptsec below (or connect
							NIP-07) to sign.
						</span>
						<div class="action-row">
							<button class="settings-action settings-action--danger" onclick={app.handleIdentityLogout}
								>Logout</button
							>
						</div>
					</div>
				{:else}
					<div class="settings-row settings-row--stack">
						<label class="settings-label" for="npub-input">npub (watch-only)</label>
						<input
							id="npub-input"
							class="settings-input"
							bind:value={npubInput}
							placeholder="npub1..."
							spellcheck="false"
							onkeydown={(e) => e.key === 'Enter' && doNpubLogin()}
						/>
						<button class="settings-action" onclick={doNpubLogin} disabled={app.identityLoading}>
							{app.identityLoading ? 'Working…' : 'Watch'}
						</button>
						<span class="settings-hint">
							The lightest way in: browse as yourself — feed, profile, and
							<strong>by:me</strong> scope to this npub. Signing needs a key
							(ncryptsec below, or NIP-07). Never paste an nsec here.
						</span>
					</div>
				{/if}
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
	</details>

	<details class="settings-group">
		<summary class="settings-group-title">Assistant identity</summary>
		<div class="settings-group-body">
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
	</details>

	<details class="settings-group">
		<summary class="settings-group-title">Editor</summary>
		<div class="settings-group-body">

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
	</details>

	<details class="settings-group">
		<summary class="settings-group-title">Compose</summary>
		<div class="settings-group-body">

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
	</details>

	<details class="settings-group">
		<summary class="settings-group-title">Network</summary>
		<div class="settings-group-body">

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
				title="Re-arm the mode-choice modal + first-run walkthrough for the next load (no data is touched)"
			>
				Reset…
			</button>
		</div>

		<div class="settings-row">
			<span class="settings-label">Reset feature tours</span>
			<button
				class="settings-action"
				onclick={() => {
					rearmFeatureTours();
					app.pushToast('Feature tours re-armed — each panel’s W glows again', 'info');
				}}
				title="Re-arm the on-demand panel tours (mode-line · reader · composer · search · menus) so each W chip replays in full"
			>
				Reset…
			</button>
		</div>
		<p class="settings-hint">
			<strong>Walkthrough</strong>: contextual tips that point out features the first
			time you reach them. Toggling on (re)arms them; off silences them. The mode-line
			<strong>W</strong> button replays them any time.<br />
			<strong>Reset first-run setup</strong>: re-arms the mode-choice modal + first-run walkthrough so they reappear on next load, as if freshly installed (no data is touched).<br />
			<strong>Reset feature tours</strong>: re-arms the on-demand panel walkthroughs — mode-line, reader, composer, search, and event menus — so each <strong>W</strong> chip glows and replays in full. Leaves the first-run walkthrough alone.
		</p>
		</div>
	</details>

	<!-- AI assistant: provider/model/auth channel + per-tool policy. Tool
	     toggles apply live (next agent turn); provider/model/auth persist to
	     config.toml and take effect on the next engine restart. -->
	<details class="settings-group">
		<summary class="settings-group-title">AI assistant</summary>
		<div class="settings-group-body">

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
	</details>

	<!-- Embeddings / semantic search. Status + manual sync/reindex for
	     the HNSW vector index, embedded in-process via ONNX. Counts and model
	     health come from /api/v1/embed/status; the buttons drive /embed/sync
	     and /embed/reindex. -->
	<details class="settings-group">
		<summary class="settings-group-title">Embeddings</summary>
		<div class="settings-group-body">

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
	</details>

	<!-- Save Settings moved to the top header. Hint kept here so
	     the explanation stays visible alongside the field group it
	     describes. -->
	<p class="settings-hint settings-hint--footer">
		Save Settings writes editor / compose / network mode / current relay set into <code>config.toml</code>. Survives restarts and is portable to another machine. <code>relays.json</code> + in-memory state stay authoritative at runtime.
	</p>

	<!-- Data / maintenance. Purge wipes the local LMDB cache and
	     re-execs the engine in place (~1 second of unavailability).
	     Relays.json, config.toml, identity ncryptsec are preserved. -->
	<details class="settings-group" open>
		<summary class="settings-group-title">Data</summary>
		<div class="settings-group-body">
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
	</details>

	<!-- Registry of everything runnable from the command palette, grouped
	     by what invoking it actually gets you — so "what can I act on"
	     is answerable without trawling the palette. -->
	<details class="settings-group">
		<summary class="settings-group-title">Commands (SPC :)</summary>
		<div class="settings-group-body">
		<p class="settings-hint">
			Everything the command palette offers (<kbd class="cmdreg-kb">SPC :</kbd>,
			<kbd class="cmdreg-kb">:</kbd>, or the modeline <em>commands</em> button),
			grouped by what running it gets you. The checkbox controls whether the
			command appears in the palette (its keybinding keeps working either way).
			Click a binding to rebind: press keys (Space records SPC), Enter saves,
			Esc cancels — a custom binding replaces the default. <em>deferred</em> =
			listed for discoverability but not wired up yet. Stored on this device.
		</p>
		{#each SCOPE_ORDER as scope (scope)}
			<div class="cmdreg-scope">
				<div class="cmdreg-scope-head">
					<span class="cmdreg-scope-label cmdreg-scope-label--{scope}">{SCOPE_META[scope].label}</span>
					<span class="cmdreg-scope-blurb">{SCOPE_META[scope].blurb}</span>
				</div>
				{#each commands.filter((c) => c.scope === scope) as cmd (cmd.id)}
					<div
						class="cmdreg-row"
						class:cmdreg-row--deferred={cmd.deferred}
						class:cmdreg-row--off={isCommandHidden(cmd)}
					>
						<input
							class="cmdreg-vis"
							type="checkbox"
							title="Show in the SPC : command palette"
							checked={!isCommandHidden(cmd)}
							onchange={(e) => setHidden(cmd.id, !e.currentTarget.checked)}
						/>
						<span class="cmdreg-name">{cmd.name}</span>
						<span class="cmdreg-desc">
							{cmd.description}{#if cmd.context}&nbsp;<em class="cmdreg-ctx">needs {cmd.context}</em>{/if}
						</span>
						{#if cmd.deferred}
							<span class="cmdreg-badge">deferred</span>
						{/if}
						{#if cmd.shells?.length === 1}
							<span class="cmdreg-badge cmdreg-badge--shell">{cmd.shells[0]}-only</span>
						{/if}
						{#if capture?.id === cmd.id}
							{#if capture.error}
								<span class="cmdreg-err">{capture.error}</span>
							{/if}
							<input
								class="cmdreg-capture"
								data-entry
								readonly
								bind:this={captureEl}
								value={capture.tokens.join(' ') || 'press keys…'}
								onkeydown={captureKeydown}
								onblur={() => (capture = null)}
							/>
						{:else}
							<button
								class="cmdreg-bindbtn"
								class:cmdreg-bindbtn--custom={commandPrefs.byId[cmd.id]?.keys}
								onclick={() => startCapture(cmd.id)}
								title="Rebind — press keys (Space = SPC), Enter saves, Esc cancels"
							>
								{effectiveKeybinding(cmd) ?? 'bind…'}
							</button>
							{#if commandPrefs.byId[cmd.id]?.keys}
								<button
									class="cmdreg-reset"
									onclick={() => clearBinding(cmd.id)}
									title="Remove custom binding{cmd.keybinding ? ` (default: ${cmd.keybinding})` : ''}"
								>↺</button>
							{/if}
						{/if}
					</div>
				{/each}
			</div>
		{/each}
		</div>
	</details>

	<!-- Registry of every live keybinding: the normal-mode base layer,
	     then the SPC leader tree (flattened from the same tree the
	     which-key popup runs on). -->
	<details class="settings-group">
		<summary class="settings-group-title">Keybindings</summary>
		<div class="settings-group-body">
		<p class="settings-hint">
			Keys work in normal mode (no field focused). This list shows the
			<em>effective</em> tree — custom bindings from the Commands section
			above are already applied. Base keys are reserved and can't be rebound.
		</p>
		<div class="cmdreg-scope">
			<div class="cmdreg-scope-head">
				<span class="cmdreg-scope-label">base keys</span>
				<span class="cmdreg-scope-blurb">The vim-style layer under everything.</span>
			</div>
			{#each BASE_KEYS as k (k.keys)}
				<div class="cmdreg-row">
					<kbd class="cmdreg-kb cmdreg-kb--lead">{k.keys}</kbd>
					<span class="cmdreg-desc">{k.desc}</span>
				</div>
			{/each}
		</div>
		<div class="cmdreg-scope">
			<div class="cmdreg-scope-head">
				<span class="cmdreg-scope-label">SPC leader</span>
				<span class="cmdreg-scope-blurb">The which-key tree, flattened.</span>
			</div>
			{#each leaderBindings as b (b.keys)}
				<div class="cmdreg-row" class:cmdreg-row--deferred={b.deferred}>
					<kbd class="cmdreg-kb cmdreg-kb--lead">{b.keys}</kbd>
					<span class="cmdreg-desc">{b.desc}</span>
					{#if b.deferred}
						<span class="cmdreg-badge">deferred</span>
					{/if}
					<span class="cmdreg-kind">{b.kind}</span>
				</div>
			{/each}
		</div>
		</div>
	</details>
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
		border-bottom: 1px solid var(--panel-border);
	}

	/* Collapsed section header — the whole row is the click target. The
	   ::before chevron is the disclosure affordance (native marker hidden);
	   it rotates when the <details> is open. */
	.settings-group-title {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 12px 16px;
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		cursor: pointer;
		user-select: none;
		list-style: none;
	}
	.settings-group-title::-webkit-details-marker {
		display: none;
	}
	.settings-group-title::before {
		content: '▸';
		font-size: 0.85em;
		line-height: 1;
		color: var(--base5);
		transition: transform 0.12s ease;
	}
	.settings-group[open] > .settings-group-title::before {
		transform: rotate(90deg);
	}
	.settings-group-title:hover {
		color: var(--fg);
		background: color-mix(in srgb, var(--fg) 4%, transparent);
	}
	.settings-group[open] > .settings-group-title {
		color: var(--base6);
	}

	.settings-group-body {
		padding: 2px 16px 12px;
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
		/* Left-aligned, not space-between: pushing the control to the far
		   panel edge opened a wide gulf between a label and the toggle it
		   drives. Now the label sits in a fixed column and its control sits
		   immediately to the right, so pairs stay close and controls line
		   up vertically down the panel. */
		justify-content: flex-start;
		padding: 6px 0;
		gap: 14px;
	}

	.settings-label {
		font-size: var(--t-sm);
		color: var(--fg);
	}

	/* Appearance toggles laid out on one horizontal line (wraps on narrow
	   panels) instead of stacked rows. Each field is its own label+control
	   cluster. */
	.appearance-controls {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 10px 24px;
		padding: 6px 0;
	}
	.appearance-field {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.appearance-field__label {
		font-size: var(--t-sm);
		color: var(--fg);
		white-space: nowrap;
	}

	/* Fixed label column for the standard (non-stacked) rows so every
	   control starts at the same x. Stacked rows (login forms) keep their
	   full-width column layout — a fixed basis there would wrongly size the
	   label's height in the column flow. */
	.settings-row:not(.settings-row--stack) > .settings-label {
		flex: 0 0 12rem;
		min-width: 0;
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

	/* --- Commands / Keybindings registries --- */
	.cmdreg-scope {
		margin: 10px 0 14px;
	}
	.cmdreg-scope-head {
		display: flex;
		align-items: baseline;
		gap: 10px;
		margin-bottom: 4px;
	}
	.cmdreg-scope-label {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		flex-shrink: 0;
	}
	.cmdreg-scope-blurb {
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.cmdreg-row {
		display: flex;
		align-items: baseline;
		gap: 8px;
		padding: 3px 0 3px 10px;
		border-left: 2px solid var(--base2);
		font-size: var(--t-sm);
	}
	.cmdreg-row--deferred {
		opacity: 0.5;
	}
	.cmdreg-name {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		flex-shrink: 0;
	}
	.cmdreg-desc {
		color: var(--fg-alt);
		font-size: var(--t-xs);
		flex: 1;
		min-width: 0;
	}
	.cmdreg-ctx {
		color: var(--base5);
		font-style: italic;
	}
	.cmdreg-badge {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		border: 1px dashed var(--base3);
		border-radius: var(--r-sm);
		padding: 0 5px;
		flex-shrink: 0;
	}
	.cmdreg-badge--shell {
		border-style: solid;
		color: var(--fg-alt);
	}
	.cmdreg-kb {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--base6);
		padding: 0 5px;
		border: 1px solid var(--base2);
		border-radius: var(--r-sm);
		background: var(--base0);
		flex-shrink: 0;
		white-space: nowrap;
	}
	.cmdreg-kb--lead {
		min-width: 90px;
		text-align: left;
	}
	.cmdreg-kind {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--base4);
		flex-shrink: 0;
	}
	.cmdreg-vis {
		flex-shrink: 0;
		margin: 0;
		accent-color: var(--id-yours);
		cursor: pointer;
	}
	.cmdreg-row--off .cmdreg-name,
	.cmdreg-row--off .cmdreg-desc {
		opacity: 0.45;
	}
	.cmdreg-bindbtn {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--base6);
		padding: 0 5px;
		border: 1px solid var(--base2);
		border-radius: var(--r-sm);
		background: var(--base0);
		flex-shrink: 0;
		white-space: nowrap;
		cursor: pointer;
	}
	.cmdreg-bindbtn:hover {
		border-color: var(--base4);
	}
	.cmdreg-bindbtn--custom {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.cmdreg-reset {
		font-size: var(--t-xs);
		color: var(--base5);
		background: none;
		border: none;
		padding: 0 2px;
		cursor: pointer;
		flex-shrink: 0;
	}
	.cmdreg-reset:hover {
		color: var(--fg);
	}
	.cmdreg-capture {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--id-yours);
		width: 130px;
		padding: 0 5px;
		border: 1px solid var(--id-yours);
		border-radius: var(--r-sm);
		background: var(--base0);
		flex-shrink: 0;
		caret-color: transparent;
	}
	.cmdreg-err {
		font-size: var(--t-2xs);
		color: var(--id-forked);
		flex-shrink: 0;
	}
</style>
