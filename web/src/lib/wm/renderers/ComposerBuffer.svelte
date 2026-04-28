<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import ComposeView from '$lib/components/ComposeView.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
</script>

<ComposeView
	compose={app.compose}
	canPublish={app.identityStatus?.state === 'unlocked'}
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
