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
	SearchResponse,
	EmbeddingStatusResponse,
	NetworkStatus,
	NetworkMode,
	DocumentFile,
	ImportResult,
	IdentityStatus,
	RepublishDiff,
	HealthResponse
} from './types';
import type { ThreadNode } from './discussions/thread';
import type { Highlight, HighlightSpan } from './discussions/highlights';
import type { ResolvedRef } from './nostr/nostrdown';

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

/** A tool in the AI catalog with its live enablement. */
export interface AiToolInfo {
	name: string;
	description: string;
	category: string;
	enabled: boolean;
}

/** Current AI assistant settings (provider/model + tool catalog). */
export interface AiSettings {
	provider: string;
	model: string;
	max_tool_turns: number;
	tools: AiToolInfo[];
}

/** Partial AI settings update; all fields optional. */
export interface AiSettingsUpdate {
	enabled_tools?: string[];
	provider?: string;
	model?: string;
}

export function getAiSettings(): Promise<AiSettings> {
	return fetchJson<AiSettings>('/api/v1/ai/settings');
}

export function saveAiSettings(update: AiSettingsUpdate): Promise<AiSettings> {
	return fetchJson<AiSettings>('/api/v1/ai/settings', {
		method: 'POST',
		body: JSON.stringify(update)
	});
}

/** Summary of a saved tendrl chat session (under <data_dir>/sessions/). */
export interface SavedSessionSummary {
	id: string;
	title: string;
	created_at: number;
	modified_at: number;
	message_count: number;
}

export function saveChatSession(
	title?: string,
	id?: string | null
): Promise<{ id: string; title: string }> {
	return fetchJson('/api/v1/chat/sessions', {
		method: 'POST',
		body: JSON.stringify({ title: title ?? null, id: id ?? null })
	});
}

export function listChatSessions(): Promise<{ sessions: SavedSessionSummary[]; count: number }> {
	return fetchJson('/api/v1/chat/sessions');
}

export function loadChatSession(id: string): Promise<ChatResponse> {
	return fetchJson(`/api/v1/chat/sessions/${encodeURIComponent(id)}`);
}

