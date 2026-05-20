<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';
	import type { SearchResult } from '$lib/types';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	let cursor = $state(0);
	let listEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		// Clamp cursor when result set changes (new search, filter, etc.).
		const len = app.searchResults.length;
		if (cursor >= len) cursor = Math.max(0, len - 1);
	});

	function scrollCursorIntoView() {
		if (!listEl) return;
		const row = listEl.querySelector<HTMLDivElement>(`[data-cursor="${cursor}"]`);
		if (!row) return;
		const listRect = listEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			listEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			listEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function handleNav(action: NavAction): boolean {
		const total = app.searchResults.length;
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
		if (action === 'select' || action === 'right') {
			const r = app.searchResults[cursor];
			if (r) onSelect(r);
			return true;
		}
		if (action === 'menu') {
			// `m` always opens the event menu — including for comments and
			// highlights that `select` short-circuits into the reader.
			// The menu is the universal inspection / routing surface.
			const r = app.searchResults[cursor];
			if (r) app.handleViewJson(r);
			return true;
		}
		return false;
	}

	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	// Enter / click on a result opens the unified event modal
	// (EventViewModal — read / find / insert / inspect), rendered by
	// +layout.svelte so it overlays the whole shell.
	//
	// Exception: NIP-22 comments (kind 1111) and NIP-84 highlights (kind
	// 9802) bypass the modal — they aren't standalone destinations.
	// `app.handleSelectResult` resolves their referenced article and
	// opens it directly with a focus marker.
	function onSelect(result: SearchResult) {
		if (result.kind === 1111 || result.kind === 9802) {
			app.handleSelectResult(result);
			return;
		}
		app.handleViewJson(result);
	}
</script>

<SearchPanel
	{cursor}
	bind:listEl
	results={app.searchResults}
	profiles={app.searchProfiles}
	tagCounts={app.searchTagCounts}
	count={app.searchCount}
	localCount={app.searchLocalCount}
	relayCount={app.searchRelayCount}
	loading={app.searchLoading}
	searchContext={app.docMode === 'empty' ? 'publications' : 'knowledge base'}
	onsearch={app.handleSearch}
	onselect={onSelect}
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
	canPromptRelays={
		app.networkStatus?.mode === 'confirm' &&
		app.searchLastQuery !== '' &&
		!app.searchLoading
	}
	onsearchrelays={app.handleSearchViaRelays}
/>
