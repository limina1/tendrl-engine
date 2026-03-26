<script lang="ts">
	import type {
		ChatResponse,
		SearchResult,
		PublicationSummary,
		PublicationDetail,
		Section,
		LazySection,
		ComposeState,
		ContextItem,
		Fragment,
		TagEntry,
		ViewMode,
		DocMode,
		SyncMode,
		ButtonLabels
	} from '$lib/types';
	import * as api from '$lib/api';
	import WorkbenchToolbar from '$lib/components/WorkbenchToolbar.svelte';
	import PanelFrame from '$lib/components/PanelFrame.svelte';
	import ChatPanel from '$lib/components/ChatPanel.svelte';
	import DocumentPanel from '$lib/components/DocumentPanel.svelte';
	import SearchPanel from '$lib/components/SearchPanel.svelte';

	// Chat state
	let chat: ChatResponse | null = $state({
		fragments: [],
		fragment_count: 0,
		edit_mode: false,
		edit_buffer: null,
		system_prompt: null,
		context_count: 0,
		generating: false
	});
	let chatLoading = $state(false);
	let systemExpanded = $state(false);
	let contextExpanded = $state(false);
	let originalEditBuffer = $state('');
	let chatHiddenFragmentIds: Set<number> = $state(new Set());

	// === Unified item pool ===
	let items: ContextItem[] = $state([]);
	const contextEntries = $derived(items.filter((i) => i.in_context));
	const composeSections = $derived(items.filter((i) => i.in_compose));

	// Map fragment_id → compose item for chat-origin items
	const chatFragmentItems = $derived(
		new Map(
			items
				.filter((i) => i.origin === 'chat' && i.source_fragment_id != null)
				.map((i) => [i.source_fragment_id!, i])
		)
	);

	// Compose publication-level metadata (separate from items)
	let composeTitle = $state('');
	let composeTags: TagEntry[] = $state([]);
	const compose = $derived<ComposeState>({
		title: composeTitle,
		tags: composeTags,
		sections: composeSections
	});

	// Document state
	let docMode: DocMode = $state('empty');
	let publication: PublicationDetail | null = $state(null);
	let sections: LazySection[] = $state([]);
	let viewMode: ViewMode = $state('outline');
	let currentSection = $state(0);
	let previewVisible = $state(false);
	let docLoading = $state(false);

	// Section loading deduplication
	const loadingPromises = new Map<number, Promise<void>>();

	// Publication feed (shown in empty state)
	let feed: PublicationSummary[] = $state([]);
	let feedLoading = $state(false);
	let feedSyncing = $state(false);
	let feedLoadingMore = $state(false);
	let feedHasMore = $state(true);

	// Search state
	let searchResults: SearchResult[] = $state([]);
	let searchCount = $state(0);
	let searchLocalCount = $state(0);
	let searchRelayCount = $state(0);
	let searchLoading = $state(false);

	// JSON modal
	let jsonModalData: unknown = $state(null);

	// Identity
	let myPubkey: string | null = $state(null);

	// Embedding
	let embeddingStatus: import('$lib/types').EmbeddingStatusResponse | null = $state(null);
	let embeddingSyncing = $state(false);

	// Relay config
	let fetchRelayUrls: string[] = $state([]);
	let authorCount = $state(0);

	// Document import
	let documentFiles: import('$lib/types').DocumentFile[] = $state([]);
	let importPages: import('$lib/types').ImportPage[] = $state([]);
	let importFilename = $state('');
	let importLoading = $state(false);

	// Ignore list
	let ignoredCount = $state(0);
	let ignoredEventIds: string[] = $state([]);
	let ignoredPubkeys: string[] = $state([]);

	async function refreshIgnoreList() {
		try {
			const il = await api.getIgnoreList();
			ignoredCount = il.ignored_event_count + il.ignored_pubkey_count;
			ignoredEventIds = il.event_ids;
			ignoredPubkeys = il.pubkeys;
		} catch {}
	}

	function handleViewIgnored() {
		docMode = 'ignored';
		refreshIgnoreList();
	}

	async function handleUnignore(type: 'event' | 'pubkey', id: string) {
		try {
			if (type === 'event') {
				await api.unignoreEvents([id]);
			} else {
				await api.unignoreEvents([], [id]);
			}
			await refreshIgnoreList();
			if (ignoredCount === 0) {
				docMode = 'empty';
				await loadFeed();
			}
		} catch (e) {
			console.error('Unignore failed:', e);
		}
	}

	async function handlePurge() {
		if (!confirm('This will show the command to delete the nostrdb database. Continue?')) return;
		try {
			const resp = await fetch('/api/v1/purge', { method: 'POST' });
			const data = await resp.json();
			alert(`To purge, stop the engine and run:\n\n${data.command}`);
		} catch (e) {
			console.error('Purge failed:', e);
		}
	}

	// Settings
	let syncMode: SyncMode = $state('explicit');
	let buttonLabels: ButtonLabels = $state('icon');

	// Panel collapse
	let chatCollapsed = $state(true);
	let docCollapsed = $state(false);
	let searchCollapsed = $state(true);

	const gridTemplate = $derived(
		[
			chatCollapsed ? 'auto' : '1fr',
			docCollapsed ? 'auto' : '2fr',
			searchCollapsed ? 'auto' : '1fr'
		].join(' ')
	);

	async function loadFeed() {
		feedLoading = true;
		try {
			const resp = await api.listPublications();
			feed = resp.publications;
			feedHasMore = resp.count >= 20;
			// If config didn't load pubkey, try to get it from feed data
			if (!myPubkey) {
				try {
					const cfg = await api.getConfig();
					myPubkey = cfg.my_pubkey;
				} catch { /* ignore */ }
			}
			// Prefetch profiles for feed authors (background)
			const pubkeys = [...new Set(resp.publications.map(p => p.author_pubkey))];
			api.prefetchProfiles(pubkeys);
		} catch {
			// Backend unavailable
		} finally {
			feedLoading = false;
		}
	}

	async function handleFeedSync() {
		feedSyncing = true;
		try {
			const resp = await api.listPublications(20, 'fetch_always');
			feed = resp.publications;
			feedHasMore = resp.count >= 20;
		} catch {
			// Relay fetch failed
		} finally {
			feedSyncing = false;
		}
	}

	async function handleFeedLoadMore() {
		if (feedLoadingMore || !feedHasMore || feed.length === 0) return;
		feedLoadingMore = true;
		try {
			const oldest = Math.min(...feed.map(p => p.created_at));
			const resp = await api.listPublications(20, 'local_only', oldest);
			if (resp.count === 0) {
				feedHasMore = false;
			} else {
				// Deduplicate by addr
				const existing = new Set(feed.map(p => `${p.addr.pubkey}:${p.addr.d_tag}`));
				const newPubs = resp.publications.filter(p => !existing.has(`${p.addr.pubkey}:${p.addr.d_tag}`));
				feed = [...feed, ...newPubs];
				feedHasMore = resp.count >= 20;
			}
		} catch {
			// silent
		} finally {
			feedLoadingMore = false;
		}
	}

	async function handleLoadSection(index: number) {
		if (index < 0 || index >= sections.length) return;
		const section = sections[index];
		if (section.status === 'loaded' || section.status === 'loading') return;
		if (loadingPromises.has(index)) return;

		sections[index] = { ...section, status: 'loading' };

		const promise = (async () => {
			try {
				const pubkey = publication!.addr.pubkey;
				const d_tag = publication!.addr.d_tag;
				const resp = await api.getSection(pubkey, d_tag, index);
				sections[index] = {
					...sections[index],
					title: resp.section.title ?? sections[index].title,
					content: resp.section.content,
					status: 'loaded'
				};
			} catch (e) {
				sections[index] = {
					...sections[index],
					status: 'error',
					error: String(e)
				};
			} finally {
				loadingPromises.delete(index);
			}
		})();

		loadingPromises.set(index, promise);
	}

	let initialized = $state(false);
	$effect(() => {
		if (initialized) return;
		initialized = true;
		(async () => {
			try {
				const cfg = await api.getConfig();
				myPubkey = cfg.my_pubkey;
				console.log('Config loaded, myPubkey:', myPubkey);
			} catch (e) {
				console.warn('Config fetch failed:', e);
			}
			try {
				chat = await api.getChat();
			} catch {
				// Backend unavailable
			}
			await loadFeed();
			try {
				embeddingStatus = await api.getEmbeddingStatus();
			} catch { /* embedding not enabled */ }
			await refreshIgnoreList();
			try {
				const rc = await api.getRelayConfig();
				fetchRelayUrls = rc.fetch.urls;
				authorCount = rc.authors.length;
			} catch {}
		})();
	});

	async function handleListDocuments() {
		try {
			const resp = await api.listDocuments();
			documentFiles = resp.files;
			importPages = [];
			importFilename = '';
		} catch (e) {
			console.error('List documents failed:', e);
		}
	}

	async function handleImportFile(file: File) {
		importLoading = true;
		try {
			const resp = await api.importDocument(file);
			importFilename = resp.filename;
			importPages = resp.pages;
			// Refresh file list
			handleListDocuments();
		} catch (e) {
			console.error('Import failed:', e);
		} finally {
			importLoading = false;
		}
	}

	async function handleParseDocument(filename: string) {
		importLoading = true;
		try {
			const resp = await api.parseDocument(filename);
			importFilename = resp.filename;
			importPages = resp.pages;
		} catch (e) {
			console.error('Parse failed:', e);
		} finally {
			importLoading = false;
		}
	}

	function handleImportPageToContext(page: import('$lib/types').ImportPage) {
		addToPool({
			title: page.title ?? `Page ${page.page_num}`,
			content: page.content,
			tags: [{ name: 'source', value: importFilename }, { name: 'page', value: String(page.page_num) }],
			original_content: page.content,
			origin: 'import' as const
		}, { context: true });
		syncContext();
	}

	function handleImportPageToCompose(page: import('$lib/types').ImportPage) {
		addToPool({
			title: page.title ?? `Page ${page.page_num}`,
			content: page.content,
			tags: [{ name: 'source', value: importFilename }, { name: 'page', value: String(page.page_num) }],
			original_content: page.content,
			origin: 'import' as const
		}, { compose: true });
		if (docMode !== 'compose') docMode = 'compose';
	}

	function handleImportPagesToContext(pages: import('$lib/types').ImportPage[]) {
		for (const page of pages) handleImportPageToContext(page);
	}

	function handleImportPagesToCompose(pages: import('$lib/types').ImportPage[]) {
		for (const page of pages) handleImportPageToCompose(page);
	}

	async function handleFetchAuthors() {
		try {
			const resp = await api.fetchAuthors();
			console.log(`Fetched ${resp.fetched} events for ${resp.authors} authors from ${resp.relays} relays`);
			await loadFeed();
		} catch (e) {
			console.error('Fetch authors failed:', e);
		}
	}

	async function handleFetchFromRelay(url: string, kinds: number[]) {
		try {
			const resp = await api.fetchFromRelay(url, kinds);
			console.log(`Fetched ${resp.fetched} events from ${resp.relay}`);
			await loadFeed();
		} catch (e) {
			console.error('Fetch from relay failed:', e);
		}
	}

	async function handleReindexEmbeddings() {
		embeddingSyncing = true;
		const pollInterval = setInterval(async () => {
			try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
		}, 1000);
		try {
			embeddingStatus = await api.reindexEmbeddings();
		} catch (e) {
			console.error('Reindex failed:', e);
		} finally {
			clearInterval(pollInterval);
			try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
			embeddingSyncing = false;
		}
	}

	async function handleSyncEmbeddings() {
		embeddingSyncing = true;

		// Poll status while sync runs in the background
		const pollInterval = setInterval(async () => {
			try {
				embeddingStatus = await api.getEmbeddingStatus();
			} catch { /* ignore poll errors */ }
		}, 1000);

		try {
			embeddingStatus = await api.syncEmbeddings();
		} catch (e) {
			console.error('Embedding sync failed:', e);
		} finally {
			clearInterval(pollInterval);
			// Final status refresh
			try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
			embeddingSyncing = false;
		}
	}

	// --- Helpers ---

	function makeItem(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean }
	): ContextItem {
		return {
			...fields,
			id: crypto.randomUUID(),
			context_content: fields.content,
			modified: false,
			readonly: false,
			in_context: target.context ?? false,
			in_compose: target.compose ?? false
		};
	}

	async function fetchEventContent(result: SearchResult): Promise<string> {
		try {
			const resp = await api.getEvent(result.event_id);
			const event = resp.event as Record<string, unknown> | null;
			return (event?.content as string) ?? result.preview;
		} catch {
			return result.preview;
		}
	}

	function resultFields(result: SearchResult, content: string) {
		return {
			title: result.title ?? '[Untitled]',
			content,
			tags: (result.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
			source_event_id: result.event_id,
			source_addr: result.addr,
			original_content: content,
			origin: 'search' as const
		};
	}

	// Remove items that belong to neither panel
	function gc() {
		items = items.filter((e) => e.in_context || e.in_compose);
	}

	// Dedup by source_event_id or source_addr — flip flags if exists, else create
	function addToPool(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean }
	) {
		const existing = items.find((e) => {
			if (fields.source_event_id && e.source_event_id === fields.source_event_id) return true;
			if (
				fields.source_addr &&
				e.source_addr &&
				fields.source_addr.kind === e.source_addr.kind &&
				fields.source_addr.pubkey === e.source_addr.pubkey &&
				fields.source_addr.d_tag === e.source_addr.d_tag
			)
				return true;
			return false;
		});
		if (existing) {
			items = items.map((e) =>
				e.id === existing.id
					? {
							...e,
							in_context: e.in_context || (target.context ?? false),
							in_compose: e.in_compose || (target.compose ?? false),
							// Snapshot content on first bridge to other panel
							...(target.context && !e.in_context ? { context_content: e.content } : {}),
							...(target.compose && !e.in_compose ? { content: e.context_content } : {})
						}
					: e
			);
		} else {
			items = [...items, makeItem(fields, target)];
		}
	}

	// --- Sync context to backend ---

	async function syncContext() {
		const ctx = items.filter((e) => e.in_context);
		try {
			chat = await api.replaceContext(
				ctx.map((e) => ({ title: e.title, content: e.context_content }))
			);
		} catch {
			// silent
		}
	}

	// --- Chat handlers ---

	async function handleSend(content: string) {
		// Optimistically show user message before waiting for LLM
		if (chat) {
			const nextId = Math.max(0, ...chat.fragments.map((f) => f.id)) + 1;
			chat = {
				...chat,
				fragments: [...chat.fragments, { id: nextId, role: 'user', content }],
				fragment_count: chat.fragment_count + 1
			};
		}
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

	// --- Item handlers (shared, used by both context and compose) ---

	function handleUpdateContextItem(id: string, title: string, contextContent: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, title, context_content: contextContent } : e
		);
		syncContext();
	}

	function handleResetContextItem(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, context_content: e.original_content } : e
		);
		syncContext();
	}

	function handleRemoveFromContext(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, in_context: false } : e
		);
		gc();
		syncContext();
	}

	// Context 🗑 → set in_context false, gc
	function handleDeleteFromContext(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_context: false } : e));
		gc();
		syncContext();
	}

	// Compose 🗑 → set in_compose false, gc
	function handleDeleteFromCompose(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_compose: false } : e));
		gc();
	}

	// 🗑🗑 → permanent delete from pool
	function handleDeletePermanent(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.filter((e) => !ids.has(e.id));
		syncContext();
	}

	// Context □ → compose (copy context_content into compose content)
	function handleContextToCompose(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
		items = items.map((e) =>
			ids.has(e.id)
				? { ...e, in_compose: true, in_context: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Compose ◂ → context (copy compose content into context_content)
	// For chat-origin items, hide the source fragment
	function handleComposeToChat(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
		// Hide chat fragments for items moving to context
		const nextHidden = new Set(chatHiddenFragmentIds);
		for (const item of checkedItems) {
			if (item.origin === 'chat' && item.source_fragment_id != null) {
				nextHidden.add(item.source_fragment_id);
			}
		}
		chatHiddenFragmentIds = nextHidden;
		items = items.map((e) =>
			ids.has(e.id)
				? { ...e, in_context: true, in_compose: true, context_content: e.content }
				: e
		);
		syncContext();
	}

	// Per-item: send single item to context (snapshot compose content)
	// For chat-origin items, hide the source fragment
	function handleSendItemToChat(id: string) {
		const item = items.find((e) => e.id === id);
		if (item?.origin === 'chat' && item.source_fragment_id != null) {
			chatHiddenFragmentIds = new Set([...chatHiddenFragmentIds, item.source_fragment_id]);
		}
		items = items.map((e) =>
			e.id === id
				? { ...e, in_context: true, in_compose: true, context_content: e.content }
				: e
		);
		syncContext();
	}

	// Per-item: send single item to compose (snapshot context content)
	function handleSendItemToCompose(id: string) {
		items = items.map((e) =>
			e.id === id
				? { ...e, in_context: true, in_compose: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Toggle readonly on any item
	// Cross-panel lock toggle (context/compose badge)
	function handleToggleReadonly(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, readonly: !e.readonly } : e
		);
	}

	// Origin lock: toggle readonly + reset to original source content when locking
	function handleLockToSource(id: string) {
		items = items.map((e) => {
			if (e.id !== id) return e;
			const locking = !e.readonly;
			if (locking) {
				return {
					...e,
					readonly: true,
					content: e.original_content,
					context_content: e.original_content,
					modified: false
				};
			}
			return { ...e, readonly: false };
		});
		syncContext();
	}

	// Cross-panel copy: overwrite other panel's content from this panel
	function handleCrossPanelCopy(id: string, fromPanel: string) {
		items = items.map((e) => {
			if (e.id !== id) return e;
			if (fromPanel === 'compose') {
				return { ...e, context_content: e.content, readonly: false };
			} else if (fromPanel === 'context') {
				return { ...e, content: e.context_content, modified: e.context_content !== e.original_content, readonly: false };
			}
			return e;
		});
		syncContext();
	}

	// Chat fragments □ → add as new items with in_compose (fragments stay visible)
	function handleChatFragmentsToCompose(fragments: Fragment[]) {
		const newItems = fragments.map((f) =>
			makeItem(
				{ title: `[${f.role}]`, content: f.content, tags: [], original_content: f.content, origin: 'chat', source_fragment_id: f.id },
				{ compose: true }
			)
		);
		items = [...items, ...newItems];
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Chat fragments ▸
	async function handleChatPublishFragments(fragments: Fragment[]) {
		if (!fragments.length) return;
		try {
			await api.publish({
				title: `Chat export ${new Date().toISOString().slice(0, 10)}`,
				tags: [],
				sections: fragments.map(f => ({
					title: `[${f.role}]`,
					content: f.content,
					tags: []
				})),
				sign: false,
				broadcast: false
			});
			await loadFeed();
		} catch (e) {
			console.error('Publish fragments failed:', e);
		}
	}

	// Compose ▸
	async function handleComposePublish(_items: ContextItem[]) {
		if (!compose.title && !compose.sections.length) return;
		try {
			const resp = await api.publish({
				title: compose.title,
				tags: compose.tags.map(t => [t.name, t.value] as [string, string]),
				sections: compose.sections.map(s => ({
					title: s.title,
					content: s.content,
					tags: s.tags.map(t => [t.name, t.value] as [string, string])
				})),
				sign: false,
				broadcast: false
			});
			console.log('Published:', resp.publication_id);
			await loadFeed();
		} catch (e) {
			console.error('Publish compose failed:', e);
		}
	}

	// --- Compose update reconciliation ---

	function handleComposeUpdate(state: ComposeState) {
		composeTitle = state.title;
		composeTags = state.tags;

		const updatedById = new Map(state.sections.map((s) => [s.id, s]));

		// Update existing compose items, remove ones dropped from sections
		items = items
			.map((item) => {
				if (!item.in_compose) return item;
				const updated = updatedById.get(item.id);
				if (updated) {
					updatedById.delete(item.id);
					return { ...updated, in_context: item.in_context, in_compose: true, context_content: item.context_content };
				}
				// Removed from compose
				if (item.in_context) return { ...item, in_compose: false };
				return null;
			})
			.filter((item): item is ContextItem => item !== null);

		// Add new items (from + Section or plain text parse)
		const existingIds = new Set(items.map((i) => i.id));
		for (const [id, section] of updatedById) {
			if (!existingIds.has(id)) {
				items = [...items, { ...section, in_context: false, in_compose: true }];
			}
		}

		syncContext();
	}

	// --- Search handlers ---

	async function handleSearch(query: string) {
		// Empty search: reset feed and clear results
		if (!query.trim()) {
			searchResults = [];
			searchCount = 0;
			searchLocalCount = 0;
			searchRelayCount = 0;
			if (docMode === 'empty') await loadFeed();
			return;
		}

		searchLoading = true;
		try {
			let effectiveQuery = query;

			// Context-aware kind filter based on document panel state
			// Only add k:30040 when there's no text content to search
			// (text search needs content-bearing kinds like 30041)
			const hasTextTerms = effectiveQuery.split(/\s+/).some(t =>
				!t.startsWith('k:') && !t.startsWith('by:') && !t.startsWith('t:') &&
				!t.startsWith('~:') && !t.startsWith('d:') && !t.startsWith('"')
			);
			const isFeedSearch = docMode === 'empty' && !query.includes('k:');
			if (isFeedSearch && !hasTextTerms) {
				effectiveQuery = `k:30040 ${effectiveQuery}`;
			}

			// Default to by:me if pubkey is configured
			if (myPubkey && !query.includes('by:')) {
				effectiveQuery = `by:me ${effectiveQuery}`;
			}

			console.log('search:', effectiveQuery, 'myPubkey:', myPubkey);
			const resp = await api.search(effectiveQuery, undefined, myPubkey ?? undefined);
			searchResults = resp.results;
			searchCount = resp.count;
			searchLocalCount = resp.local_count;
			searchRelayCount = resp.relay_count;

			// In feed mode, search results drive the feed display
			if (docMode === 'empty') {
				const pubs = resp.results.filter(r => r.kind === 30040 && r.addr);
				if (pubs.length > 0) {
					// Deduplicate by pubkey:d_tag (nostrdb returns all versions)
					const seen = new Set<string>();
					feed = [];
					for (const r of pubs) {
						const key = `${r.addr!.pubkey}:${r.addr!.d_tag}`;
						if (seen.has(key)) continue;
						seen.add(key);
						feed.push({
							addr: r.addr!,
							title: r.title,
							summary: r.preview || null,
							image: null,
							author_pubkey: r.author,
							version: null,
							created_at: r.created_at,
							section_count: r.tags.filter(t => t[0] === 'a').length
						});
					}
					feedHasMore = false;
				}
			}
		} catch (e) {
			console.error('Search failed:', e);
		} finally {
			searchLoading = false;
		}
	}

	// Search ◂ → add or flag in_context
	async function handleAddToContext(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { context: true });
		syncContext();
	}

	// Search □ → add or flag in_compose
	async function handleAddToCompose(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { compose: true });
		if (docMode !== 'compose') docMode = 'compose';
	}

	// Search bulk ◂
	async function handleAddManyToContext(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { context: true });
		}
		syncContext();
	}

	// Search bulk □
	async function handleAddManyToCompose(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { compose: true });
		}
		if (docMode !== 'compose') docMode = 'compose';
	}

	// JSON modal
	async function handleViewJson(result: SearchResult) {
		try {
			const resp = await api.getEvent(result.event_id);
			jsonModalData = resp.event;
		} catch {
			jsonModalData = result;
		}
	}

	async function handleIgnoreEvent(result: SearchResult) {
		try {
			await api.ignoreEvents([result.event_id]);
			await refreshIgnoreList();
			searchResults = searchResults.filter(r => r.event_id !== result.event_id);
			searchCount = searchResults.length;
			if (docMode === 'empty') await loadFeed();
		} catch (e) {
			console.error('Ignore failed:', e);
		}
	}

	async function handleIgnorePubkey(result: SearchResult) {
		try {
			await api.ignoreEvents([], [result.author]);
			await refreshIgnoreList();
			searchResults = searchResults.filter(r => r.author !== result.author);
			searchCount = searchResults.length;
			if (docMode === 'empty') await loadFeed();
		} catch (e) {
			console.error('Ignore pubkey failed:', e);
		}
	}

	// --- Document handlers ---

	async function openPublication(pubkey: string, d_tag: string) {
		// Switch to reading mode immediately so the user sees "Loading..."
		docMode = 'reading';
		docLoading = true;
		publication = null;
		sections = [];
		loadingPromises.clear();
		try {
			const pubResp = await api.getPublication(pubkey, d_tag, 'local_first');
			publication = pubResp.publication;

			// Build sections from TOC (includes both 30041 sections and 30040 nested)
			sections = pubResp.toc.map((entry, i) => ({
				addr: entry.addr,
				title: entry.title,
				content: null,
				position: i,
				status: 'pending' as const
			}));

			viewMode = 'outline';
			currentSection = 0;
			previewVisible = false;
		} catch (e) {
			console.error('Failed to open publication:', pubkey, d_tag, e);
			// Go back to feed on error
			docMode = 'empty';
		} finally {
			docLoading = false;
		}
	}

	async function handleSelectResult(result: SearchResult) {
		if (!result.addr) return;
		if (result.kind === 30040) {
			// Publication index — open directly
			await openPublication(result.addr.pubkey, result.addr.d_tag);
		} else if (result.kind === 30041) {
			// Section — look for parent publication via its tags
			try {
				const resp = await api.getEvent(result.event_id);
				const event = resp.event as Record<string, unknown> | null;
				const tags = (event?.tags as string[][] | undefined) ?? [];
				const aTag = tags.find((t) => t[0] === 'a' && t[1]?.startsWith('30040:'));
				if (aTag) {
					const [, ref] = aTag;
					const parts = ref.split(':');
					if (parts.length >= 3) {
						await openPublication(parts[1], parts.slice(2).join(':'));
						// Navigate to the section within the publication
						const idx = sections.findIndex(
							(s) => s.addr?.d_tag === result.addr!.d_tag && s.addr?.pubkey === result.addr!.pubkey
						);
						if (idx >= 0) {
							currentSection = idx;
							viewMode = 'paginated';
						}
						return;
					}
				}
			} catch {
				// Fall through to direct load attempt
			}
			// No parent found — try loading as standalone
			await openPublication(result.addr.pubkey, result.addr.d_tag).catch(() => {});
		} else {
			// Other kinds — try loading, silently fail
			await openPublication(result.addr.pubkey, result.addr.d_tag).catch(() => {});
		}
	}

	function handleOpenFeedPublication(pub_summary: PublicationSummary) {
		openPublication(pub_summary.addr.pubkey, pub_summary.addr.d_tag);
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
		// Clear compose flags, gc, start fresh
		items = [
			...items.map((e) => ({ ...e, in_compose: false })).filter((e) => e.in_context),
			makeItem({ title: '', content: '', tags: [], original_content: '', origin: 'compose' }, { compose: true })
		];
		composeTitle = '';
		composeTags = [];
		docMode = 'compose';
		previewVisible = false;
	}

	function handleCancelCompose() {
		docMode = publication ? 'reading' : 'empty';
	}

	// Document reading ◂ → add or flag sections in_context
	function handleDocToChat() {
		if (!sections.length) return;
		for (const s of sections) {
			if (!s.content) continue;
			addToPool(
				{
					title: s.title ?? '[Section]',
					content: s.content ?? '',
					tags: [],
					source_addr: s.addr,
					original_content: s.content ?? '',
					origin: 'search'
				},
				{ context: true }
			);
		}
		syncContext();
	}

	// Document reading ▸
	async function handleDocPublish() {
		if (!publication || !sections.length) return;
		try {
			const loadedSections = sections.filter(s => s.status === 'loaded' && s.content);
			if (!loadedSections.length) return;
			const resp = await api.publish({
				title: publication.title ?? 'Untitled',
				tags: [],
				sections: loadedSections.map(s => ({
					title: s.title ?? '',
					content: s.content ?? '',
					tags: []
				})),
				sign: false,
				broadcast: false
			});
			console.log('Published from reader:', resp.publication_id);
			await loadFeed();
		} catch (e) {
			console.error('Publish doc failed:', e);
		}
	}
