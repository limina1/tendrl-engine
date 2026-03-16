<script lang="ts">
	import { onMount } from 'svelte';
	import type {
		ChatResponse,
		SearchResult,
		PublicationDetail,
		Section,
		ComposeState,
		ContextItem,
		Fragment,
		TagEntry,
		ViewMode,
		DocMode,
		SyncMode,
		ButtonLabels
	} from '$lib/types';
	import * as api from '$lib/api';
	import WorkbenchToolbar from '$lib/components/WorkbenchToolbar.svelte';
	import PanelFrame from '$lib/components/PanelFrame.svelte';
	import ChatPanel from '$lib/components/ChatPanel.svelte';
	import DocumentPanel from '$lib/components/DocumentPanel.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';

	// Chat state
	let chat: ChatResponse | null = $state({
		fragments: [],
		fragment_count: 0,
		edit_mode: false,
		edit_buffer: null,
		system_prompt: null,
		context_count: 0,
		generating: false
	});
	let chatLoading = $state(false);
	let systemExpanded = $state(false);
	let contextExpanded = $state(false);
	let originalEditBuffer = $state('');
	let chatHiddenFragmentIds: Set<number> = $state(new Set());

	// === Unified item pool ===
	let items: ContextItem[] = $state([]);
	const contextEntries = $derived(items.filter((i) => i.in_context));
	const composeSections = $derived(items.filter((i) => i.in_compose));

	// Map fragment_id → compose item for chat-origin items
	const chatFragmentItems = $derived(
		new Map(
			items
				.filter((i) => i.origin === 'chat' && i.source_fragment_id != null)
				.map((i) => [i.source_fragment_id!, i])
		)
	);

	// Compose publication-level metadata (separate from items)
	let composeTitle = $state('');
	let composeTags: TagEntry[] = $state([]);
	const compose = $derived<ComposeState>({
		title: composeTitle,
		tags: composeTags,
		sections: composeSections
	});

	// Document state
	let docMode: DocMode = $state('empty');
	let publication: PublicationDetail | null = $state(null);
	let sections: Section[] = $state([]);
	let viewMode: ViewMode = $state('outline');
	let currentSection = $state(0);
	let previewVisible = $state(false);
	let docLoading = $state(false);

	// Search state
	let searchResults: SearchResult[] = $state([]);
	let searchCount = $state(0);
	let searchLocalCount = $state(0);
	let searchRelayCount = $state(0);
	let searchLoading = $state(false);

	// JSON modal
	let jsonModalData: unknown = $state(null);

	// Settings
	let syncMode: SyncMode = $state('explicit');
	let buttonLabels: ButtonLabels = $state('icon');

	// Panel collapse
	let chatCollapsed = $state(true);
	let docCollapsed = $state(false);
	let searchCollapsed = $state(true);

	const gridTemplate = $derived(
		[
			chatCollapsed ? 'auto' : '1fr',
			docCollapsed ? 'auto' : '2fr',
			searchCollapsed ? 'auto' : '1fr'
		].join(' ')
	);

	onMount(async () => {
		try {
			chat = await api.getChat();
		} catch {
			// Backend unavailable — keep default empty state
		}
	});

	// --- Helpers ---

	function makeItem(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean }
	): ContextItem {
		return {
			...fields,
			id: crypto.randomUUID(),
			context_content: fields.content,
			modified: false,
			readonly: false,
			in_context: target.context ?? false,
			in_compose: target.compose ?? false
		};
	}

	async function fetchEventContent(result: SearchResult): Promise<string> {
		try {
			const resp = await api.getEvent(result.event_id);
			const event = resp.event as Record<string, unknown> | null;
			return (event?.content as string) ?? result.preview;
		} catch {
			return result.preview;
		}
	}

	function resultFields(result: SearchResult, content: string) {
		return {
			title: result.title ?? '[Untitled]',
			content,
			tags: (result.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
			source_event_id: result.event_id,
			source_addr: result.addr,
			original_content: content,
			origin: 'search' as const
		};
	}

	// Remove items that belong to neither panel
	function gc() {
		items = items.filter((e) => e.in_context || e.in_compose);
	}

	// Dedup by source_event_id or source_addr — flip flags if exists, else create
	function addToPool(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean }
	) {
		const existing = items.find((e) => {
			if (fields.source_event_id && e.source_event_id === fields.source_event_id) return true;
			if (
				fields.source_addr &&
				e.source_addr &&
				fields.source_addr.kind === e.source_addr.kind &&
				fields.source_addr.pubkey === e.source_addr.pubkey &&
				fields.source_addr.d_tag === e.source_addr.d_tag
			)
				return true;
			return false;
		});
		if (existing) {
			items = items.map((e) =>
				e.id === existing.id
					? {
							...e,
							in_context: e.in_context || (target.context ?? false),
							in_compose: e.in_compose || (target.compose ?? false),
							// Snapshot content on first bridge to other panel
							...(target.context && !e.in_context ? { context_content: e.content } : {}),
							...(target.compose && !e.in_compose ? { content: e.context_content } : {})
						}
					: e
			);
		} else {
			items = [...items, makeItem(fields, target)];
		}
	}

	// --- Sync context to backend ---

	async function syncContext() {
		const ctx = items.filter((e) => e.in_context);
		try {
			chat = await api.replaceContext(
				ctx.map((e) => ({ title: e.title, content: e.context_content }))
			);
		} catch {
			// silent
		}
	}

	// --- Chat handlers ---

	async function handleSend(content: string) {
		// Optimistically show user message before waiting for LLM
		if (chat) {
			const nextId = Math.max(0, ...chat.fragments.map((f) => f.id)) + 1;
			chat = {
				...chat,
				fragments: [...chat.fragments, { id: nextId, role: 'user', content }],
				fragment_count: chat.fragment_count + 1
			};
		}
		chatLoading = true;
		try {
			chat = await api.sendMessage(content);
		} finally {
			chatLoading = false;
		}
	}

	async function handleReset() {
		chatLoading = true;
		try {
			chat = await api.resetChat();
		} finally {
			chatLoading = false;
		}
	}

	async function handleEdit() {
		chatLoading = true;
		try {
			chat = await api.enterEditMode();
			if (chat.edit_buffer) originalEditBuffer = chat.edit_buffer;
		} finally {
			chatLoading = false;
		}
	}

	async function handleApplyEdit(buffer: string) {
		chatLoading = true;
		try {
			chat = await api.exitEditMode(buffer);
		} finally {
			chatLoading = false;
		}
	}

	async function handleCancelEdit() {
		chatLoading = true;
		try {
			chat = await api.exitEditMode(originalEditBuffer);
		} finally {
			chatLoading = false;
		}
	}

	async function handleSetSystem(prompt: string) {
		chatLoading = true;
		try {
			chat = await api.setSystemPrompt(prompt);
		} finally {
			chatLoading = false;
		}
	}

	// --- Item handlers (shared, used by both context and compose) ---

	function handleUpdateContextItem(id: string, title: string, contextContent: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, title, context_content: contextContent } : e
		);
		syncContext();
	}

	function handleResetContextItem(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, context_content: e.original_content } : e
		);
		syncContext();
	}

	function handleRemoveFromContext(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, in_context: false } : e
		);
		gc();
		syncContext();
	}

	// Context 🗑 → set in_context false, gc
	function handleDeleteFromContext(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_context: false } : e));
		gc();
		syncContext();
	}

	// Compose 🗑 → set in_compose false, gc
	function handleDeleteFromCompose(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_compose: false } : e));
		gc();
	}

	// 🗑🗑 → permanent delete from pool
	function handleDeletePermanent(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.filter((e) => !ids.has(e.id));
		syncContext();
	}

	// Context □ → compose (copy context_content into compose content)
	function handleContextToCompose(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
		items = items.map((e) =>
			ids.has(e.id)
				? { ...e, in_compose: true, in_context: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Compose ◂ → context (copy compose content into context_content)
	// For chat-origin items, hide the source fragment
	function handleComposeToChat(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
		// Hide chat fragments for items moving to context
		const nextHidden = new Set(chatHiddenFragmentIds);
		for (const item of checkedItems) {
			if (item.origin === 'chat' && item.source_fragment_id != null) {
				nextHidden.add(item.source_fragment_id);
			}
		}
		chatHiddenFragmentIds = nextHidden;
		items = items.map((e) =>
			ids.has(e.id)
				? { ...e, in_context: true, in_compose: true, context_content: e.content }
				: e
		);
		syncContext();
	}

	// Per-item: send single item to context (snapshot compose content)
	// For chat-origin items, hide the source fragment
	function handleSendItemToChat(id: string) {
		const item = items.find((e) => e.id === id);
		if (item?.origin === 'chat' && item.source_fragment_id != null) {
			chatHiddenFragmentIds = new Set([...chatHiddenFragmentIds, item.source_fragment_id]);
		}
		items = items.map((e) =>
			e.id === id
				? { ...e, in_context: true, in_compose: true, context_content: e.content }
				: e
		);
		syncContext();
	}

	// Per-item: send single item to compose (snapshot context content)
	function handleSendItemToCompose(id: string) {
		items = items.map((e) =>
			e.id === id
				? { ...e, in_context: true, in_compose: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Toggle readonly on any item
	// Cross-panel lock toggle (context/compose badge)
	function handleToggleReadonly(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, readonly: !e.readonly } : e
		);
	}

	// Origin lock: toggle readonly + reset to original source content when locking
	function handleLockToSource(id: string) {
		items = items.map((e) => {
			if (e.id !== id) return e;
			const locking = !e.readonly;
			if (locking) {
				return {
					...e,
					readonly: true,
					content: e.original_content,
					context_content: e.original_content,
					modified: false
				};
			}
			return { ...e, readonly: false };
		});
		syncContext();
	}

	// Cross-panel copy: overwrite other panel's content from this panel
	function handleCrossPanelCopy(id: string, fromPanel: string) {
		items = items.map((e) => {
			if (e.id !== id) return e;
			if (fromPanel === 'compose') {
				return { ...e, context_content: e.content, readonly: false };
			} else if (fromPanel === 'context') {
				return { ...e, content: e.context_content, modified: e.context_content !== e.original_content, readonly: false };
			}
			return e;
		});
		syncContext();
	}

	// Chat fragments □ → add as new items with in_compose (fragments stay visible)
	function handleChatFragmentsToCompose(fragments: Fragment[]) {
		const newItems = fragments.map((f) =>
			makeItem(
				{ title: `[${f.role}]`, content: f.content, tags: [], original_content: f.content, origin: 'chat', source_fragment_id: f.id },
				{ compose: true }
			)
		);
		items = [...items, ...newItems];
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Chat fragments ▸
	function handleChatPublishFragments(_fragments: Fragment[]) {
		// TODO: create local nostr events
	}

	// Compose ▸
	function handleComposePublish(_items: ContextItem[]) {
		// TODO: create local nostr events
	}

	// --- Compose update reconciliation ---

	function handleComposeUpdate(state: ComposeState) {
		composeTitle = state.title;
		composeTags = state.tags;

		const updatedById = new Map(state.sections.map((s) => [s.id, s]));

		// Update existing compose items, remove ones dropped from sections
		items = items
			.map((item) => {
				if (!item.in_compose) return item;
				const updated = updatedById.get(item.id);
				if (updated) {
					updatedById.delete(item.id);
					return { ...updated, in_context: item.in_context, in_compose: true, context_content: item.context_content };
				}
				// Removed from compose
				if (item.in_context) return { ...item, in_compose: false };
				return null;
			})
			.filter((item): item is ContextItem => item !== null);

		// Add new items (from + Section or plain text parse)
		const existingIds = new Set(items.map((i) => i.id));
		for (const [id, section] of updatedById) {
			if (!existingIds.has(id)) {
				items = [...items, { ...section, in_context: false, in_compose: true }];
			}
		}

		syncContext();
	}

	// --- Search handlers ---

	async function handleSearch(query: string) {
		searchLoading = true;
		try {
			const resp = await api.search(query);
			searchResults = resp.results;
			searchCount = resp.count;
			searchLocalCount = resp.local_count;
			searchRelayCount = resp.relay_count;
		} finally {
			searchLoading = false;
		}
	}

	// Search ◂ → add or flag in_context
	async function handleAddToContext(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { context: true });
		syncContext();
	}

	// Search □ → add or flag in_compose
	async function handleAddToCompose(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { compose: true });
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Search bulk ◂
	async function handleAddManyToContext(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { context: true });
		}
		syncContext();
	}

	// Search bulk □
	async function handleAddManyToCompose(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { compose: true });
		}
		if (docMode !== 'compose') docMode = 'compose';
	}

	// JSON modal
	async function handleViewJson(result: SearchResult) {
		try {
			const resp = await api.getEvent(result.event_id);
			jsonModalData = resp.event;
		} catch {
			jsonModalData = result;
		}
	}

	// --- Document handlers ---

	async function handleSelectResult(result: SearchResult) {
		if (!result.addr) return;
		docLoading = true;
		try {
			const pubResp = await api.getPublication(result.addr.pubkey, result.addr.d_tag);
			publication = pubResp.publication;
			const secResp = await api.loadSections(result.addr.pubkey, result.addr.d_tag);
			sections = secResp.sections;
			docMode = 'reading';
			viewMode = 'outline';
			currentSection = 0;
			previewVisible = false;
		} catch {
			// Non-publication results can't be loaded yet
		} finally {
			docLoading = false;
		}
	}

	function handleViewMode(mode: ViewMode) {
		viewMode = mode;
	}

	function handleTogglePreview() {
		previewVisible = !previewVisible;
	}

	function handleNavigate(index: number) {
		currentSection = index;
	}

	function handleCompose() {
		// Clear compose flags, gc, start fresh
		items = [
			...items.map((e) => ({ ...e, in_compose: false })).filter((e) => e.in_context),
			makeItem({ title: '', content: '', tags: [], original_content: '', origin: 'compose' }, { compose: true })
		];
		composeTitle = '';
		composeTags = [];
		docMode = 'compose';
		previewVisible = false;
	}

	function handleCancelCompose() {
		docMode = publication ? 'reading' : 'empty';
	}

	// Document reading ◂ → add or flag sections in_context
	function handleDocToChat() {
		if (!sections.length) return;
		for (const s of sections) {
			if (!s.content) continue;
			addToPool(
				{
					title: s.title ?? '[Section]',
					content: s.content ?? '',
					tags: [],
					source_addr: s.addr,
					original_content: s.content ?? '',
					origin: 'search'
				},
				{ context: true }
			);
		}
		syncContext();
	}

	// Document reading ▸
	function handleDocPublish() {
		// TODO: create local nostr event
	}
