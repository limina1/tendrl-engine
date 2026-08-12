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
	HealthResponse,
	NostrEvent
} from './types';
import type { ThreadNode } from './discussions/thread';
import type { Highlight, HighlightSpan } from './discussions/highlights';
import type { ResolvedRef, ParsedToken } from './nostr/nostrdown';

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

/** Human-readable message from a fetchJson error: unwraps the engine's
 *  `{"error":{"message":…}}` body out of the `<status>: <body>` string. */
export function errorMessage(e: unknown, fallback = 'Request failed'): string {
	const raw = e instanceof Error ? e.message : String(e);
	const body = raw.replace(/^\d{3}:\s*/, '');
	try {
		const parsed = JSON.parse(body);
		if (typeof parsed?.error?.message === 'string') return parsed.error.message;
	} catch {
		// not JSON — fall through
	}
	return body || fallback;
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
	| { kind: 'nprofile'; pubkey: string; relays?: string[] }
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

/** The tagged shape `POST /api/v1/decode` returns — mirrors `nip19::Decoded`. */
export type Decoded =
	| { kind: 'npub'; pubkey: string }
	| { kind: 'note'; event_id: string }
	| { kind: 'nprofile'; pubkey: string; relays: string[] }
	| {
			kind: 'nevent';
			event_id: string;
			relays: string[];
			author: string | null;
			kind_int: number | null;
	  }
	| { kind: 'naddr'; kind_int: number; pubkey: string; d_tag: string; relays: string[] };

/** Decode a NIP-19 bech32 identifier (optionally `nostr:`-prefixed) via the
 *  engine — the inverse of `encode`; the web ships no bech32 of its own. */
export function decode(input: string): Promise<Decoded> {
	return fetchJson<Decoded>('/api/v1/decode', {
		method: 'POST',
		body: JSON.stringify({ input })
	});
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
	// Session-cached like resolveNostrdown: content + the highlight set
	// (ids + anchors) key each item, so steady-state effect re-runs stop
	// POSTing full documents; a new/changed highlight changes the key.
	const merged: Record<string, HighlightSpan[]> = {};
	const uncached: typeof items = [];
	// 9802 events are immutable, so the sorted id set identifies the
	// highlight side; content fingerprint the text side.
	const keyFor = (i: (typeof items)[number]) =>
		`${ndFingerprint(i.content)}|${i.highlights
			.map((h) => h.id)
			.sort()
			.join(',')}`;
	for (const i of items) {
		const hit = hlResolveCache.get(keyFor(i));
		if (hit) merged[i.key] = hit;
		else uncached.push(i);
	}
	if (uncached.length > 0) {
		const resp = await fetchJson<{ spans: Record<string, HighlightSpan[]> }>(
			'/api/v1/highlights/resolve',
			{ method: 'POST', body: JSON.stringify({ items: uncached }) }
		);
		for (const i of uncached) {
			merged[i.key] = resp.spans[i.key] ?? [];
			sessionCacheStore(hlResolveCache, keyFor(i), merged[i.key]);
		}
	}
	return merged;
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
export interface ResolveNostrdownItem {
	key: string;
	content: string;
	publication?: string;
	author?: string;
	/** The coordinate (`"kind:pubkey:dtag"`) of the event this content came
	 *  from. When `publication` is omitted — an isolated doc view — the
	 *  engine derives the containing 30040 from it so sibling refs resolve. */
	coord?: string;
	/** Sibling sections of an unsigned draft (title + synthetic d-tag) so
	 *  `{{ref:slug}}` resolves in the composer's draft-reader preview before
	 *  anything is published. Omit for published reads. */
	siblings?: { title?: string; d_tag: string }[];
}

export async function resolveNostrdown(
	items: ResolveNostrdownItem[],
	policy?: 'local_only' | 'local_first',
	modeConfirm = false
): Promise<Record<string, ResolvedRef[]>> {
	if (items.length === 0) return {};
	const body: Record<string, unknown> = { items };
	if (policy) body.policy = policy;
	// User-initiated fetch: the engine gates the relay lookup through the
	// Confirm-mode intent flow (one modal for the whole batch) instead of
	// silently downgrading to local-only.
	if (modeConfirm) body.mode_confirm = true;
	const resp = await fetchJson<{ refs: Record<string, ResolvedRef[]> }>(
		'/api/v1/nostrdown/resolve',
		{ method: 'POST', body: JSON.stringify(body) }
	);
	return resp.refs;
}

// Session cache for resolved nostrdown refs, keyed by the item's full
// resolution context. Buffer renderers unmount on every switch, so without
// this every alternation re-resolved from scratch — and worse, re-ran the
// relay backfill. Address-keyed with content/sibling fingerprints: an edited
// draft or replaced section changes the fingerprint, so stale entries age
// out of the LRU naturally — drafts no longer opt out of caching entirely
// (the old null-key exclusion made every composer-preview tick a cold,
// relay-reaching, full-document resolve).
const ndResolveCache = new Map<string, { refs: ResolvedRef[]; backfilled: boolean }>();
const ND_CACHE_MAX = 200;

// Sibling session caches for the other two per-section POSTs the reader
// fires — parse (pure tokenizer) and highlight-span resolution. Without
// these, every effect re-run re-POSTed full documents (and did so every 2 s
// while the network-status poll churned identity).
const parseCache = new Map<string, ParsedToken[]>();
const hlResolveCache = new Map<string, HighlightSpan[]>();
const SESSION_CACHE_MAX = 300;
function sessionCacheStore<T>(cache: Map<string, T>, key: string, value: T) {
	if (!cache.has(key) && cache.size >= SESSION_CACHE_MAX) {
		const oldest = cache.keys().next().value;
		if (oldest !== undefined) cache.delete(oldest);
	}
	cache.set(key, value);
}
// Compact string fingerprint for cache keys (length + two independent
// 32-bit FNV-1a passes ≈ 64 bits — not cryptographic, just collision-safe
// at this cache's scale). Full content strings made keys megabytes.
function ndFingerprint(s: string): string {
	let a = 0x811c9dc5;
	let b = 0x811c9dc5 ^ 0x9e3779b9;
	for (let i = 0; i < s.length; i++) {
		const c = s.charCodeAt(i);
		a ^= c;
		a = Math.imul(a, 0x01000193);
		b ^= c;
		b = Math.imul(b, 0x01000193) ^ 0x5bd1e995;
	}
	return `${s.length.toString(36)}.${(a >>> 0).toString(36)}.${(b >>> 0).toString(36)}`;
}
function ndCacheKey(i: ResolveNostrdownItem): string {
	const sib = i.siblings?.length
		? ndFingerprint(i.siblings.map((s) => `${s.d_tag}:${s.title ?? ''}`).join('\n'))
		: '';
	return `${i.publication ?? ''}|${i.author ?? ''}|${i.coord ?? i.key}|${ndFingerprint(i.content)}|${sib}`;
}
function ndCountFound(m: Record<string, ResolvedRef[]>): number {
	return Object.values(m).reduce((n, refs) => n + refs.filter((r) => r.found).length, 0);
}
function ndCacheStore(key: string, refs: ResolvedRef[], backfilled: boolean) {
	if (!ndResolveCache.has(key) && ndResolveCache.size >= ND_CACHE_MAX) {
		const oldest = ndResolveCache.keys().next().value;
		if (oldest !== undefined) ndResolveCache.delete(oldest);
	}
	ndResolveCache.set(key, { refs, backfilled });
}

/**
 * Two-pass nostrdown resolution: an instant `local_only` pass paints every ref
 * (found ones link, unresolved wiki topics still click through to search), then
 * — when `fetch` is set, i.e. network mode is Auto — a background `local_first`
 * pass backfills missing topics from relays and repaints. `apply` is called
 * once per landed pass; guard staleness there. `onFetched` fires after the
 * relay pass with the count of refs that flipped to found (0 = nothing new).
 *
 * Results are session-cached per item: a remounted buffer repaints its links
 * synchronously from cache, and an item whose backfill already settled never
 * hits the relays again (fresh events still surface once something else
 * ingests them and the buffer next remounts, via the cached-refs local pass
 * being replaced below).
 */
export async function resolveNostrdownProgressive(
	items: ResolveNostrdownItem[],
	apply: (refs: Record<string, ResolvedRef[]>) => void,
	opts?: { fetch?: boolean; onFetched?: (newlyFound: number) => void }
): Promise<void> {
	if (items.length === 0) return;
	const countFound = ndCountFound;

	// Pass 0: synchronous repaint from cache; only cache misses hit the engine.
	const merged: Record<string, ResolvedRef[]> = {};
	const uncached: ResolveNostrdownItem[] = [];
	for (const i of items) {
		const hit = ndResolveCache.get(ndCacheKey(i));
		if (hit) merged[i.key] = hit.refs;
		else uncached.push(i);
	}
	if (Object.keys(merged).length > 0) apply({ ...merged });

	if (uncached.length > 0) {
		try {
			const local = await resolveNostrdown(uncached, 'local_only');
			for (const i of uncached) {
				merged[i.key] = local[i.key] ?? [];
				ndCacheStore(ndCacheKey(i), merged[i.key], false);
			}
			apply({ ...merged });
		} catch {
			if (Object.keys(merged).length === 0) apply({});
			return;
		}
	}
	if (!opts?.fetch) return;

	// Relay backfill, once per item per session — a re-opened buffer whose
	// topics already went out doesn't hammer the relays again. Chunked ON
	// PURPOSE: one request-wide flush was fast but silent — the modeline
	// counter sat still and then teleported (10→300), indistinguishable
	// from a hang on a slow relay. Applying per chunk makes the progress
	// pill advance stepwise; server-side the extra requests are cheap
	// post-batching (the tree loads once per request, and topics resolved
	// by an earlier chunk hit local in later ones). No mode_confirm here,
	// so chunking never multiplies Confirm-mode modals.
	const toBackfill = items.filter((i) => !ndResolveCache.get(ndCacheKey(i))?.backfilled);
	if (toBackfill.length === 0) return;
	const BACKFILL_CHUNK = 8;
	const before = countFound(merged);
	try {
		for (let at = 0; at < toBackfill.length; at += BACKFILL_CHUNK) {
			const batch = toBackfill.slice(at, at + BACKFILL_CHUNK);
			const fetched = await resolveNostrdown(batch, 'local_first');
			for (const i of batch) {
				merged[i.key] = fetched[i.key] ?? merged[i.key] ?? [];
				ndCacheStore(ndCacheKey(i), merged[i.key], true);
			}
			apply({ ...merged });
		}
		opts.onFetched?.(Math.max(0, countFound(merged) - before));
	} catch {
		// Keep whatever landed — earlier chunks stay painted; the relay
		// backfill is best-effort.
	}
}

/**
 * User-initiated "resolve everything here" — the modeline button. Runs the
 * relay lookup regardless of the session backfill cache, routed through the
 * Confirm-mode intent flow (ONE modal approval for the whole batch; opens
 * immediately in Auto). Updates the cache, repaints via `apply`, and resolves
 * to the number of refs that flipped to found.
 */
export async function resolveNostrdownForce(
	items: ResolveNostrdownItem[],
	apply: (refs: Record<string, ResolvedRef[]>) => void
): Promise<number> {
	if (items.length === 0) return 0;
	const before: Record<string, ResolvedRef[]> = {};
	for (const i of items) {
		const hit = ndResolveCache.get(ndCacheKey(i));
		if (hit) before[i.key] = hit.refs;
	}
	const fetched = await resolveNostrdown(items, 'local_first', true);
	const merged: Record<string, ResolvedRef[]> = {};
	for (const i of items) {
		merged[i.key] = fetched[i.key] ?? before[i.key] ?? [];
		ndCacheStore(ndCacheKey(i), merged[i.key], true);
	}
	apply(merged);
	return Math.max(0, ndCountFound(merged) - ndCountFound(before));
}

/**
 * Locate + classify nostrdown `{{ }}`/`[[ ]]` tokens in text — the engine's pure
 * tokenizer, no resolution. The single home of the grammar: the editor decorates
 * these spans and the reader chips them before `/resolve` lands, so no frontend
 * re-implements the token regexes. Batched (key → content) like `resolveNostrdown`.
 */
export async function parseNostrdown(
	items: { key: string; content: string }[]
): Promise<Record<string, ParsedToken[]>> {
	if (items.length === 0) return {};
	// Session-cached by content fingerprint — the tokenizer is pure, so a
	// content string always parses the same way.
	const merged: Record<string, ParsedToken[]> = {};
	const uncached: typeof items = [];
	for (const i of items) {
		const hit = parseCache.get(ndFingerprint(i.content));
		if (hit) merged[i.key] = hit;
		else uncached.push(i);
	}
	if (uncached.length > 0) {
		const map = Object.fromEntries(uncached.map((i) => [i.key, i.content]));
		const resp = await fetchJson<{ tokens: Record<string, ParsedToken[]> }>(
			'/api/v1/nostrdown/parse',
			{ method: 'POST', body: JSON.stringify({ items: map }) }
		);
		for (const i of uncached) {
			merged[i.key] = resp.tokens[i.key] ?? [];
			sessionCacheStore(parseCache, ndFingerprint(i.content), merged[i.key]);
		}
	}
	return merged;
}

/**
 * NIP-54-normalize a batch of strings to slugs, engine-side — the single home of
 * slug normalization, matching the tokenizer's own `target` normalization exactly.
 * The composer uses it for slug matching (sibling-title filter, heading scroll,
 * autocomplete). Positionally aligned with `values`.
 */
export async function normalizeNostrdown(values: string[]): Promise<string[]> {
	if (values.length === 0) return [];
	const resp = await fetchJson<{ slugs: string[] }>('/api/v1/nostrdown/normalize', {
		method: 'POST',
		body: JSON.stringify({ values })
	});
	return resp.slugs;
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

/** Single-event lookups resolve a miss to `{ event: null }` instead of
 *  throwing — the engine's 404 carries that body already, and every
 *  caller branches on a null event rather than a raised string. Other
 *  statuses still throw. */
async function fetchEventOrNull(url: string): Promise<{ event: unknown | null }> {
	try {
		return await fetchJson<{ event: unknown }>(url);
	} catch (e) {
		const raw = e instanceof Error ? e.message : String(e);
		if (/^404:/.test(raw)) return { event: null };
		throw e;
	}
}

/** Build the `?policy=…&confirm=true` suffix shared by the single-event
 *  endpoints. `bypassOffline` maps to `confirm=true`: the engine treats
 *  the request as user-initiated and runs the Confirm-mode intent flow
 *  (modal approval) instead of silently downgrading to a local read. */
function eventFetchQs(opts: {
	policy?: 'local_only' | 'local_first' | 'fetch_always';
	bypassOffline?: boolean;
}): string {
	const params = new URLSearchParams();
	if (opts.policy) params.set('policy', opts.policy);
	if (opts.bypassOffline) params.set('confirm', 'true');
	const qs = params.toString();
	return qs ? `?${qs}` : '';
}

export function getEvent(
	eventId: string,
	opts: { policy?: 'local_only' | 'local_first' | 'fetch_always'; bypassOffline?: boolean } = {}
) {
	return fetchEventOrNull(`/api/v1/events/${eventId}${eventFetchQs(opts)}`);
}

/** Fetch an addressable event (latest version for the kind/pubkey/d_tag
 *  triple). Used by the reader to render non-30040 addressables like
 *  NIP-23 long-form articles (30023) and NKBIP-02 wikis (30818). */
export function getAddressable(
	kind: number,
	pubkey: string,
	d_tag: string,
	policy?: 'local_only' | 'local_first' | 'fetch_always',
	opts: { bypassOffline?: boolean } = {}
) {
	const qs = eventFetchQs({ policy, bypassOffline: opts.bypassOffline });
	return fetchEventOrNull(
		`/api/v1/addressable/${kind}/${pubkey}/${encodeURIComponent(d_tag)}${qs}`
	);
}

export function queryEvents(filters: Record<string, unknown>[], policy = 'local_first') {
	return fetchJson<{ events: unknown[]; count: number; source: { local_count: number; relay_count: number } }>('/api/v1/query', {
		method: 'POST',
		body: JSON.stringify({ filters, policy })
	});
}

// Spell API (kind 777 — NIP-A7 saved queries + tendrl composition extension).
// Parsing happens engine-side; the web only renders what these return.

export interface SpellParamInfo {
	name: string;
	prompt: string | null;
}

export interface SpellInfo {
	id: string | null;
	cmd: 'REQ' | 'COUNT' | 'PIPE';
	name: string | null;
	description: string;
	params: SpellParamInfo[];
	kinds: number[];
	topics: string[];
	stages: { spell_id: string; combinator: 'map' | 'join' | null; relays: string[] }[];
	/** `in` chaining: input spell whose results feed this spell's $in.*. */
	input: string | null;
	/** Relay hints for finding the input spell (from an nevent or explicit). */
	input_relays: string[];
	relays: string[];
	limit: number | null;
	since: string | null;
	until: string | null;
	search: string | null;
}

/** One preview line: a search-DSL clause + optional non-literal note. */
export interface SpellClause {
	clause: string;
	annotation?: string;
}

export interface SpellEntry {
	event: NostrEvent;
	/** Parsed spell, null when the kind-777 event doesn't parse. */
	spell: SpellInfo | null;
	required_args: string[];
	/** References $in.* with no `in` input — only runs as a pipeline stage. */
	partial: boolean;
	needs_identity: boolean;
	error: string | null;
	/** Search-DSL preview (empty for PIPE — stages unpack via inspect). */
	clauses: SpellClause[];
	query_string: string | null;
}

export function listSpells(pubkey: string, limit = 50, policy = 'local_only') {
	return fetchJson<{ entries: SpellEntry[]; count: number }>('/api/v1/spell/list', {
		method: 'POST',
		body: JSON.stringify({ pubkey, limit, policy })
	});
}

export interface StageInspection {
	spell_id: string;
	combinator: 'map' | 'join' | null;
	name: string | null;
	clauses: SpellClause[];
	query_string: string | null;
	error: string | null;
}

export interface SpellInspection {
	spell: SpellInfo;
	required_args: string[];
	partial: boolean;
	needs_identity: boolean;
	filter: Record<string, unknown> | null;
	unresolved: string | null;
	clauses: SpellClause[];
	query_string: string;
	/** PIPE only: per-stage clause blocks. */
	stages: StageInspection[] | null;
}

export function inspectSpell(req: {
	id?: string;
	event?: unknown;
	args?: Record<string, string>;
	policy?: string;
}) {
	return fetchJson<SpellInspection>('/api/v1/spell/inspect', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface SpellComposeResponse {
	/** Unsigned kind-777 template for /api/v1/identity/sign. */
	template: SignTemplateRequest['template'];
	spell: SpellInfo;
	clauses: SpellClause[];
	query_string: string;
	/** What degraded in translation (multi-char tags, text → NIP-50, …). */
	warnings: string[];
	/** Pipeline preview: per-stage clause blocks (local-only lookup). */
	stages: StageInspection[] | null;
}

export function composeSpell(req: {
	/** Search string (filter spells) — empty when composing a pipeline. */
	query?: string;
	name?: string;
	description?: string;
	topics?: string[];
	/** value present = replace that literal with $arg; absent = declare only. */
	params?: { name: string; prompt?: string; value?: string }[];
	/** 'REQ' (default) or 'COUNT'. */
	cmd?: string;
	/** Raw spell time values: '7d', 'now', or unix seconds. */
	since?: string;
	until?: string;
	/** Raw author values: $me, $contacts, or 64-hex pubkeys. */
	authors?: string[];
	/** Pipeline stages — composes a PIPE spell (query must be empty).
	 * spell_id accepts a 64-hex id, note1…, or nevent1… (relay hints unpack). */
	stages?: { spell_id: string; combinator?: 'map' | 'join' }[];
	/** `in` chaining: input spell — 64-hex id, note1…, or nevent1…
	 * (an nevent's relay hints unpack). Exclusive with stages. */
	input?: string;
	/** Explicit "find the input spell on these relays" hints. */
	input_relays?: string[];
	/** Raw id values: 64-hex ids or $in.* projections (need `input`). */
	ids?: string[];
}) {
	return fetchJson<SpellComposeResponse>('/api/v1/spell/compose', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

/** Ingest one signed event into local nostrdb (local-first save). */
export function ingestEvent(event: unknown) {
	return fetchJson<{ ingested: number; errors: number }>('/api/v1/ingest', {
		method: 'POST',
		body: JSON.stringify(event)
	});
}

// Spellbooks (kind 30777): addressable e-tag sets referencing spells by
// any author. "Bookmark" = add someone's spell to my book.

export interface SpellbookEntryRef {
	event_id: string;
	relay_hint?: string;
	author_hint?: string;
}

export interface Spellbook {
	id: string | null;
	author: string;
	d: string;
	title: string | null;
	description: string | null;
	created_at: number;
	entries: SpellbookEntryRef[];
}

export interface BookEntry {
	reference: SpellbookEntryRef;
	entry: SpellEntry | null;
	missing: boolean;
}

export interface SpellBookView {
	book: Spellbook;
	/** Raw newest 30777 event — used to re-broadcast a local book. */
	event: NostrEvent;
	entries: BookEntry[];
	/** Signed+ingested but not yet accepted by any relay. */
	local: boolean;
}

export function getSpellBooks(pubkey: string, policy = 'local_only', d?: string) {
	return fetchJson<{ books: SpellBookView[] }>('/api/v1/spell/book', {
		method: 'POST',
		body: JSON.stringify({ pubkey, policy, d })
	});
}

export function spellBookTemplate(req: {
	action: 'add' | 'remove' | 'create';
	spell_event_id?: string;
	d?: string;
	title?: string;
	description?: string;
}) {
	return fetchJson<{
		template: SignTemplateRequest['template'];
		book: Spellbook;
		created: boolean;
	}>('/api/v1/spell/book/template', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export function saveSpellBook(req: { event: unknown; broadcast: boolean; relays?: string[] }) {
	return fetchJson<{
		ingested: boolean;
		coordinate: string;
		local: boolean;
		broadcast_results: { relay_url: string; success: boolean; message: string }[] | null;
	}>('/api/v1/spell/book/save', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export interface SpellStageReport {
	spell_id: string | null;
	name: string | null;
	combinator: 'map' | 'join' | null;
	fetched: number;
	output: number;
	truncated: boolean;
}

export interface SpellOutcome {
	cmd: string;
	name: string | null;
	count: number;
	events: NostrEvent[];
	auxiliary: NostrEvent[];
	/** Referent event id → labeling event ids (map-stage provenance). */
	provenance: Record<string, string[]>;
	stages: SpellStageReport[];
	/** Oldest created_at the source stage fetched — the load-older cursor
	 * (re-run with until = oldest_source - 1); null when it fetched nothing. */
	oldest_source: number | null;
}

export function executeSpell(req: {
	id?: string;
	event?: unknown;
	args?: Record<string, string>;
	policy?: string;
	mode_confirm?: boolean;
	/** Page the source stage: only events at or before this timestamp. */
	until?: number;
}) {
	return fetchJson<SpellOutcome>('/api/v1/spell/execute', {
		method: 'POST',
		body: JSON.stringify(req)
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

/** Watch-only login: npub1… or 64-hex pubkey → state "watching". */
export function loginIdentityNpub(npub: string) {
	return fetchJson<IdentityStatus>('/api/v1/identity/login-npub', {
		method: 'POST',
		body: JSON.stringify({ npub })
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
	kind: 'nip07' | 'nip46' | 'nip55';
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
	source: 'engine' | 'nip07' | 'nip46' | 'nip55';
	signer_id?: string;
	/** External signer pubkey (hex). Pass when source is external
	 *  (nip07/nip46/nip55) so /identity status surfaces a non-null pubkey. */
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
	options: { modeConfirm?: boolean; search?: string; until?: number } = {}
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
	// Backfill cursor — NIP-01 `until` bound (callers pass oldest - 1
	// to page strictly older events).
	if (options.until != null) body.until = options.until;
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

/** Reset ALL relay working sets to the first-boot defaults (re-seed
 *  from `initial_relays`, broadcast cleared, discovery built-ins).
 *  Named sets survive. Local-only — published lists untouched. */
export function resetRelaysToDefaults() {
	return fetchJson<{ updated: boolean; message: string }>('/api/v1/config/update', {
		method: 'POST',
		body: JSON.stringify({ reset_relays: true })
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

/** Download + load the embedding model if not cached. The promise stays
 *  pending for the whole download (~90 MB first time); resolves with the
 *  refreshed status (model_ready = true). */
export function prefetchEmbeddingModel() {
	return fetchJson<EmbeddingStatusResponse>('/api/v1/embed/prefetch', { method: 'POST' });
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

/** Kill one in-flight relay fetch by id, or all of them when id is omitted.
 *  Resolves to how many were signalled. */
export async function killFetch(id?: number): Promise<number> {
	const resp = await fetchJson<{ killed: number }>('/api/v1/network/fetch-kill', {
		method: 'POST',
		body: JSON.stringify(id === undefined ? {} : { id })
	});
	return resp.killed;
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

// Discussion authoring: the engine builds the NIP-22/84/09 tags, signs via
// the active source (engine key / NIP-07 / NIP-46), ingests locally FIRST,
// then broadcasts to the publish relay set. Because ingest precedes the
// response, the right refresh after a 200 is a cheap
// `getDiscussionList({policy: 'local_only', threaded: true})` — never
// client-side thread splicing.

export interface BroadcastSummary {
	successful: number;
	total: number;
	results: { relay_url: string; success: boolean; message?: string; event_id: string }[];
}

export interface DiscussionPublishResponse {
	/** The signed event, already queryable locally. */
	event: DiscussionEvent;
	/** Per-relay fan-out results; 0/0 when the publish set is empty or the
	 *  broadcast was declined in Confirm mode — the event is still local. */
	broadcast: BroadcastSummary;
}

/** The root a comment scopes to — exactly one form: an addressable/replaceable
 *  coordinate, a regular event (kind+pubkey only needed when uncached), or a
 *  NIP-73 external id (engine normalizes it). */
export interface CommentRootRef {
	address?: string;
	event_id?: string;
	kind?: number;
	pubkey?: string;
	external?: string;
	id_kind?: string;
	hint?: string;
}

export function publishComment(req: {
	root?: CommentRootRef;
	/** Present = reply; root is chased from the parent's own tags. `event`
	 *  is the fallback copy for when the engine hasn't cached the parent. */
	parent?: { event_id: string; event?: DiscussionEvent };
	content: string;
	relays?: string[];
}) {
	return fetchJson<DiscussionPublishResponse>('/api/v1/discussions/comment', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export function publishHighlight(req: {
	/** Exactly one source family: a nostr target (`address`/`event_id`), a
	 *  web `url` (NIP-84 `r` tag, tracker-cleaned engine-side), or an
	 *  `external` NIP-73 id (`i` tag — isbn/doi/…). */
	target: {
		address?: string;
		event_id?: string;
		url?: string;
		external?: { id: string; id_kind: string };
	};
	/** The selected text — slice it from the source content so `offset`
	 *  agrees byte-for-byte (the engine rejects mismatches). */
	content: string;
	/** UTF-16 code units into the pinned version's content. */
	offset?: [number, number];
	context?: string;
	/** Optional annotation → NIP-84 quote highlight. */
	comment?: string;
	relays?: string[];
}) {
	return fetchJson<DiscussionPublishResponse>('/api/v1/discussions/highlight', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

/** The exact unsigned kind-9802 template `publishHighlight` would sign for
 *  this request — same engine builder + validation, client tag stamped,
 *  pubkey filled when a signer is active. `id`/`sig` appear at signing. */
export function previewHighlight(req: {
	target: {
		address?: string;
		event_id?: string;
		url?: string;
		external?: { id: string; id_kind: string };
	};
	content: string;
	offset?: [number, number];
	context?: string;
	comment?: string;
}): Promise<{
	event: {
		kind: number;
		created_at: number;
		tags: string[][];
		content: string;
		pubkey?: string;
	};
}> {
	return fetchJson('/api/v1/discussions/highlight/preview', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}

export function deleteDiscussion(req: { event_id: string; reason?: string; relays?: string[] }) {
	return fetchJson<DiscussionPublishResponse>('/api/v1/discussions/delete', {
		method: 'POST',
		body: JSON.stringify(req)
	});
}