</script>

<div class="workbench">
	<WorkbenchToolbar
		{syncMode}
		{buttonLabels}
		{embeddingStatus}
		{embeddingSyncing}
		{ignoredCount}
		onsetsyncmode={(m: SyncMode) => (syncMode = m)}
		onsetbuttonlabels={(m: ButtonLabels) => (buttonLabels = m)}
		onhome={() => { docMode = 'empty'; publication = null; sections = []; docCollapsed = false; if (searchCount === 0) loadFeed(); }}
		onsyncembeddings={handleSyncEmbeddings}
		onreindexembeddings={handleReindexEmbeddings}
		onviewignored={handleViewIgnored}
		onpurge={handlePurge}
	/>

	<div class="workbench-panels" style:grid-template-columns={gridTemplate}>
		<PanelFrame title="Chat" collapsed={chatCollapsed} ontoggle={() => (chatCollapsed = !chatCollapsed)}>
			<ChatPanel
				{chat}
				loading={chatLoading}
				{systemExpanded}
				{contextExpanded}
				{contextEntries}
				ontogglesystem={() => (systemExpanded = !systemExpanded)}
				ontogglecontext={() => (contextExpanded = !contextExpanded)}
				onsend={handleSend}
				onreset={handleReset}
				onedit={handleEdit}
				onapplyedit={handleApplyEdit}
				oncanceledit={handleCancelEdit}
				onsetsystem={handleSetSystem}
				onupdatecontext={handleUpdateContextItem}
				onresetcontext={handleResetContextItem}
				onremovecontext={handleRemoveFromContext}
				onsendtocompose={handleContextToCompose}
				onsendfragmentstocompose={handleChatFragmentsToCompose}
				onpublishfragments={handleChatPublishFragments}
				ondeletecontext={handleDeleteFromContext}
				ondeletepermanentcontext={handleDeletePermanent}
				{syncMode}
				ontogglereadonly={handleToggleReadonly}
				onlocksource={handleLockToSource}
				oncrosspanelcopy={handleCrossPanelCopy}
				onsenditemtocompose={handleSendItemToCompose}
				{chatHiddenFragmentIds}
				{chatFragmentItems}
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
				{feed}
				{feedLoading}
				{feedSyncing}
				{feedLoadingMore}
				{feedHasMore}
				onviewmode={handleViewMode}
				ontogglepreview={handleTogglePreview}
				oncompose={handleCompose}
				onnavigate={handleNavigate}
				oncomposeupdate={handleComposeUpdate}
				oncancelcompose={handleCancelCompose}
				onsendtochat={handleComposeToChat}
				onpublishcompose={handleComposePublish}
				ondeletecompose={handleDeleteFromCompose}
				ondeletepermanentcompose={handleDeletePermanent}
				ondoctochat={handleDocToChat}
				ondocpublish={handleDocPublish}
				onopenpub={handleOpenFeedPublication}
				onfeedsync={handleFeedSync}
				onfetchfromrelay={handleFetchFromRelay}
				onfetchauthors={handleFetchAuthors}
				fetchRelays={fetchRelayUrls}
				{authorCount}
				onfeedloadmore={handleFeedLoadMore}
				onloadsection={handleLoadSection}
				onignoreevent={async (id) => { try { await api.ignoreEvents([id]); await refreshIgnoreList(); await loadFeed(); } catch {} }}
				onignorepubkey={async (pk) => { try { await api.ignoreEvents([], [pk]); await refreshIgnoreList(); await loadFeed(); } catch {} }}
				{ignoredEventIds}
				{ignoredPubkeys}
				onunignore={handleUnignore}
				{syncMode}
				onsenditemtochat={handleSendItemToChat}
				ontogglereadonly={handleToggleReadonly}
				onlocksource={handleLockToSource}
				oncrosspanelcopy={handleCrossPanelCopy}
			/>
		</PanelFrame>

		<PanelFrame title="Search" collapsed={searchCollapsed} ontoggle={() => (searchCollapsed = !searchCollapsed)}>
			<SearchPanel
				results={searchResults}
				count={searchCount}
				localCount={searchLocalCount}
				relayCount={searchRelayCount}
				loading={searchLoading}
				searchContext={docMode === 'empty' ? 'publications' : 'knowledge base'}
				onsearch={handleSearch}
				onselect={handleSelectResult}
				onviewjson={handleViewJson}
				onaddtocontext={handleAddToContext}
				onaddtocompose={handleAddToCompose}
				onaddmanytocontext={handleAddManyToContext}
				onaddmanytocompose={handleAddManyToCompose}
				onignore={handleIgnoreEvent}
				onignorepubkey={handleIgnorePubkey}
				{documentFiles}
				{importPages}
				{importFilename}
				{importLoading}
				onlistdocuments={handleListDocuments}
				onimportfile={handleImportFile}
				onparsedocument={handleParseDocument}
				onimportpagetocontext={handleImportPageToContext}
				onimportpagetocompose={handleImportPageToCompose}
				onimportpagestocontext={handleImportPagesToContext}
				onimportpagestocompose={handleImportPagesToCompose}
				{items}
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
