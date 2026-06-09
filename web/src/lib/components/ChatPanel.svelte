<script lang="ts">
	import type { ChatResponse, ContextItem, Fragment, SyncMode, ClaudeSessionSummary, ClaudeSessionMessage } from '$lib/types';
	import type { SavedSessionSummary } from '$lib/api';
	import ChatLog from './ChatLog.svelte';
	import ChatInput from './ChatInput.svelte';
	import EditView from './EditView.svelte';
	import SystemPrompt from './SystemPrompt.svelte';
	import ContextPanel from './ContextPanel.svelte';
	import ClaudeSessionView from './ClaudeSessionView.svelte';

	let {
		chat,
		loading = false,
		systemExpanded = false,
		contextExpanded = false,
		contextEntries,
		syncMode,
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
		ondeletepermanentcontext,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy,
		onsenditemtocompose,
		chatHiddenFragmentIds,
		chatFragmentItems,
		claudeSessions = [],
		claudeSessionDetail = null,
		claudeSessionsLoading = false,
		sessionsExpanded = false,
		ontogglesessions,
		onclaudesessionselect,
		onclaudesessionback,
		onloadsessiontochat,
		savedSessions = [],
		activeSessionId = null,
		onsavechat,
		onloadsavedsession,
		ondeletesavedsession
	}: {
		chat: ChatResponse | null;
		loading?: boolean;
		systemExpanded?: boolean;
		contextExpanded?: boolean;
		contextEntries: ContextItem[];
		syncMode: SyncMode;
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
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
		onsenditemtocompose: (id: string) => void;
		chatHiddenFragmentIds: Set<number>;
		chatFragmentItems: Map<number, ContextItem>;
		claudeSessions?: ClaudeSessionSummary[];
		claudeSessionDetail?: { id: string; messages: ClaudeSessionMessage[]; count: number } | null;
		claudeSessionsLoading?: boolean;
		sessionsExpanded?: boolean;
		ontogglesessions?: () => void;
		onclaudesessionselect?: (id: string) => void;
		onclaudesessionback?: () => void;
		onloadsessiontochat?: (session: { id: string; messages: ClaudeSessionMessage[] }) => void;
		savedSessions?: SavedSessionSummary[];
		activeSessionId?: string | null;
		onsavechat?: (opts?: { asNew?: boolean }) => void;
		onloadsavedsession?: (id: string) => void;
		ondeletesavedsession?: (id: string) => void;
	} = $props();

	let checkedFragmentIds: Set<number> = $state(new Set());
	let localHiddenIds: Set<number> = $state(new Set());

	const visibleFragments = $derived(
		chat?.fragments.filter((f) => !localHiddenIds.has(f.id) && !chatHiddenFragmentIds.has(f.id)) ?? []
	);

	function toggleFragmentCheck(id: number) {
		const next = new Set(checkedFragmentIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedFragmentIds = next;
	}

	function selectAllFragments() {
		checkedFragmentIds = new Set(visibleFragments.map((f) => f.id));
	}

	function invertFragmentSelection() {
		const next = new Set<number>();
		for (const f of visibleFragments) {
			if (!checkedFragmentIds.has(f.id)) next.add(f.id);
		}
		checkedFragmentIds = next;
	}

	function sendToCompose() {
		const checked = visibleFragments.filter((f) => checkedFragmentIds.has(f.id));
		if (checked.length === 0) return;
		onsendfragmentstocompose(checked);
		// Fragments stay visible — they'll show compose status via chatFragmentItems
		checkedFragmentIds = new Set();
	}

	function publish() {
		const checked = visibleFragments.filter((f) => checkedFragmentIds.has(f.id));
		if (checked.length === 0) return;
		onpublishfragments(checked);
		checkedFragmentIds = new Set();
	}

	function deleteFragments() {
		const next = new Set(localHiddenIds);
		for (const id of checkedFragmentIds) next.add(id);
		localHiddenIds = next;
		checkedFragmentIds = new Set();
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
		<button onclick={ontogglesessions} class:active={sessionsExpanded} disabled={loading}>Sessions</button>
		<button
			onclick={() => onsavechat?.()}
			disabled={loading || visibleFragments.length === 0}
			title={activeSessionId
				? 'Update the saved session this chat is linked to'
				: 'Save this chat to your sessions folder'}>{activeSessionId ? 'Update' : 'Save'}</button
		>
		{#if activeSessionId}
			<button
				onclick={() => onsavechat?.({ asNew: true })}
				disabled={loading || visibleFragments.length === 0}
				title="Save a separate copy as a new session">Save as new</button
			>
		{/if}
		<span class="toolbar-spacer"></span>
		<button class="sel-btn" onclick={selectAllFragments} disabled={loading || visibleFragments.length === 0} title="Select all">All</button>
		<button class="sel-btn" onclick={invertFragmentSelection} disabled={loading || visibleFragments.length === 0} title="Invert selection">Inv</button>
		<button class="icon-btn" onclick={sendToCompose} disabled={loading || !hasChecked} title="Send to compose">□</button>
		<button class="icon-btn" onclick={publish} disabled={loading || !hasChecked} title="Publish locally">▸</button>
		<button class="icon-btn trash-btn" onclick={deleteFragments} disabled={loading || !hasChecked} title="Delete from chat">🗑</button>
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
			{syncMode}
			onupdate={onupdatecontext}
			onreset={onresetcontext}
			onremove={onremovecontext}
			{onsendtocompose}
			ondelete={ondeletecontext}
			ondeletepermanent={ondeletepermanentcontext}
			{ontogglereadonly}
			{onlocksource}
			{oncrosspanelcopy}
			{onsenditemtocompose}
			disabled={loading}
		/>
	{/if}

	{#if sessionsExpanded}
		<div class="sessions-container">
			<div class="saved-sessions">
				<div class="saved-sessions-title">Saved chats</div>
				{#if (savedSessions ?? []).length === 0}
					<p class="saved-empty">No saved chats yet — use “Save” to keep this one.</p>
				{:else}
					{#each savedSessions ?? [] as s (s.id)}
						<div class="saved-row">
							<button
								class="saved-load"
								onclick={() => onloadsavedsession?.(s.id)}
								disabled={loading}
								title="Load this chat"
							>
								<span class="saved-title">{s.title}</span>
								<span class="saved-meta">{s.message_count} msg{s.message_count === 1 ? '' : 's'}</span>
							</button>
							<button
								class="saved-del"
								onclick={() => ondeletesavedsession?.(s.id)}
								disabled={loading}
								title="Delete saved chat">🗑</button
							>
						</div>
					{/each}
				{/if}
			</div>
			<ClaudeSessionView
				sessions={claudeSessions ?? []}
				selectedSession={claudeSessionDetail ?? null}
				loading={claudeSessionsLoading ?? false}
				onselect={onclaudesessionselect}
				onback={onclaudesessionback}
				onload={onloadsessiontochat && claudeSessionDetail ? () => onloadsessiontochat?.(claudeSessionDetail!) : undefined}
			/>
		</div>
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
				fragments={visibleFragments}
				checkedIds={checkedFragmentIds}
				ontogglecheck={toggleFragmentCheck}
				{chatFragmentItems}
				{loading}
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

	.trash-btn {
		font-size: 0.75rem;
	}

	.sessions-container {
		flex: 1;
		min-height: 0;
		overflow: hidden;
		display: flex;
		flex-direction: column;
		border-bottom: 1px solid var(--border);
	}

	.saved-sessions {
		flex-shrink: 0;
		max-height: 40%;
		overflow-y: auto;
		padding: 8px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.saved-sessions-title {
		font-size: 0.72rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--fg-muted);
		margin-bottom: 6px;
	}

	.saved-empty {
		font-size: 0.8rem;
		color: var(--fg-muted);
		margin: 4px 0;
	}

	.saved-row {
		display: flex;
		align-items: center;
		gap: 4px;
	}

	.saved-load {
		flex: 1;
		min-width: 0;
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		gap: 8px;
		text-align: left;
		padding: 5px 8px;
		background: transparent;
		border: none;
		border-radius: 4px;
		color: var(--fg);
		cursor: pointer;
	}

	.saved-load:hover:not(:disabled) {
		background: var(--bg-hover, rgba(127, 127, 127, 0.12));
	}

	.saved-title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.saved-meta {
		flex-shrink: 0;
		font-size: 0.72rem;
		color: var(--fg-muted);
	}

	.saved-del {
		flex-shrink: 0;
		background: transparent;
		border: none;
		cursor: pointer;
		opacity: 0.6;
		padding: 2px 4px;
	}

	.saved-del:hover:not(:disabled) {
		opacity: 1;
	}
</style>
