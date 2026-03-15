<script lang="ts">
	import type { ChatResponse, ContextItem, Fragment } from '$lib/types';
	import ChatLog from './ChatLog.svelte';
	import ChatInput from './ChatInput.svelte';
	import EditView from './EditView.svelte';
	import SystemPrompt from './SystemPrompt.svelte';
	import ContextPanel from './ContextPanel.svelte';

	let {
		chat,
		loading = false,
		systemExpanded = false,
		contextExpanded = false,
		contextEntries,
		ontogglesystem,
		ontogglecontext,
		onsend,
		onreset,
		onedit,
		onapplyedit,
		oncanceledit,
		onsetsystem,
		onupdatecontext,
		onresetcontext,
		onremovecontext,
		onsendtocompose,
		onsendfragmentstocompose,
		onpublishfragments,
		ondeletecontext,
		ondeletepermanentcontext
	}: {
		chat: ChatResponse | null;
		loading?: boolean;
		systemExpanded?: boolean;
		contextExpanded?: boolean;
		contextEntries: ContextItem[];
		ontogglesystem: () => void;
		ontogglecontext: () => void;
		onsend: (content: string) => void;
		onreset: () => void;
		onedit: () => void;
		onapplyedit: (buffer: string) => void;
		oncanceledit: () => void;
		onsetsystem: (prompt: string) => void;
		onupdatecontext: (id: string, title: string, content: string) => void;
		onresetcontext: (id: string) => void;
		onremovecontext: (id: string) => void;
		onsendtocompose: (items: ContextItem[]) => void;
		onsendfragmentstocompose: (fragments: Fragment[]) => void;
		onpublishfragments: (fragments: Fragment[]) => void;
		ondeletecontext: (items: ContextItem[]) => void;
		ondeletepermanentcontext: (items: ContextItem[]) => void;
	} = $props();

	let checkedFragmentIds: Set<number> = $state(new Set());

	function toggleFragmentCheck(id: number) {
		const next = new Set(checkedFragmentIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedFragmentIds = next;
	}

	function selectAllFragments() {
		if (!chat) return;
		checkedFragmentIds = new Set(chat.fragments.map((f) => f.id));
	}

	function invertFragmentSelection() {
		if (!chat) return;
		const next = new Set<number>();
		for (const f of chat.fragments) {
			if (!checkedFragmentIds.has(f.id)) next.add(f.id);
		}
		checkedFragmentIds = next;
	}

	function sendToCompose() {
		if (!chat) return;
		const checked = chat.fragments.filter((f) => checkedFragmentIds.has(f.id));
		if (checked.length > 0) {
			onsendfragmentstocompose(checked);
			checkedFragmentIds = new Set();
		}
	}

	function publish() {
		if (!chat) return;
		const checked = chat.fragments.filter((f) => checkedFragmentIds.has(f.id));
		if (checked.length > 0) {
			onpublishfragments(checked);
			checkedFragmentIds = new Set();
		}
	}

	const hasChecked = $derived(checkedFragmentIds.size > 0);
</script>

<div class="chat-panel">
	<div class="chat-toolbar">
		<button onclick={ontogglesystem} class:active={systemExpanded} disabled={loading}>
			System
		</button>
		<button onclick={ontogglecontext} class:active={contextExpanded} disabled={loading}>
			Context
			{#if contextEntries.length > 0}
				<span class="toolbar-badge">{contextEntries.length}</span>
			{/if}
		</button>
		<button onclick={onedit} disabled={loading || (chat?.edit_mode ?? false)}>Edit</button>
		<button onclick={onreset} disabled={loading}>Reset</button>
		<span class="toolbar-spacer"></span>
		<button class="sel-btn" onclick={selectAllFragments} disabled={loading || !chat || chat.fragments.length === 0} title="Select all">All</button>
		<button class="sel-btn" onclick={invertFragmentSelection} disabled={loading || !chat || chat.fragments.length === 0} title="Invert selection">Inv</button>
		<button class="icon-btn" onclick={sendToCompose} disabled={loading || !hasChecked} title="Send to compose">□</button>
		<button class="icon-btn" onclick={publish} disabled={loading || !hasChecked} title="Publish locally">▸</button>
	</div>

	{#if systemExpanded}
		<SystemPrompt
			currentPrompt={chat?.system_prompt ?? null}
			onset={onsetsystem}
			disabled={loading}
		/>
	{/if}

	{#if contextExpanded}
		<ContextPanel
			entries={contextEntries}
			onupdate={onupdatecontext}
			onreset={onresetcontext}
			onremove={onremovecontext}
			{onsendtocompose}
			ondelete={ondeletecontext}
			ondeletepermanent={ondeletepermanentcontext}
			disabled={loading}
		/>
	{/if}

	{#if chat}
		{#if chat.edit_mode && chat.edit_buffer != null}
			<EditView
				editBuffer={chat.edit_buffer}
				onapply={onapplyedit}
				oncancel={oncanceledit}
			/>
		{:else}
			<ChatLog
				fragments={chat.fragments}
				checkedIds={checkedFragmentIds}
				ontogglecheck={toggleFragmentCheck}
			/>
		{/if}
	{:else}
		<div class="chat-empty">Loading...</div>
	{/if}

	{#if !chat?.edit_mode}
		<ChatInput {onsend} disabled={loading || !chat} />
	{/if}
</div>

<style>
	.chat-panel {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.chat-toolbar {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		align-items: center;
	}

	.toolbar-spacer {
		flex: 1;
	}

	.chat-empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}

	.toolbar-badge {
		background: rgba(255, 255, 255, 0.3);
		font-size: 0.7rem;
		padding: 0 5px;
		border-radius: 8px;
		margin-left: 2px;
	}

	.sel-btn {
		font-size: 0.65rem;
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		padding: 4px 8px;
		font-size: 0.85rem;
		min-width: 28px;
	}
</style>