export function deleteChatSession(id: string): Promise<{ deleted: boolean }> {
	return fetchJson(`/api/v1/chat/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
}

/** The editable Markdown system prompt prepended to every agent turn. */
export function getAiPrompt(): Promise<{ content: string; path: string }> {
	return fetchJson('/api/v1/ai/prompt');
}

export function saveAiPrompt(content: string): Promise<{ saved: boolean; path: string }> {
	return fetchJson('/api/v1/ai/prompt', {
		method: 'PUT',
		body: JSON.stringify({ content })
	});
}

/** One event from the agent SSE stream: `{ type, data }`. */
export interface AgentEvent {
	type: 'text' | 'thinking' | 'tool_call' | 'tool_result' | 'done' | 'error';
	data: Record<string, unknown>;
}

/**
 * POST a user turn to the tool-calling agent loop and stream the transcript.
 * The response body is an SSE stream (request-scoped — not the global
 * fetch-events channel); each `data:` frame is one {@link AgentEvent}.
 * `onEvent` is invoked per event; the promise resolves when the stream ends.
 */
export async function streamAgentTurn(
	content: string,
	onEvent: (ev: AgentEvent) => void
): Promise<void> {
	const res = await fetch(`${CHAT}/agent`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify({ content })
	});
	if (!res.ok || !res.body) {
		const text = await res.text().catch(() => '');
		throw new Error(`${res.status}: ${text}`);
	}
	const reader = res.body.getReader();
	const decoder = new TextDecoder();
	let buf = '';
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		buf += decoder.decode(value, { stream: true });
		let sep: number;
		// SSE frames are separated by a blank line.
		while ((sep = buf.indexOf('\n\n')) !== -1) {
			const frame = buf.slice(0, sep);
			buf = buf.slice(sep + 2);
			for (const line of frame.split('\n')) {
				const t = line.trim();
				if (!t.startsWith('data:')) continue;
				const payload = t.slice(5).trim();
				if (!payload) continue;
				try {
					onEvent(JSON.parse(payload) as AgentEvent);
				} catch (e) {
					console.error('[agent] bad SSE frame', e, payload);
				}
			}
		}
	}
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

export function replaceContext(notes: { title: string; content: string }[]): Promise<ChatResponse> {
	const body: InjectContextRequest = { notes };
	return fetchJson<ChatResponse>(`${CHAT}/context`, { method: 'PUT', body: JSON.stringify(body) });
}

// NIP-19 encode
//
// Bech32 NIP-19 derivation lives in the Rust engine (the inverse of
// `/decode`), so every frontend gets identical, spec-correct output without
// shipping its own bech32 implementation. `kind`-tagged to mirror the
// `Decoded` shape the decode endpoint returns.

export type EncodeRequest =
	| { kind: 'npub'; pubkey: string }
	| { kind: 'note'; event_id: string }
	| {
			kind: 'nevent';
			event_id: string;
			relays?: string[];
			author?: string;
			kind_int?: number;
	  }
	| { kind: 'naddr'; kind_int: number; pubkey: string; d_tag: string; relays?: string[] }
	| { kind: 'atag'; a_tag: string; relays?: string[] };

/** Encode structured fields into a NIP-19 bech32 identifier via the engine. */
export async function encode(req: EncodeRequest): Promise<string> {
	const resp = await fetchJson<{ encoded: string }>('/api/v1/encode', {
		method: 'POST',
		body: JSON.stringify(req)
	});
	return resp.encoded;
}

/**
 * Detect that a same-title publication of the user's already exists and return
 * a section-level diff (matched / added / removed by title slug) so a republish
 * can reuse identifiers instead of forking. The slug-matching, TOC flatten, and
 * diff all run in the engine. Returns `null` when there's no existing match
 * (the normal first publish) or no signed-in identity.
 */
export function republishDiff(
	title: string,
	sections: { title: string; content: string }[]
): Promise<RepublishDiff | null> {
	return fetchJson<RepublishDiff | null>('/api/v1/publish/republish-diff', {
		method: 'POST',
		body: JSON.stringify({ title, sections })
	});
}

/**
 * Resolve NIP-84 highlight positions within section text, engine-side — the
 * replacement for the former client-side `computeHighlightSegments`. Batched:
 * pass every visible section (keyed by addr) in one round trip; the response
 * maps each key to its non-overlapping `HighlightSpan[]` (UTF-16 offsets). The
 * caller renders `<mark>`s via `segmentsFromSpans`.
 */
export async function resolveHighlights(
	items: { key: string; content: string; highlights: Highlight[] }[]
): Promise<Record<string, HighlightSpan[]>> {
	if (items.length === 0) return {};
	const resp = await fetchJson<{ spans: Record<string, HighlightSpan[]> }>(
		'/api/v1/highlights/resolve',
		{ method: 'POST', body: JSON.stringify({ items }) }
	);
	return resp.spans;
}

/**
 * Resolve nostrdown `{{ref|wiki|embed:…}}` references within section text,
 * engine-side — parsing + target lookup so every frontend renders identical
 * links and transclusions (the nostrdown analogue of `resolveHighlights`).
 * Batched: pass every visible section (keyed by addr) plus its context — the
 * containing publication coordinate (`"30040:pubkey:dtag"`) for sibling `ref:`
 * resolution and the section author for `wiki:` scoping. The response maps each
 * key to its `ResolvedRef[]` (UTF-16 offsets); the caller renders via
 * `buildSegments`.
 */
export async function resolveNostrdown(
	items: {
		key: string;
		content: string;
		publication?: string;
		author?: string;
		/** Sibling sections of an unsigned draft (title + synthetic d-tag) so
		 *  `{{ref:slug}}` resolves in the composer's draft-reader preview before
		 *  anything is published. Omit for published reads. */
		siblings?: { title?: string; d_tag: string }[];
	}[]
): Promise<Record<string, ResolvedRef[]>> {
	if (items.length === 0) return {};
	const resp = await fetchJson<{ refs: Record<string, ResolvedRef[]> }>(
		'/api/v1/nostrdown/resolve',
		{ method: 'POST', body: JSON.stringify({ items }) }
	);
	return resp.refs;
}

/** Force-fetch one nostrdown `embed` entity (naddr/nevent/note) from the search
 *  relays and return its (re)resolved ref. Drives the EmbedCard's "fetch from
 *  search relays" action: `FetchAlways`, so in Confirm mode the engine raises a
 *  network intent the FetchConfirmModal must approve (this call blocks until
 *  then). On success `pending` clears and the card fills with the event. */
export async function fetchNostrdownEntity(
	entity: string,
	wantContent = true
): Promise<ResolvedRef> {
	const resp = await fetchJson<{ ref: ResolvedRef }>('/api/v1/nostrdown/fetch-entity', {
		method: 'POST',
		body: JSON.stringify({ entity, want_content: wantContent })
	});
	return resp.ref;
}

// Drafts API — local unsigned-publication storage (engine DraftStore).
// Persists the full compose state to <data_dir>/drafts/ so a draft survives a
// refresh, can be listed, and resumed. A draft is never signed.

export interface DraftSummary {
	draft_id: string;
	title: string;
	/** Publication identity — saves of one publication share it, so the web
	 *  groups them into a version list. */
	d_tag: string;
	created_at: number;
	modified_at: number;
	section_count: number;
}

// Version diff between two draft snapshots (engine diff_draft_versions).

export interface FieldChange {
	old: string;
	new: string;
}
export interface TagDiff {
	added?: [string, string][];
	removed?: [string, string][];
}
export interface SectionVersionDiff {
	title: string;
	/** Title slug — the match key. */
	t: string;
	status: 'matched' | 'added' | 'removed';
	level: number;
	contentChanged?: boolean;
	levelChanged?: boolean;
	tags?: TagDiff;
}
export interface VersionDiff {
	titleChanged?: FieldChange;
	indexTags?: TagDiff;
	sections: SectionVersionDiff[];
}

/** Diff two draft snapshots (from_id → to_id), engine-side. */
export function draftDiff(fromId: string, toId: string): Promise<VersionDiff> {
	return fetchJson<VersionDiff>('/api/v1/drafts/diff', {
		method: 'POST',
		body: JSON.stringify({ from_id: fromId, to_id: toId })
	});
}

/**
 * Diff the current compose (the live working state) against the last *published*
 * (signed) version of that article. `published:false` when nothing's been
 * published yet. Diff direction is published → current.
 */
export function diffVsPublished(payload: SaveDraftPayload): Promise<{
	published: boolean;
	diff?: VersionDiff;
	existingAddr?: { kind: number; pubkey: string; d_tag: string };
}> {
	return fetchJson('/api/v1/publish/diff', {
		method: 'POST',
		body: JSON.stringify(payload)
	});
}

export interface DraftComposeSection {
	title: string;
	content: string;
	level: number;
	d_tag?: string;
	tags: { name: string; value: string }[];
	/** Transclude slot target (naddr/coordinate) — set when this item is a slot,
	 *  restored on resume so the `{{slot:…}}` line comes back. */
	slot?: string;
}

export interface DraftComposeState {
	title: string;
	d_tag?: string;
	/** Output kind — absent/30040 = publication; any other kind = atomic draft
	 *  (blog/wiki/custom) so resume reopens the composer in the right mode. */
	kind?: number;
	tags: { name: string; value: string }[];
	sections: DraftComposeSection[];
}

export interface DraftPublication {
	draft_id: string;
	title: string;
	created_at: number;
	modified_at: number;
	index_event: unknown;
	section_events: unknown[];
	compose_state: DraftComposeState;
}

export interface SaveDraftPayload {
	title: string;
	tags: [string, string][];
	sections: {
		title: string;
		content: string;
		level?: number;
		tags: [string, string][];
		d_tag?: string;
		/** Transclude slot target (naddr/coordinate to a 30040/30041). */
		slot?: string;
	}[];
	d_tag?: string;
	/** Output kind — absent/30040 = publication; other kinds mark an atomic
	 *  draft so resume reopens in the right mode. */
	kind?: number;
}

/** Save (or snapshot) a draft from compose state. Returns its draft_id and the
 *  publication d_tag — thread the latter onto later saves to version, not fork. */
export function saveDraft(
	payload: SaveDraftPayload
): Promise<{ draft_id: string; d_tag: string }> {
	return fetchJson<{ draft_id: string; d_tag: string }>('/api/v1/drafts', {
		method: 'POST',
		body: JSON.stringify(payload)
	});
}

/**
 * Broadcast an already-signed publication (its 30040 index + signed 30041
 * sections) to the publish relays in one operation — the separate step after
 * signing a local snapshot. No re-signing. Clears the "local" pill on success.
 */
export function broadcastPublication(
	pubkey: string,
	dTag: string,
	relays?: string[]
): Promise<{
	successful: number;
	total: number;
	event_count: number;
	broadcast_results: { relay: string; success: boolean; message: string | null; event_id: string }[];
}> {
	return fetchJson(`/api/v1/publications/${pubkey}/${encodeURIComponent(dTag)}/broadcast`, {
		method: 'POST',
		body: JSON.stringify({ relays: relays ?? null })
	});
}

/** List draft summaries, newest first. */
export function listDrafts(): Promise<{ drafts: DraftSummary[]; count: number }> {
	return fetchJson<{ drafts: DraftSummary[]; count: number }>('/api/v1/drafts');
}

/** Load a full draft (incl. compose state) for resuming. */
export function loadDraft(id: string): Promise<DraftPublication> {
	return fetchJson<DraftPublication>(`/api/v1/drafts/${encodeURIComponent(id)}`);
}

export function deleteDraft(id: string): Promise<{ deleted: string }> {
	return fetchJson<{ deleted: string }>(`/api/v1/drafts/${encodeURIComponent(id)}`, {
		method: 'DELETE'
	});
}

// Publications API

export function listPublications(
	limit = 20,
	policy = 'local_only',
	before?: number,
	general = false
) {
	let url = `/api/v1/publications?limit=${limit}&policy=${policy}`;
	if (before) url += `&before=${before}`;
	if (general) url += `&general=true`;
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
export function getAddressable(
	kind: number,
	pubkey: string,
	d_tag: string,
	policy?: 'local_only' | 'local_first' | 'fetch_always'
) {
	const qs = policy ? `?policy=${policy}` : '';
	return fetchJson<{ event: unknown }>(
		`/api/v1/addressable/${kind}/${pubkey}/${encodeURIComponent(d_tag)}${qs}`
	);
}

export function queryEvents(filters: Record<string, unknown>[], policy = 'local_first') {
	return fetchJson<{ events: unknown[]; count: number; source: { local_count: number; relay_count: number } }>('/api/v1/query', {
		method: 'POST',
		body: JSON.stringify({ filters, policy })
	});
}

// Config API

export function getConfig() {
	return fetchJson<{
		data_dir: string;
	}>('/api/v1/config');
}

/** Engine liveness + the running build's version (env!("CARGO_PKG_VERSION")). */
export function getHealth(): Promise<HealthResponse> {
	return fetchJson<HealthResponse>('/health');
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

/** Set the engine auto-lock timeout on the live session. `0` = never.
 *  Applies immediately; persisting across restarts is a separate
 *  snapshotConfig({ identity_lock_timeout_minutes }) call. */
export function setLockTimeout(minutes: number) {
	return fetchJson<IdentityStatus>('/api/v1/identity/lock-timeout', {
		method: 'POST',
		body: JSON.stringify({ minutes })
	});
}

export function logoutIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/identity/logout', { method: 'POST' });
}

// Assistant identity API — a second identity established by pasting a key
// (nsec or ncryptsec), persisted in the OS keyring (never config). Drives
// `by:assistant` / feed scoping.

export function getAssistantIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/assistant-identity');
}

/** Establish the assistant identity. `key` is an `nsec1…` (plaintext → live
 *  signer immediately) or `ncryptsec1…` (encrypted → locked, needs unlock). */
export function loginAssistantIdentity(key: string) {
	return fetchJson<IdentityStatus>('/api/v1/assistant-identity/login', {
		method: 'POST',
		body: JSON.stringify({ key })
	});
}

export function unlockAssistantIdentity(password: string) {
	return fetchJson<IdentityStatus>('/api/v1/assistant-identity/unlock', {
		method: 'POST',
		body: JSON.stringify({ password })
	});
}

export function logoutAssistantIdentity() {
	return fetchJson<IdentityStatus>('/api/v1/assistant-identity/logout', { method: 'POST' });
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
	/** External signer pubkey (hex). Pass when source is nip07/nip46
	 *  so /identity status surfaces a non-null pubkey. */
	pubkey?: string;
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
	sections: {
		title: string;
		content: string;
		tags: [string, string][];
		level?: number;
		/** Reuse this section d-tag (republish replace) instead of minting. */
		d_tag?: string;
		/** Transclude *slot*: an naddr or kind:pubkey:d-tag (a 30040/30041) to
		 *  reference as a child of the index here, instead of authoring content.
		 *  The engine emits an ["a", coord] in the 30040 and mints no 30041. */
		slot?: string;
	}[];
	sign: boolean;
	broadcast: boolean;
	relays?: string[];
	/** Reuse this index d-tag (republish replace) instead of minting. */
	d_tag?: string;
	/** Emit a single atomic event of this kind (NIP-23 30023, NIP-54 30818, or
	 *  any custom replaceable kind) instead of the 30040/30041 graph. Absent or
	 *  30040 keeps the section-graph path; `content` carries the body. */
	kind?: number;
	content?: string;
	/** Notes mode: publish each detected section as a standalone 30041 with no
	 *  30040 index over them. Within the publication path; sections unchanged. */
	notes?: boolean;
}

export interface PublishResponse {
	publication_id: string;
	section_ids: string[];
	signed: boolean;
	ingested: boolean;
	broadcast_results?: {
		relay: string;
		success: boolean;
		message: string | null;
		event_id: string;
	}[];
	/** Full event JSON (index first, then sections) for the inspector. */
	events?: unknown[];
}

export function publish(req: PublishRequest) {
	return fetchJson<PublishResponse>('/api/v1/publish', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

/** Build the unsigned 30040/30041 event graph for a compose without
 *  signing/ingesting/broadcasting — feeds the compose JSON preview. */
export function previewPublication(req: PublishRequest) {
	return fetchJson<{ events: unknown[] }>('/api/v1/publish/preview', {
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
	| {
			kind: 'editable';
			title: string;
			tags: [string, string][];
			content: string;
			/** Reuse this 30041 d-tag (nanoid) on republish; omit to mint fresh. */
			d_tag?: string;
	  }
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
			/** Reuse this 30041 d-tag (nanoid) on republish; omit to mint fresh. */
			d_tag?: string;
	  };

export interface PublishBlocksRequest {
	title: string;
	tags: [string, string][];
	/** Reuse this index d-tag (nanoid) on republish; omit to mint fresh. */
	d_tag?: string;
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

/** One annotated entry from the blocks preview: the would-be event plus its
 *  provenance. `linked` entries carry the exact original event the 30040
 *  will reference (null when it isn't cached locally). */
export interface PreviewEventEntry {
	status: 'new' | 'forked' | 'linked';
	title: string;
	event: unknown | null;
	original?: {
		addr: string;
		kind: number;
		pubkey: string;
		author_name: string | null;
		found?: boolean;
	};
}

/** Build the unsigned event graph for a block-based compose — mirrors what
 *  /publish/blocks will emit (fork markers, linked originals) without
 *  signing/ingesting/broadcasting. */
export function previewPublicationBlocks(req: PublishBlocksRequest) {
	return fetchJson<{ events: PreviewEventEntry[] }>('/api/v1/publish/blocks/preview', {
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

/** Batch-fetch a publication's missing 30041 sections + nested 30040
 *  indexes from relays in ONE op (one confirm modal in confirm mode
 *  instead of one per section). `depth` controls how many tree levels
 *  to walk when collecting missing children. */
export function backfillPublication(pubkey: string, d_tag: string, depth?: number) {
	const qs = depth != null ? `?depth=${depth}` : '';
	return fetchJson<{ requested: number; fetched: number; depth: number }>(
		`/api/v1/publications/${pubkey}/${encodeURIComponent(d_tag)}/backfill${qs}`,
		{ method: 'POST' }
	);
}

/** Pull a user's relay-list events (kinds 10002 / 10007 / 10086 /
 *  10088 / 30002) through the engine's indexer composition — read
 *  relays first, falling through to indexer.default → indexer.fallback
 *  if the primary returns zero. Honors NetworkMode::Confirm via the
 *  activity-event modal. The web reads the events back from local
 *  nostrdb via api.search after this resolves. */
export function pullUserData(pubkey: string, modeConfirm = true) {
	return fetchJson<{ fetched: number; kinds: number[]; author: string }>(
		'/api/v1/pull-user-data',
		{
			method: 'POST',
			body: JSON.stringify({ pubkey, mode_confirm: modeConfirm })
		}
	);
}

export function fetchAuthors() {
	return fetchJson<{ fetched: number; authors: number; relays: number }>('/api/v1/fetch/authors', {
		method: 'POST'
	});
}

export interface NamedRelaySet {
	d_tag: string;
	title: string;
	urls: string[];
}

/** Two-tier membership for a discovery class — `default` joins the
 *  primary fan-out (or replaces read with `exclusive`), `fallback`
 *  kicks in only on default-miss. */
export interface DiscoveryClass {
	default: string[];
	fallback: string[];
}

export function getRelayConfig() {
	return fetchJson<{
		general: { urls: string[]; kinds: number[] };
		publish: { urls: string[]; kinds: number[] };
		fetch: { urls: string[]; kinds: number[] };
		broadcast: { urls: string[]; kinds: number[] };
		search: DiscoveryClass;
		indexer: DiscoveryClass;
		exclusive: { search: boolean; indexer: boolean };
		named_sets: NamedRelaySet[];
		authors: string[];
		initial_relays: string[];
	}>('/api/v1/relays');
}

export function createNamedSet(d_tag: string, title: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ create_named_set: { d_tag, title } })
	});
}

export function deleteNamedSet(d_tag: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ delete_named_set: d_tag })
	});
}

export function renameNamedSet(d_tag: string, title: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ rename_named_set: { d_tag, title } })
	});
}

export function addToNamedSet(d_tag: string, url: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ add_to_named_set: { d_tag, url } })
	});
}

export function removeFromNamedSet(d_tag: string, url: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ remove_from_named_set: { d_tag, url } })
	});
}

// Config update API

export function addRelay(set: string, url: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ add_relay: { set, url } })
	});
}

export function removeRelay(set: string, url: string) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ remove_relay: { set, url } })
	});
}

/** What publishing a relay-list event would overwrite (replaceable
 *  events replace wholesale). `null` = no current event, nothing to
 *  overwrite. Computed engine-side (src/relay_diff.rs). */
export interface RelayListDiff {
	kind: number;
	current_event_id: string | null;
	current_created_at: number | null;
	added: string[];
	removed: string[];
	changed: { url: string; current: string; proposed: string }[];
	unchanged: number;
	dropped_tags: string[][];
	drops_content: boolean;
	current_opaque: boolean;
}

export function relayListPublishDiff(req: {
	kind: number;
	d_tag?: string;
	proposed_tags?: string[][];
	proposed_urls?: string[];
	current_urls?: string[];
}) {
	return fetchJson<RelayListDiff | null>('/api/v1/relays/publish-diff', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

/** Toggle the `exclusive` flag for a discovery class. ON = read relays
 *  bypassed entirely for this class's lookup type. */
export function setDiscoveryExclusive(klass: 'search' | 'indexer', value: boolean) {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ set_exclusive: { class: klass, value } })
	});
}

/** Merge the engine's well-known indexer/search defaults into the
 *  current `default` tiers. Idempotent — already-present URLs skip. */
export function restoreDiscoveryDefaults() {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ restore_discovery_defaults: true })
	});
}

export interface SnapshotPayload {
	include_relays?: boolean;
	editor?: { line_numbers: boolean; vim_mode: boolean; insert_mode: string };
	compose?: { default_mode: string; sync_mode: string; button_labels: string };
	network_mode?: string;
	identity_source?: string;
	identity_lock_timeout_minutes?: number;
}

/** Snapshot live state into config.toml. Pass nothing to default to
 *  relays-only; pass any combination of editor/compose/network_mode to
 *  also persist those settings. relays.json + in-memory state stay
 *  authoritative at runtime — this is just for portability / restart
 *  defaults. */
export function snapshotConfig(payload?: SnapshotPayload) {
	const init: RequestInit = { method: 'POST' };
	if (payload) {
		init.body = JSON.stringify(payload);
		init.headers = { 'Content-Type': 'application/json' };
	}
	return fetchJson<{
		updated: boolean;
		wrote?: string[];
		relay_count?: number;
		path?: string;
		message: string;
	}>('/api/v1/config/snapshot', init);
}

/** Fetch editor/compose/network defaults from config.toml so the web can
 *  hydrate the SettingsBuffer with the user's last-saved choices instead
 *  of hard-coded defaults. */
export function getSettings() {
	return fetchJson<{
		editor: { line_numbers: boolean; vim_mode: boolean; insert_mode: string };
		compose: { default_mode: string; sync_mode: string; button_labels: string };
		network: { mode: string };
		identity: { source: string | null; lock_timeout_minutes?: number };
	}>('/api/v1/settings');
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
	ignored_coordinate_count: number;
	event_ids: string[];
	pubkeys: string[];
	/** Addressable coordinates (`kind:pubkey:d_tag`) — hidden publications,
	 *  across every version. */
	coordinates: string[];
}

export function getIgnoreList() {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore');
}

export function ignoreEvents(
	event_ids: string[] = [],
	pubkeys: string[] = [],
	coordinates: string[] = []
) {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore', {
		method: 'POST',
		body: JSON.stringify({ event_ids, pubkeys, coordinates })
	});
}

export function unignoreEvents(
	event_ids: string[] = [],
	pubkeys: string[] = [],
	coordinates: string[] = []
) {
	return fetchJson<IgnoreListResponse>('/api/v1/ignore', {
		method: 'DELETE',
		body: JSON.stringify({ event_ids, pubkeys, coordinates })
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

/** Set which event kinds are eligible for embedding. Persists engine-side and
 *  returns the refreshed status. */
export function setEmbedKinds(kinds: number[]) {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/config', {
		method: 'POST',
		body: JSON.stringify({ kinds })
	});
}

/** Toggle auto-embed on retrieval + publishing. Persists engine-side and
 *  returns the refreshed status. */
export function setAutoEmbed(auto_embed: boolean) {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/config', {
		method: 'POST',
		body: JSON.stringify({ auto_embed })
	});
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

/** Re-arm the first-run network-mode choice modal: clears the engine's
 *  `mode_chosen` flag so the one-time picker shows again on next load. */
export function resetNetworkModeChoice() {
	return fetchJson<{ mode_chosen: boolean }>('/api/v1/network/reset-mode-choice', {
		method: 'POST'
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

// NIP-22 comment (kind 1111) / NIP-84 highlight (kind 9802) tallies per
// addressable event, computed engine-side. Drives the reader's discussion
// badges. Delivered as the `counts` field on the discussions/list response
// (the engine reduces the same event set it returns), so the reader never
// re-derives these from the events client-side.

export interface DiscussionCount {
	comments: number;
	highlights: number;
}

// Discussions list: full event payloads plus engine-computed `counts`. Used
// by DiscussionsListBuffer and the inline section-disclosure to render
// comments + highlights, and by the reader outline for the badge counts.

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
	/** Per-address NIP-22/84 tallies, keyed by `kind:pubkey:d-tag`. Computed
	 *  by the engine over the same event set; the reader reads these straight
	 *  into its badges instead of re-counting. Empty for event-id-only queries. */
	counts: Record<string, DiscussionCount>;
	source: { local_count: number; relay_count: number };
	/** Unix seconds at which the engine computed the result. Pass back
	 *  as `since` on a subsequent call for incremental refresh. */
	refreshed_at: number;
	/** NIP-22 thread forest grouped by referenced address (kind-1111 only),
	 *  built engine-side when `threaded` was set on an address query. The
	 *  reader renders these directly instead of threading the events itself.
	 *  Absent when `threaded` was not requested or the query was event-id-only. */
	threads_by_address?: Record<string, ThreadNode[]>;
	/** Flat NIP-22 thread forest, built engine-side when `threaded` was set on
	 *  an *event-id* query (no address to group by). Absent otherwise. */
	threads?: ThreadNode[];
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
		/** Ask the engine to thread the kind-1111 comments and return the
		 *  forest (`threads_by_address` for address queries, `threads` for
		 *  event-id queries) instead of the caller threading client-side. */
		threaded?: boolean;
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
			relays: options.relays ?? [],
			threaded: options.threaded ?? false
		})
	});
}
