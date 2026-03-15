<script lang="ts">
	import { onMount } from 'svelte';
	import type {
		ChatResponse,
		SearchResult,
		PublicationDetail,
		Section,
		ComposeState,
		ViewMode,
		DocMode
	} from '$lib/types';
	import * as api from '$lib/api';
	import WorkbenchToolbar from '$lib/components/WorkbenchToolbar.svelte';
	import PanelFrame from '$lib/components/PanelFrame.svelte';
	import ChatPanel from '$lib/components/ChatPanel.svelte';
	import DocumentPanel from '$lib/components/DocumentPanel.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';

	// Chat state
	let chat: ChatResponse | null = $state(null);
	let chatLoading = $state(false);
	let systemExpanded = $state(false);
	let contextExpanded = $state(false);
	let originalEditBuffer = $state('');

	// Document state
	let docMode: DocMode = $state('empty');
	let publication: PublicationDetail | null = $state(null);
	let sections: Section[] = $state([]);
	let viewMode: ViewMode = $state('outline');
	let currentSection = $state(0);
	let previewVisible = $state(false);
	let docLoading = $state(false);
	let compose: ComposeState = $state({ title: '', tags: [], sections: [] });

	// Search state
	let searchResults: SearchResult[] = $state([]);
	let searchCount = $state(0);
	let searchLocalCount = $state(0);
	let searchRelayCount = $state(0);
	let searchLoading = $state(false);

	// JSON modal
	let jsonModalData: unknown = $state(null);

	// Panel collapse
	let chatCollapsed = $state(false);
	let docCollapsed = $state(false);
	let searchCollapsed = $state(false);

	const gridTemplate = $derived(
		[
			chatCollapsed ? 'auto' : '1fr',
			docCollapsed ? 'auto' : '2fr',
			searchCollapsed ? 'auto' : '1fr'
		].join(' ')
	);

	onMount(async () => {
		chat = await api.getChat();
	});

	// Chat handlers
	async function handleSend(content: string) {
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

	async function handleInjectContext(title: string, content: string) {
		chatLoading = true;
		try {
			chat = await api.injectContext([{ title, content }]);
		} finally {
			chatLoading = false;
		}
	}

	// Search handlers
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

	// JSON modal handler
	async function handleViewJson(result: SearchResult) {
		try {
			const resp = await api.getEvent(result.event_id);
			jsonModalData = resp.event;
		} catch {
			jsonModalData = result;
		}
	}

	// Document handlers
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
		compose = { title: '', tags: [], sections: [{ title: '', content: '', tags: [] }] };
		docMode = 'compose';
		previewVisible = false;
	}

	function handleComposeUpdate(state: ComposeState) {
		compose = state;
	}

	function handleCancelCompose() {
		docMode = publication ? 'reading' : 'empty';
	}
</script>

<div class="workbench">
	<WorkbenchToolbar />

	<div class="workbench-panels" style:grid-template-columns={gridTemplate}>
		<PanelFrame title="Chat" collapsed={chatCollapsed} ontoggle={() => (chatCollapsed = !chatCollapsed)}>
			<ChatPanel
				{chat}
				loading={chatLoading}
				{systemExpanded}
				{contextExpanded}
				ontogglesystem={() => (systemExpanded = !systemExpanded)}
				ontogglecontext={() => (contextExpanded = !contextExpanded)}
				onsend={handleSend}
				onreset={handleReset}
				onedit={handleEdit}
				onapplyedit={handleApplyEdit}
				oncanceledit={handleCancelEdit}
				onsetsystem={handleSetSystem}
				oninjectcontext={handleInjectContext}
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
