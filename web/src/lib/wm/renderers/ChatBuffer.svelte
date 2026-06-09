<script lang="ts">
	import { onMount } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import ChatPanel from '$lib/components/ChatPanel.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	// Hydrate the chat (incl. the system prompt the engine reads from prompt.md)
	// whenever the chat panel mounts, so it's present on load — not only after
	// the first message.
	onMount(() => {
		app.refreshChat();
	});
</script>

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
	savedSessions={app.savedSessions}
	onsavechat={app.handleSaveChat}
	onloadsavedsession={app.handleLoadSavedSession}
	ondeletesavedsession={app.handleDeleteSavedSession}
/>
