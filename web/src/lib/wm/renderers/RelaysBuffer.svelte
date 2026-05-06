<script lang="ts">
	import * as api from '$lib/api';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	type RelayRow = {
		url: string;
		read: boolean;
		write: boolean;
		auth: boolean; // placeholder — engine doesn't track this yet
	};

	let rows = $state<RelayRow[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	// Fetch the engine's three relay sets and unify them into a single list
	// keyed by URL. Read = appears in `general` or `fetch`; write = appears
	// in `general` or `publish`. Auth is a placeholder (no engine support).
	async function load() {
		loading = true;
		try {
			const cfg = await api.getRelayConfig();
			const map = new Map<string, RelayRow>();
			const ensure = (url: string): RelayRow => {
				let r = map.get(url);
				if (!r) {
					r = { url, read: false, write: false, auth: false };
					map.set(url, r);
				}
				return r;
			};
			for (const url of cfg.general?.urls ?? []) {
				const r = ensure(url);
				r.read = true;
				r.write = true;
			}
			for (const url of cfg.fetch?.urls ?? []) ensure(url).read = true;
			for (const url of cfg.publish?.urls ?? []) ensure(url).write = true;
			rows = [...map.values()].sort((a, b) => a.url.localeCompare(b.url));
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
	});

	function toggle(url: string, field: 'read' | 'write' | 'auth') {
		// Placeholder — local state only. Real persistence will go through
		// /api/v1/config/update with the new relay-set membership rules.
		rows = rows.map((r) => (r.url === url ? { ...r, [field]: !r[field] } : r));
	}

	function shorten(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}
</script>

<div class="relays-view">
	<div class="relays-header">
		<span>Relay configuration</span>
		<span class="relays-hint">placeholder — toggles don't persist yet</span>
	</div>

	{#if loading}
		<p class="empty">Loading…</p>
	{:else if error}
		<p class="empty error">{error}</p>
	{:else if rows.length === 0}
		<p class="empty">No relays configured</p>
	{:else}
		<div class="relays-grid">
			<div class="grid-head">
				<span>relay</span>
				<span class="col-toggle">read</span>
				<span class="col-toggle">write</span>
				<span class="col-toggle">auth</span>
			</div>
			{#each rows as row (row.url)}
				<div class="grid-row">
					<span class="relay-url" title={row.url}>{shorten(row.url)}</span>
					<label class="col-toggle"
						><input
							type="checkbox"
							checked={row.read}
							onchange={() => toggle(row.url, 'read')}
						/></label
					>
					<label class="col-toggle"
						><input
							type="checkbox"
							checked={row.write}
							onchange={() => toggle(row.url, 'write')}
						/></label
					>
					<label class="col-toggle"
						><input
							type="checkbox"
							checked={row.auth}
							onchange={() => toggle(row.url, 'auth')}
							title="Some relays (e.g. paid / private) require NIP-42 auth — placeholder"
						/></label
					>
				</div>
			{/each}
		</div>

		<div class="relays-footer">
			<button class="btn-add" disabled title="Will prompt for a relay URL">+ Add relay</button>
			<button class="btn-refresh" onclick={load}>Refresh</button>
		</div>
	{/if}
</div>

<style>
	.relays-view {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 0 0 24px;
	}

	.relays-header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}

	.relays-hint {
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
		color: var(--base5);
		font-style: italic;
	}

	.empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.empty.error {
		color: var(--id-draft);
	}

	.relays-grid {
		display: flex;
		flex-direction: column;
		padding: 6px 0;
	}

	.grid-head,
	.grid-row {
		display: grid;
		grid-template-columns: 1fr 60px 60px 60px;
		gap: 8px;
		align-items: center;
		padding: 6px 14px;
	}

	.grid-head {
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		border-bottom: 1px solid var(--panel-border);
		padding-bottom: 8px;
		margin-bottom: 4px;
	}

	.grid-row {
		font-size: var(--t-sm);
		border-bottom: 1px solid var(--panel-border);
	}
	.grid-row:hover {
		background: var(--bg-surface);
	}

	.relay-url {
		font-family: var(--font-mono);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.col-toggle {
		display: flex;
		justify-content: center;
		align-items: center;
		cursor: pointer;
	}

	.relays-footer {
		display: flex;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
		margin-top: 8px;
	}
	.btn-add,
	.btn-refresh {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
	}
	.btn-add[disabled] {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
