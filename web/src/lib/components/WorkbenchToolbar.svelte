<script lang="ts">
	import type { SyncMode, ButtonLabels, EmbeddingStatusResponse, NetworkStatus, NetworkMode } from '$lib/types';

	let {
		syncMode,
		buttonLabels,
		embeddingStatus = null,
		embeddingSyncing = false,
		ignoredCount = 0,
		networkStatus = null,
		onsetsyncmode,
		onsetbuttonlabels,
		onhome,
		onsyncembeddings,
		onreindexembeddings,
		onviewignored,
		onpurge,
		passthrough = false,
		onsetpassthrough,
		onsetnetworkmode,
		onexport,
		exporting = false,
		onimport,
		importing = false,
		importProgress = null
	}: {
		syncMode: SyncMode;
		buttonLabels: ButtonLabels;
		embeddingStatus?: EmbeddingStatusResponse | null;
		embeddingSyncing?: boolean;
		ignoredCount?: number;
		networkStatus?: NetworkStatus | null;
		onsetsyncmode: (mode: SyncMode) => void;
		onsetbuttonlabels: (mode: ButtonLabels) => void;
		onhome?: () => void;
		onsyncembeddings?: () => void;
		onreindexembeddings?: () => void;
		onviewignored?: () => void;
		onpurge?: () => void;
		passthrough?: boolean;
		onsetpassthrough?: (v: boolean) => void;
		onsetnetworkmode?: (mode: NetworkMode) => void;
		onexport?: () => void;
		exporting?: boolean;
		onimport?: (file: File) => void;
		importing?: boolean;
		importProgress?: { total: number; sent: number; ingested: number; skipped: number; errors: number; done: boolean } | null;
	} = $props();

	let settingsOpen = $state(false);
	let networkLogOpen = $state(false);

	function toggleNetworkMode() {
		if (!networkStatus || !onsetnetworkmode) return;
		onsetnetworkmode(networkStatus.mode === 'online' ? 'offline' : 'online');
	}

	function shortRelay(url: string): string {
		return url.replace('wss://', '').replace('ws://', '').replace(/\/$/, '');
	}

	function timeAgo(ts: number): string {
		if (!ts) return '';
		const diff = Math.floor(Date.now() / 1000) - ts;
		if (diff < 60) return `${diff}s ago`;
		if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
		if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
		return `${Math.floor(diff / 86400)}d ago`;
	}
</script>

