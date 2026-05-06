<script lang="ts">
	import { untrack } from 'svelte';
	import type { EditorView } from '@codemirror/view';
	import { getAppState } from '$lib/state.svelte';
	import ComposeView from '$lib/components/ComposeView.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	type ComposeMode = 'full' | 'plain' | 'preview';

	let cursor = $state(0);
	let mode = $state<ComposeMode>(app.composeDefaultMode);
	let sectionsListEl: HTMLDivElement | undefined = $state();
	let plainCmView: EditorView | null = $state(null);

	$effect(() => {
		const len = app.compose.sections.length;
		if (cursor >= len) cursor = Math.max(0, len - 1);
	});

	function scrollCursorIntoView() {
		if (!sectionsListEl) return;
		const row = sectionsListEl.querySelector<HTMLElement>(`[data-cursor="${cursor}"]`);
		if (!row) return;
		const listRect = sectionsListEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			sectionsListEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			sectionsListEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function focusCursoredSection(): boolean {
		if (!sectionsListEl) return false;
		const row = sectionsListEl.querySelector<HTMLElement>(`[data-cursor="${cursor}"]`);
		if (!row) return false;
		// Prefer the content textarea over the title input — the textarea is
		// the main editing surface. focusin will flip mode → 'insert'.
		const textarea = row.querySelector<HTMLTextAreaElement>('textarea');
		const target =
			textarea ?? row.querySelector<HTMLInputElement>('input[class~="compose-section-title"]');
		if (!target) return false;
		target.focus();
		return true;
	}

	function focusPlainEditor(): boolean {
		if (!plainCmView) return false;
		plainCmView.focus();
		return true;
	}

	function handleNav(action: NavAction): boolean {
		// In plain (and preview) mode there's no per-section cursor — h/l
		// still cycles back to full. 'insert' focuses the plain textarea.
		if (mode !== 'full') {
			if (action === 'left' || action === 'right') {
				mode = mode === 'plain' ? 'full' : 'plain';
				return true;
			}
			if (action === 'insert' && mode === 'plain') return focusPlainEditor();
			return false;
		}

		const total = app.compose.sections.length;
		if (action === 'left' || action === 'right') {
			mode = 'plain';
			return true;
		}
		if (total === 0) return false;
		if (action === 'down') {
			cursor = Math.min(total - 1, cursor + 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'up') {
			cursor = Math.max(0, cursor - 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'top') {
			cursor = 0;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'bottom') {
			cursor = total - 1;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'select' || action === 'insert') {
			return focusCursoredSection();
		}
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
</script>

<ComposeView
	bind:mode
	bind:sectionsListEl
	bind:plainCmView
	{cursor}
	compose={app.compose}
	canPublish={app.identityStatus?.state === 'unlocked'}
	onupdate={app.handleComposeUpdate}
	oncancel={app.handleCancelCompose}
	onsendtochat={app.handleComposeToChat}
	onpublish={app.handleComposePublish}
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
