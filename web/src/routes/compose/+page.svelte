<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import ComposeView from '$lib/components/ComposeView.svelte';

	const app = getAppState();
</script>

<svelte:head>
	<title>Compose - tendrl</title>
</svelte:head>

<div class="document-panel">
	<div class="doc-content">
		<ComposeView
			compose={app.compose}
			onupdate={app.handleComposeUpdate}
			oncancel={app.handleCancelCompose}
			onsendtochat={app.handleComposeToChat}
			onpublish={app.handleComposePublish}
			ondelete={app.handleDeleteFromCompose}
			ondeletepermanent={app.handleDeletePermanent}
			syncMode={app.syncMode}
			onsenditemtochat={app.handleSendItemToChat}
			ontogglereadonly={app.handleToggleReadonly}
			onlocksource={app.handleLockToSource}
			oncrosspanelcopy={app.handleCrossPanelCopy}
		/>
	</div>
</div>

<style>
	.document-panel {
		flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden;
	}
	.doc-content { flex: 1; position: relative; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
</style>
