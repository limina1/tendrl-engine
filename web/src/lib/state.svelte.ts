import { goto } from '$app/navigation';
import type {
	ChatResponse,
	SearchResult,
	PublicationSummary,
	PublicationDetail,
	LazySection,
	ComposeState,
	ContextItem,
	Fragment,
	TagEntry,
	ViewMode,
	DocMode,
	SyncMode,
	ButtonLabels,
	EditorInsertMode,
	ComposeDefaultMode,
	ImportPage,
	DocumentFile,
	EmbeddingStatusResponse,
	NetworkStatus,
	NetworkMode,
	ClaudeSessionSummary,
	ClaudeSessionMessage,
	IdentityStatus,
	NAddr,
} from '$lib/types';
import * as api from '$lib/api';

let _app: ReturnType<typeof _createAppState> | null = null;

export function createAppState() {
	if (_app) return _app;
	_app = _createAppState();
	return _app;
}

export function getAppState() {
	if (!_app) throw new Error('App state not initialized — call createAppState() first');
	return _app;
}

function _createAppState() {
	// --- Chat state ---
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

	// --- Unified item pool ---
	let items: ContextItem[] = $state([]);
	const contextEntries = $derived(items.filter((i) => i.in_context));
	const composeSections = $derived(items.filter((i) => i.in_compose));
	const chatFragmentItems = $derived(
		new Map(
			items
				.filter((i) => i.origin === 'chat' && i.source_fragment_id != null)
				.map((i) => [i.source_fragment_id!, i])
		)
	);

	// --- Compose metadata ---
	let composeTitle = $state('');
	let composeTags: TagEntry[] = $state([]);
	// Provenance for the publication being edited (set when a draft is seeded
	// from an existing 30040). Drives fork-marker tag emission and the
	// "structural change" gate on publish.
	let composeSourcePubAddr: NAddr | null = $state(null);
	let composeSourcePubEventId: string | null = $state(null);
	let composeSourceSectionOrder: NAddr[] = $state([]);
	const compose = $derived<ComposeState>({
		title: composeTitle,
		tags: composeTags,
		sections: composeSections,
		source_publication_addr: composeSourcePubAddr,
		source_publication_event_id: composeSourcePubEventId,
		source_section_order: composeSourceSectionOrder
	});

	// --- Document state (shared for reading mode) ---
	let docMode: DocMode = $state('empty');
	let publication: PublicationDetail | null = $state(null);
	let sections: LazySection[] = $state([]);
	let viewMode: ViewMode = $state('outline');
	let currentSection = $state(0);
	let previewVisible = $state(false);
	let docLoading = $state(false);
	const loadingPromises = new Map<number, Promise<void>>();

	// --- Feed ---
	let feed: PublicationSummary[] = $state([]);
	let feedLoading = $state(false);
	let feedSyncing = $state(false);
	let feedLoadingMore = $state(false);
	let feedHasMore = $state(true);

	// --- Search ---
	let searchResults: SearchResult[] = $state([]);
	let searchCount = $state(0);
	let searchLocalCount = $state(0);
	let searchRelayCount = $state(0);
	let searchLoading = $state(false);

	// --- JSON modal ---
	let jsonModalData: unknown = $state(null);

	// --- Search action modal ---
	// When set, the SearchActionModal overlay is open for this result.
	// Acts as a singleton — only one modal at a time, regardless of how
	// many SearchBuffers are open.
	let actionModalResult: SearchResult | null = $state(null);

	// --- Profile ---
	let profilePubkey: string | null = $state(null);

	// --- Identity ---
	let myPubkey: string | null = $state(null);
	let assistantPubkey: string | null = $state(null);
	let identityStatus: IdentityStatus | null = $state(null);
	let identityLoading = $state(false);
	let identityError: string | null = $state(null);
	let identityPollInterval: ReturnType<typeof setInterval> | null = null;
	let identityDisplayName: string | null = $state(null);
	const localPubkeys = $derived((() => {
		const pks: string[] = [];
		if (myPubkey) pks.push(myPubkey);
		if (assistantPubkey) pks.push(assistantPubkey);
		return new Set(pks);
	})());

	// --- Embedding ---
	let embeddingStatus: EmbeddingStatusResponse | null = $state(null);
	let embeddingSyncing = $state(false);

	// --- Network ---
	let networkStatus: NetworkStatus | null = $state(null);

	// --- Relay config ---
	let fetchRelayUrls: string[] = $state([]);
	let authorCount = $state(0);

	// --- Claude sessions ---
	let claudeSessions: ClaudeSessionSummary[] = $state([]);
	let claudeSessionDetail: { id: string; messages: ClaudeSessionMessage[]; count: number } | null = $state(null);
	let claudeSessionsLoading = $state(false);
	let sessionsExpanded = $state(false);
	let sessionPollInterval: ReturnType<typeof setInterval> | null = null;
	let watchingSessionId: string | null = null;
	let loadedSessionId: string | null = null;
	let loadedSessionMessageCount = 0;

	// --- Document import ---
	let documentFiles: DocumentFile[] = $state([]);
	let importPages: ImportPage[] = $state([]);
	let importFilename = $state('');
	let importLoading = $state(false);

	// --- Ignore list ---
	let ignoredCount = $state(0);
	let ignoredEventIds: string[] = $state([]);
	let ignoredPubkeys: string[] = $state([]);

	// --- Settings ---
	let syncMode: SyncMode = $state('explicit');
	let passthrough = $state(false);
	let buttonLabels: ButtonLabels = $state('icon');
	let editorInsertMode: EditorInsertMode = $state('append');
	let editorLineNumbers: boolean = $state(true);
	let editorVimMode: boolean = $state(true);
	let composeDefaultMode: ComposeDefaultMode = $state('full');

	// --- Panel collapse ---
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

	// --- Export/Import ---
	let exporting = $state(false);
	let importing = $state(false);
	let importProgress: { total: number; sent: number; ingested: number; skipped: number; errors: number; done: boolean } | null = $state(null);

	// ===================== Helpers =====================

	function makeItem(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean }
	): ContextItem {
		return {
			...fields,
			id: crypto.randomUUID(),
			context_content: fields.content,
			modified: false,
			// Sections imported from a published 30040 default to locked —
			// this matches the read-mode default ("I'm transcluding the
			// original as-is, attributed to its author"). The user unlocks
			// (yellow) to claim, or modifies (purple) to fork. Items from
			// other origins (chat, search, fresh compose) stay unlocked.
			readonly: fields.origin === 'import',
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

	function gc() {
		items = items.filter((e) => e.in_context || e.in_compose);
	}

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
							...(target.context && !e.in_context ? { context_content: e.content } : {}),
							...(target.compose && !e.in_compose ? { content: e.context_content } : {})
						}
					: e
			);
		} else {
			items = [...items, makeItem(fields, target)];
		}
	}

	// ===================== Context sync =====================

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

	// ===================== Feed =====================

	async function loadFeed() {
		feedLoading = true;
		try {
			const resp = await api.listPublications();
			feed = resp.publications;
			feedHasMore = resp.count >= 20;
			if (!myPubkey) {
				try {
					const cfg = await api.getConfig();
					myPubkey = cfg.my_pubkey;
				} catch { /* ignore */ }
			}
			const pubkeys = [...new Set(resp.publications.map(p => p.author_pubkey))];
			if (myPubkey) pubkeys.push(myPubkey);
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
			api.prefetchProfiles([...new Set(resp.publications.map(p => p.author_pubkey))]);
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
				const existing = new Set(feed.map(p => `${p.addr.pubkey}:${p.addr.d_tag}`));
				const newPubs = resp.publications.filter(p => !existing.has(`${p.addr.pubkey}:${p.addr.d_tag}`));
				feed = [...feed, ...newPubs];
				feedHasMore = resp.count >= 20;
				api.prefetchProfiles([...new Set(newPubs.map(p => p.author_pubkey))]);
			}
		} catch {
			// silent
		} finally {
			feedLoadingMore = false;
		}
	}

	// ===================== Ignore list =====================

	async function refreshIgnoreList() {
		try {
			const il = await api.getIgnoreList();
			ignoredCount = il.ignored_event_count + il.ignored_pubkey_count;
			ignoredEventIds = il.event_ids;
			ignoredPubkeys = il.pubkeys;
		} catch {}
	}

	function handleViewIgnored() {
		refreshIgnoreList();
		goto('/ignored');
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
				navigateHome();
			}
		} catch (e) {
			console.error('Unignore failed:', e);
		}
	}

	// ===================== Chat handlers =====================

	async function handleSend(content: string) {
		if (chat) {
			const nextId = Math.max(0, ...chat.fragments.map((f) => f.id)) + 1;
			chat = {
				...chat,
				fragments: [...chat.fragments, { id: nextId, role: 'user', content }],
				fragment_count: chat.fragment_count + 1
			};
		}
		if (loadedSessionId) {
			try {
				await api.appendClaudeSessionMessage(loadedSessionId, content);
			} catch (e) {
				console.error('Failed to append to Claude session:', e);
			}
		}
		if (passthrough) {
			if (loadedSessionId) loadedSessionMessageCount += 1;
			return;
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
		if (loadedSessionId) {
			stopSessionPoll();
			loadedSessionId = null;
			loadedSessionMessageCount = 0;
		}
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

	// ===================== Item handlers =====================

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

	function handleDeleteFromContext(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_context: false } : e));
		gc();
		syncContext();
	}

	function handleDeleteFromCompose(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.map((e) => (ids.has(e.id) ? { ...e, in_compose: false } : e));
		gc();
	}

	function handleDeletePermanent(deleteItems: ContextItem[]) {
		const ids = new Set(deleteItems.map((i) => i.id));
		items = items.filter((e) => !ids.has(e.id));
		syncContext();
	}

	function handleContextToCompose(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
		items = items.map((e) =>
			ids.has(e.id)
				? { ...e, in_compose: true, in_context: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') navigateToCompose();
	}

	function handleComposeToChat(checkedItems: ContextItem[]) {
		const ids = new Set(checkedItems.map((i) => i.id));
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

	function handleSendItemToCompose(id: string) {
		items = items.map((e) =>
			e.id === id
				? { ...e, in_context: true, in_compose: true, content: e.context_content, modified: e.context_content !== e.original_content }
				: e
		);
		syncContext();
		if (docMode !== 'compose') navigateToCompose();
	}

	function handleToggleReadonly(id: string) {
		items = items.map((e) =>
			e.id === id ? { ...e, readonly: !e.readonly } : e
		);
	}

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

	function handleChatFragmentsToCompose(fragments: Fragment[]) {
		const newItems = fragments.map((f) =>
			makeItem(
				{ title: `[${f.role}]`, content: f.content, tags: [], original_content: f.content, origin: 'chat', source_fragment_id: f.id },
				{ compose: true }
			)
		);
		items = [...items, ...newItems];
		if (docMode !== 'compose') navigateToCompose();
	}

	async function handleChatPublishFragments(fragments: Fragment[]) {
		if (!fragments.length) return;
		try {
			const canSign = identityStatus?.state === 'unlocked';
			await api.publish({
				title: `Chat export ${new Date().toISOString().slice(0, 10)}`,
				tags: [],
				sections: fragments.map(f => ({
					title: `[${f.role}]`,
					content: f.content,
					tags: []
				})),
				sign: canSign,
				broadcast: canSign
			});
			await loadFeed();
		} catch (e) {
			console.error('Publish fragments failed:', e);
		}
	}

	async function handleComposePublish(items: ContextItem[]) {
		const sections = items.length > 0 ? items : compose.sections;
		if (!sections.length) return;
		const canSign = identityStatus?.state === 'unlocked';

		// If any section has a source_addr OR the draft was seeded from a
		// publication, route through the block endpoint so we emit fork-
		// marker tags. Otherwise fall back to the legacy publish.
		const hasProvenance =
			!!composeSourcePubAddr || sections.some((s) => !!s.source_addr);

		try {
			if (hasProvenance) {
				const blocks: api.PublishBlock[] = sections.map((s) => {
					const baseTags = s.tags.map(
						(t) => [t.name, t.value] as [string, string]
					);
					if (!s.source_addr) {
						return {
							kind: 'editable',
							title: s.title,
							tags: baseTags,
							content: s.content
						};
					}
					const diverged = s.content !== s.original_content;
					if (diverged) {
						return {
							kind: 'forked',
							title: s.title,
							tags: baseTags,
							original_addr: s.source_addr,
							content: s.content,
							original_author: s.source_addr.pubkey
						};
					}
					return {
						kind: 'imported',
						title: s.title,
						tags: baseTags,
						source_addr: s.source_addr,
						content: s.content,
						author: s.source_addr.pubkey
					};
				});
				const resp = await api.publishBlocks({
					title: compose.title,
					tags: compose.tags.map((t) => [t.name, t.value] as [string, string]),
					blocks,
					source_publication_addr: composeSourcePubAddr,
					source_publication_event_id: composeSourcePubEventId,
					sign: canSign,
					broadcast: canSign
				});
				console.log('Published (blocks):', resp.publication_id);
			} else {
				const resp = await api.publish({
					title: compose.title,
					tags: compose.tags.map((t) => [t.name, t.value] as [string, string]),
					sections: sections.map((s) => ({
						title: s.title,
						content: s.content,
						tags: s.tags.map((t) => [t.name, t.value] as [string, string])
					})),
					sign: canSign,
					broadcast: canSign
				});
				console.log('Published:', resp.publication_id);
			}
			await loadFeed();
		} catch (e) {
			console.error('Publish compose failed:', e);
		}
	}

	function handleComposeUpdate(state: ComposeState) {
		composeTitle = state.title;
		composeTags = state.tags;

		const updatedById = new Map(state.sections.map((s) => [s.id, s]));

		items = items
			.map((item) => {
				if (!item.in_compose) return item;
				const updated = updatedById.get(item.id);
				if (updated) {
					updatedById.delete(item.id);
					return { ...updated, in_context: item.in_context, in_compose: true, context_content: item.context_content };
				}
				if (item.in_context) return { ...item, in_compose: false };
				return null;
			})
			.filter((item): item is ContextItem => item !== null);

		const existingIds = new Set(items.map((i) => i.id));
		for (const [id, section] of updatedById) {
			if (!existingIds.has(id)) {
				items = [...items, { ...section, in_context: false, in_compose: true }];
			}
		}

		syncContext();
	}

	// ===================== Search =====================

	async function handleSearch(query: string, opts: { scopeToMe?: boolean } = {}) {
		const scopeToMe = opts.scopeToMe ?? true;
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
			if (scopeToMe && myPubkey && !query.includes('by:') && !query.includes('~:')) {
				effectiveQuery = `by:me ${effectiveQuery}`;
			}

			const resp = await api.search(effectiveQuery, undefined, myPubkey ?? undefined);
			searchResults = resp.results;
			searchCount = resp.count;
			searchLocalCount = resp.local_count;
			searchRelayCount = resp.relay_count;

			const searchPubkeys = [...new Set(resp.results.map(r => r.author))];
			api.prefetchProfiles(searchPubkeys);

			if (resp.doc_results && resp.doc_results.length > 0) {
				importPages = resp.doc_results.map(d => ({
					page_num: d.page_num,
					title: d.title ?? `${d.filename} p.${d.page_num}`,
					content: d.content
				}));
				importFilename = resp.doc_results[0].filename;
			}

			if (docMode === 'empty') {
				const pubs = resp.results.filter(r => r.kind === 30040 && r.addr);
				const seen = new Set<string>();
				const feedPubs = [];
				for (const r of pubs) {
					const key = `${r.addr!.pubkey}:${r.addr!.d_tag}`;
					if (seen.has(key)) continue;
					seen.add(key);
					feedPubs.push({
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
				if (feedPubs.length > 0) {
					feed = feedPubs;
					feedHasMore = false;
				}
			}
		} catch (e) {
			console.error('Search failed:', e);
		} finally {
			searchLoading = false;
		}
	}

	async function handleAddToContext(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { context: true });
		syncContext();
	}

	async function handleAddToCompose(result: SearchResult) {
		const content = await fetchEventContent(result);
		addToPool(resultFields(result, content), { compose: true });
		if (docMode !== 'compose') navigateToCompose();
	}

	// --- Active plain-mode CM6 view ---
	// ComposerBuffer publishes its plain CodeMirror view here so cross-buffer
	// actions (e.g. SearchBuffer's "insert at cursor") can dispatch into it
	// without prop-drilling. `unknown` to avoid pulling @codemirror/view into
	// every state import; callers cast at the use site.
	let composerActiveView: unknown = null;
	function setComposerActiveView(v: unknown) {
		composerActiveView = v;
	}

	// Insert a search result into the composer per the configured mode.
	// 'cursor' inserts at the active plain-mode caret; 'append' appends to
	// either the plain-mode buffer or the compose section pool depending on
	// whether the plain editor is active.
	async function handleInsertEvent(result: SearchResult, mode: EditorInsertMode) {
		const content = await fetchEventContent(result);
		const view = composerActiveView as
			| { state: { doc: { length: number; toString: () => string }; selection: { main: { from: number } } }; dispatch: (spec: unknown) => void; focus: () => void }
			| null;
		if (view) {
			const title = result.title?.trim() || '[Untitled]';
			// `==` is the section-heading prefix in compose's plain-mode parser
			// (single `=` is reserved for the publication title).
			const text = `\n== ${title}\n\n${content}\n`;
			const pos = mode === 'cursor' ? view.state.selection.main.from : view.state.doc.length;
			view.dispatch({
				changes: { from: pos, insert: text },
				selection: { anchor: pos + text.length }
			});
			view.focus();
			if (docMode !== 'compose') navigateToCompose();
			return;
		}
		// Plain editor not active — fall back to pool append. Mark origin
		// 'import' so the new section defaults to locked: the user is
		// transcluding an existing event, not authoring fresh text.
		const fields = { ...resultFields(result, content), origin: 'import' as const };
		addToPool(fields, { compose: true });
		if (docMode !== 'compose') navigateToCompose();
	}

	// Import a (already-loaded) section into the compose pool. Used by
	// ReaderBuffer's "edit this" affordance to send the active publication
	// into the composer without re-fetching from the engine.
	function importSectionToCompose(
		addr: NAddr,
		title: string | null,
		content: string,
		tags: { name: string; value: string }[] = []
	) {
		addToPool(
			{
				title: title ?? '[Untitled section]',
				content,
				tags,
				source_addr: addr,
				original_content: content,
				origin: 'import' as const
			},
			{ compose: true }
		);
	}

	// Drop everything currently in the compose pool. Called before an
	// "edit this" action to avoid mixing the new edit target with stale
	// imports from a previous session. Also clears publication-source
	// provenance so a follow-up seed can reset it cleanly.
	function clearComposePool() {
		items = items.map((e) => (e.in_compose ? { ...e, in_compose: false } : e));
		composeSourcePubAddr = null;
		composeSourcePubEventId = null;
		composeSourceSectionOrder = [];
	}

	// Move an in-compose section up or down by one position in the
	// section list. Reorder operates on the underlying `items` array so
	// the derived `composeSections` reflects the new order. No-op if the
	// section is already at the boundary.
	function reorderComposeSection(id: string, direction: 'up' | 'down') {
		const composeIds = items.filter((i) => i.in_compose).map((i) => i.id);
		const localIdx = composeIds.indexOf(id);
		if (localIdx < 0) return;
		const swapWith = direction === 'up' ? localIdx - 1 : localIdx + 1;
		if (swapWith < 0 || swapWith >= composeIds.length) return;
		const aId = composeIds[localIdx];
		const bId = composeIds[swapWith];
		const aIdx = items.findIndex((i) => i.id === aId);
		const bIdx = items.findIndex((i) => i.id === bId);
		if (aIdx < 0 || bIdx < 0) return;
		const next = items.slice();
		[next[aIdx], next[bIdx]] = [next[bIdx], next[aIdx]];
		items = next;
	}

	// Switch the user back to the read view of the draft.
	// - If the draft was seeded from a published 30040, navigate to its
	//   ReaderBuffer; that buffer's "draft mode" check picks up the
	//   matching compose state and renders editable lock/reorder UI.
	// - Otherwise (from-scratch draft), this is a no-op for now.
	function previewDraft() {
		const src = composeSourcePubAddr;
		if (!src) return;
		navigateToPublication(src.pubkey, src.d_tag);
	}

	// Set the publication-level draft fields (title + topic tags) and
	// optional source provenance. Used by ReaderBuffer's "Edit" so both the
	// 30040 metadata and the fork lineage survive the round trip from
	// reader → composer.
	function seedDraftMetadata(
		title: string | null,
		tags: TagEntry[],
		source?: {
			pub_addr?: NAddr | null;
			pub_event_id?: string | null;
			section_order?: NAddr[];
		}
	) {
		composeTitle = title ?? '';
		composeTags = tags;
		composeSourcePubAddr = source?.pub_addr ?? null;
		composeSourcePubEventId = source?.pub_event_id ?? null;
		composeSourceSectionOrder = source?.section_order ?? [];
	}

	async function handleAddManyToContext(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { context: true });
		}
		syncContext();
	}

	async function handleAddManyToCompose(results: SearchResult[]) {
		for (const r of results) {
			const content = await fetchEventContent(r);
			addToPool(resultFields(r, content), { compose: true });
		}
		if (docMode !== 'compose') navigateToCompose();
	}

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
			if (result.addr) {
				const aTag = `${result.addr.kind}:${result.addr.pubkey}:${result.addr.d_tag}`;
				feed = feed.filter(p => `${p.addr.kind}:${p.addr.pubkey}:${p.addr.d_tag}` !== aTag);
			}
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
			feed = feed.filter(p => p.author_pubkey !== result.author);
		} catch (e) {
			console.error('Ignore pubkey failed:', e);
		}
	}

	// ===================== Document handlers =====================

	async function openPublication(pubkey: string, d_tag: string) {
		docMode = 'reading';
		docLoading = true;
		publication = null;
		sections = [];
		loadingPromises.clear();
		try {
			const pubResp = await api.getPublication(pubkey, d_tag, 'local_first');
			publication = pubResp.publication;
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
			navigateHome();
		} finally {
			docLoading = false;
		}
	}

	async function openStandaloneSection(result: SearchResult) {
		docMode = 'reading';
		docLoading = true;
		publication = null;
		sections = [];
		loadingPromises.clear();
		try {
			const content = await fetchEventContent(result);
			publication = {
				addr: result.addr!,
				title: result.title,
				summary: null,
				image: null,
				author_pubkey: result.author,
				version: null,
				created_at: result.created_at,
				index: null
			};
			sections = [{
				addr: result.addr!,
				title: result.title,
				content,
				position: 0,
				status: 'loaded' as const
			}];
			viewMode = 'paginated';
			currentSection = 0;
			previewVisible = false;
		} catch (e) {
			console.error('Failed to open standalone section:', e);
			navigateHome();
		} finally {
			docLoading = false;
		}
	}

	async function handleSelectResult(result: SearchResult) {
		if (!result.addr) return;
		if (result.kind === 30040) {
			navigateToPublication(result.addr.pubkey, result.addr.d_tag);
		} else if (result.kind === 30041) {
			try {
				const resp = await api.getEvent(result.event_id);
				const event = resp.event as Record<string, unknown> | null;
				const tags = (event?.tags as string[][] | undefined) ?? [];
				const aTag = tags.find((t) => t[0] === 'a' && t[1]?.startsWith('30040:'));
				if (aTag) {
					const [, ref] = aTag;
					const parts = ref.split(':');
					if (parts.length >= 3) {
						// Navigate to publication, then we need to find the section index
						// For now, navigate to publication and let the route handle it
						navigateToPublication(parts[1], parts.slice(2).join(':'));
						return;
					}
				}
			} catch {
				// Fall through to standalone view
			}
			await openStandaloneSection(result);
		} else {
			await openStandaloneSection(result);
		}
	}

	function handleLoadSection(index: number) {
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

	function handleOpenFeedPublication(pub_summary: PublicationSummary) {
		navigateToPublication(pub_summary.addr.pubkey, pub_summary.addr.d_tag);
	}

	function handleViewProfile(pubkey: string) {
		if (!pubkey) {
			navigateHome();
			return;
		}
		navigateToProfile(pubkey);
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
		items = [
			...items.map((e) => ({ ...e, in_compose: false })).filter((e) => e.in_context),
			makeItem({ title: '', content: '', tags: [], original_content: '', origin: 'compose' }, { compose: true })
		];
		composeTitle = '';
		composeTags = [];
		previewVisible = false;
		navigateToCompose();
	}

	function handleCancelCompose() {
		if (publication) {
			navigateToPublication(publication.addr.pubkey, publication.addr.d_tag);
		} else {
			navigateHome();
		}
	}

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

	async function handleDocPublish() {
		if (!publication || !sections.length) return;
		const canSign = identityStatus?.state === 'unlocked';
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
				sign: canSign,
				broadcast: canSign
			});
			console.log('Published from reader:', resp.publication_id);
			await loadFeed();
		} catch (e) {
			console.error('Publish doc failed:', e);
		}
	}

	// ===================== Embedding =====================

	async function handleSyncEmbeddings() {
		embeddingSyncing = true;
		const pollInterval = setInterval(async () => {
			try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
		}, 1000);
		try {
			embeddingStatus = await api.syncEmbeddings();
		} catch (e) {
			console.error('Embedding sync failed:', e);
		} finally {
			clearInterval(pollInterval);
			try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
			embeddingSyncing = false;
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

	// ===================== Network =====================

	async function handleSetNetworkMode(mode: NetworkMode) {
		try {
			await api.setNetworkMode(mode);
			networkStatus = await api.getNetworkStatus();
		} catch (e) {
			console.error('Failed to set network mode:', e);
		}
	}

	// ===================== Purge / Export / Import =====================

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

	async function handleExport() {
		exporting = true;
		try {
			const result = await api.downloadExport();
			alert(`Exported ${result.count} events to ${result.filename}`);
		} catch (e) {
			console.error('Export failed:', e);
			alert('Export failed: ' + (e as Error).message);
		} finally {
			exporting = false;
		}
	}

	async function handleImport(file: File) {
		importing = true;
		importProgress = null;
		try {
			const result = await api.importJsonl(file, (p) => { importProgress = { ...p }; });
			if (result.ingested > 0) {
				await loadFeed();
				handleSyncEmbeddings();
			}
		} catch (e) {
			console.error('Import failed:', e);
			alert('Import failed: ' + (e as Error).message);
		} finally {
			importing = false;
		}
	}

	// ===================== Document import =====================

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

	function handleImportPageToContext(page: ImportPage) {
		addToPool({
			title: page.title ?? `Page ${page.page_num}`,
			content: page.content,
			tags: [{ name: 'source', value: importFilename }, { name: 'page', value: String(page.page_num) }],
			original_content: page.content,
			origin: 'import' as const
		}, { context: true });
		syncContext();
	}

	function handleImportPageToCompose(page: ImportPage) {
		addToPool({
			title: page.title ?? `Page ${page.page_num}`,
			content: page.content,
			tags: [{ name: 'source', value: importFilename }, { name: 'page', value: String(page.page_num) }],
			original_content: page.content,
			origin: 'import' as const
		}, { compose: true });
		if (docMode !== 'compose') navigateToCompose();
	}

	function handleImportPagesToContext(pages: ImportPage[]) {
		for (const page of pages) handleImportPageToContext(page);
	}

	function handleImportPagesToCompose(pages: ImportPage[]) {
		for (const page of pages) handleImportPageToCompose(page);
	}

	// ===================== Fetch =====================

	async function handleFetchAuthors() {
		try {
			const resp = await api.fetchAuthors();
			console.log(`Fetched ${resp.fetched} events for ${resp.authors} authors from ${resp.relays} relays`);
			await loadFeed();
		} catch (e) {
			console.error('Fetch authors failed:', e);
		}
	}

	async function handleFetchSections() {
		try {
			const resp = await api.fetchSections();
			console.log(`Fetch sections: ${resp.total_referenced} referenced, ${resp.missing} missing, ${resp.fetched} fetched`);
			await loadFeed();
		} catch (e) {
			console.error('Fetch sections failed:', e);
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

	// ===================== Claude sessions =====================

	function messagesToFragments(
		messages: ClaudeSessionMessage[],
		startId: number
	): Fragment[] {
		const fragments: Fragment[] = [];
		let id = startId;
		let pendingToolFragments: Fragment[] = [];

		for (const msg of messages) {
			const hasText = msg.blocks.some(b => b.type === 'text');
			const hasToolUse = msg.blocks.some(b => b.type === 'tool_use');
			const hasToolResult = msg.blocks.some(b => b.type === 'tool_result');

			if (hasText) {
				pendingToolFragments = [];
				const text = msg.blocks.filter(b => b.type === 'text').map(b => b.text ?? '').join('\n');
				fragments.push({ id: id++, role: msg.role, content: text, blocks: msg.blocks });
			} else if (hasToolUse) {
				const frag: Fragment = { id: -(id++), role: 'tool', content: '', blocks: [...msg.blocks] };
				pendingToolFragments.push(frag);
				fragments.push(frag);
			} else if (hasToolResult && pendingToolFragments.length > 0) {
				const target = pendingToolFragments.shift()!;
				const resultBlocks = msg.blocks.filter(b => b.type === 'tool_result');
				target.blocks = [...(target.blocks ?? []), ...resultBlocks];
			}
		}
		return fragments;
	}

	async function handleToggleSessions() {
		sessionsExpanded = !sessionsExpanded;
		if (!sessionsExpanded && !loadedSessionId) stopSessionPoll();
		if (sessionsExpanded && claudeSessions.length === 0) {
			claudeSessionsLoading = true;
			try {
				const resp = await api.listClaudeSessions();
				claudeSessions = resp.sessions;
			} catch (e) {
				console.error('Failed to load Claude sessions:', e);
			} finally {
				claudeSessionsLoading = false;
			}
		}
	}

	async function handleSelectClaudeSession(id: string) {
		stopSessionPoll();
		claudeSessionsLoading = true;
		try {
			claudeSessionDetail = await api.getClaudeSession(id);
			watchingSessionId = id;
			startSessionPoll(id);
		} catch (e) {
			console.error('Failed to load session:', e);
		} finally {
			claudeSessionsLoading = false;
		}
	}

	function startSessionPoll(id: string) {
		sessionPollInterval = setInterval(async () => {
			if (watchingSessionId !== id) return;
			try {
				const offset = loadedSessionId === id
					? loadedSessionMessageCount
					: (claudeSessionDetail?.messages.length ?? 0);

				const resp = await api.getClaudeSession(id, offset);
				if (resp.messages.length === 0) return;

				if (claudeSessionDetail && watchingSessionId === id) {
					claudeSessionDetail = {
						...claudeSessionDetail,
						messages: [...claudeSessionDetail.messages, ...resp.messages],
						count: claudeSessionDetail.count + resp.messages.length,
					};
				}

				if (loadedSessionId === id && chat) {
					const newFragments = messagesToFragments(resp.messages, chat.fragments.length);
					chat = {
						...chat,
						fragments: [...chat.fragments, ...newFragments],
						fragment_count: chat.fragment_count + newFragments.length,
					};
					loadedSessionMessageCount += resp.messages.length;
				}
			} catch { /* ignore poll errors */ }
		}, 2000);
	}

	function stopSessionPoll() {
		if (sessionPollInterval) {
			clearInterval(sessionPollInterval);
			sessionPollInterval = null;
		}
		watchingSessionId = null;
	}

	function handleClaudeSessionBack() {
		if (!loadedSessionId || loadedSessionId !== watchingSessionId) {
			stopSessionPoll();
		}
		claudeSessionDetail = null;
	}

	async function handleLoadSessionToChat(session: { id: string; messages: ClaudeSessionMessage[] }) {
		chatLoading = true;
		try {
			const textFragments = session.messages
				.filter(m => m.blocks.some(b => b.type === 'text'))
				.map(m => ({
					role: m.role,
					content: m.blocks.filter(b => b.type === 'text').map(b => b.text ?? '').join('\n')
				}));
			chat = await api.loadChatFragments(textFragments);

			if (chat) {
				const enriched = messagesToFragments(session.messages, 0);
				let backendIdx = 0;
				for (const frag of enriched) {
					if (frag.role !== 'tool' && backendIdx < chat.fragments.length) {
						frag.id = chat.fragments[backendIdx].id;
						backendIdx++;
					}
				}
				chat = { ...chat, fragments: enriched, fragment_count: enriched.length };
			}

			loadedSessionId = session.id;
			loadedSessionMessageCount = session.messages.length;

			if (watchingSessionId !== session.id) {
				stopSessionPoll();
				watchingSessionId = session.id;
				startSessionPoll(session.id);
			}

			sessionsExpanded = false;
		} catch (e) {
			console.error('Failed to load session to chat:', e);
		} finally {
			chatLoading = false;
		}
	}

	// ===================== Navigation =====================

	// When set (by the WM shell), navigation calls invoke these instead of
	// goto-ing route URLs. Lets the shell stay on its single URL while
	// spawning/focusing buffers in response to the same handlers that drive
	// the legacy multi-route chrome.
	type NavigationHandlers = {
		onPublication?: (pubkey: string, d_tag: string) => void;
		onProfile?: (pubkey: string) => void;
		onCompose?: () => void;
		onHome?: () => void;
	};
	let navHandlers: NavigationHandlers | null = null;

	function setNavigationHandlers(h: NavigationHandlers | null) {
		navHandlers = h;
	}

	function navigateToPublication(pubkey: string, d_tag: string) {
		docMode = 'reading';
		if (navHandlers?.onPublication) {
			navHandlers.onPublication(pubkey, d_tag);
		} else {
			goto(`/p/${pubkey}/${d_tag}`);
		}
	}

	function navigateToProfile(pubkey: string) {
		docMode = 'profile';
		if (navHandlers?.onProfile) {
			navHandlers.onProfile(pubkey);
		} else {
			goto(`/profile/${pubkey}`);
		}
	}

	function navigateToCompose() {
		docMode = 'compose';
		if (navHandlers?.onCompose) {
			navHandlers.onCompose();
		} else {
			goto('/compose');
		}
	}

	function navigateHome() {
		docMode = 'empty';
		profilePubkey = null;
		publication = null;
		sections = [];
		if (navHandlers?.onHome) {
			navHandlers.onHome();
		} else {
			goto('/');
		}
	}

	// ===================== Initialization =====================

	async function initialize() {
		try {
			const cfg = await api.getConfig();
			myPubkey = cfg.my_pubkey;
			assistantPubkey = cfg.assistant_pubkey;
			console.log('Config loaded, myPubkey:', myPubkey, 'assistantPubkey:', assistantPubkey);
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
		try {
			networkStatus = await api.getNetworkStatus();
		} catch {}
		await refreshIgnoreList();
		try {
			const rc = await api.getRelayConfig();
			fetchRelayUrls = rc.fetch.urls;
			authorCount = rc.authors.length;
		} catch {}
		// Load identity session status
		try {
			identityStatus = await api.getIdentity();
			if (identityStatus.pubkey) {
				myPubkey = identityStatus.pubkey;
				resolveIdentityName(identityStatus.pubkey);
			}
		} catch {}
	}

	function startNetworkPoll() {
		const networkPoll = setInterval(async () => {
			if (document.hidden) return;
			try { networkStatus = await api.getNetworkStatus(); } catch {}
		}, 2000);
		// Identity poll — detect server-side lock timeout
		identityPollInterval = setInterval(async () => {
			if (document.hidden) return;
			if (!identityStatus || identityStatus.state === 'none') return;
			try { identityStatus = await api.getIdentity(); } catch {}
		}, 30_000);
		return () => {
			clearInterval(networkPoll);
			if (identityPollInterval) clearInterval(identityPollInterval);
		};
	}

	// ===================== Identity actions =====================

	async function resolveIdentityName(pubkey: string) {
		try {
			const profile = await api.getProfile(pubkey);
			if (profile.found) {
				identityDisplayName = profile.display_name || profile.name;
			}
		} catch { /* profile fetch optional */ }
	}

	async function handleIdentityLogin(ncryptsec: string) {
		identityError = null;
		identityLoading = true;
		try {
			identityStatus = await api.loginIdentity(ncryptsec);
			if (identityStatus.pubkey) {
				myPubkey = identityStatus.pubkey;
				resolveIdentityName(identityStatus.pubkey);
			}
		} catch (e: unknown) {
			identityError = e instanceof Error ? e.message : String(e);
		} finally {
			identityLoading = false;
		}
	}

	async function handleIdentityUnlock(password: string) {
		identityError = null;
		identityLoading = true;
		try {
			identityStatus = await api.unlockIdentity(password);
			if (identityStatus.pubkey) {
				myPubkey = identityStatus.pubkey;
				if (!identityDisplayName) resolveIdentityName(identityStatus.pubkey);
			}
		} catch (e: unknown) {
			identityError = e instanceof Error ? e.message : String(e);
		} finally {
			identityLoading = false;
		}
	}

	async function handleIdentityLock() {
		try {
			identityStatus = await api.lockIdentity();
		} catch (e) {
			console.error('Lock failed:', e);
		}
	}

	async function handleIdentityLogout() {
		try {
			identityStatus = await api.logoutIdentity();
			myPubkey = null;
			identityDisplayName = null;
		} catch (e) {
			console.error('Logout failed:', e);
		}
	}

	// --- External signer (NIP-07) state ---
	// Registration is user-initiated from the settings buffer. The
	// teardown closure closes the EventSource and reverts the engine
	// source back to `engine`. The pubkey is cached so the settings
	// buffer can show it without re-querying window.nostr.
	let externalSignerPubkey: string | null = $state(null);
	let externalSignerTeardown: (() => void) | null = null;

	async function handleSelectNip07Source() {
		identityError = null;
		identityLoading = true;
		try {
			const { detectNip07, registerNip07Signer } = await import('$lib/identity/signer');
			if (!detectNip07()) {
				throw new Error('No window.nostr signer detected');
			}
			// Cache pubkey before registering so the UI can surface it
			// even if the engine status hasn't refreshed yet.
			externalSignerPubkey = await window.nostr!.getPublicKey();
			externalSignerTeardown = await registerNip07Signer();
			identityStatus = await api.getIdentity();
			myPubkey = externalSignerPubkey;
			resolveIdentityName(externalSignerPubkey);
		} catch (e: unknown) {
			identityError = e instanceof Error ? e.message : String(e);
			externalSignerPubkey = null;
		} finally {
			identityLoading = false;
		}
	}

	async function handleSelectEngineSource() {
		try {
			if (externalSignerTeardown) {
				externalSignerTeardown();
				externalSignerTeardown = null;
			}
			externalSignerPubkey = null;
			identityStatus = await api.useIdentitySource({ source: 'engine' });
		} catch (e) {
			console.error('switch to engine source failed:', e);
		}
	}

	// ===================== Return public API =====================

	return {
		// Chat
		get chat() { return chat; },
		get chatLoading() { return chatLoading; },
		get systemExpanded() { return systemExpanded; },
		set systemExpanded(v: boolean) { systemExpanded = v; },
		get contextExpanded() { return contextExpanded; },
		set contextExpanded(v: boolean) { contextExpanded = v; },
		get chatHiddenFragmentIds() { return chatHiddenFragmentIds; },
		get chatFragmentItems() { return chatFragmentItems; },

		// Items
		get items() { return items; },
		get contextEntries() { return contextEntries; },
		get composeSections() { return composeSections; },
		get compose() { return compose; },
		get composeTitle() { return composeTitle; },
		set composeTitle(v: string) { composeTitle = v; },
		get composeTags() { return composeTags; },
		set composeTags(v: TagEntry[]) { composeTags = v; },

		// Document
		get docMode() { return docMode; },
		set docMode(v: DocMode) { docMode = v; },
		get publication() { return publication; },
		set publication(v: PublicationDetail | null) { publication = v; },
		get sections() { return sections; },
		set sections(v: LazySection[]) { sections = v; },
		get viewMode() { return viewMode; },
		set viewMode(v: ViewMode) { viewMode = v; },
		get currentSection() { return currentSection; },
		set currentSection(v: number) { currentSection = v; },
		get previewVisible() { return previewVisible; },
		set previewVisible(v: boolean) { previewVisible = v; },
		get docLoading() { return docLoading; },
		set docLoading(v: boolean) { docLoading = v; },
		get loadingPromises() { return loadingPromises; },

		// Feed
		get feed() { return feed; },
		set feed(v: PublicationSummary[]) { feed = v; },
		get feedLoading() { return feedLoading; },
		get feedSyncing() { return feedSyncing; },
		get feedLoadingMore() { return feedLoadingMore; },
		get feedHasMore() { return feedHasMore; },

		// Search
		get searchResults() { return searchResults; },
		get searchCount() { return searchCount; },
		get searchLocalCount() { return searchLocalCount; },
		get searchRelayCount() { return searchRelayCount; },
		get searchLoading() { return searchLoading; },

		// JSON modal
		get jsonModalData() { return jsonModalData; },
		set jsonModalData(v: unknown) { jsonModalData = v; },
		get actionModalResult() { return actionModalResult; },
		set actionModalResult(v: SearchResult | null) { actionModalResult = v; },

		// Profile
		get profilePubkey() { return profilePubkey; },
		set profilePubkey(v: string | null) { profilePubkey = v; },

		// Identity
		get myPubkey() { return myPubkey; },
		get assistantPubkey() { return assistantPubkey; },
		get localPubkeys() { return localPubkeys; },
		get identityStatus() { return identityStatus; },
		get identityLoading() { return identityLoading; },
		get identityError() { return identityError; },
		get identityDisplayName() { return identityDisplayName; },
		set identityError(v: string | null) { identityError = v; },
		handleIdentityLogin,
		handleIdentityUnlock,
		handleIdentityLock,
		handleIdentityLogout,
		get externalSignerPubkey() { return externalSignerPubkey; },
		handleSelectNip07Source,
		handleSelectEngineSource,

		// Embedding
		get embeddingStatus() { return embeddingStatus; },
		get embeddingSyncing() { return embeddingSyncing; },

		// Network
		get networkStatus() { return networkStatus; },

		// Relay config
		get fetchRelayUrls() { return fetchRelayUrls; },
		get authorCount() { return authorCount; },

		// Claude sessions
		get claudeSessions() { return claudeSessions; },
		get claudeSessionDetail() { return claudeSessionDetail; },
		get claudeSessionsLoading() { return claudeSessionsLoading; },
		get sessionsExpanded() { return sessionsExpanded; },

		// Document import
		get documentFiles() { return documentFiles; },
		get importPages() { return importPages; },
		get importFilename() { return importFilename; },
		get importLoading() { return importLoading; },

		// Ignore list
		get ignoredCount() { return ignoredCount; },
		get ignoredEventIds() { return ignoredEventIds; },
		get ignoredPubkeys() { return ignoredPubkeys; },

		// Settings
		get syncMode() { return syncMode; },
		set syncMode(v: SyncMode) { syncMode = v; },
		get passthrough() { return passthrough; },
		set passthrough(v: boolean) { passthrough = v; },
		get buttonLabels() { return buttonLabels; },
		set buttonLabels(v: ButtonLabels) { buttonLabels = v; },
		get editorInsertMode() { return editorInsertMode; },
		set editorInsertMode(v: EditorInsertMode) { editorInsertMode = v; },
		get editorLineNumbers() { return editorLineNumbers; },
		set editorLineNumbers(v: boolean) { editorLineNumbers = v; },
		get editorVimMode() { return editorVimMode; },
		set editorVimMode(v: boolean) { editorVimMode = v; },
		get composeDefaultMode() { return composeDefaultMode; },
		set composeDefaultMode(v: ComposeDefaultMode) { composeDefaultMode = v; },

		// Panel collapse
		get chatCollapsed() { return chatCollapsed; },
		set chatCollapsed(v: boolean) { chatCollapsed = v; },
		get docCollapsed() { return docCollapsed; },
		set docCollapsed(v: boolean) { docCollapsed = v; },
		get searchCollapsed() { return searchCollapsed; },
		set searchCollapsed(v: boolean) { searchCollapsed = v; },
		get gridTemplate() { return gridTemplate; },

		// Export/Import
		get exporting() { return exporting; },
		get importing() { return importing; },
		get importProgress() { return importProgress; },

		// Handler functions
		handleSend,
		handleReset,
		handleEdit,
		handleApplyEdit,
		handleCancelEdit,
		handleSetSystem,
		handleUpdateContextItem,
		handleResetContextItem,
		handleRemoveFromContext,
		handleDeleteFromContext,
		handleDeleteFromCompose,
		handleDeletePermanent,
		handleContextToCompose,
		handleComposeToChat,
		handleSendItemToChat,
		handleSendItemToCompose,
		handleToggleReadonly,
		handleLockToSource,
		handleCrossPanelCopy,
		handleChatFragmentsToCompose,
		handleChatPublishFragments,
		handleComposePublish,
		handleComposeUpdate,
		handleSearch,
		handleAddToContext,
		handleAddToCompose,
		handleAddManyToContext,
		handleAddManyToCompose,
		handleInsertEvent,
		setComposerActiveView,
		handleViewJson,
		handleIgnoreEvent,
		handleIgnorePubkey,
		handleSelectResult,
		handleOpenFeedPublication,
		handleViewProfile,
		handleViewMode,
		handleTogglePreview,
		handleNavigate,
		handleCompose,
		handleCancelCompose,
		handleDocToChat,
		handleDocPublish,
		handleLoadSection,
		handleSyncEmbeddings,
		handleReindexEmbeddings,
		handleSetNetworkMode,
		handlePurge,
		handleExport,
		handleImport,
		handleViewIgnored,
		handleUnignore,
		handleListDocuments,
		handleImportFile,
		handleParseDocument,
		handleImportPageToContext,
		handleImportPageToCompose,
		handleImportPagesToContext,
		handleImportPagesToCompose,
		handleFetchAuthors,
		handleFetchSections,
		handleFetchFromRelay,
		handleToggleSessions,
		handleSelectClaudeSession,
		handleClaudeSessionBack,
		handleLoadSessionToChat,
		handleFeedSync,
		handleFeedLoadMore,
		loadFeed,
		openPublication,

		// Navigation
		navigateToPublication,
		navigateToProfile,
		navigateToCompose,
		navigateHome,
		setNavigationHandlers,
		importSectionToCompose,
		clearComposePool,
		seedDraftMetadata,
		reorderComposeSection,
		previewDraft,

		// Lifecycle
		initialize,
		startNetworkPoll,

		// Feed ignore inline handlers
		async ignoreEvent(id: string) {
			try {
				await api.ignoreEvents([id]);
				await refreshIgnoreList();
				feed = feed.filter(p => `${p.addr.kind}:${p.addr.pubkey}:${p.addr.d_tag}` !== id);
			} catch {}
		},
		async ignorePubkey(pk: string) {
			try {
				await api.ignoreEvents([], [pk]);
				await refreshIgnoreList();
				feed = feed.filter(p => p.author_pubkey !== pk);
			} catch {}
		}
	};
}

export type AppState = ReturnType<typeof createAppState>;
