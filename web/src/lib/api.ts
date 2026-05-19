import type {
	ChatResponse,
	SendMessageRequest,
	EditBufferRequest,
	SystemPromptRequest,
	InjectContextRequest,
	PublicationSummary,
	PublicationDetail,
	TocEntry,
	Section,
	SectionMeta,
	SearchResponse,
	EmbeddingStatusResponse,
	NetworkStatus,
	NetworkMode,
	DocumentFile,
	ImportResult,
	IdentityStatus
} from './types';

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
	const res = await fetch(url, {
		headers: { 'Content-Type': 'application/json' },
		...init
	});
	if (!res.ok) {
		const text = await res.text();
		throw new Error(`${res.status}: ${text}`);
	}
	return res.json();
}

// Chat API

const CHAT = '/api/v1/chat';

export function getChat(): Promise<ChatResponse> {
	return fetchJson<ChatResponse>(CHAT);
}

export function resetChat(): Promise<ChatResponse> {
	return fetchJson<ChatResponse>(CHAT, { method: 'DELETE' });
}

export function sendMessage(content: string): Promise<ChatResponse> {
	const body: SendMessageRequest = { content };
	return fetchJson<ChatResponse>(`${CHAT}/message`, { method: 'POST', body: JSON.stringify(body) });
}

export function enterEditMode(): Promise<ChatResponse> {
	return fetchJson<ChatResponse>(`${CHAT}/edit`, { method: 'POST' });
}

export function exitEditMode(buffer: string): Promise<ChatResponse> {
	const body: EditBufferRequest = { buffer };
	return fetchJson<ChatResponse>(`${CHAT}/edit`, { method: 'PUT', body: JSON.stringify(body) });
}

export function loadChatFragments(fragments: { role: string; content: string }[]): Promise<ChatResponse> {
	return fetchJson<ChatResponse>(`${CHAT}/load`, { method: 'PUT', body: JSON.stringify(fragments) });
}

export function setSystemPrompt(prompt: string): Promise<ChatResponse> {
	const body: SystemPromptRequest = { prompt };
	return fetchJson<ChatResponse>(`${CHAT}/system`, { method: 'POST', body: JSON.stringify(body) });
}

export function injectContext(notes: { title: string; content: string }[]): Promise<ChatResponse> {
	const body: InjectContextRequest = { notes };
	return fetchJson<ChatResponse>(`${CHAT}/context`, { method: 'POST', body: JSON.stringify(body) });
}

export function replaceContext(notes: { title: string; content: string }[]): Promise<ChatResponse> {
	const body: InjectContextRequest = { notes };
	return fetchJson<ChatResponse>(`${CHAT}/context`, { method: 'PUT', body: JSON.stringify(body) });
}

// Publications API

export function listPublications(limit = 20, policy = 'local_only', before?: number) {
	let url = `/api/v1/publications?limit=${limit}&policy=${policy}`;
	if (before) url += `&before=${before}`;
	return fetchJson<{ publications: PublicationSummary[]; count: number }>(url);
}

/** Fetch a publication and its table of contents.
 *
 *  `depth` controls eager expansion of nested 30040 indexes: 0 = this index
 *  and its own sections; N = recurse N levels of nesting (sections are leaves
 *  and never consume a level). The returned `toc` is a recursive tree —
 *  entries carry `depth`, `is_publication`, and (for sections in horizon)
 *  `content`. Defaults to 2: a publication plus one level of sub-publications.
 *  DB-first; misses are backfilled from relays per `policy`. */
export function getPublication(
	pubkey: string,
	d_tag: string,
	policy = 'local_first',
	depth = 2,
	signal?: AbortSignal
) {
	return fetchJson<{
		publication: PublicationDetail;
		toc: TocEntry[];
		depth: number;
		section_count: number;
	}>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}?policy=${policy}&depth=${depth}`,
		signal ? { signal } : undefined
	);
}

/** Open an SSE stream of per-node publication-load events (`PubLoadEvent`).
 *  The caller OWNS the returned EventSource and MUST call `.close()` — closing
 *  it drops the engine's channel receiver, which aborts the server-side
 *  recursive loader. Unlike `getPublication` (one batched response), this
 *  surfaces each event as it resolves, for a live per-event load counter. */
export function streamPublication(
	pubkey: string,
	d_tag: string,
	policy = 'local_first',
	depth = 2
): EventSource {
	return new EventSource(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}/stream` +
			`?policy=${policy}&depth=${depth}`
	);
}

