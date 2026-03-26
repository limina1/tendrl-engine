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
	EmbeddingStatusResponse
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

// Config API

export function getConfig() {
	return fetchJson<{ my_pubkey: string | null }>('/api/v1/config');
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

// Fetch API

export function fetchFromRelay(relay: string, kinds: number[], authors: string[] = [], limit = 200) {
	return fetchJson<{ fetched: number; relay: string; kinds: number[] }>('/api/v1/fetch', {
		method: 'POST',
		body: JSON.stringify({ relay, kinds, authors, limit })
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

export async function getProfile(pubkey: string): Promise<Profile> {
	const cached = profileCache.get(pubkey);
	if (cached) return cached;
	const profile = await fetchJson<Profile>(`/api/v1/profile/${pubkey}`);
	if (profile.found) profileCache.set(pubkey, profile);
	return profile;
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
