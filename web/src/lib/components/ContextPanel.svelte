<script lang="ts">
	import type { ContextItem, SyncMode } from '$lib/types';
	import ItemBadge from './ItemBadge.svelte';

	let {
		entries,
		disabled = false,
		syncMode,
		onupdate,
		onreset,
		onremove,
		onsendtocompose,
		ondelete,
		ondeletepermanent,
		ontogglereadonly,
		onsenditemtocompose
	}: {
		entries: ContextItem[];
		disabled?: boolean;
		syncMode: SyncMode;
		onupdate: (id: string, title: string, content: string) => void;
		onreset: (id: string) => void;
		onremove: (id: string) => void;
		onsendtocompose: (items: ContextItem[]) => void;
		ondelete: (items: ContextItem[]) => void;
		ondeletepermanent: (items: ContextItem[]) => void;
		ontogglereadonly: (id: string) => void;
		onsenditemtocompose: (id: string) => void;
	} = $props();

	let checkedIds: Set<string> = $state(new Set());
	let trashPending: ContextItem[] = $state([]);
	let trashTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let trashCountdown = $state(0);
	let countdownInterval: ReturnType<typeof setInterval> | null = $state(null);

	function toggleCheck(id: string) {
		const next = new Set(checkedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedIds = next;
		clearTrash();
	}

	function selectAll() {
		checkedIds = new Set(entries.map((e) => e.id));
	}

	function invertSelection() {
		const next = new Set<string>();
		for (const e of entries) {
			if (!checkedIds.has(e.id)) next.add(e.id);
		}
		checkedIds = next;
	}

	function sendChecked() {
		const items = entries.filter((e) => checkedIds.has(e.id));
		if (items.length > 0) {
			onsendtocompose(items);
			checkedIds = new Set();
		}
		clearTrash();
	}

	function clearTrash() {
		trashPending = [];
		trashCountdown = 0;
		if (trashTimer) clearTimeout(trashTimer);
		trashTimer = null;
		if (countdownInterval) clearInterval(countdownInterval);
		countdownInterval = null;
	}

	function handleTrash() {
		if (trashPending.length > 0) {
			ondeletepermanent(trashPending);
			checkedIds = new Set();
			clearTrash();
			return;
		}
		const items = entries.filter((e) => checkedIds.has(e.id));
		if (items.length === 0) return;
		ondelete(items);
		trashPending = items;
		checkedIds = new Set();
		trashCountdown = 10;
		trashTimer = setTimeout(clearTrash, 10000);
		countdownInterval = setInterval(() => {
			trashCountdown--;
			if (trashCountdown <= 0) clearTrash();
		}, 1000);
	}

	const trashActive = $derived(trashPending.length > 0);
</script>

<div class="context-panel">
	<div class="context-header">
		<span class="context-label">
			Context
			{#if entries.length > 0}
				<span class="badge">{entries.length}</span>
			{/if}
		</span>
		<div class="header-actions">
			<button class="sel-btn" onclick={selectAll} disabled={disabled || entries.length === 0} title="Select all">All</button>
			<button class="sel-btn" onclick={invertSelection} disabled={disabled || entries.length === 0} title="Invert selection">Inv</button>
			<button
				class="icon-btn"
				onclick={sendChecked}
				disabled={disabled || checkedIds.size === 0}
				title="Send to compose"
			>□</button>
			<button
				class="icon-btn trash-btn"
				class:trash-armed={trashActive}
				onclick={handleTrash}
				disabled={disabled || (checkedIds.size === 0 && !trashActive)}
				title={trashActive ? 'Delete everywhere' : 'Remove from context'}
			>🗑</button>
			{#if trashActive}
				<span class="trash-warn" style:opacity={trashCountdown / 10}>delete everywhere ({trashCountdown}s)</span>
			{/if}
		</div>
	</div>

	<div class="context-list">
		{#each entries as entry (entry.id)}
			<div class="context-card" class:modified={entry.modified}>
				<div class="card-header">
					<label class="check">
						<input
							type="checkbox"
							checked={checkedIds.has(entry.id)}
							onchange={() => toggleCheck(entry.id)}
						/>
					</label>
					<input
						class="card-title"
						value={entry.title}
						oninput={(e) => onupdate(entry.id, e.currentTarget.value, entry.content)}
						placeholder="Title"
						disabled={disabled || entry.readonly}
					/>
					<ItemBadge item={entry} {syncMode} panel="context" />
					<button class="icon-btn-sm" onclick={() => onsenditemtocompose(entry.id)} disabled={disabled} title="Send to compose">□</button>
					<button class="icon-btn-sm" onclick={() => ontogglereadonly(entry.id)} disabled={disabled} title={entry.readonly ? 'Unlock' : 'Lock'}>{entry.readonly ? '🔓' : '🔒'}</button>
					<button class="remove-btn" onclick={() => onremove(entry.id)} {disabled}>×</button>
				</div>
				<textarea
					value={entry.content}
					oninput={(e) => onupdate(entry.id, entry.title, e.currentTarget.value)}
					placeholder="Content..."
					rows="3"
					disabled={disabled || entry.readonly}
				></textarea>
				{#if entry.modified}
					<div class="modified-banner">
						<span>Modified</span>
						<button class="reset-btn" onclick={() => onreset(entry.id)} {disabled}>Reset</button>
					</div>
				{/if}
			</div>
		{/each}

		{#if entries.length === 0}
			<p class="empty">Add context from search results</p>
		{/if}
	</div>
</div>

<style>
	.context-panel {
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		max-height: 300px;
		display: flex;
		flex-direction: column;
	}

	.context-header {
		padding: 8px 16px;
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.context-label {
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.badge {
		background: var(--accent);
		color: white;
		font-size: 0.7rem;
		padding: 1px 7px;
		border-radius: 10px;
	}

	.header-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.sel-btn {
		font-size: 0.65rem;
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		font-size: 0.85rem;
		padding: 4px 8px;
		min-width: 28px;
	}

	.trash-btn {
		font-size: 0.75rem;
	}

	.trash-armed {
		background: #dc2626;
		border-color: #dc2626;
		color: white;
	}

	.trash-warn {
		font-size: 0.7rem;
		color: #dc2626;
		font-weight: 600;
		white-space: nowrap;
	}

	.context-list {
		flex: 1;
		overflow-y: auto;
		padding: 0 16px 8px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.context-card {
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 8px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.context-card.modified {
		border-color: var(--modified-border);
		background: var(--modified-bg);
	}

	.card-header {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.check {
		display: flex;
		align-items: center;
	}

	.card-title {
		flex: 1;
		font-family: inherit;
		font-size: 0.85rem;
		font-weight: 600;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 4px 8px;
		outline: none;
	}

	.card-title:focus {
		border-color: var(--accent);
	}

	.icon-btn-sm {
		padding: 2px 6px;
		font-size: 0.75rem;
		min-width: 22px;
	}

	.remove-btn {
		padding: 2px 8px;
		font-size: 0.85rem;
		line-height: 1;
	}

	textarea {
		width: 100%;
		font-size: 0.8rem;
		line-height: 1.4;
	}

	.modified-banner {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 8px;
		border-radius: 4px;
		background: var(--modified-bg);
		color: var(--modified-fg);
		font-size: 0.75rem;
		font-weight: 600;
		border: 1px solid var(--modified-border);
	}

	.reset-btn {
		font-size: 0.7rem;
		padding: 2px 8px;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 16px;
		font-size: 0.8rem;
	}
</style>
