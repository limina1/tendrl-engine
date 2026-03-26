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
	SearchResponse
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
