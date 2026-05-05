<script lang="ts">
	import { untrack } from 'svelte';
	import { getAppState } from '$lib/state.svelte';
	import * as api from '$lib/api';
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
		return false;
	}

	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	function spawnReader(pubkey: string, d_tag: string, label: string | null) {
		store.openBuffer({
			className: 'work',
			buffer: {
				id: `reader:30040:${pubkey}:${d_tag}`,
				kind: 'reader',
				label: 'reader',
				kicker: label ?? d_tag
			}
		});
	}

	async function onSelect(result: SearchResult) {
		if (!result.addr) return;
		if (result.kind === 30040) {
			spawnReader(result.addr.pubkey, result.addr.d_tag, result.title);
			return;
		}
		if (result.kind === 30041) {
			// Section: look up parent publication via the 'a' tag.
			try {
				const resp = await api.getEvent(result.event_id);
				const event = resp.event as Record<string, unknown> | null;
				const tags = (event?.tags as string[][] | undefined) ?? [];
				const aTag = tags.find((t) => t[0] === 'a' && t[1]?.startsWith('30040:'));
				if (aTag) {
					const [, ref] = aTag;
					const parts = ref.split(':');
					if (parts.length >= 3) {
						spawnReader(parts[1], parts.slice(2).join(':'), result.title);
						return;
					}
				}
			} catch {
				// fall through
			}
		}
		// Fallback: delegate to AppState's standalone-section path. This sets
		// app.publication / app.sections singletons; the shell's ReaderBuffer
		// doesn't render those, so this is a placeholder until we add a
		// reader:event:<id> id format. For now, log so the user knows.
		console.warn('[SearchBuffer] no shell-aware open path for kind', result.kind, 'event', result.event_id);
		app.handleSelectResult(result);
	}
</script>

<SearchPanel
	{cursor}
	bind:listEl
	results={app.searchResults}
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
/>