<div class="workbench-toolbar">
	<button class="workbench-title" onclick={onhome}>tendrl</button>
	{#if networkStatus}
		<button
			class="network-mode-btn"
			class:offline={networkStatus.mode === 'offline'}
			onclick={toggleNetworkMode}
			title={networkStatus.mode === 'online' ? 'Online — click to go offline' : 'Offline — click to go online'}
		>
			{networkStatus.mode === 'online' ? '●' : '○'}
			<span class="mode-label">{networkStatus.mode}</span>
		</button>
		{#if networkStatus.active_fetches > 0}
			<button class="fetch-indicator pulse" onclick={() => (networkLogOpen = !networkLogOpen)} title="Active relay fetches">
				↓{networkStatus.active_fetches}
			</button>
		{:else if networkStatus.recent.length > 0}
			<button class="fetch-indicator" onclick={() => (networkLogOpen = !networkLogOpen)} title="Recent fetch activity">
				↓
			</button>
		{/if}
	{/if}
	{#if ignoredCount > 0}
		<button class="ignored-btn" onclick={onviewignored} title="{ignoredCount} events/authors hidden">{ignoredCount} hidden</button>
	{/if}
	<span class="spacer"></span>
	{#if embeddingStatus?.enabled}
		{@const pct = embeddingStatus.total_events > 0 ? Math.round((embeddingStatus.indexed_count / embeddingStatus.total_events) * 100) : 0}
		<span class="embed-status" class:offline={!embeddingStatus.sidecar_available}>
			{#if !embeddingStatus.sidecar_available}
				embed offline
			{:else}
				{embeddingStatus.indexed_count}/{embeddingStatus.total_events}{#if embeddingStatus.stale_count > 0} ({embeddingStatus.stale_count} stale){/if}{#if embeddingStatus.missing_sections > 0} +{embeddingStatus.missing_sections} unfetched{/if}
			{/if}
		</span>
		{#if embeddingSyncing}
			<div class="embed-progress">
				<div class="embed-progress-bar" style:width="{pct}%"></div>
			</div>
		{/if}
		<button class="embed-sync-btn" onclick={onsyncembeddings} disabled={embeddingSyncing} title="Embed new events">
			{embeddingSyncing ? '...' : '↻'}
		</button>
		<button class="embed-sync-btn reindex-btn" onclick={() => { if (confirm('Clear embedding index and re-embed all events?')) onreindexembeddings?.(); }} disabled={embeddingSyncing} title="Reindex all embeddings">
			Re
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
		<span class="settings-spacer"></span>
		<label class="settings-check">
			<input type="checkbox" checked={passthrough} onchange={() => onsetpassthrough?.(!passthrough)} />
			<span>Passthrough</span>
		</label>
		<button class="settings-btn export-btn" onclick={onexport} disabled={exporting} title="Export all events as JSONL">
			{exporting ? 'Exporting...' : 'Export JSONL'}
		</button>
		<input type="file" accept=".jsonl,.ndjson" id="import-file" style="display:none"
			onchange={(e) => { const f = (e.target as HTMLInputElement).files?.[0]; if (f) onimport?.(f); (e.target as HTMLInputElement).value = ''; }} />
		<button class="settings-btn import-btn" onclick={() => document.getElementById('import-file')?.click()} disabled={importing} title="Import events from JSONL file">
			{importing ? 'Importing...' : 'Import JSONL'}
		</button>
		{#if importProgress}
			<div class="import-progress" title="+{importProgress.ingested} ingested, {importProgress.skipped} skipped, {importProgress.errors} errors">
				<div class="import-bar-track">
					<div class="import-bar-fill" style="width: {importProgress.total ? (importProgress.sent / importProgress.total * 100) : 0}%"></div>
				</div>
				<span class="import-stats">
					{importProgress.sent}/{importProgress.total}
					{#if importProgress.done}
						— +{importProgress.ingested}, {importProgress.skipped} skipped
					{/if}
				</span>
			</div>
		{/if}
		<button class="settings-btn purge-btn" onclick={onpurge}>Purge DB</button>
	</div>
{/if}

{#if networkLogOpen && networkStatus?.recent?.length}
	<div class="network-log">
		<div class="network-log-header">
			<span>Fetch activity — {networkStatus.total_events_fetched} events fetched total</span>
			{#if networkStatus.last_fetch_timestamp}
				<span class="log-time">last: {timeAgo(networkStatus.last_fetch_timestamp)}</span>
			{/if}
		</div>
		<div class="network-log-entries">
			{#each networkStatus.recent as record}
				<div class="log-entry" class:error={!record.success}>
					<span class="log-relay">{shortRelay(record.relay)}</span>
					<span class="log-summary">{record.filter_summary}</span>
					<span class="log-count">{record.event_count}</span>
					<span class="log-duration">{record.duration_ms}ms</span>
					<span class="log-trigger">{record.trigger}</span>
					{#if record.error}
						<span class="log-error">{record.error}</span>
					{/if}
				</div>
			{/each}
		</div>
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

	/* Network mode */
	.network-mode-btn {
		display: flex;
		align-items: center;
		gap: 3px;
		font-size: 0.65rem;
		padding: 2px 6px;
		margin-left: 8px;
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		cursor: pointer;
		border-radius: var(--radius);
	}

	.network-mode-btn:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}

	.network-mode-btn:not(.offline) {
		color: #22c55e;
		border-color: #22c55e40;
	}

	.network-mode-btn.offline {
		color: var(--fg-muted);
		border-color: var(--border);
	}

	.mode-label {
		text-transform: uppercase;
		font-weight: 600;
		letter-spacing: 0.04em;
	}

	/* Fetch indicator */
	.fetch-indicator {
		font-size: 0.65rem;
		padding: 2px 5px;
		margin-left: 4px;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-family: monospace;
	}

	.fetch-indicator:hover {
		color: var(--fg);
	}

	.fetch-indicator.pulse {
		color: var(--accent);
		animation: pulse 1.2s ease-in-out infinite;
	}

	@keyframes pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.4; }
	}

	.ignored-btn {
		font-size: 0.65rem;
		color: #ef4444;
		opacity: 0.7;
		margin-left: 8px;
		background: none;
		border: none;
		cursor: pointer;
		padding: 2px 4px;
	}

	.ignored-btn:hover {
		opacity: 1;
		text-decoration: underline;
	}

	.settings-spacer {
		flex: 1;
	}

	.export-btn {
		color: var(--accent) !important;
		border-color: var(--accent) !important;
	}

	.export-btn:disabled, .import-btn:disabled {
		opacity: 0.5;
	}

	.import-btn {
		color: var(--accent) !important;
		border-color: var(--accent) !important;
	}

	.import-progress {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 120px;
	}

	.import-bar-track {
		flex: 1;
		height: 4px;
		background: var(--border, #333);
		border-radius: 2px;
		overflow: hidden;
		min-width: 60px;
	}

	.import-bar-fill {
		height: 100%;
		background: var(--accent, #4ade80);
		border-radius: 2px;
		transition: width 0.15s ease;
	}

	.import-stats {
		font-size: 0.6rem;
		opacity: 0.7;
		white-space: nowrap;
	}

	.purge-btn {
		color: #ef4444 !important;
		border-color: #ef4444 !important;
	}

	.settings-check {
		display: flex;
		align-items: center;
		gap: 4px;
		font-size: 0.65rem;
		color: var(--fg-muted);
		cursor: pointer;
	}

	.settings-check input {
		margin: 0;
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

	.reindex-btn {
		color: #ef4444;
		border-color: #ef4444;
		font-size: 0.6rem;
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

	/* Network activity log */
	.network-log {
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		font-family: monospace;
		font-size: 0.6rem;
		max-height: 200px;
		overflow-y: auto;
	}

	.network-log-header {
		display: flex;
		justify-content: space-between;
		padding: 4px 16px;
		color: var(--fg-muted);
		font-weight: 600;
		border-bottom: 1px solid var(--border);
	}

	.log-time {
		font-weight: 400;
	}

	.network-log-entries {
		padding: 2px 16px;
	}

	.log-entry {
		display: flex;
		gap: 8px;
		padding: 1px 0;
		color: var(--fg-muted);
	}

	.log-entry.error {
		color: #ef4444;
	}

	.log-relay {
		min-width: 120px;
		color: var(--fg);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.log-summary {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.log-count {
		min-width: 30px;
		text-align: right;
	}

	.log-duration {
		min-width: 50px;
		text-align: right;
	}

	.log-trigger {
		min-width: 80px;
		color: var(--fg-muted);
		opacity: 0.7;
	}

	.log-error {
		color: #ef4444;
		flex: 1;
	}
</style>
