<script lang="ts">
	import { untrack } from 'svelte';
	import type { EditorView } from '@codemirror/view';
	import { getAppState } from '$lib/state.svelte';
	import ComposeView from '$lib/components/ComposeView.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	type ComposeMode = 'plain' | 'preview';

	// View state survives buffer switches: a remount restores the last mode
	// from the store's per-buffer stash. (A stash from before Composer v2 may
	// say 'full' — the one editor took its place.)
	type ComposerViewState = { mode: ComposeMode | 'full' };
	// The initial-value capture is deliberate: BufferRenderer keys this
	// component on buffer.id, so a different id is a fresh mount anyway.
	// svelte-ignore state_referenced_locally
	const savedView = store.bufferState.get(buffer.id) as ComposerViewState | undefined;

	let mode = $state<ComposeMode>(savedView?.mode === 'preview' ? 'preview' : 'plain');

	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, { mode } satisfies ComposerViewState);
		};
	});
	let plainCmView: EditorView | null = $state(null);

	function focusPlainEditor(): boolean {
		if (!plainCmView) return false;
		plainCmView.focus();
		return true;
	}

	function handleNav(action: NavAction): boolean {
		// One editor: `insert` / `select` focus it; h/l flip to the read view
		// and back.
		if (action === 'left' || action === 'right') {
			mode = mode === 'plain' ? 'preview' : 'plain';
			return true;
		}
		if ((action === 'insert' || action === 'select') && mode === 'plain') return focusPlainEditor();
		return false;
	}

	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	// Publish the plain-mode CM view to AppState so cross-buffer actions
	// (e.g. SearchBuffer's "insert at cursor") can dispatch into it.
	$effect(() => {
		const v = mode === 'plain' ? plainCmView : null;
		untrack(() => app.setComposerActiveView(v));
	});

	// Clear on unmount — handlers shouldn't dispatch into a disposed view.
	$effect(() => {
		return () => untrack(() => app.setComposerActiveView(null));
	});

	// Refresh the saved-drafts list when the composer mounts.
	$effect(() => {
		untrack(() => app.refreshComposeDrafts());
	});
</script>

<ComposeView
	bind:mode
	bind:plainCmView
	compose={app.compose}
	canPublish={identityCanSign(app.identityStatus)}
	onupdate={app.handleComposeUpdate}
	oncancel={app.handleCancelCompose}
	onsendtochat={app.handleComposeToChat}
	onpublish={app.handleComposePublish}
	onpreview={app.handleComposePreview}
	onsavedraft={app.handleComposeSaveDraft}
	drafts={app.composeDrafts}
	onloaddraft={app.handleLoadDraft}
	ondeletedraft={app.handleDeleteDraft}
	ondelete={app.handleDeleteFromCompose}
	ondeletepermanent={app.handleDeletePermanent}
	syncMode={app.syncMode}
	lineNumbers={app.editorLineNumbers}
	vimMode={app.editorVimMode}
	onsenditemtochat={app.handleSendItemToChat}
	ontogglereadonly={app.handleToggleReadonly}
	onlocksource={app.handleLockToSource}
	oncrosspanelcopy={app.handleCrossPanelCopy}
	onreorder={app.reorderComposeSection}
/>
