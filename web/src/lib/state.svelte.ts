import { goto } from '$app/navigation';
import {
	searchConfig,
	loadSearchConfig,
	applySearchDefaults
} from '$lib/search/search-config.svelte';
import type {
	ChatResponse,
	SearchResult,
	ProfileResult,
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
	NostrEvent,
	EventsModalItem,
	RepublishDiff,
} from '$lib/types';
import type { Buffer } from '$lib/wm/types';
import * as api from '$lib/api';
import { identityCanSign } from '$lib/identity/signer';

/** Replaceable kind-0 events can pile up multiple historical versions in
 *  the DB / relay results. A search should surface only the *current*
 *  profile, so collapse kind-0 hits to the newest per author. Non-kind-0
 *  results pass through untouched, order preserved. */
function dedupeLatestProfiles(results: SearchResult[]): SearchResult[] {
	const latest = new Map<string, number>();
	for (const r of results) {
		if (r.kind !== 0) continue;
		const seen = latest.get(r.author);
		if (seen == null || r.created_at > seen) latest.set(r.author, r.created_at);
	}
	const kept = new Set<string>();
	return results.filter((r) => {
		if (r.kind !== 0) return true;
		if (r.created_at !== latest.get(r.author) || kept.has(r.author)) return false;
		kept.add(r.author);
		return true;
	});
}

/**
 * One node in the app-level search-history stack. Three shapes:
 * - `query`  — string + opts. Replay calls `handleSearch(query, opts)`.
 * - `nevent` — single event id. Replay fetches the event and shows the modal.
 * - `naddr`  — coordinate `(kind, pubkey, d_tag)`. Replay runs the
 *   equivalent `k:K by:<pk> #d:<d>` query.
 *
 * `title` is a display cache populated lazily when the entry is resolved.
 */