</script>

<div class="workbench">
	<WorkbenchToolbar
		{syncMode}
		{buttonLabels}
		onsetsyncmode={(m: SyncMode) => (syncMode = m)}
		onsetbuttonlabels={(m: ButtonLabels) => (buttonLabels = m)}
	/>

	<div class="workbench-panels" style:grid-template-columns={gridTemplate}>
		<PanelFrame title="Chat" collapsed={chatCollapsed} ontoggle={() => (chatCollapsed = !chatCollapsed)}>
			<ChatPanel
				{chat}
				loading={chatLoading}
				{systemExpanded}
				{contextExpanded}
				{contextEntries}
				ontogglesystem={() => (systemExpanded = !systemExpanded)}
				ontogglecontext={() => (contextExpanded = !contextExpanded)}
				onsend={handleSend}
				onreset={handleReset}
				onedit={handleEdit}
				onapplyedit={handleApplyEdit}
				oncanceledit={handleCancelEdit}
				onsetsystem={handleSetSystem}
				onupdatecontext={handleUpdateContextItem}
				onresetcontext={handleResetContextItem}
				onremovecontext={handleRemoveFromContext}
				onsendtocompose={handleContextToCompose}
				onsendfragmentstocompose={handleChatFragmentsToCompose}
				onpublishfragments={handleChatPublishFragments}
				ondeletecontext={handleDeleteFromContext}
				ondeletepermanentcontext={handleDeletePermanent}
				{syncMode}
				ontogglereadonly={handleToggleReadonly}
				onlocksource={handleLockToSource}
				oncrosspanelcopy={handleCrossPanelCopy}
				onsenditemtocompose={handleSendItemToCompose}
				{chatHiddenFragmentIds}
				{chatFragmentItems}
			/>
		</PanelFrame>

		<PanelFrame title="Document" collapsed={docCollapsed} ontoggle={() => (docCollapsed = !docCollapsed)}>
			<DocumentPanel
				{docMode}
				{publication}
				{sections}
				{viewMode}
				{currentSection}
				{previewVisible}
				{compose}
				loading={docLoading}
				onviewmode={handleViewMode}
				ontogglepreview={handleTogglePreview}
				oncompose={handleCompose}
				onnavigate={handleNavigate}
				oncomposeupdate={handleComposeUpdate}
				oncancelcompose={handleCancelCompose}
				onsendtochat={handleComposeToChat}
				onpublishcompose={handleComposePublish}
				ondeletecompose={handleDeleteFromCompose}
				ondeletepermanentcompose={handleDeletePermanent}
				ondoctochat={handleDocToChat}
				ondocpublish={handleDocPublish}
				{syncMode}
				onsenditemtochat={handleSendItemToChat}
				ontogglereadonly={handleToggleReadonly}
				onlocksource={handleLockToSource}
				oncrosspanelcopy={handleCrossPanelCopy}
			/>
		</PanelFrame>

		<PanelFrame title="Search" collapsed={searchCollapsed} ontoggle={() => (searchCollapsed = !searchCollapsed)}>
			<SearchPanel
				results={searchResults}
				count={searchCount}
				localCount={searchLocalCount}
				relayCount={searchRelayCount}
				loading={searchLoading}
				onsearch={handleSearch}
				onselect={handleSelectResult}
				onviewjson={handleViewJson}
				onaddtocontext={handleAddToContext}
				onaddtocompose={handleAddToCompose}
				onaddmanytocontext={handleAddManyToContext}
				onaddmanytocompose={handleAddManyToCompose}
				{items}
			/>
		</PanelFrame>
	</div>
</div>

{#if jsonModalData}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="json-modal-backdrop" onclick={() => (jsonModalData = null)} role="presentation">
		<div class="json-modal" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
			<div class="json-modal-header">
				<span>Event JSON</span>
				<button onclick={() => (jsonModalData = null)}>Close</button>
			</div>
			<pre class="json-modal-body">{JSON.stringify(jsonModalData, null, 2)}</pre>
		</div>
	</div>
{/if}

<style>
	.workbench {
		display: flex;
		flex-direction: column;
		height: 100dvh;
	}

	.workbench-panels {
		flex: 1;
		display: grid;
		min-height: 0;
	}

	.workbench-panels > :global(*) {
		border-right: 1px solid var(--border);
		min-height: 0;
	}

	.workbench-panels > :global(*:last-child) {
		border-right: none;
	}

	.json-modal-backdrop {
		position: fixed;
		inset: 0;
		z-index: 100;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.json-modal {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: 90vw;
		max-width: 720px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
	}

	.json-modal-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
		font-weight: 600;
		font-size: 0.85rem;
	}

	.json-modal-body {
		flex: 1;
		overflow: auto;
		padding: 14px;
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.8rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
