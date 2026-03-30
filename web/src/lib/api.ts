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
	ImportResult
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

export function getPublication(pubkey: string, d_tag: string, policy = 'local_first') {
	return fetchJson<{ publication: PublicationDetail; toc: TocEntry[]; section_count: number }>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}?policy=${policy}`
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

// Search API

export function search(query: string, limit?: number, my_pubkey?: string, policy = 'local_only') {
	return fetchJson<SearchResponse>('/api/v1/search', {
		method: 'POST',
		body: JSON.stringify({ query, limit, my_pubkey, policy })
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

export function fetchFromRelay(relay: string, kinds: number[], authors: string[] = [], limit = 200) {
	return fetchJson<{ fetched: number; relay: string; kinds: number[] }>('/api/v1/fetch', {
		method: 'POST',
		body: JSON.stringify({ relay, kinds, authors, limit })
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

/// Batch-prefetch profiles: first fetch missing from relays, then populate cache
export async function prefetchProfiles(pubkeys: string[]) {
	const unique = [...new Set(pubkeys)].filter(pk => !profileCache.has(pk) && pk.length === 64);
	if (unique.length === 0) return;

	// Ask backend to fetch missing profiles from general relays (ingests into nostrdb)
	try {
		await fetchJson<{ fetched: number }>('/api/v1/profiles/fetch', {
			method: 'POST',
			body: JSON.stringify({ pubkeys: unique })
		});
	} catch { /* ignore */ }

	// Now populate cache from local (all should be in nostrdb now)
	await Promise.all(unique.map(pk => getProfile(pk)));
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