export type ModalNavEntry =
	| { kind: 'query'; query: string; opts: { scopeToMe: boolean }; lastRunAt: number }
	| { kind: 'nevent'; eventId: string; title?: string; lastRunAt: number }
	| {
			kind: 'naddr';
			coord: { kind: number; pubkey: string; d_tag: string };
			title?: string;
			lastRunAt: number;
	  };

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
	// Saved drafts (engine DraftStore), newest first. Refreshed when the
	// composer mounts and after any save/delete.
	let composeDrafts: api.DraftSummary[] = $state([]);
	// The current compose session's publication d-tag, once it's been saved or
	// resumed. Threaded onto subsequent saves so they version the same
	// publication (new snapshot, same d_tag) rather than minting a fresh draft.
	let composeDTag: string | null = $state(null);
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
	// Guards the one-time cold-cache auto-fetch in loadFeed() so an empty
	// db doesn't re-pop the fetch-confirm modal on every loadFeed() call
	// (FeedBuffer mount, search-clear, etc.). Plain boolean, not $state —
	// it's an internal latch, never rendered. Resets on page reload.
	let coldFetchAttempted = false;

	// --- Toasts ---
	//
	// Lightweight transient notifications for quick acknowledgments —
	// "copied to clipboard", "publish queued", "broadcast complete", etc.
	// Stack mounts lower-right via <ToastStack /> in the layout.
	//
	// Two interaction modes:
	//   1. Default: auto-dismisses after `ttlMs` with a clock-style
	//      radial countdown visible on the toast.
	//   2. Pinned: user clicks the toast → countdown stops, close (×)
	//      button appears, and the toast stays until manually dismissed.
	//      Pinned toasts with an `activity` field get an "Expand" button
	//      that opens the FetchActivityModal for the structured detail
	//      view (filters / composition / per-relay rows / DSL footer).
	//
	// SSE-driven toasts (fetch + publish operations from the engine)
	// carry an `activity` field. Quick acknowledgment toasts (copy,
	// settings saved, etc.) leave it undefined.
	type RelayRowStatus =
		| { kind: 'connecting' }
		| { kind: 'eose'; event_count: number }
		| { kind: 'error'; msg: string }
		| { kind: 'timeout' }
		| { kind: 'accepted' }
		| { kind: 'rejected'; msg: string };
	type ToastActivity = {
		operation_id: string;
		// Structured request summary from the engine's Intent event.
		summary?: import('./types').RequestSummary;
		// Phase the toast is rendering (typically pulled from the Intent
		// or the first RelayStatus event). Multi-phase ops produce
		// multiple toasts; each phase = one toast.
		phase?: import('./types').Phase;
		// Per-relay status rows, keyed by URL. Updated by relay_status
		// events as they stream in.
		relays: Record<string, RelayRowStatus>;
		// Was this a publish or fetch? Drives the icon / label.
		mode: 'fetch' | 'publish';
	};
	type Toast = {
		id: number;
		message: string;
		// `pending` is the "working in progress" state — violet with a
		// soft pulse. Callers flip to `success` (green) or `error` (red)
		// via updateToast when the operation settles.
		kind: 'success' | 'info' | 'error' | 'pending';
		// Pinned toasts don't auto-dismiss; the countdown UI is hidden.
		pinned: boolean;
		// Original TTL + start time — used to render the countdown.
		// Frozen when `pinned = true`.
		ttlMs: number;
		startedAt: number;
		// Present for SSE-driven activity toasts. Drives Expand button
		// + modal content.
		activity?: ToastActivity;
	};
	let toasts: Toast[] = $state([]);
	let nextToastId = 1;
	const toastTimers = new Map<number, ReturnType<typeof setTimeout>>();
	// Which activity toast (by id) the FetchActivityModal is showing.
	// Null = modal closed.
	let activityModalToastId: number | null = $state(null);

	function pushToast(message: string, kind: Toast['kind'] = 'success', ttlMs = 2000): number {
		const id = nextToastId++;
		toasts = [
			...toasts,
			{ id, message, kind, pinned: false, ttlMs, startedAt: Date.now() }
		];
		toastTimers.set(
			id,
			setTimeout(() => dismissToast(id), ttlMs)
		);
		return id;
	}

	/** Variant of pushToast for engine-driven SSE operations. Attaches a
	 *  ToastActivity payload so the toast can expand into the
	 *  FetchActivityModal with structured detail. */
	function pushActivityToast(
		message: string,
		ttlMs: number,
		activity: ToastActivity
	): number {
		const id = nextToastId++;
		toasts = [
			...toasts,
			{
				id,
				message,
				kind: 'pending',
				pinned: false,
				ttlMs,
				startedAt: Date.now(),
				activity
			}
		];
		toastTimers.set(
			id,
			setTimeout(() => dismissToast(id), ttlMs)
		);
		return id;
	}

	function dismissToast(id: number): void {
		const t = toastTimers.get(id);
		if (t) {
			clearTimeout(t);
			toastTimers.delete(id);
		}
		toasts = toasts.filter((t) => t.id !== id);
		// If the activity modal was viewing this toast, close it too.
		if (activityModalToastId === id) activityModalToastId = null;
	}

	/** Pin a toast — clears its auto-dismiss timer and flips
	 *  `pinned = true`. Idempotent: pinning a pinned toast is a no-op.
	 *  Click handler on the toast row calls this. */
	function pinToast(id: number): void {
		const existing = toastTimers.get(id);
		if (existing) {
			clearTimeout(existing);
			toastTimers.delete(id);
		}
		toasts = toasts.map((t) => (t.id === id ? { ...t, pinned: true } : t));
	}

	/** Open the FetchActivityModal for a specific toast's activity.
	 *  No-op if the toast doesn't have an `activity` field. */
	function expandActivityToast(id: number): void {
		const t = toasts.find((x) => x.id === id);
		if (!t?.activity) return;
		// Auto-pin on expand — otherwise the toast underneath could
		// auto-dismiss while the modal is open, leaving an orphaned modal.
		pinToast(id);
		activityModalToastId = id;
	}

	function closeActivityModal(): void {
		activityModalToastId = null;
	}

	/** Update one relay's row inside an activity toast. Called by
	 *  fetch-events SSE handler when a `relay_status` event arrives. */
	function updateActivityRelay(
		operationId: string,
		relay: string,
		status: RelayRowStatus
	): void {
		toasts = toasts.map((t) => {
			if (!t.activity || t.activity.operation_id !== operationId) return t;
			return {
				...t,
				activity: {
					...t.activity,
					relays: { ...t.activity.relays, [relay]: status }
				}
			};
		});
	}

	/**
	 * Patch a toast in place — message and/or kind — and optionally
	 * reset its auto-dismiss timer. Used for "tick" toasts that flip
	 * from `info` (purple) while an async action is running to
	 * `success` (green) when it finishes, then vanish on the new TTL.
	 * Pinned toasts don't restart their timer (the pin is sacred).
	 */
	function updateToast(
		id: number,
		patch: Partial<Pick<Toast, 'message' | 'kind'>>,
		ttlMs?: number
	): void {
		toasts = toasts.map((t) => (t.id === id ? { ...t, ...patch } : t));
		if (ttlMs !== undefined) {
			const target = toasts.find((t) => t.id === id);
			if (target?.pinned) return;
			const existing = toastTimers.get(id);
			if (existing) clearTimeout(existing);
			toastTimers.set(
				id,
				setTimeout(() => dismissToast(id), ttlMs)
			);
			toasts = toasts.map((t) =>
				t.id === id ? { ...t, ttlMs, startedAt: Date.now() } : t
			);
		}
	}

	// --- Search ---
	let searchResults: SearchResult[] = $state([]);
	// The "people" half of search's fan-out — kind-0 author matches,
	// surfaced as a category distinct from content `searchResults`.
	let searchProfiles: ProfileResult[] = $state([]);
	let searchCount = $state(0);
	let searchLocalCount = $state(0);
	let searchRelayCount = $state(0);
	let searchLoading = $state(false);
	// Separate from searchLoading so the empty-results UI can distinguish
	// "scanning local DB" from "fanning out to relays" — they look
	// different to the user (one is fast and local; the other waits on
	// the network and is the answer to "is this even findable?").
	let searchRelayLoading = $state(false);
	// The effective query (with auto-`by:me` etc. applied) of the most
	// recent run. The offline "Search relays?" CTA replays this exact
	// string with bypass_offline=true so we hit the same filter set, not
	// whatever's currently typed in the input.
	let searchLastQuery = $state('');
	// Populated only when the query contained a `count:NAME` operator.
	// Switches the SearchPanel into "grouped" view: top-level rows are the
	// histogram buckets, each unfolds to its contributing events.
	let searchTagCounts: Record<string, import('$lib/types').TagValueCount[]> = $state({});

	// --- Event view modal ---
	// Set by handleViewJson; rendered by the structured EventViewModal.
	let eventModalData: NostrEvent | SearchResult | null = $state(null);

	// --- Legacy JSON dump modal ---
	// Set by the M-x `tendrl-show-event-json` command (buffer inspector) and
	// by PublishProgressBuffer's raw-event button. Still renders through the
	// legacy <pre> dump in +layout.svelte. Narrowed so each call site is
	// explicit about which shape it's pushing.
	let jsonModalData: { buffer: Buffer } | { rawEvent: unknown } | null = $state(null);

	// Rich multi-event JSON inspector (publish results + compose preview):
	// a list of events each independently expandable, plus expand-all.
	let eventsModal: { title: string; events: EventsModalItem[] } | null = $state(null);

	// Republish diff prompt: set when Publish detects a same-title
	// publication. ComparePublishModal renders the diff and calls
	// confirmRepublish/cancelRepublish. Holds the pending publish args so
	// the chosen action can proceed without re-deriving them.
	let republishPrompt: {
		diff: RepublishDiff;
		sections: ContextItem[];
		pubTitle: string;
		pubTags: TagEntry[];
	} | null = $state(null);

	// --- Search history ---
	// App-level navigation history. Every query / event-view / coord lookup
	// is appended (deduped) here. `currentEntry` tracks what's on screen;
	// `previousEntry` is the depth-1 breadcrumb (the entry that was current
	// just before this one). See docs/workbench-architecture.org Search
	// Invariants for the model.
	let searchHistory: ModalNavEntry[] = $state([]);
	let currentEntry: ModalNavEntry | null = $state(null);
	let previousEntry: ModalNavEntry | null = $state(null);

	// --- Profile ---
	let profilePubkey: string | null = $state(null);

	// --- Identity ---
	let myPubkey: string | null = $state(null);
	let assistantPubkey: string | null = $state(null);
	// Engine's nostrdb data directory — surfaced so the Settings/Purge
	// confirm prompt can show exactly which path is about to be wiped.
	let dataDir: string | null = $state(null);
	let identityStatus: IdentityStatus | null = $state(null);
	let identityLoading = $state(false);
	let identityError: string | null = $state(null);
	let identityPollInterval: ReturnType<typeof setInterval> | null = null;
	let identityDisplayName: string | null = $state(null);
	// Source last persisted via Save Settings → config.toml. Loaded from
	// `/api/v1/settings` at init, BEFORE the auto-reconnect runs. The
	// SettingsBuffer uses it as a fallback for `currentSource` so the
	// radio reflects the user's intent immediately on reload, instead of
	// flashing "engine" for ~2s while the NIP-07 reconnect is in flight.
	let savedIdentitySource: string | null = $state(null);
	// Network mode last persisted in config.toml. Loaded from
	// `/api/v1/settings` at init, BEFORE the live `networkStatus`
	// arrives via `/api/v1/network/status`. The modeline pill uses
	// it as a fallback so the pill doesn't briefly disappear (or
	// flash an em-dash placeholder) at page load.
	//
	// Seeded SYNCHRONOUSLY from localStorage at module-load so the
	// modeline pill renders with the last-known mode before any HTTP
	// fetch completes — important in Vite dev mode where bundle
	// compilation can push initialize() out by seconds. Updated
	// whenever the engine confirms a new mode (live status, settings,
	// or a user-driven toggle).
	// Default to 'auto' when the cache is empty — that's the engine's
	// default mode (per src/network.rs NetworkMode default), so for a
	// fresh user it matches reality without ever showing a "loading"
	// fallback. Returning users get their actual last-known mode from
	// localStorage. If the live state ever turns out to differ, the
	// init/poll/toggle paths overwrite this within milliseconds.
	let savedNetworkMode: 'auto' | 'confirm' = $state(
		((): 'auto' | 'confirm' => {
			if (typeof localStorage === 'undefined') return 'auto';
			const v = localStorage.getItem('tendrl.savedNetworkMode');
			return v === 'confirm' ? 'confirm' : 'auto';
		})()
	);

	function persistNetworkMode(mode: 'auto' | 'confirm') {
		if (typeof localStorage !== 'undefined') {
			try {
				localStorage.setItem('tendrl.savedNetworkMode', mode);
			} catch {
				/* quota / privacy mode — fall back to in-memory only */
			}
		}
	}
	// True while the auto-reconnect path is actively trying to detect +
	// register window.nostr. Lets the UI render a "reconnecting…" state
	// instead of the default engine login form during that window.
	let identityAutoReconnecting = $state(false);
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
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'held' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean; held?: boolean }
	): ContextItem {
		// Auto-hold: any context/compose entry — i.e. anything the user
		// actively routed somewhere — also lands in refs. Refs is the
		// recency history of pool activity; the user removes things from
		// it via drop. Chat-origin items skip the auto-hold (they're
		// internal LLM fragments, not events the user picked).
		const autoHold = (target.context === true || target.compose === true) && fields.origin !== 'chat';
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
			in_compose: target.compose ?? false,
			held: target.held ?? autoHold
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
		items = items.filter((e) => e.in_context || e.in_compose || e.held);
	}

	// Derived view: held items only — SearchPanel's Refs tab reads this.
	// An item with any of in_context / in_compose / held set survives gc();
	// held marks the "in the pool but not yet routed" state explicitly.
	const heldEntries = $derived(items.filter((i) => i.held));

	// ----- Pool helpers keyed by a viewed event (NostrEvent | SearchResult) -----
	//
	// The EventViewModal is opened on a generic Nostr event surface. To
	// reach into the pool we need both an identity key (so we can find the
	// matching ContextItem) and a `fields` payload (so we can create one
	// if it doesn't exist). These two helpers shape both ends from either
	// input form.

	function poolKey(input: NostrEvent | SearchResult): { source_event_id: string; source_addr: NAddr | null } {
		if ('event_id' in input) {
			return { source_event_id: input.event_id, source_addr: input.addr ?? null };
		}
		const dTagEntry = input.tags.find((t) => t[0] === 'd');
		const dTag = dTagEntry?.[1];
		const source_addr: NAddr | null = dTag
			? { kind: input.kind, pubkey: input.pubkey, d_tag: dTag }
			: null;
		return { source_event_id: input.id, source_addr };
	}

	function eventToPoolFields(
		input: NostrEvent | SearchResult
	): Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'held' | 'readonly' | 'context_content'> {
		const { source_event_id, source_addr } = poolKey(input);
		if ('event_id' in input) {
			// SearchResult — preview is what we have without an extra fetch.
			// Callers that need the full body can pre-fetch via fetchEventContent.
			const content = input.preview;
			return {
				title: input.title ?? '[Untitled]',
				content,
				tags: (input.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
				source_event_id,
				source_addr,
				original_content: content,
				origin: 'search'
			};
		}
		const titleTag = input.tags.find((t) => t[0] === 'title');
		const content = input.content;
		return {
			title: titleTag?.[1] ?? '[Untitled]',
			content,
			tags: (input.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
			source_event_id,
			source_addr,
			original_content: content,
			origin: 'search'
		};
	}

	function findPoolItem(input: NostrEvent | SearchResult): ContextItem | null {
		const { source_event_id, source_addr } = poolKey(input);
		return (
			items.find((e) => {
				if (source_event_id && e.source_event_id === source_event_id) return true;
				if (
					source_addr &&
					e.source_addr &&
					source_addr.kind === e.source_addr.kind &&
					source_addr.pubkey === e.source_addr.pubkey &&
					source_addr.d_tag === e.source_addr.d_tag
				)
					return true;
				return false;
			}) ?? null
		);
	}

	/** Pool lookup keyed by an NAddr alone — used by surfaces that only
	 *  carry an addressable coordinate (feed rows, profile cards, reader
	 *  sections) and need to know membership state without reconstructing
	 *  a full NostrEvent. */
	function findPoolItemByAddr(addr: NAddr): ContextItem | null {
		return (
			items.find((e) =>
				e.source_addr != null &&
				e.source_addr.kind === addr.kind &&
				e.source_addr.pubkey === addr.pubkey &&
				e.source_addr.d_tag === addr.d_tag
			) ?? null
		);
	}

	/** Pool lookup keyed by a raw event id. Comments, highlights, and
	 *  any other non-addressable kind need this fallback. */
	function findPoolItemByEventId(eventId: string): ContextItem | null {
		return items.find((e) => e.source_event_id === eventId) ?? null;
	}

	/** Toggle one membership flag on the pool item for `input`. If the item
	 *  doesn't exist yet, it's created with that flag set. If toggling off
	 *  leaves the item with no flags, gc() prunes it.
	 *
	 *  For 'context', we also trigger syncContext() so the chat panel reflects
	 *  the new membership the same way the +context button does. */
	function togglePoolMembership(input: NostrEvent | SearchResult, kind: 'context' | 'compose' | 'held') {
		const existing = findPoolItem(input);
		if (!existing) {
			addToPool(eventToPoolFields(input), { [kind]: true });
			if (kind === 'context') syncContext();
			return;
		}
		const flagKey = kind === 'context' ? 'in_context' : kind === 'compose' ? 'in_compose' : 'held';
		if (existing[flagKey]) {
			items = items.map((e) => (e.id === existing.id ? { ...e, [flagKey]: false } : e));
			gc();
			if (kind === 'context') syncContext();
		} else {
			addToPool(eventToPoolFields(input), { [kind]: true });
			if (kind === 'context') syncContext();
		}
	}

	function togglePoolReadonly(input: NostrEvent | SearchResult) {
		const existing = findPoolItem(input);
		if (!existing) return;
		items = items.map((e) => (e.id === existing.id ? { ...e, readonly: !e.readonly } : e));
	}

	/** Drop the item entirely — clears every flag and prunes via gc(). */
	function dropFromPool(input: NostrEvent | SearchResult) {
		const existing = findPoolItem(input);
		if (!existing) return;
		items = items.map((e) =>
			e.id === existing.id ? { ...e, in_context: false, in_compose: false, held: false } : e
		);
		gc();
		// If the dropped item had been syncing into context, the chat
		// panel needs to forget it.
		if (existing.in_context) syncContext();
	}

	/** Convenience: enter the pool with no routing intent (held only). */
	function holdEvent(input: NostrEvent | SearchResult) {
		addToPool(eventToPoolFields(input), { held: true });
	}

	/** Clear `held` on the item with this UUID and gc(). Keyed by the
	 *  ContextItem id so callers (the Refs tab today, any future
	 *  surface) don't have to reconstruct a NostrEvent input. With the
	 *  auto-hold rule this is rarely the right call — drop is usually
	 *  what the user wants. Kept available for partial-release UX. */
	function releaseHeldItem(itemId: string) {
		const target = items.find((e) => e.id === itemId);
		if (!target) return;
		items = items.map((e) => (e.id === itemId ? { ...e, held: false } : e));
		gc();
	}

	/** Toggle in_context on a pool item by id. Pills on the search/refs
	 *  rows are state indicators that double as toggle buttons — click
	 *  to flip. syncContext always fires so the chat panel reflects the
	 *  new shape. The item's stored content is what gets sent; if it
	 *  came in as a truncated SearchResult preview, that's what the LLM
	 *  sees. (Full-content fetch on toggle is a follow-up.) */
	function routeHeldToContext(itemId: string) {
		const target = items.find((e) => e.id === itemId);
		if (!target) return;
		items = items.map((e) =>
			e.id === itemId ? { ...e, in_context: !e.in_context } : e
		);
		gc();
		syncContext();
	}

	/** Toggle in_compose on a pool item by id. The composer's reactive
	 *  merge picks up the change via composeSections (items.filter
	 *  in_compose). */
	function routeHeldToCompose(itemId: string) {
		const target = items.find((e) => e.id === itemId);
		if (!target) return;
		items = items.map((e) =>
			e.id === itemId ? { ...e, in_compose: !e.in_compose } : e
		);
		gc();
	}

	/** Token-formatted coordinate for the search input. For addressable
	 *  events (have source_addr), uses NIP-01 `a:kind:pubkey:d` notation
	 *  which the search parser understands. For non-addressable kinds
	 *  (comments, highlights, plain notes), falls back to `id:hex`. */
	function coordTokenForItem(itemId: string): string | null {
		const item = items.find((e) => e.id === itemId);
		if (!item) return null;
		if (item.source_addr) {
			const { kind, pubkey, d_tag } = item.source_addr;
			return `a:${kind}:${pubkey}:${d_tag}`;
		}
		if (item.source_event_id) {
			return `id:${item.source_event_id}`;
		}
		return null;
	}

	/** Pill action dispatcher keyed by addressable coordinate. Used by
	 *  feed rows, profile cards, reader outline + paginated header —
	 *  any surface that knows an event by its NAddr.
	 *
	 *  If the item is already in the pool, toggles the relevant flag
	 *  (or drops). If it isn't, fetches the latest local event for
	 *  the coordinate and adds to the pool with the target flag. The
	 *  async path mirrors openAddressableInModal's query shape. */
	async function pillActionByAddr(addr: NAddr, kind: 'context' | 'compose' | 'drop') {
		const existing = findPoolItemByAddr(addr);
		if (kind === 'drop') {
			if (existing) dropPoolItem(existing.id);
			return;
		}
		if (existing) {
			if (kind === 'context') routeHeldToContext(existing.id);
			else routeHeldToCompose(existing.id);
			return;
		}
		// Fresh — fetch the latest replaceable event for this coord and
		// shape it into addToPool fields. Local-only — surfaces that
		// don't have the event yet can use the m menu to broadcast a
		// fetch first.
		try {
			const resp = await api.queryEvents(
				[{ kinds: [addr.kind], authors: [addr.pubkey], '#d': [addr.d_tag] }],
				'local_only'
			);
			const evts = (resp?.events ?? []) as NostrEvent[];
			evts.sort((a, b) => b.created_at - a.created_at);
			const ev = evts[0];
			if (!ev) return;
			const title = ev.tags.find((t) => t[0] === 'title')?.[1] ?? '[Untitled]';
			addToPool(
				{
					title,
					content: ev.content,
					tags: (ev.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
					source_event_id: ev.id,
					source_addr: { kind: ev.kind, pubkey: ev.pubkey, d_tag: addr.d_tag },
					original_content: ev.content,
					origin: 'search'
				},
				{ [kind]: true }
			);
			if (kind === 'context') syncContext();
		} catch (e) {
			console.error('pillActionByAddr failed', e);
		}
	}

	/** Pill action dispatcher keyed by event id. For non-addressable
	 *  kinds (comments, highlights) that don't carry an NAddr. */
	async function pillActionByEventId(eventId: string, kind: 'context' | 'compose' | 'drop') {
		const existing = findPoolItemByEventId(eventId);
		if (kind === 'drop') {
			if (existing) dropPoolItem(existing.id);
			return;
		}
		if (existing) {
			if (kind === 'context') routeHeldToContext(existing.id);
			else routeHeldToCompose(existing.id);
			return;
		}
		try {
			const resp = await api.getEvent(eventId);
			const ev = resp.event as NostrEvent | null;
			if (!ev) return;
			const title = ev.tags.find((t) => t[0] === 'title')?.[1] ?? '[Untitled]';
			addToPool(
				{
					title,
					content: ev.content,
					tags: (ev.tags ?? []).map((t) => ({ name: t[0] ?? '', value: t.slice(1).join(', ') })),
					source_event_id: ev.id,
					source_addr: null,
					original_content: ev.content,
					origin: 'search'
				},
				{ [kind]: true }
			);
			if (kind === 'context') syncContext();
		} catch (e) {
			console.error('pillActionByEventId failed', e);
		}
	}

	/** Drop a pool item entirely — clears every membership and gc()s it
	 *  out. Keyed by the ContextItem's own UUID so refs-row drop buttons
	 *  don't need to reconstruct a NostrEvent input. If the item was in
	 *  context, syncContext() so the chat panel forgets it too. */
	function dropPoolItem(itemId: string) {
		const target = items.find((e) => e.id === itemId);
		if (!target) return;
		items = items.map((e) =>
			e.id === itemId ? { ...e, in_context: false, in_compose: false, held: false } : e
		);
		gc();
		if (target.in_context) syncContext();
	}

	function addToPool(
		fields: Omit<ContextItem, 'id' | 'modified' | 'in_context' | 'in_compose' | 'held' | 'readonly' | 'context_content'>,
		target: { context?: boolean; compose?: boolean; held?: boolean }
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
			// Auto-hold rule (same as makeItem): any context/compose entry
			// also lands in refs, so re-routing an event surfaces it in the
			// refs tab again (refs = recency history of pool activity).
			const autoHold = (target.context === true || target.compose === true) && fields.origin !== 'chat';
			items = items.map((e) =>
				e.id === existing.id
					? {
							...e,
							in_context: e.in_context || (target.context ?? false),
							in_compose: e.in_compose || (target.compose ?? false),
							held: e.held || (target.held ?? false) || autoHold,
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
			let resp = await api.listPublications();
			// Cold-cache fallback: if local nostrdb has nothing (fresh
			// install, post-purge, etc.), retry ONCE with `fetch_always`
			// so the user sees content without manually hitting Sync. In
			// confirm mode the engine pops the FetchConfirmModal; in auto
			// it fires silently and the activity toast tracks progress.
			//
			// The `coldFetchAttempted` guard is essential: loadFeed() is
			// called from ~10 sites — notably an $effect on every
			// FeedBuffer mount, plus search-clear and post-publish — so
			// without it an empty db would re-fire this fetch (and re-pop
			// the confirm modal) on every buffer switch. We attempt the
			// single composite query at most once per session; afterwards
			// the empty-state's "Fetch from relays" button is the way to
			// re-trigger it deliberately.
			if (resp.publications.length === 0 && !coldFetchAttempted) {
				coldFetchAttempted = true;
				try {
					const fetched = await api.listPublications(20, 'fetch_always');
					if (fetched.publications.length > 0) resp = fetched;
				} catch {
					/* relay fetch failed — keep the empty local result */
				}
			}
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
			const canSign = identityCanSign(identityStatus);
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

	// Sign the current compose into a local snapshot in the db. Signing — not a
	// passive unsigned write — is how a draft becomes a committed, versioned
	// event; broadcasting to relays is a separate, later step.
	async function handleComposePublish(
		items: ContextItem[],
		meta?: { title: string; tags: TagEntry[] }
	) {
		const sections = items.length > 0 ? items : compose.sections;
		if (!sections.length) {
			pushToast('Nothing to sign — no sections detected', 'error', 4000);
			return;
		}
		// Prefer the title/tags parsed at click time (plain mode) over the
		// reactive compose state, which can lag a same-tick edit and sign
		// an empty title.
		const pubTitle = meta?.title ?? compose.title;
		const pubTags = meta?.tags ?? compose.tags;

		// Signing writes a snapshot to the db, which needs an unlocked identity.
		// Without a signer the user's path is "Save draft" — we never write
		// unsigned events (the engine rejects sign:false).
		if (!identityCanSign(identityStatus)) {
			pushToast('Signing needs an unlocked identity — log in, or Save draft.', 'error', 4500);
			return;
		}

		// Republish guard: a *signed* publication of yours with the same title
		// already exists → show the diff so identifiers are reused (replace,
		// same a-tag → only the latest version shows) rather than forked.
		const diff = await detectRepublish(pubTitle, sections);
		if (diff) {
			republishPrompt = { diff, sections, pubTitle, pubTags };
			return; // ComparePublishModal drives reuse / sign-as-new / cancel
		}

		await executePublish(sections, pubTitle, pubTags);
	}

	// Reuse identifiers from the matched publication (replace) or publish
	// fresh. Called by ComparePublishModal.
	async function confirmRepublish(reuse: boolean) {
		const p = republishPrompt;
		republishPrompt = null;
		if (!p) return;
		// Reuse map keyed by exact section title. The diff's matched entries
		// carry the incoming section's title alongside the existing d-tag, and
		// executePublish republishes the same `sections` array — so a title
		// lookup is exact and needs no client-side slug (the slug matching
		// already happened engine-side).
		const overrides = reuse
			? {
					pubDTag: p.diff.pubDTag,
					dTagByTitle: Object.fromEntries(
						p.diff.matched
							.filter((m) => m.dTag != null)
							.map((m) => [m.title, m.dTag as string])
					)
				}
			: undefined;
		await executePublish(p.sections, p.pubTitle, p.pubTags, overrides);
	}

	function cancelRepublish() {
		republishPrompt = null;
	}

	// Sign the compose into a local snapshot — sign:true, broadcast:false. The
	// signature is the snapshot; nostrdb keeps every version. Broadcasting is a
	// separate, explicit step. The caller guarantees an unlocked identity.
	async function executePublish(
		sections: ContextItem[],
		pubTitle: string,
		pubTags: TagEntry[],
		overrides?: { pubDTag: string; dTagByTitle: Record<string, string> }
	) {
		const progressToast = pushToast(
			`Signing ${sections.length + 1} events… (${identityStatus?.source ?? 'engine'})`,
			'pending',
			120000
		);

		// If any section has a source_addr OR the draft was seeded from a
		// publication, route through the block endpoint so we emit fork-
		// marker tags. Otherwise use the flat publish (where republish d-tag
		// reuse is honored).
		const hasProvenance =
			!!composeSourcePubAddr || sections.some((s) => !!s.source_addr);

		try {
			let resp: api.PublishResponse;
			if (hasProvenance) {
				// NOTE: republish d-tag reuse is not wired through the block/
				// fork path yet — see docs/republish-diff.md (deferred).
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
				resp = await api.publishBlocks({
					title: pubTitle,
					tags: pubTags.map((t) => [t.name, t.value] as [string, string]),
					blocks,
					source_publication_addr: composeSourcePubAddr,
					source_publication_event_id: composeSourcePubEventId,
					sign: true,
					broadcast: false
				});
				console.log('Signed (blocks):', resp.publication_id);
			} else {
				resp = await api.publish({
					title: pubTitle,
					tags: pubTags.map((t) => [t.name, t.value] as [string, string]),
					sections: sections.map((s) => ({
						title: s.title,
						content: s.content,
						level: s.level,
						tags: s.tags.map((t) => [t.name, t.value] as [string, string]),
						// Reuse the matched section's d-tag on republish so the
						// 30041 replaces rather than forks.
						d_tag: overrides?.dTagByTitle[s.title]
					})),
					d_tag: overrides?.pubDTag,
					sign: true,
					broadcast: false
				});
				console.log('Signed:', resp.publication_id);
			}

			updateToast(
				progressToast,
				{ message: 'Signed — local snapshot. Broadcast it when ready.', kind: 'success' },
				4000
			);
			await loadFeed();
		} catch (e) {
			console.error('Sign compose failed:', e);
			updateToast(
				progressToast,
				{ message: `Sign failed: ${e instanceof Error ? e.message : String(e)}`, kind: 'error' },
				6000
			);
		}
	}

	// Diff the current compose against the last *published* (signed) version of
	// this article — "what have I changed since I published?". Held for the
	// PublishedDiffModal; null = closed.
	let publishedDiff: api.VersionDiff | null = $state(null);

	async function handleComposeDiffPublished(
		items: ContextItem[],
		meta?: { title: string; tags: TagEntry[] }
	) {
		const sections = items.length > 0 ? items : compose.sections;
		const title = meta?.title ?? compose.title;
		const tags = meta?.tags ?? compose.tags;
		try {
			const resp = await api.diffVsPublished({
				title,
				tags: tags.map((t) => [t.name, t.value] as [string, string]),
				sections: sections.map((s) => ({
					title: s.title,
					content: s.content,
					level: s.level,
					tags: s.tags.map((t) => [t.name, t.value] as [string, string])
				})),
				// The PUBLISHED d-tag (when this compose was seeded from a published
				// publication), NOT the draft's d-tag — those are independent nanoid
				// spaces. With no source pub the engine matches by title slug.
				d_tag: composeSourcePubAddr?.d_tag ?? undefined
			});
			if (!resp.published || !resp.diff) {
				pushToast('Not published yet — nothing to compare against.', 'info', 4000);
				return;
			}
			publishedDiff = resp.diff;
		} catch (e) {
			pushToast(`Diff failed: ${e instanceof Error ? e.message : String(e)}`, 'error', 5000);
		}
	}

	function closePublishedDiff() {
		publishedDiff = null;
	}

	// Broadcast an already-signed local snapshot to the publish relays — the
	// explicit step after Sign. No re-signing; flips the feed pill local→relay.
	async function handleBroadcastPublication(addr: NAddr) {
		const t = pushToast('Broadcasting…', 'pending', 60000);
		try {
			const resp = await api.broadcastPublication(addr.pubkey, addr.d_tag);
			updateToast(
				t,
				{
					message: `Broadcast ${resp.event_count} event${resp.event_count === 1 ? '' : 's'} — ${resp.successful}/${resp.total} relay acks`,
					kind: resp.successful > 0 ? 'success' : 'error'
				},
				4500
			);
			await loadFeed();
		} catch (e) {
			updateToast(
				t,
				{ message: `Broadcast failed: ${e instanceof Error ? e.message : String(e)}`, kind: 'error' },
				6000
			);
		}
	}

	// Does a publication of mine with this title already exist? If so the engine
	// builds a section-level diff (matched by title slug) so a republish can
	// reuse identifiers and replace instead of forking. The slug-matching, TOC
	// flatten, and diff all live in Rust now (POST /publish/republish-diff);
	// the engine resolves "mine" from the active identity. Fail-open: a lookup
	// or network error must never block a publish, so we swallow it and treat
	// it as "no existing match".
	async function detectRepublish(
		pubTitle: string,
		sections: ContextItem[]
	): Promise<RepublishDiff | null> {
		if (!pubTitle.trim()) return null;
		try {
			return await api.republishDiff(
				pubTitle,
				sections.map((s) => ({ title: s.title, content: s.content }))
			);
		} catch (e) {
			console.warn('Republish detection skipped:', e);
			return null;
		}
	}

	// Turn a built nostr event (Value) into an inspector row.
	function eventToModalItem(ev: unknown, idx: number): EventsModalItem {
		const obj = (ev ?? {}) as { kind?: number; id?: string; tags?: unknown };
		const tags = Array.isArray(obj.tags) ? (obj.tags as string[][]) : [];
		const titleTag = tags.find((t) => t[0] === 'title')?.[1];
		const dTag = tags.find((t) => t[0] === 'd')?.[1];
		return {
			label: titleTag || dTag || `event ${idx + 1}`,
			kind: obj.kind,
			id: obj.id,
			json: ev
		};
	}

	// ===================== Drafts (engine DraftStore) =====================

	async function refreshComposeDrafts() {
		try {
			composeDrafts = (await api.listDrafts()).drafts;
		} catch (e) {
			console.warn('List drafts failed:', e);
		}
	}

	// Save the current compose as a draft. Mirrors the publish payload (title,
	// tags, sections) minus signing — a draft is never signed. Section d-tags
	// are minted + persisted engine-side, so we don't supply them here.
	async function handleComposeSaveDraft(
		items: ContextItem[],
		meta?: { title: string; tags: TagEntry[] }
	) {
		const sections = items.length > 0 ? items : compose.sections;
		const title = meta?.title ?? compose.title;
		const tags = meta?.tags ?? compose.tags;
		if (!title.trim() && sections.length === 0) {
			pushToast('Nothing to save — the draft is empty', 'error', 3000);
			return;
		}
		try {
			const resp = await api.saveDraft({
				title,
				tags: tags.map((t) => [t.name, t.value] as [string, string]),
				sections: sections.map((s) => ({
					title: s.title,
					content: s.content,
					level: s.level,
					tags: s.tags.map((t) => [t.name, t.value] as [string, string])
				})),
				// Thread the session's d-tag so this save versions the same
				// publication instead of forking a new one.
				d_tag: composeDTag ?? undefined
			});
			composeDTag = resp.d_tag;
			pushToast('Draft saved', 'success');
			await refreshComposeDrafts();
		} catch (e) {
			pushToast(`Save draft failed: ${e instanceof Error ? e.message : String(e)}`, 'error', 5000);
		}
	}

	// Resume a saved draft: replace any current compose sections with the
	// draft's, restore title/tags, and open the composer. Existing context-only
	// pool items are preserved (we only clear `in_compose`).
	async function handleLoadDraft(draftId: string) {
		try {
			const draft = await api.loadDraft(draftId);
			const cs = draft.compose_state;
			const draftItems = cs.sections.map((s) =>
				makeItem(
					{
						title: s.title,
						content: s.content,
						tags: s.tags.map((t) => ({ name: t.name, value: t.value })),
						original_content: s.content,
						origin: 'compose' as const,
						level: s.level
					},
					{ compose: true }
				)
			);
			items = [...items.map((e) => ({ ...e, in_compose: false })), ...draftItems];
			composeTitle = cs.title;
			composeTags = cs.tags.map((t) => ({ name: t.name, value: t.value }));
			// Resume onto the same publication identity so the next save adds a
			// version rather than forking.
			composeDTag = cs.d_tag ?? null;
			composeSourcePubAddr = null;
			composeSourcePubEventId = null;
			composeSourceSectionOrder = [];
			previewVisible = false;
			navigateToCompose();
			pushToast(`Draft "${cs.title || 'untitled'}" loaded`, 'success');
		} catch (e) {
			pushToast(`Couldn't load draft: ${e instanceof Error ? e.message : String(e)}`, 'error', 5000);
		}
	}

	async function handleDeleteDraft(draftId: string) {
		try {
			await api.deleteDraft(draftId);
			await refreshComposeDrafts();
		} catch (e) {
			pushToast(`Delete draft failed: ${e instanceof Error ? e.message : String(e)}`, 'error', 5000);
		}
	}

	// Build the would-be event graph for the current compose and open the
	// JSON inspector — no signing/ingest/broadcast. Mirrors the publish
	// request shape so the preview matches what publishing emits.
	async function handleComposePreview(items: ContextItem[], meta?: { title: string; tags: TagEntry[] }) {
		const sections = items.length > 0 ? items : compose.sections;
		if (!sections.length) {
			pushToast('Nothing to preview — no sections detected', 'error', 4000);
			return;
		}
		const pubTitle = meta?.title ?? compose.title;
		const pubTags = meta?.tags ?? compose.tags;
		try {
			const resp = await api.previewPublication({
				title: pubTitle,
				tags: pubTags.map((t) => [t.name, t.value] as [string, string]),
				sections: sections.map((s) => ({
					title: s.title,
					content: s.content,
					level: s.level,
					tags: s.tags.map((t) => [t.name, t.value] as [string, string])
				})),
				sign: false,
				broadcast: false
			});
			eventsModal = {
				title: `Preview — ${pubTitle || 'untitled'} (${resp.events.length} events)`,
				events: resp.events.map((e, i) => eventToModalItem(e, i))
			};
		} catch (e) {
			pushToast(`Preview failed: ${e instanceof Error ? e.message : String(e)}`, 'error', 6000);
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

	// ===================== Search history =====================

	function normalizeQuery(q: string): string {
		return q.trim().replace(/\s+/g, ' ');
	}

	function entryKey(e: ModalNavEntry): string {
		if (e.kind === 'query') return `q|${normalizeQuery(e.query)}|s=${e.opts.scopeToMe}`;
		if (e.kind === 'nevent') return `e|${e.eventId.toLowerCase()}`;
		return `a|${e.coord.kind}:${e.coord.pubkey}:${e.coord.d_tag}`;
	}

	function pushHistoryEntry(entry: ModalNavEntry) {
		const key = entryKey(entry);

		// Same as current → no pointer move. Still bump the in-list timestamp
		// so recency tracks user activity even when no navigation happens.
		if (currentEntry && entryKey(currentEntry) === key) {
			const existing = searchHistory.find((e) => entryKey(e) === key);
			if (existing) existing.lastRunAt = entry.lastRunAt;
			return;
		}

		// New entry displaces current — move the depth-1 highlight.
		previousEntry = currentEntry;
		currentEntry = entry;

		// Dedupe into the list. Existing entries keep their position; only
		// the timestamp (and title, if newly known) update.
		const idx = searchHistory.findIndex((e) => entryKey(e) === key);
		if (idx >= 0) {
			const existing = searchHistory[idx] as ModalNavEntry & { title?: string };
			existing.lastRunAt = entry.lastRunAt;
			const newTitle = (entry as { title?: string }).title;
			if (newTitle && !existing.title) existing.title = newTitle;
			return;
		}
		searchHistory = [entry, ...searchHistory];
	}

	// ===================== Containing publications =====================
	//
	// For an event (section / wiki / long-form / publication), look up
	// kind-30040 publication indexes that reference it via `#a` or `#e`
	// tags. Cached per event id to avoid re-querying as the user paginates
	// or chains through the modal.

	type ContainingResult = {
		status: 'loading' | 'loaded' | 'failed';
		indexes: { id: string; pubkey: string; d_tag: string; title: string }[];
	};
	const containingCache = new Map<string, ContainingResult>();

	function addressOf(event: NostrEvent): string | null {
		const d = event.tags.find((t: string[]) => t[0] === 'd')?.[1];
		if (!d) return null;
		return `${event.kind}:${event.pubkey}:${d}`;
	}

	async function findContainingIndexes(
		event: NostrEvent | SearchResult
	): Promise<ContainingResult> {
		// Normalize id / kind across the two shapes.
		const id = ('event_id' in event ? event.event_id : event.id).toLowerCase();
		const kind = event.kind;
		const ALLOWED = new Set([30041, 30818, 30040, 30023]);
		if (!ALLOWED.has(kind)) {
			const empty: ContainingResult = { status: 'loaded', indexes: [] };
			containingCache.set(id, empty);
			return empty;
		}

		const cached = containingCache.get(id);
		if (cached) return cached;

		// Build the addressOf reference if the event has a d-tag. For
		// SearchResult, the `addr` field already gives us the components;
		// for NostrEvent, walk the tags.
		let aRef: string | null = null;
		if ('addr' in event && event.addr) {
			aRef = `${event.kind}:${event.addr.pubkey}:${event.addr.d_tag}`;
		} else if ('tags' in event && Array.isArray(event.tags)) {
			aRef = addressOf(event as NostrEvent);
		}

		const loading: ContainingResult = { status: 'loading', indexes: [] };
		containingCache.set(id, loading);

		try {
			const [byA, byE] = await Promise.all([
				aRef
					? api.queryEvents([{ kinds: [30040], '#a': [aRef] }], 'local_only')
					: Promise.resolve({ events: [] }),
				api.queryEvents([{ kinds: [30040], '#e': [id] }], 'local_only')
			]);
			const seen = new Set<string>();
			const indexes: ContainingResult['indexes'] = [];
			for (const ev of [...(byA?.events ?? []), ...(byE?.events ?? [])]) {
				const e = ev as NostrEvent;
				if (seen.has(e.id)) continue;
				seen.add(e.id);
				const title = e.tags.find((t: string[]) => t[0] === 'title')?.[1];
				const d_tag = e.tags.find((t: string[]) => t[0] === 'd')?.[1];
				if (!title || !d_tag) continue;
				indexes.push({ id: e.id, pubkey: e.pubkey, d_tag, title });
				if (indexes.length >= 5) break;
			}
			const result: ContainingResult = { status: 'loaded', indexes };
			containingCache.set(id, result);
			return result;
		} catch (e) {
			console.error('findContainingIndexes failed:', e);
			const failed: ContainingResult = { status: 'failed', indexes: [] };
			containingCache.set(id, failed);
			return failed;
		}
	}

	/**
	 * Open the structured EventViewModal on the newest event matching the
	 * given replaceable-event coordinate (kind:pubkey:d_tag). Used by the
	 * reader's "JSON" affordances (publication-level + per-section).
	 *
	 * Local-only per the search invariant; if the event isn't locally
	 * indexed yet, the modal stays closed and we log. Pushes a naddr
	 * history entry so the user can return to it via the popover.
	 */
	async function openAddressableInModal(coord: {
		kind: number;
		pubkey: string;
		d_tag: string;
	}): Promise<void> {
		try {
			const resp = await api.queryEvents(
				[
					{
						kinds: [coord.kind],
						authors: [coord.pubkey],
						'#d': [coord.d_tag]
					}
				],
				'local_only'
			);
			const evts = (resp?.events ?? []) as NostrEvent[];
			evts.sort((a, b) => b.created_at - a.created_at);
			const ev = evts[0];
			if (!ev) {
				console.warn('openAddressableInModal: no local event for', coord);
				return;
			}
			eventModalData = ev;
			pushHistoryEntry({
				kind: 'naddr',
				coord,
				title: ev.tags.find((t: string[]) => t[0] === 'title')?.[1],
				lastRunAt: Date.now()
			});
		} catch (e) {
			console.error('openAddressableInModal failed:', e);
		}
	}

	async function getEventForModal(eventId: string) {
		const id = eventId.toLowerCase();
		try {
			const resp = await api.getEvent(id);
			const ev = resp.event as NostrEvent;
			eventModalData = ev;
			const titleTag = Array.isArray(ev?.tags)
				? ev.tags.find((t: string[]) => t[0] === 'title')
				: undefined;
			pushHistoryEntry({
				kind: 'nevent',
				eventId: id,
				title: titleTag?.[1],
				lastRunAt: Date.now()
			});
		} catch (e) {
			console.error('getEventForModal failed:', e);
		}
	}

	// ===================== Search =====================

	async function handleSearch(query: string, opts: { scopeToMe?: boolean } = {}) {
		const scopeToMe = opts.scopeToMe ?? true;
		if (!query.trim()) {
			searchResults = [];
			searchProfiles = [];
			searchCount = 0;
			searchLocalCount = 0;
			searchRelayCount = 0;
			searchTagCounts = {};
			if (docMode === 'empty') await loadFeed();
			return;
		}

		// Push synchronously, before any await. Two back-to-back searches
		// must preserve user-perceived order in the history list.
		pushHistoryEntry({
			kind: 'query',
			query,
			opts: { scopeToMe },
			lastRunAt: Date.now()
		});

		searchLoading = true;
		try {
			// Apply the configured search defaults — kind scope, author
			// scope, time window — to the raw query. A query that writes
			// its own k:/by:/since:/until: keeps it; `scopeToMe=false`
			// (some history replays) suppresses the author default. This
			// is the same scope the offline "Search relays" fallback
			// inherits, since it replays searchLastQuery.
			const effectiveQuery = applySearchDefaults(query, {
				scopeAuthor: scopeToMe,
				hasIdentity: !!myPubkey
			});

			searchLastQuery = effectiveQuery;
			const resp = await api.search(
				effectiveQuery,
				searchConfig.limit,
				myPubkey ?? undefined
			);
			searchResults = dedupeLatestProfiles(resp.results);
			searchProfiles = resp.profiles ?? [];
			searchCount = searchResults.length;
			searchLocalCount = resp.local_count;
			searchRelayCount = resp.relay_count;
			searchTagCounts = resp.tag_counts ?? {};

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
						section_count: r.tags.filter(t => t[0] === 'a').length,
						// Search results don't carry relay/signature provenance.
						relays: [],
						signed: true
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

		// Auto-fan-out to relays when the local query returned 0 hits.
		// Applies in BOTH modes: in auto the engine fans out silently
		// (the previous behavior left the user staring at an empty
		// panel with no signal); in confirm the modal pops as the
		// approve-gate. Either way the user always learns whether
		// relays have it, without an extra click.
		const mode = networkStatus?.mode;
		if (
			searchCount === 0 &&
			searchLastQuery &&
			(mode === 'auto' || mode === 'confirm')
		) {
			await handleSearchViaRelays();
		}
	}

	// "Search relays" — re-run the current query with fetch_always so the
	// engine reaches relays. In Confirm mode the engine emits a confirm
	// Intent (rendered by the SSE-driven modal); in Auto mode it fetches
	// straight away. The fetch toast is driven by the engine's progress
	// events, so none is pushed here.
	async function handleSearchViaRelays() {
		if (!searchLastQuery) return;
		searchRelayLoading = true;
		try {
			const resp = await api.search(
				searchLastQuery,
				searchConfig.limit,
				myPubkey ?? undefined,
				'fetch_always',
				{
					relays: searchConfig.relays.length > 0 ? searchConfig.relays : undefined,
					bypassOffline: true
				}
			);
			searchResults = dedupeLatestProfiles(resp.results);
			searchProfiles = resp.profiles ?? [];
			searchCount = searchResults.length;
			searchLocalCount = resp.local_count;
			searchRelayCount = resp.relay_count;
			searchTagCounts = resp.tag_counts ?? {};
			const pks = [...new Set(resp.results.map((r) => r.author))];
			api.prefetchProfiles(pks);
		} catch (e) {
			console.error('Relay search failed:', e);
		} finally {
			searchRelayLoading = false;
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
			eventModalData = resp.event as NostrEvent;
		} catch {
			eventModalData = result;
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
			searchProfiles = searchProfiles.filter(p => p.pubkey !== result.author);
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

			// Auto-backfill missing sections + nested indexes from
			// relays. In confirm mode the user gets ONE modal listing
			// what's about to be fetched (instead of N modals during
			// per-section lazy load). In auto mode the activity toast
			// tracks per-relay progress. After the backfill ingest, the
			// reader's lazy section getter sees fresh data.
			backfillCurrentPublication(pubkey, d_tag).catch((e) => {
				console.warn('Auto-backfill failed:', e);
			});
		} catch (e) {
			console.error('Failed to open publication:', pubkey, d_tag, e);
			navigateHome();
		} finally {
			docLoading = false;
		}
	}

	/** Fire the backfill endpoint and, if it ingested anything new,
	 *  reload the TOC so the lazy section reads see the fresh content. */
	async function backfillCurrentPublication(pubkey: string, d_tag: string): Promise<void> {
		const resp = await api.backfillPublication(pubkey, d_tag);
		if (resp.fetched > 0 && publication?.addr.pubkey === pubkey && publication?.addr.d_tag === d_tag) {
			// Re-load the publication tree so newly-ingested sections
			// show up. local_first finds them in cache now.
			try {
				const fresh = await api.getPublication(pubkey, d_tag, 'local_first');
				publication = fresh.publication;
				sections = fresh.toc.map((entry, i) => ({
					addr: entry.addr,
					title: entry.title,
					content: null,
					position: i,
					status: 'pending' as const
				}));
			} catch {
				/* keep the previous TOC if re-load fails */
			}
		}
	}

	// Resolve a kind 1111 (NIP-22) or 9802 (NIP-84) result to the article
	// it cites and open that in the reader. For comments we prefer the
	// uppercase A/E tag (root scope of the thread) over lowercase a/e
	// (immediate parent) so clicking any reply lands on the article it
	// was replying inside, not on the parent comment. For highlights we
	// prefer lowercase `a` (NIP-84 uses lowercase only).
	async function openReferencedTarget(result: SearchResult) {
		try {
			const resp = await api.getEvent(result.event_id);
			const ev = resp.event as
				| { kind?: number; tags?: string[][]; pubkey?: string }
				| null;
			if (!ev) {
				console.warn('openReferencedTarget: event not found', result.event_id);
				return;
			}
			const tags = ev.tags ?? [];

			const findTag = (name: string) =>
				tags.find((t) => t[0] === name && t[1])?.[1];

			// NIP-22 conventionally puts uppercase A/E (root scope) on
			// every comment. Highlights use lowercase a/e. Try both orders.
			const aValue =
				result.kind === 1111
					? findTag('A') ?? findTag('a')
					: findTag('a') ?? findTag('A');
			const eValue =
				result.kind === 1111
					? findTag('E') ?? findTag('e')
					: findTag('e') ?? findTag('E');

			const marker =
				result.kind === 9802
					? `highlight=${result.event_id}`
					: `focus_comment=${result.event_id}`;

			if (aValue) {
				const parts = aValue.split(':');
				if (parts.length >= 3) {
					const kind = parseInt(parts[0], 10);
					const pubkey = parts[1];
					const d_tag = parts.slice(2).join(':');
					const bufId = `reader:${kind}:${pubkey}:${d_tag}?${marker}`;
					navigateToReader(
						bufId,
						kind === 30040 ? 'reader' : 'event',
						d_tag.length > 24 ? d_tag.slice(0, 24) + '…' : d_tag
					);
					return;
				}
			}
			if (eValue) {
				navigateToReader(
					`reader:event:${eValue}?${marker}`,
					'event',
					eValue.slice(0, 8) + '…'
				);
				return;
			}

			// No parseable parent — fall back to opening the event in the
			// reader on its own. This still beats silently swallowing the
			// click; the user at least sees the comment/highlight body.
			navigateToReader(
				`reader:event:${result.event_id}`,
				result.kind === 9802 ? 'highlight' : 'comment',
				result.event_id.slice(0, 8) + '…'
			);
		} catch (e) {
			console.warn('openReferencedTarget failed', e);
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
		// NIP-22 comments and NIP-84 highlights aren't destinations on
		// their own — they're citations of an article section. Resolve
		// the referenced target and open *that* in the reader, with a
		// marker so the matching thread/highlight is in view.
		if (result.kind === 1111 || result.kind === 9802) {
			await openReferencedTarget(result);
			return;
		}
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
		composeDTag = null; // fresh publication identity
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
		const canSign = identityCanSign(identityStatus);
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

	// Lightweight status refresh (no sync). Used when opening the
	// Settings panel so the embedding section reflects current sidecar
	// health / index counts without triggering a (heavy) embed pass.
	async function refreshEmbeddingStatus() {
		try { embeddingStatus = await api.getEmbeddingStatus(); } catch {}
	}

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

	// Persist which kinds get embedded (engine-side). The returned status
	// reflects the new selection's total_events, so the panel re-renders in
	// one round trip.
	async function handleSetEmbedKinds(kinds: number[]) {
		try {
			embeddingStatus = await api.setEmbedKinds(kinds);
		} catch (e) {
			console.error('Set embed kinds failed:', e);
		}
	}

	async function handleSetAutoEmbed(enabled: boolean) {
		try {
			embeddingStatus = await api.setAutoEmbed(enabled);
		} catch (e) {
			console.error('Set auto-embed failed:', e);
		}
	}

	// ===================== Network =====================

	async function handleSetNetworkMode(mode: NetworkMode) {
		try {
			await api.setNetworkMode(mode);
			networkStatus = await api.getNetworkStatus();
			// User-driven toggle — update the cache immediately so
			// the next page-load instant-paints the new mode.
			if (mode === 'auto' || mode === 'confirm') {
				savedNetworkMode = mode;
				persistNetworkMode(mode);
			}
		} catch (e) {
			console.error('Failed to set network mode:', e);
		}
	}

	// ===================== Purge / Export / Import =====================

	/** Trigger an engine cache purge. The engine deletes its LMDB
	 *  files and re-execs itself in-place (~1 second of unavailability).
	 *  This shows a pending toast that resolves once the engine comes
	 *  back, by polling /api/v1/network/status. */
	async function handlePurge(): Promise<void> {
		const toastId = pushToast('Purging local cache…', 'pending', 120_000);
		let resp: Response;
		try {
			resp = await fetch('/api/v1/purge', { method: 'POST' });
		} catch (e) {
			updateToast(
				toastId,
				{ message: `Purge request failed: ${e instanceof Error ? e.message : String(e)}`, kind: 'error' },
				5000
			);
			return;
		}
		if (!resp.ok) {
			updateToast(toastId, { message: `Purge request failed: ${resp.status}`, kind: 'error' }, 5000);
			return;
		}
		// Engine acknowledged; it'll re-exec in ~150ms. Poll until the
		// new engine answers /network/status, then flip to success.
		updateToast(toastId, { message: 'Engine restarting…' });
		const startedAt = Date.now();
		const deadline = startedAt + 15_000; // 15s should be plenty
		while (Date.now() < deadline) {
			await new Promise((r) => setTimeout(r, 300));
			try {
				const r = await fetch('/api/v1/network/status');
				if (r.ok) {
					updateToast(toastId, { message: 'Purged + reconnected', kind: 'success' }, 2500);
					// Re-load anything the engine just lost (relay
					// config still survives since it's in relays.json,
					// but identity session resets — refresh it).
					try {
						identityStatus = await api.getIdentity();
					} catch {}
					return;
				}
			} catch {
				/* engine still down, keep polling */
			}
		}
		updateToast(
			toastId,
			{ message: "Engine didn't come back in 15s — check the terminal", kind: 'error' },
			7000
		);
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
			const resp = await api.fetchFromRelay([url], kinds, [], 200, { modeConfirm: true });
			console.log(`Fetched ${resp.fetched} events from ${resp.relays.join(', ')}`);
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
		onDiscussion?: (event_id: string, kind: number) => void;
		/** Open an arbitrary reader buffer by its id. Used when the id
		 *  needs to carry a marker (`?focus_comment=`, `?highlight=`)
		 *  that the structured handlers above can't express. */
		onReader?: (buffer_id: string, label: string, kicker: string) => void;
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

	// NIP-22 comments (kind 1111) and NIP-84 highlights (kind 9802) open
	// in a dedicated DiscussionViewBuffer rather than the normal reader
	// because they're meta-events *about* other content: the UI needs to
	// surface the parent reference and (for comments) the thread, not the
	// raw event content alone.
	function navigateToDiscussion(event_id: string, kind: number) {
		docMode = 'reading';
		if (navHandlers?.onDiscussion) {
			navHandlers.onDiscussion(event_id, kind);
		} else {
			goto(`/d/${event_id}`);
		}
	}

	function navigateToReader(buffer_id: string, label: string, kicker: string) {
		docMode = 'reading';
		if (navHandlers?.onReader) {
			navHandlers.onReader(buffer_id, label, kicker);
		} else {
			// No-op fallback when running outside the shell — there isn't a
			// route that maps to arbitrary reader buffer ids today.
			console.warn('navigateToReader: no handler registered', buffer_id);
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
		loadSearchConfig();
		// Fire all three cheap GETs in parallel — none depend on each
		// other, and the modeline pills + chat composer all need this
		// data ASAP. Previously getConfig was awaited before the other
		// two, adding one extra round-trip's worth of wait before the
		// network pill could light up.
		const [cfgResult, networkStatusResult, settingsResult] = await Promise.allSettled([
			api.getConfig(),
			api.getNetworkStatus(),
			api.getSettings()
		]);
		if (cfgResult.status === 'fulfilled') {
			const cfg = cfgResult.value;
			myPubkey = cfg.my_pubkey;
			assistantPubkey = cfg.assistant_pubkey;
			dataDir = cfg.data_dir ?? null;
			console.log('Config loaded, myPubkey:', myPubkey, 'assistantPubkey:', assistantPubkey);
		} else {
			console.warn('Config fetch failed:', cfgResult.reason);
		}
		if (networkStatusResult.status === 'fulfilled') {
			networkStatus = networkStatusResult.value;
			// Mirror the live mode into the saved cache so the next
			// reload reflects what the engine actually said, not just
			// what config.toml hints.
			const m = networkStatusResult.value.mode;
			if (m === 'auto' || m === 'confirm') {
				savedNetworkMode = m;
				persistNetworkMode(m);
			}
		}
		// Hydrate editor / compose defaults from config.toml so a reload
		// reflects the user's last-saved settings (instead of resetting
		// to hard-coded defaults). Settings page's "Save settings" writes
		// these back via the snapshot endpoint.
		if (settingsResult.status === 'fulfilled') {
			const s = settingsResult.value;
			editorLineNumbers = s.editor.line_numbers;
			editorVimMode = s.editor.vim_mode;
			editorInsertMode = s.editor.insert_mode as EditorInsertMode;
			composeDefaultMode = s.compose.default_mode as ComposeDefaultMode;
			syncMode = s.compose.sync_mode as SyncMode;
			buttonLabels = s.compose.button_labels as ButtonLabels;
			savedIdentitySource = s.identity?.source ?? null;
			if (s.network?.mode === 'auto' || s.network?.mode === 'confirm') {
				savedNetworkMode = s.network.mode;
				persistNetworkMode(s.network.mode);
			}
		}
		// Auto-reconnect to the previously-chosen signing source. NIP-07
		// is a per-session connection that the browser holds; persisting
		// the *intent* in config.toml lets us re-establish on reload
		// without making the user re-click "use NIP-07". If the extension
		// isn't reachable (uninstalled / not yet injected) the call
		// surfaces a soft error and we fall back to engine source.
		if (savedIdentitySource === 'nip07') {
			// Poll for window.nostr — most extensions inject at
			// document_start but slow ones (and dev-mode reloads) can
			// take ~hundreds of ms. Try a few times with increasing
			// delays before giving up.
			identityAutoReconnecting = true;
			try {
				const { detectNip07 } = await import('$lib/identity/signer');
				let detected = false;
				for (const delay of [0, 100, 250, 500, 1000]) {
					if (delay > 0) await new Promise((r) => setTimeout(r, delay));
					if (detectNip07()) {
						detected = true;
						break;
					}
				}
				if (detected) {
					console.log('[identity] auto-reconnecting NIP-07 signer (saved in config.toml)');
					await handleSelectNip07Source();
					if (identityError) {
						console.warn('[identity] auto-reconnect failed:', identityError);
						pushToast(
							`NIP-07 auto-reconnect failed: ${identityError}. Re-pick "nip07" in Settings.`,
							'error',
							7000
						);
					} else {
						console.log(
							'[identity] auto-reconnected:',
							identityStatus?.source,
							identityStatus?.npub?.slice(0, 16)
						);
					}
				} else {
					console.warn(
						'[identity] saved source = nip07 but window.nostr not reachable after ~2s — staying on engine. Pick NIP-07 manually if the extension is now available.'
					);
					pushToast(
						'NIP-07 extension not detected after 2s — saved source is nip07 but staying on engine. Re-pick in Settings once the extension is reachable.',
						'info',
						7000
					);
				}
			} finally {
				identityAutoReconnecting = false;
			}
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
			try {
				const ns = await api.getNetworkStatus();
				networkStatus = ns;
				// Keep the localStorage cache in sync with the live
				// engine so the next reload's instant-paint matches
				// reality (e.g. user toggled mode in a different tab).
				if (ns.mode === 'auto' || ns.mode === 'confirm') {
					if (savedNetworkMode !== ns.mode) {
						savedNetworkMode = ns.mode;
						persistNetworkMode(ns.mode);
					}
				}
			} catch {}
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

	// Set the engine auto-lock timeout (minutes; 0 = never) on the live
	// session. Persisting it across restarts happens on "Save settings"
	// (the snapshot carries identity_lock_timeout_minutes).
	async function handleSetLockTimeout(minutes: number) {
		try {
			identityStatus = await api.setLockTimeout(minutes);
		} catch (e) {
			pushToast(
				`Failed to set lock timeout: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
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
			// even if the engine status hasn't refreshed yet. Pass it
			// through to registerNip07Signer so it doesn't re-prompt
			// the extension — important for extensions that don't
			// cache approval (e.g. soapbox-signer prompts twice).
			externalSignerPubkey = await window.nostr!.getPublicKey();
			externalSignerTeardown = await registerNip07Signer(externalSignerPubkey);
			identityStatus = await api.getIdentity();
			myPubkey = externalSignerPubkey;
			resolveIdentityName(externalSignerPubkey);
			// Persist the *intent* so the next reload auto-reconnects
			// without a Settings trip. `/identity/use` only switches the
			// in-memory session; the boot auto-reconnect keys off
			// `config.toml [identity] source`, which is written only here
			// (and by the Settings "Save"). Skip the write when it's
			// already nip07 — the boot reconnect path re-enters this fn
			// with the source already saved.
			if (savedIdentitySource !== 'nip07') {
				try {
					await api.snapshotConfig({ identity_source: 'nip07' });
					savedIdentitySource = 'nip07';
				} catch (e) {
					console.warn('[identity] failed to persist nip07 source:', e);
				}
			}
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
		get heldEntries() { return heldEntries; },
		findPoolItem,
		findPoolItemByAddr,
		findPoolItemByEventId,
		togglePoolMembership,
		togglePoolReadonly,
		dropFromPool,
		holdEvent,
		releaseHeldItem,
		dropPoolItem,
		routeHeldToContext,
		routeHeldToCompose,
		coordTokenForItem,
		pillActionByAddr,
		pillActionByEventId,
		get compose() { return compose; },
		get composeDrafts() { return composeDrafts; },
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
		get searchProfiles() { return searchProfiles; },
		get searchTagCounts() { return searchTagCounts; },
		get searchCount() { return searchCount; },
		get searchLocalCount() { return searchLocalCount; },
		get searchRelayCount() { return searchRelayCount; },
		get searchLoading() { return searchLoading; },
		get searchRelayLoading() { return searchRelayLoading; },

		// Event view modal (structured)
		get eventModalData() { return eventModalData; },
		set eventModalData(v: NostrEvent | SearchResult | null) { eventModalData = v; },

		// Legacy JSON dump modal (buffer inspector + rawEvent)
		get jsonModalData() { return jsonModalData; },
		set jsonModalData(v: { buffer: Buffer } | { rawEvent: unknown } | null) { jsonModalData = v; },

		get eventsModal() { return eventsModal; },
		set eventsModal(v: { title: string; events: EventsModalItem[] } | null) { eventsModal = v; },
		openEventsModal(title: string, events: EventsModalItem[]) { eventsModal = { title, events }; },

		// Profile
		get profilePubkey() { return profilePubkey; },
		set profilePubkey(v: string | null) { profilePubkey = v; },

		// Identity
		get myPubkey() { return myPubkey; },
		get assistantPubkey() { return assistantPubkey; },
		get dataDir() { return dataDir; },
		get localPubkeys() { return localPubkeys; },
		get identityStatus() { return identityStatus; },
		get identityLoading() { return identityLoading; },
		get identityError() { return identityError; },
		get identityDisplayName() { return identityDisplayName; },
		get savedIdentitySource() { return savedIdentitySource; },
		setSavedIdentitySource(source: string | null) { savedIdentitySource = source; },
		get identityAutoReconnecting() { return identityAutoReconnecting; },
		/** Force a fresh fetch of /api/v1/identity AND /api/v1/settings.
		 *  Called by SettingsBuffer onmount so opening Settings always
		 *  shows the live engine state, even if the engine was restarted
		 *  since the last poll tick. */
		async refreshIdentity() {
			try {
				identityStatus = await api.getIdentity();
				if (identityStatus.pubkey) {
					myPubkey = identityStatus.pubkey;
				}
			} catch {}
			try {
				const s = await api.getSettings();
				savedIdentitySource = s.identity?.source ?? null;
				if (s.network?.mode === 'auto' || s.network?.mode === 'confirm') {
					savedNetworkMode = s.network.mode;
				}
			} catch {}
		},
		get savedNetworkMode() { return savedNetworkMode; },
		set identityError(v: string | null) { identityError = v; },
		handleIdentityLogin,
		handleIdentityUnlock,
		handleIdentityLock,
		handleIdentityLogout,
		handleSetLockTimeout,
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
		handleComposePreview,
		handleComposeSaveDraft,
		handleLoadDraft,
		handleDeleteDraft,
		refreshComposeDrafts,
		handleBroadcastPublication,
		handleComposeDiffPublished,
		closePublishedDiff,
		get publishedDiff() { return publishedDiff; },
		get republishPrompt() { return republishPrompt; },
		confirmRepublish,
		cancelRepublish,
		handleComposeUpdate,
		handleSearch,
		handleSearchViaRelays,
		get searchLastQuery() { return searchLastQuery; },
		pushHistoryEntry,
		getEventForModal,
		openAddressableInModal,
		findContainingIndexes,
		get toasts() { return toasts; },
		pushToast,
		pushActivityToast,
		updateToast,
		updateActivityRelay,
		dismissToast,
		pinToast,
		expandActivityToast,
		closeActivityModal,
		get activityModalToastId() { return activityModalToastId; },
		get activityModalToast() {
			return activityModalToastId != null
				? toasts.find((t) => t.id === activityModalToastId) ?? null
				: null;
		},
		get searchHistory() { return searchHistory; },
		get currentEntry() { return currentEntry; },
		get previousEntry() { return previousEntry; },
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
		handleSetEmbedKinds,
		handleSetAutoEmbed,
		refreshEmbeddingStatus,
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
		navigateToDiscussion,
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