export function loadSections(pubkey: string, d_tag: string, policy = 'local_first') {
	return fetchJson<{ sections: Section[]; loaded_count: number; total_count: number }>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}/sections?policy=${policy}`,
		{ method: 'POST' }
	);
}

export function loadSectionsMeta(pubkey: string, d_tag: string, policy = 'local_only') {
	return fetchJson<{ sections_meta: SectionMeta[]; total_count: number }>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}/sections/metadata?policy=${policy}`,
		{ method: 'POST' }
	);
}

export function getSection(pubkey: string, d_tag: string, index: number, policy = 'local_first') {
	return fetchJson<{ section: Section & { event?: unknown } }>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}/sections/${index}?policy=${policy}`
	);
}

// Events API

export function getEvent(eventId: string) {
	return fetchJson<{ event: unknown }>(`/api/v1/events/${eventId}`);
}

/** Fetch an addressable event (latest version for the kind/pubkey/d_tag
 *  triple). Used by the reader to render non-30040 addressables like
 *  NIP-23 long-form articles (30023) and NKBIP-02 wikis (30818). */
export function getAddressable(kind: number, pubkey: string, d_tag: string) {
	return fetchJson<{ event: unknown }>(`/api/v1/addressable/${kind}/${pubkey}/${encodeURIComponent(d_tag)}`);
}

export function queryEvents(filters: Record<string, unknown>[], policy = 'local_first') {
	return fetchJson<{ events: unknown[]; count: number; source: { local_count: number; relay_count: number } }>('/api/v1/query', {
		method: 'POST',
		body: JSON.stringify({ filters, policy })
	});
}

// Config API

export function getConfig() {
	return fetchJson<{ my_pubkey: string | null; assistant_pubkey: string | null }>('/api/v1/config');
}

// Identity API

export function getIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/identity');
}

export function loginIdentity(ncryptsec: string) {
	return fetchJson<IdentityStatus>('/api/v1/identity/login', {
		method: 'POST',
		body: JSON.stringify({ ncryptsec })
	});
}

export function unlockIdentity(password: string) {
	return fetchJson<IdentityStatus>('/api/v1/identity/unlock', {
		method: 'POST',
		body: JSON.stringify({ password })
	});
}

export function lockIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/identity/lock', { method: 'POST' });
}

export function logoutIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/identity/logout', { method: 'POST' });
}

// External signer / signing-source API (Phase 3 + 4 of identity plan)

export interface SignerCapabilities {
	sign_event?: boolean;
	nip04_encrypt?: boolean;
	nip04_decrypt?: boolean;
	nip44_encrypt?: boolean;
	nip44_decrypt?: boolean;
	auto_approve_kinds?: number[];
}

export interface SignerRegisterRequest {
	kind: 'nip07' | 'nip46';
	pubkey: string;
	capabilities?: SignerCapabilities;
}

export interface SignerRegisterResponse {
	signer_id: string;
	token: string;
}

export function registerSigner(req: SignerRegisterRequest) {
	return fetchJson<SignerRegisterResponse>('/api/v1/identity/signer-register', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface UseSourceRequest {
	source: 'engine' | 'nip07' | 'nip46';
	signer_id?: string;
}

export function useIdentitySource(req: UseSourceRequest) {
	return fetchJson<IdentityStatus>('/api/v1/identity/use', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface SignTemplateRequest {
	template: {
		kind: number;
		created_at: number;
		tags: string[][];
		content: string;
		pubkey?: string;
	};
}

export interface SignTemplateResponse {
	signed_event: unknown;
}

export function signTemplate(req: SignTemplateRequest) {
	return fetchJson<SignTemplateResponse>('/api/v1/identity/sign', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface SignResponseRequest {
	signer_id: string;
	req_id: string;
	signed_event?: unknown;
	error?: string;
}

export function postSignResponse(req: SignResponseRequest) {
	return fetchJson<{ resolved: boolean }>('/api/v1/identity/sign-response', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface BroadcastRequest {
	event: unknown;
	relays?: string[];
}

export interface RelayPublishResult {
	relay_url: string;
	success: boolean;
	message: string | null;
}

export interface BroadcastResponse {
	successful: number;
	total: number;
	results: RelayPublishResult[];
}

export function broadcastEvent(req: BroadcastRequest) {
	return fetchJson<BroadcastResponse>('/api/v1/broadcast', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

// Search API

export function search(
	query: string,
	limit?: number,
	my_pubkey?: string,
	policy = 'local_only',
	options: { relays?: string[]; bypassOffline?: boolean } = {}
) {
	const body: Record<string, unknown> = { query, limit, my_pubkey, policy };
	if (options.relays && options.relays.length > 0) body.relays = options.relays;
	if (options.bypassOffline) body.mode_confirm = true;
	return fetchJson<SearchResponse>('/api/v1/search', {
		method: 'POST',
		body: JSON.stringify(body)
	});
}

// Publish API

export interface PublishRequest {
	title: string;
	tags: [string, string][];
	sections: { title: string; content: string; tags: [string, string][] }[];
	sign: boolean;
	broadcast: boolean;
	relays?: string[];
}

export interface PublishResponse {
	publication_id: string;
	section_ids: string[];
	signed: boolean;
	ingested: boolean;
	broadcast_results?: { relay: string; success: boolean; message: string | null }[];
}

export function publish(req: PublishRequest) {
	return fetchJson<PublishResponse>('/api/v1/publish', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

// Block-based publish (NIP-54-style fork support).
//
// Each block is one of:
//   { kind: "editable",  content }
//   { kind: "imported",  source_addr, content, author }       — emits no 30041; TOC references source addr
//   { kind: "forked",    original_addr, content, original_author } — emits a 30041 with `fork`-marker `a`/`e` tags
//
// When source_publication_addr is set, the new 30040 also emits `fork`
// `a`/`e` tags pointing at the parent publication.

export type PublishBlock =
	| { kind: 'editable'; title: string; tags: [string, string][]; content: string }
	| {
			kind: 'imported';
			title: string;
			tags: [string, string][];
			source_addr: { kind: number; pubkey: string; d_tag: string };
			content: string;
			author: string;
	  }
	| {
			kind: 'forked';
			title: string;
			tags: [string, string][];
			original_addr: { kind: number; pubkey: string; d_tag: string };
			content: string;
			original_author: string;
	  };

export interface PublishBlocksRequest {
	title: string;
	tags: [string, string][];
	blocks: PublishBlock[];
	source_publication_addr?: { kind: number; pubkey: string; d_tag: string } | null;
	source_publication_event_id?: string | null;
	sign: boolean;
	broadcast: boolean;
	relays?: string[];
}

export function publishBlocks(req: PublishBlocksRequest) {
	return fetchJson<PublishResponse>('/api/v1/publish/blocks', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

// Document Import API

export function listDocuments() {
	return fetchJson<{ path: string; files: DocumentFile[]; count: number }>('/api/v1/documents');
}

export async function importDocument(file: File): Promise<ImportResult> {
	const formData = new FormData();
	formData.append('file', file);
	const res = await fetch('/api/v1/import', { method: 'POST', body: formData });
	if (!res.ok) throw new Error(`${res.status}: ${await res.text()}`);
	return res.json();
}

export function parseDocument(filename: string) {
	return fetchJson<ImportResult>('/api/v1/documents/parse', {
		method: 'POST',
		body: JSON.stringify({ filename })
	});
}

// Fetch API

/** Fetch from one or more relays. The engine treats the whole relay
 *  set as a single confirm operation. */
export function fetchFromRelay(
	relays: string[],
	kinds: number[],
	authors: string[] = [],
	limit = 200,
	options: { modeConfirm?: boolean; search?: string } = {}
) {
	const body: Record<string, unknown> = {
		relays,
		kinds,
		authors,
		limit,
		mode_confirm: options.modeConfirm ?? false
	};
	// NIP-50: include `search` only when set so relays that don't
	// implement the spec aren't confused by an empty-string filter.
	if (options.search && options.search.length > 0) body.search = options.search;
	return fetchJson<{ fetched: number; relays: string[]; kinds: number[] }>('/api/v1/fetch', {
		method: 'POST',
		body: JSON.stringify(body)
	});
}

export function fetchSections() {
	return fetchJson<{ total_referenced: number; missing: number; fetched: number }>('/api/v1/fetch/sections', {
		method: 'POST'
	});
}

export function fetchAuthors() {
	return fetchJson<{ fetched: number; authors: number; relays: number }>('/api/v1/fetch/authors', {
		method: 'POST'
	});
}

export function getRelayConfig() {
	return fetchJson<{
		general: { urls: string[]; kinds: number[] };
		publish: { urls: string[]; kinds: number[] };
		fetch: { urls: string[]; kinds: number[] };
		authors: string[];
	}>('/api/v1/relays');
}

// Config update API

export function addRelay(set: string, url: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ add_relay: { set, url } })
	});
}

export function addAuthor(author: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ add_author: author })
	});
}

export function removeAuthor(author: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ remove_author: author })
	});
}

// Profile API

export interface Profile {
	pubkey: string;
	name: string | null;
	display_name: string | null;
	picture: string | null;
	about: string | null;
	nip05: string | null;
	found: boolean;
}

const profileCache = new Map<string, Profile>();
const pendingProfiles = new Map<string, Promise<Profile>>();
let profileVersion = 0;
const profileListeners = new Set<() => void>();

/** Subscribe to profile cache updates (returns unsubscribe) */
export function onProfileUpdate(fn: () => void): () => void {
	profileListeners.add(fn);
	return () => profileListeners.delete(fn);
}

function notifyProfileUpdate() {
	profileVersion++;
	for (const fn of profileListeners) fn();
}

/** Synchronous read of the profile cache. Returns null when no profile
 *  is known yet — callers that want to drive UI should subscribe to
 *  `onProfileUpdate` and re-read. */
export function getCachedProfile(pubkey: string): Profile | null {
	return profileCache.get(pubkey) ?? null;
}

/** Drop entries from the web cache. Used by the refresh flow so a
 *  force-fetched profile replaces a stale one rather than being short-
 *  circuited by the in-memory cache hit. */
export function evictProfiles(pubkeys: string[]) {
	for (const pk of pubkeys) profileCache.delete(pk);
	notifyProfileUpdate();
}

/** Force-refetch the given pubkeys' kind 0 from the engine's general
 *  relays, then drop and reload the web cache so consumers see the new
 *  names. Resolves once both legs are done. */
export async function refreshProfiles(pubkeys: string[]): Promise<void> {
	const distinct = [...new Set(pubkeys.filter((pk) => pk.length === 64))];
	if (distinct.length === 0) return;
	try {
		await fetchJson<{ fetched: number; total: number }>('/api/v1/profiles/fetch', {
			method: 'POST',
			body: JSON.stringify({ pubkeys: distinct, force: true })
		});
	} catch (e) {
		console.warn('[refreshProfiles] relay fetch failed', e);
	}
	evictProfiles(distinct);
	await Promise.all(distinct.map((pk) => getProfile(pk)));
	notifyProfileUpdate();
}

export async function getProfile(pubkey: string): Promise<Profile> {
	const cached = profileCache.get(pubkey);
	if (cached) return cached;

	// Deduplicate in-flight requests for the same pubkey
	const pending = pendingProfiles.get(pubkey);
	if (pending) return pending;

	const promise = fetchJson<Profile>(`/api/v1/profile/${pubkey}`)
		.then(profile => {
			if (profile.found) {
				profileCache.set(pubkey, profile);
				notifyProfileUpdate();
			}
			pendingProfiles.delete(pubkey);
			return profile;
		})
		.catch(() => {
			pendingProfiles.delete(pubkey);
			return { pubkey, name: null, display_name: null, picture: null, about: null, nip05: null, found: false };
		});

	pendingProfiles.set(pubkey, promise);
	return promise;
}

/// Debounced profile prefetch — collects pubkeys over 300ms then fires once
let prefetchQueue: Set<string> = new Set();
let prefetchTimer: ReturnType<typeof setTimeout> | null = null;

export function prefetchProfiles(pubkeys: string[]) {
	for (const pk of pubkeys) {
		if (!profileCache.has(pk) && pk.length === 64) prefetchQueue.add(pk);
	}
	if (prefetchQueue.size === 0) return;
	if (prefetchTimer) clearTimeout(prefetchTimer);
	prefetchTimer = setTimeout(flushPrefetch, 300);
}

async function flushPrefetch() {
	const batch = [...prefetchQueue];
	prefetchQueue.clear();
	prefetchTimer = null;
	if (batch.length === 0) return;

	try {
		await fetchJson<{ fetched: number }>('/api/v1/profiles/fetch', {
			method: 'POST',
			body: JSON.stringify({ pubkeys: batch })
		});
	} catch { /* ignore */ }

	await Promise.all(batch.map(pk => getProfile(pk)));
	notifyProfileUpdate();
}

// Ignore List API

export interface IgnoreListResponse {
	ignored_event_count: number;
	ignored_pubkey_count: number;
	event_ids: string[];
	pubkeys: string[];
}

export function getIgnoreList() {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore');
}

export function ignoreEvents(event_ids: string[] = [], pubkeys: string[] = []) {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore', {
		method: 'POST',
		body: JSON.stringify({ event_ids, pubkeys })
	});
}

export function unignoreEvents(event_ids: string[] = [], pubkeys: string[] = []) {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore', {
		method: 'DELETE',
		body: JSON.stringify({ event_ids, pubkeys })
	});
}

// Embedding API

export function getEmbeddingStatus() {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/status');
}

export function syncEmbeddings() {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/sync', { method: 'POST' });
}

export function reindexEmbeddings() {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/reindex', { method: 'POST' });
}

// Claude Code Sessions API

import type { ClaudeSessionSummary, ClaudeSessionMessage } from './types';

export function listClaudeSessions() {
	return fetchJson<{ sessions: ClaudeSessionSummary[]; count: number }>('/api/v1/claude-sessions');
}

export function appendClaudeSessionMessage(id: string, content: string) {
	return fetchJson<{ uuid: string; session_id: string }>(`/api/v1/claude-sessions/${id}/message`, {
		method: 'POST',
		body: JSON.stringify({ content })
	});
}

export function getClaudeSession(id: string, offset?: number) {
	const params = offset ? `?offset=${offset}` : '';
	return fetchJson<{ id: string; messages: ClaudeSessionMessage[]; count: number; offset?: number }>(
		`/api/v1/claude-sessions/${id}${params}`
	);
}

// Network mode & activity API

// Export API

export interface ExportManifest {
	event_count: number;
	kinds: Record<string, number>;
	authors: number;
	embedding_count: number;
}

export function getExportManifest(kinds?: string) {
	const params = kinds ? `?kinds=${kinds}` : '';
	return fetchJson<ExportManifest>(`/api/v1/export/manifest${params}`);
}

export async function downloadExport(kinds?: string) {
	const params = kinds ? `?kinds=${kinds}` : '';
	const res = await fetch(`/api/v1/export${params}`);
	if (!res.ok) throw new Error(`${res.status}: ${await res.text()}`);
	const count = res.headers.get('x-event-count') || '0';
	const blob = await res.blob();
	const date = new Date().toISOString().slice(0, 10);
	const filename = `tendrl-export-${date}-${count}events.jsonl`;

	const url = URL.createObjectURL(blob);
	const a = document.createElement('a');
	a.href = url;
	a.download = filename;
	a.click();
	URL.revokeObjectURL(url);
	return { filename, count: parseInt(count) };
}

// Import API

export interface IngestResult {
	ingested: number;
	skipped: number;
	errors: number;
	duration_ms: number;
	embedding_sync: string;
}

export interface IngestProgress {
	total: number;
	sent: number;
	ingested: number;
	skipped: number;
	errors: number;
	done: boolean;
}

const CHUNK_SIZE = 200;

export async function importJsonl(
	file: File,
	onProgress?: (progress: IngestProgress) => void
): Promise<IngestResult> {
	const text = await file.text();
	const lines = text.split('\n').filter((l) => l.trim());
	const total = lines.length;
	let ingested = 0;
	let skipped = 0;
	let errors = 0;
	let sent = 0;

	for (let i = 0; i < lines.length; i += CHUNK_SIZE) {
		const chunk = lines.slice(i, i + CHUNK_SIZE).join('\n');
		const res = await fetch('/api/v1/ingest', {
			method: 'POST',
			headers: { 'Content-Type': 'application/x-ndjson' },
			body: chunk
		});
		if (!res.ok) throw new Error(`${res.status}: ${await res.text()}`);
		const r: IngestResult = await res.json();
		ingested += r.ingested;
		skipped += r.skipped;
		errors += r.errors;
		sent = Math.min(i + CHUNK_SIZE, lines.length);
		onProgress?.({ total, sent, ingested, skipped, errors, done: false });
	}

	onProgress?.({ total, sent: total, ingested, skipped, errors, done: true });
	return { ingested, skipped, errors, duration_ms: 0, embedding_sync: 'started' };
}

// Network mode & activity API

export function getNetworkStatus() {
	return fetchJson<NetworkStatus>('/api/v1/network/status');
}

export function setNetworkMode(mode: NetworkMode) {
	return fetchJson<NetworkStatus>('/api/v1/network/mode', {
		method: 'POST',
		body: JSON.stringify({ mode })
	});
}

/** Reply to a confirm Intent — approve or decline a pending fetch
 *  operation, optionally overriding the relay set. */
export function confirmFetch(operationId: string, approved: boolean, relays?: string[]) {
	return fetchJson<{ resolved: boolean }>('/api/v1/network/fetch-confirm', {
		method: 'POST',
		body: JSON.stringify({ operation_id: operationId, approved, relays })
	});
}

// Discussion counts: NIP-22 comments (kind 1111) and NIP-84 highlights
// (kind 9802) referencing the given addressable events via their `a` tag.

export interface DiscussionCount {
	comments: number;
	highlights: number;
}

export interface DiscussionCountsResponse {
	counts: Record<string, DiscussionCount>;
	source: { local_count: number; relay_count: number };
}

export function getDiscussionCounts(
	addresses: string[],
	policy: 'local_only' | 'local_first' | 'fetch_always' = 'local_first',
	options: { bypassOffline?: boolean } = {}
) {
	return fetchJson<DiscussionCountsResponse>('/api/v1/discussions/counts', {
		method: 'POST',
		body: JSON.stringify({
			addresses,
			policy,
			mode_confirm: options.bypassOffline ?? false
		})
	});
}

// Discussions list: full event payloads (not just counts) for the same
// shape of query as discussions/counts. Used by DiscussionsListBuffer
// and the inline section-disclosure to render comments + highlights.

export interface DiscussionEvent {
	id: string;
	kind: number;
	pubkey: string;
	created_at: number;
	content: string;
	tags: string[][];
	sig?: string;
}

export interface DiscussionsListResponse {
	events: DiscussionEvent[];
	source: { local_count: number; relay_count: number };
	/** Unix seconds at which the engine computed the result. Pass back
	 *  as `since` on a subsequent call for incremental refresh. */
	refreshed_at: number;
}

export function getDiscussionList(
	options: {
		addresses?: string[];
		eventIds?: string[];
		kinds?: number[];
		policy?: 'local_only' | 'local_first' | 'fetch_always';
		limit?: number;
		since?: number;
		bypassOffline?: boolean;
		relays?: string[];
	} = {}
) {
	// POST, not GET: a deep publication tree references hundreds of
	// section coordinates — packing them into the URL overflows the
	// server's request-line/header limit (HTTP 431). They travel in the
	// body instead. Field names are snake_case to match the Rust struct.
	return fetchJson<DiscussionsListResponse>('/api/v1/discussions/list', {
		method: 'POST',
		body: JSON.stringify({
			addresses: options.addresses ?? [],
			event_ids: options.eventIds ?? [],
			kinds: options.kinds ?? [],
			policy: options.policy,
			limit: options.limit,
			since: options.since,
			mode_confirm: options.bypassOffline ?? false,
			relays: options.relays ?? []
		})
	});
}
