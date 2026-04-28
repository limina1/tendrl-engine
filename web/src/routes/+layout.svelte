<script lang="ts">
	import '$lib/styles/tokens.css';
	import '../app.css';
	import { page } from '$app/state';
	import { createAppState } from '$lib/state.svelte';
	import WorkbenchToolbar from '$lib/components/WorkbenchToolbar.svelte';
	import PanelFrame from '$lib/components/PanelFrame.svelte';
	import ChatPanel from '$lib/components/ChatPanel.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';

	let { children } = $props();

	const app = createAppState();

	const bare = $derived(page.url.pathname === '/design' || page.url.pathname.startsWith('/design/'));

	let initialized = $state(false);
	$effect(() => {
		if (initialized) return;
		initialized = true;
		app.initialize();
		const cleanup = app.startNetworkPoll();
		return cleanup;
	});
</script>

{#if bare}
	{@render children()}
{:else}
<div class="workbench">
	<WorkbenchToolbar
		syncMode={app.syncMode}
		buttonLabels={app.buttonLabels}
		embeddingStatus={app.embeddingStatus}
		embeddingSyncing={app.embeddingSyncing}
		ignoredCount={app.ignoredCount}
		networkStatus={app.networkStatus}
		identityStatus={app.identityStatus}
		identityLoading={app.identityLoading}
		identityError={app.identityError}
		identityDisplayName={app.identityDisplayName}
		onsetsyncmode={(m) => (app.syncMode = m)}
		onsetbuttonlabels={(m) => (app.buttonLabels = m)}
		onhome={() => app.navigateHome()}
		onsyncembeddings={app.handleSyncEmbeddings}
		onreindexembeddings={app.handleReindexEmbeddings}
		onviewignored={app.handleViewIgnored}
		onpurge={app.handlePurge}
		onexport={app.handleExport}
		exporting={app.exporting}
		onimport={app.handleImport}
		importing={app.importing}
		importProgress={app.importProgress}
		passthrough={app.passthrough}
		onsetpassthrough={(v) => (app.passthrough = v)}
		onsetnetworkmode={app.handleSetNetworkMode}
		onidentitylogin={app.handleIdentityLogin}
		onidentityunlock={app.handleIdentityUnlock}
		onidentitylock={app.handleIdentityLock}
		onidentitylogout={app.handleIdentityLogout}
		onclearidentityerror={() => (app.identityError = null)}
		onviewprofile={app.handleViewProfile}
	/>

	<div class="workbench-panels" style:grid-template-columns={app.gridTemplate}>
		<PanelFrame title="Chat" collapsed={app.chatCollapsed} ontoggle={() => (app.chatCollapsed = !app.chatCollapsed)}>
			<ChatPanel
				chat={app.chat}
				loading={app.chatLoading}
				systemExpanded={app.systemExpanded}
				contextExpanded={app.contextExpanded}
				contextEntries={app.contextEntries}
				ontogglesystem={() => (app.systemExpanded = !app.systemExpanded)}
				ontogglecontext={() => (app.contextExpanded = !app.contextExpanded)}
				onsend={app.handleSend}
				onreset={app.handleReset}
				onedit={app.handleEdit}
				onapplyedit={app.handleApplyEdit}
				oncanceledit={app.handleCancelEdit}
				onsetsystem={app.handleSetSystem}
				onupdatecontext={app.handleUpdateContextItem}
				onresetcontext={app.handleResetContextItem}
				onremovecontext={app.handleRemoveFromContext}
				onsendtocompose={app.handleContextToCompose}
				onsendfragmentstocompose={app.handleChatFragmentsToCompose}
				onpublishfragments={app.handleChatPublishFragments}
				ondeletecontext={app.handleDeleteFromContext}
				ondeletepermanentcontext={app.handleDeletePermanent}
				syncMode={app.syncMode}
				ontogglereadonly={app.handleToggleReadonly}
				onlocksource={app.handleLockToSource}
				oncrosspanelcopy={app.handleCrossPanelCopy}
				onsenditemtocompose={app.handleSendItemToCompose}
				chatHiddenFragmentIds={app.chatHiddenFragmentIds}
				chatFragmentItems={app.chatFragmentItems}
				claudeSessions={app.claudeSessions}
				claudeSessionDetail={app.claudeSessionDetail}
				claudeSessionsLoading={app.claudeSessionsLoading}
				sessionsExpanded={app.sessionsExpanded}
				ontogglesessions={app.handleToggleSessions}
				onclaudesessionselect={app.handleSelectClaudeSession}
				onclaudesessionback={app.handleClaudeSessionBack}
				onloadsessiontochat={app.handleLoadSessionToChat}
			/>
		</PanelFrame>

		<PanelFrame title="Document" collapsed={app.docCollapsed} ontoggle={() => (app.docCollapsed = !app.docCollapsed)}>
			{@render children()}
		</PanelFrame>

		<PanelFrame title="Search" collapsed={app.searchCollapsed} ontoggle={() => (app.searchCollapsed = !app.searchCollapsed)}>
			<SearchPanel
				results={app.searchResults}
				count={app.searchCount}
				localCount={app.searchLocalCount}
				relayCount={app.searchRelayCount}
				loading={app.searchLoading}
				searchContext={app.docMode === 'empty' ? 'publications' : 'knowledge base'}
				onsearch={app.handleSearch}
				onselect={app.handleSelectResult}
				onviewjson={app.handleViewJson}
				onaddtocontext={app.handleAddToContext}
				onaddtocompose={app.handleAddToCompose}
				onaddmanytocontext={app.handleAddManyToContext}
				onaddmanytocompose={app.handleAddManyToCompose}
				onignore={app.handleIgnoreEvent}
				onignorepubkey={app.handleIgnorePubkey}
				documentFiles={app.documentFiles}
				importPages={app.importPages}
				importFilename={app.importFilename}
				importLoading={app.importLoading}
				onlistdocuments={app.handleListDocuments}
				onimportfile={app.handleImportFile}
				onparsedocument={app.handleParseDocument}
				onimportpagetocontext={app.handleImportPageToContext}
				onimportpagetocompose={app.handleImportPageToCompose}
				onimportpagestocontext={app.handleImportPagesToContext}
				onimportpagestocompose={app.handleImportPagesToCompose}
				items={app.items}
				localPubkeys={app.localPubkeys}
				onviewprofile={app.handleViewProfile}
			/>
		</PanelFrame>
	</div>
</div>

{/if}

{#if app.jsonModalData}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="json-modal-backdrop" onclick={() => (app.jsonModalData = null)} role="presentation">
		<div class="json-modal" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
			<div class="json-modal-header">
				<span>Event JSON</span>
				<button onclick={() => (app.jsonModalData = null)}>Close</button>
			</div>
			<pre class="json-modal-body">{JSON.stringify(app.jsonModalData, null, 2)}</pre>
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
