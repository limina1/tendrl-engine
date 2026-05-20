<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';
	import type { SearchResult, ContextItem } from '$lib/types';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	// Three sibling sub-views — like the reader's outline/paginated/
	// continuous. h/l cycles them; j/k walks the active tab's list.
	// Order is internal → external: Search and Refs both work with
	// already-known events; KB last because its pages are external
	// structures being pulled into the pool.
	type Tab = 'search' | 'refs' | 'import';
	const TAB_ORDER: Tab[] = ['search', 'refs', 'import'];
	let activeTab: Tab = $state('search');

	// Per-tab cursor — kept here so each tab remembers its position when
	// the user h/l-cycles away and back.
	let searchCursor = $state(0);
	let importCursor = $state(0);
	let refsCursor = $state(0);

	let listEl: HTMLDivElement | undefined = $state();

	function listLength(tab: Tab): number {
		if (tab === 'search') return app.searchResults.length;
		if (tab === 'import') return app.importPages.length;
		return app.heldEntries.length;
	}

	function getCursor(tab: Tab): number {
		if (tab === 'search') return searchCursor;
		if (tab === 'import') return importCursor;
		return refsCursor;
	}

	function setCursor(tab: Tab, v: number) {
		if (tab === 'search') searchCursor = v;
		else if (tab === 'import') importCursor = v;
		else refsCursor = v;
	}

	$effect(() => {
		// Clamp every tab's cursor when its list shrinks.
		for (const tab of TAB_ORDER) {
			const len = listLength(tab);
			const cur = getCursor(tab);
			if (cur >= len) setCursor(tab, Math.max(0, len - 1));
		}
	});

	function scrollCursorIntoView() {
		if (!listEl) return;
		const cur = getCursor(activeTab);
		const row = listEl.querySelector<HTMLDivElement>(`[data-cursor="${cur}"]`);
		if (!row) return;
		const listRect = listEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			listEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			listEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function cycleTab(dir: 1 | -1) {
		const i = TAB_ORDER.indexOf(activeTab);
		const next = TAB_ORDER[(i + dir + TAB_ORDER.length) % TAB_ORDER.length];
		activeTab = next;
		// On entering Import, lazy-load the document list if neither
		// page parse nor file list has been touched yet — same behaviour
		// the tab button has.
		if (next === 'import' && app.importPages.length === 0 && app.documentFiles.length === 0) {
			app.handleListDocuments();
		}
		queueMicrotask(scrollCursorIntoView);
	}

	function openHeldItem(item: ContextItem) {
		// Same routing the standalone RefsBuffer uses — prefer the
		// addressable coordinate (latest replaceable version) and fall
		// back to the pinned event id.
		if (item.source_addr) app.openAddressableInModal(item.source_addr);
		else if (item.source_event_id) app.getEventForModal(item.source_event_id);
	}

	function handleNav(action: NavAction): boolean {
		// h/l cycles tabs — outside any list, so it works even when the
		// active tab is empty.
		if (action === 'left') { cycleTab(-1); return true; }
		if (action === 'right') { cycleTab(1); return true; }

		const total = listLength(activeTab);
		if (total === 0) return false;
		let cur = getCursor(activeTab);

		if (action === 'down') {
			setCursor(activeTab, Math.min(total - 1, cur + 1));
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'up') {
			setCursor(activeTab, Math.max(0, cur - 1));
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'top') {
			setCursor(activeTab, 0);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'bottom') {
			setCursor(activeTab, total - 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		cur = getCursor(activeTab);
		if (action === 'select') {
			if (activeTab === 'search') {
				const r = app.searchResults[cur];
				if (r) onSelect(r);
			} else if (activeTab === 'import') {
				const p = app.importPages[cur];
				if (p) app.handleImportPageToContext(p);
			} else {
				const item = app.heldEntries[cur];
				if (item) openHeldItem(item);
			}
			return true;
		}
		if (action === 'menu') {
			// `m` always opens the event menu — including for comments and
			// highlights that `select` short-circuits into the reader.
			// The menu is the universal inspection / routing surface.
			if (activeTab === 'search') {
				const r = app.searchResults[cur];
				if (r) app.handleViewJson(r);
			} else if (activeTab === 'refs') {
				const item = app.heldEntries[cur];
				if (item) openHeldItem(item);
			}
			// Import pages aren't events — no menu to open.
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
	cursor={searchCursor}
	{importCursor}
	{refsCursor}
	bind:activeTab
	heldItems={app.heldEntries}
	onopenheld={openHeldItem}
	onreleaseheld={app.dropPoolItem}
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
