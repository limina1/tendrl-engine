<script lang="ts">
	import type { SearchResult, EditorInsertMode } from '$lib/types';

	let {
		result,
		insertMode,
		onclose,
		onread,
		onfindcontaining,
		oninsert,
		onopensettings
	}: {
		result: SearchResult;
		insertMode: EditorInsertMode;
		onclose: () => void;
		onread: (r: SearchResult) => void;
		onfindcontaining: (r: SearchResult) => void;
		oninsert: (r: SearchResult, mode: EditorInsertMode) => void;
		onopensettings: () => void;
	} = $props();

	type Action = {
		key: 'read' | 'find' | 'insert';
		label: string;
		hint?: string;
		shortcut: string;
	};

	const readLabel = $derived(
		result.kind === 30041
			? 'Read section'
			: result.kind === 30040
				? 'Read publication'
				: 'Read event'
	);
	const readHint = $derived(
		result.kind === 30041
			? 'open just this section'
			: result.kind === 30040
				? 'open the full publication'
				: 'open in reader'
	);

	const actions: Action[] = $derived([
		{ key: 'read', label: readLabel, hint: readHint, shortcut: 'r' },
		{ key: 'find', label: 'Find containing publications', hint: 'parent index / collection', shortcut: 'f' },
		{
			key: 'insert',
			label: 'Insert into compose',
			hint: insertMode === 'cursor' ? 'at cursor (plain mode)' : 'append at bottom',
			shortcut: 'i'
		}
	]);

	let cursor = $state(0);

	function fire(action: Action) {
		if (action.key === 'read') onread(result);
		else if (action.key === 'find') onfindcontaining(result);
		else if (action.key === 'insert') oninsert(result, insertMode);
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			onclose();
			return;
		}
		if (e.key === 'ArrowDown' || e.key === 'j') {
			e.preventDefault();
			cursor = Math.min(actions.length - 1, cursor + 1);
			return;
		}
		if (e.key === 'ArrowUp' || e.key === 'k') {
			e.preventDefault();
			cursor = Math.max(0, cursor - 1);
			return;
		}
		if (e.key === 'Enter') {
			e.preventDefault();
			fire(actions[cursor]);
			return;
		}
		// Letter shortcuts
		const direct = actions.find((a) => a.shortcut === e.key.toLowerCase());
		if (direct) {
			e.preventDefault();
			fire(direct);
		}
	}

	const KIND_LABEL: Record<number, string> = {
		30040: 'index',
		30041: 'section',
		1: 'note'
	};

	function onMount(el: HTMLDivElement) {
		// Focus container so keys land here.
		el.focus();
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="action-modal-backdrop" onclick={onclose} role="presentation">
	<div
		class="action-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-label="Event actions"
		tabindex="-1"
		{onkeydown}
		use:onMount
	>
		<div class="action-modal-header">
			<span class="action-modal-title">{result.title ?? '[Untitled]'}</span>
			<span class="kind-badge">{KIND_LABEL[result.kind] ?? result.kind}</span>
		</div>

		<div class="action-modal-list">
			{#each actions as action, i (action.key)}
				<button
					class="action-row"
					class:cursor={i === cursor}
					onclick={() => fire(action)}
					onmouseenter={() => (cursor = i)}
				>
					<span class="action-shortcut">{action.shortcut}</span>
					<span class="action-label">{action.label}</span>
					{#if action.hint}
						<span class="action-hint">{action.hint}</span>
					{/if}
				</button>
			{/each}
		</div>

		<div class="action-modal-footer">
			<button class="footer-btn" onclick={onopensettings}>Configure → settings</button>
			<span class="footer-hint">Esc to close · Enter to confirm</span>
		</div>
	</div>
</div>

<style>
	.action-modal-backdrop {
		position: fixed;
		inset: 0;
		z-index: 110;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.action-modal {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: 90vw;
		max-width: 480px;
		display: flex;
		flex-direction: column;
		outline: none;
	}

	.action-modal-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.action-modal-title {
		flex: 1;
		font-weight: 600;
		font-size: 0.9rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.kind-badge {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
	}

	.action-modal-list {
		display: flex;
		flex-direction: column;
		padding: 6px 0;
	}

	.action-row {
		display: flex;
		align-items: baseline;
		gap: 10px;
		padding: 8px 14px;
		background: none;
		border: none;
		text-align: left;
		font-size: 0.85rem;
		color: var(--fg);
		cursor: pointer;
	}

	.action-row.cursor {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		box-shadow: inset 4px 0 0 var(--id-yours);
	}

	.action-shortcut {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		min-width: 14px;
		color: var(--fg-muted);
	}

	.action-label {
		flex: 1;
	}

	.action-hint {
		font-size: 0.7rem;
		color: var(--fg-muted);
	}

	.action-modal-footer {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 14px;
		border-top: 1px solid var(--border);
		font-size: 0.7rem;
		color: var(--fg-muted);
	}

	.footer-btn {
		background: none;
		border: none;
		color: var(--accent);
		cursor: pointer;
		padding: 0;
		font-size: 0.7rem;
	}
	.footer-btn:hover {
		text-decoration: underline;
	}

	.footer-hint {
		font-family: var(--font-mono);
	}
</style>
