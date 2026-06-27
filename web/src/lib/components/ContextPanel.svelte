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
		onlocksource,
		oncrosspanelcopy,
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
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
		onsenditemtocompose: (id: string) => void;
	} = $props();

	let checkedIds: Set<string> = $state(new Set());
	// "Peek" model: pristine items render as a compact snippet you can skim;
	// click to expand into the editable textarea. Modified items always expand
	// (you're working on them). Tracks the *explicitly* expanded ids.
	let expandedIds: Set<string> = $state(new Set());
	let trashPending: ContextItem[] = $state([]);
	let trashTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let trashCountdown = $state(0);
	let countdownInterval: ReturnType<typeof setInterval> | null = $state(null);

	function togglePeek(id: string) {
		const next = new Set(expandedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		expandedIds = next;
	}

	function snippet(text: string): string {
		const t = text.trim().replace(/\s+/g, ' ');
		return t.length > 160 ? t.slice(0, 160) + '…' : t || '(empty)';
	}

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
			{@const contextModified = entry.context_content !== entry.original_content}
			{@const expanded = expandedIds.has(entry.id) || contextModified}
			<div class="context-card" class:modified={contextModified}>
				<div class="card-header">
					<label class="check">
						<input
							type="checkbox"
							checked={checkedIds.has(entry.id)}
							onchange={() => toggleCheck(entry.id)}
						/>
					</label>
					<button
						class="peek-toggle"
						onclick={() => togglePeek(entry.id)}
						disabled={contextModified}
						title={contextModified ? 'Modified — always expanded' : expanded ? 'Collapse' : 'Peek / expand'}
						aria-expanded={expanded}
					>{expanded ? '▾' : '▸'}</button>
					<input
						class="card-title"
						value={entry.title}
						oninput={(e) => onupdate(entry.id, e.currentTarget.value, entry.context_content)}
						placeholder="Title"
						disabled={disabled || entry.readonly}
					/>
					<ItemBadge item={entry} {syncMode} panel="context" {ontogglereadonly} {onlocksource} {oncrosspanelcopy} />
					<button class="icon-btn-sm" onclick={() => onsenditemtocompose(entry.id)} disabled={disabled} title="Send to compose">□</button>
					<button class="remove-btn" onclick={() => onremove(entry.id)} {disabled}>×</button>
				</div>
				{#if expanded}
					<textarea
						value={entry.context_content}
						oninput={(e) => onupdate(entry.id, entry.title, e.currentTarget.value)}
						placeholder="Content..."
						rows="3"
						disabled={disabled || entry.readonly}
					></textarea>
					{#if contextModified}
						<div class="modified-banner">
							<span>Modified</span>
							<button class="reset-btn" onclick={() => onreset(entry.id)} {disabled}>Reset</button>
						</div>
					{/if}
				{:else}
					<button
						class="peek-snippet"
						onclick={() => togglePeek(entry.id)}
						title="Peek / expand"
					>{snippet(entry.context_content)}</button>
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
		font-size: var(--t-2xs);
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
		font-size: var(--t-3xs);
		padding: 1px 7px;
		border-radius: 10px;
	}

	.header-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.sel-btn {
		font-size: var(--t-3xs);
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		font-size: var(--t-xs);
		padding: 4px 8px;
		min-width: 28px;
	}

	.trash-btn {
		font-size: var(--t-2xs);
	}

	.trash-armed {
		background: var(--danger-strong);
		border-color: var(--danger-strong);
		color: white;
	}

	.trash-warn {
		font-size: var(--t-3xs);
		color: var(--danger-strong);
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
		font-size: var(--t-xs);
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

	.peek-toggle {
		padding: 2px 4px;
		font-size: var(--t-3xs);
		min-width: 18px;
		line-height: 1;
		color: var(--fg-muted);
	}

	.peek-snippet {
		text-align: left;
		width: 100%;
		font-size: var(--t-2xs);
		line-height: 1.4;
		color: var(--fg-muted);
		background: transparent;
		border: 1px dashed var(--border);
		border-radius: var(--radius);
		padding: 6px 8px;
		cursor: pointer;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.peek-snippet:hover {
		border-color: var(--accent);
		color: var(--fg);
	}

	.icon-btn-sm {
		padding: 2px 6px;
		font-size: var(--t-2xs);
		min-width: 22px;
	}

	.remove-btn {
		padding: 2px 8px;
		font-size: var(--t-xs);
		line-height: 1;
	}

	textarea {
		width: 100%;
		font-size: var(--t-2xs);
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
		font-size: var(--t-2xs);
		font-weight: 600;
		border: 1px solid var(--modified-border);
	}

	.reset-btn {
		font-size: var(--t-3xs);
		padding: 2px 8px;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 16px;
		font-size: var(--t-2xs);
	}
</style>
