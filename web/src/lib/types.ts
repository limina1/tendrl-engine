export interface Fragment {
	id: number;
	role: string;
	content: string;
	/** Optional structured blocks from Claude Code sessions */
	blocks?: ClaudeSessionBlock[];
}

export interface ChatResponse {
	fragments: Fragment[];
	fragment_count: number;
	edit_mode: boolean;
	edit_buffer: string | null;
	system_prompt: string | null;
	context_count: number;
	generating: boolean;
}

export interface SendMessageRequest {
	content: string;
}

export interface EditBufferRequest {
	buffer: string;
}

export interface SystemPromptRequest {
	prompt: string;
}

export interface NoteRequest {
	title: string;
	content: string;
}

export interface InjectContextRequest {
	notes: NoteRequest[];
}

export interface NAddr {
	kind: number;
	pubkey: string;
	d_tag: string;
}

export interface PublicationSummary {
	addr: NAddr;
	title: string | null;
	summary: string | null;
	image: string | null;
	author_pubkey: string;
	version: string | null;
	created_at: number;
	section_count: number;
	/** Relays the index event has been seen on. Empty = written locally, not published. */
	relays: string[];
	/** False = unsigned draft (placeholder signature). With the signed-snapshot
	 *  model this is effectively always true for publications. */
	signed: boolean;
	/** True = a signed snapshot the user created that hasn't been broadcast to
	 *  any relay yet (engine LocalPublicationTracker). Drives the "local" pill;
	 *  flips false once a relay accepts it. Optional on older payloads. */
	local?: boolean;
	/** True when the index event carries a NIP-54 `a`/`e` tag with the
	 *  `fork` marker — drives the `fork` provenance pill. Optional on the
	 *  type so older payloads (engine didn't emit it) don't trip
	 *  destructuring; PoolStateBadges treats `undefined` as "not forked". */
	forked?: boolean;
	/** Publications (kind 30040) that reference this one as a child via an `a`
	 *  tag — i.e. the works this publication is part of. Computed engine-side
	 *  (reverse a-tag lookup, local store). Empty/absent = contained in nothing;
	 *  drives the "part of N" badge, which searches for the containers on click. */
	contained_in?: NAddr[];
}

export interface PublicationDetail {
	addr: NAddr;
	title: string | null;
	summary: string | null;
	image: string | null;
	author_pubkey: string;
	version: string | null;
	created_at: number;
	index: unknown;
	/** Relays the index event has been seen on. Empty = local-only.
	 *  Populated by ReaderBuffer's stream consumer from the root
	 *  PubLoadEvent::Index. */
	relays?: string[];
	signed?: boolean;
	/** True when the publication's index event carries a NIP-54 fork
	 *  marker — see PublicationSummary.forked. */
	forked?: boolean;
}

export interface TocEntry {
	title: string | null;
	addr: NAddr;
	depth: number;
	position: number;
	loaded: boolean;
	/** True for a nested 30040 index, false for a 30041 section leaf. */
	is_publication: boolean;
	/** Section body, when this is a resolved 30041 within the depth horizon.
	 *  Null for nested indexes and for sections not yet loaded. */
	content: string | null;
	children: TocEntry[];
}

/** A child reference inside a streamed `index` event — mirrors the Rust
 *  `PubChildRef`. `in_horizon` = this child's own events will be streamed and
 *  it counts toward the load total `N`; a non-in_horizon nested index is a
 *  frontier stub: rendered and refocus-able, but not counted. */
export interface PubChildRef {
	addr: NAddr;
	is_index: boolean;
	in_horizon: boolean;
}

/** One progress event from the streaming publication loader
 *  (`GET /api/v1/publications/:pubkey/:d_tag/stream`). Mirrors the Rust
 *  `PubLoadEvent` — the `type` tag and field names are its serde output. */
export type PubLoadEvent =
	| {
			type: 'index';
			addr: NAddr;
			depth: number;
			title: string | null;
			is_root: boolean;
			children: PubChildRef[];
			/** Provenance from the index event itself — drives the reader
			 *  publication header's draft / relay-label pill. Optional on
			 *  the type so the parser tolerates older engines that don't
			 *  emit them; engines that don't ship them default `signed` to
			 *  true (vast majority) and `relays` to []. */
			relays?: string[];
			signed?: boolean;
			/** NIP-54 fork marker on the index event. Drives the `fork`
			 *  provenance pill. Older engines that don't emit this leave
			 *  it undefined → renders as "not forked". */
			forked?: boolean;
	  }
	| {
			type: 'leaf';
			addr: NAddr;
			depth: number;
			title: string | null;
			content: string | null;
			/** Same provenance as the index variant — drives the per-section
			 *  pill in the reader outline + paginated/continuous header. */
			relays?: string[];
			signed?: boolean;
	  }
	| { type: 'error'; addr: NAddr; depth: number; message: string }
	| { type: 'done'; total: number };

export interface Section {
	addr: NAddr;
	title: string | null;
	content: string | null;
	position: number;
	loaded: boolean;
}

export type SectionStatus = 'pending' | 'loading' | 'loaded' | 'error';

export interface LazySection {
	addr: NAddr;
	title: string | null;
	content: string | null;
	position: number;
	status: SectionStatus;
	error?: string;
	/** Nesting depth in the publication tree (0 = top level). Drives the
	 *  indented render. A 30040 entry (`addr.kind === 30040`) is a nested
	 *  index the reader can refocus into rather than a readable section. */
	depth?: number;
	/** Relay provenance for this leaf/index event — populated from the
	 *  matching PubLoadEvent so the reader outline + paginated/continuous
	 *  header pills can render. Empty / missing = local-only. */
	relays?: string[];
	/** False = unsigned draft (placeholder all-zero signature). Optional so
	 *  callers that don't have the info yet (e.g. before the leaf streams)
	 *  can leave it undefined; PoolStateBadges suppresses the pill on
	 *  undefined. */
	signed?: boolean;
	/** NIP-54 fork marker — only meaningful for nested 30040 entries
	 *  (sections never carry it). Drives the `fork` pill on the
	 *  outline's nested-publication rows. */
	forked?: boolean;
}

export interface SectionMeta {
	addr: NAddr;
	title: string | null;
	position: number;
	loaded: boolean;
}

export interface SearchResult {
	addr: NAddr | null;
	event_id: string;
	title: string | null;
	preview: string;
	author: string;
	kind: number;
	tags: string[][];
	created_at: number;
	semantic_score: number | null;
}

/** A profile (kind-0) hit — an author match. Search returns these as a
 *  category distinct from content `results` (see search-architecture). */
export interface ProfileResult {
	pubkey: string;
	name?: string;
	display_name?: string;
	nip05?: string;
	picture?: string;
	about?: string;
	/** Match strength — 0 strongest (name prefix), higher = weaker. */
	score: number;
	source: 'local' | 'relay';
}

export interface DocPageResult {
	filename: string;
	page_num: number;
	title: string | null;
	content: string;
	semantic_score: number;
}

/** One bucket of a `count:NAME` histogram. */
export interface TagValueCount {
	value: string;
	count: number;
	event_ids: string[];
}

export interface SearchResponse {
	results: SearchResult[];
	/** Profile (kind-0) hits — an author match, distinct from `results`.
	 *  Omitted from the JSON when empty. */
	profiles?: ProfileResult[];
	count: number;
	local_count: number;
	relay_count: number;
	/** True when the engine actually queried relays for this search —
	 *  false for local-only scans and declined confirm-mode fetches. */
	relays_queried: boolean;
	doc_results?: DocPageResult[];
	/** Histograms from `count:NAME` operators. Keyed by tag name; each list
	 *  is sorted by count desc. Omitted from the JSON when empty. */
	tag_counts?: Record<string, TagValueCount[]>;
}

export interface TagEntry {
	name: string;
	value: string;
}

export interface ContextItem {
	id: string;
	title: string;
	content: string;
	context_content: string;
	tags: TagEntry[];
	source_event_id?: string;
	source_addr?: NAddr | null;
	source_fragment_id?: number;
	original_content: string;
	/** Snapshot of title/tags at pool-entry time, for divergence detection
	 *  on sourced items (content is covered by original_content). Editing
	 *  any of the three forks the section. Absent on legacy/draft items —
	 *  those axes then can't diverge. */
	original_title?: string;
	original_tags?: TagEntry[];
	modified: boolean;
	in_context: boolean;
	in_compose: boolean;
	/** Held in the reference pool without an active routing intent — a
	 *  bookmark/staging state. Items can be held in addition to (not instead
	 *  of) `in_context` / `in_compose`; gc() keeps anything with any of the
	 *  three flags set. SearchPanel's Refs tab is the held-filtered view. */
	held: boolean;
	origin: 'chat' | 'search' | 'compose' | 'import';
	readonly: boolean;
	/** Heading depth in the compose outline. 2 = top-level section
	 *  (current default), 3+ = nested under the previous shallower
	 *  sibling. Drives the engine's nested-30040 emission at publish
	 *  time. Optional/legacy items default to 2 when absent. */
	level?: number;
	/** When set, this item is a block-level transclude *slot*: the
	 *  naddr/coordinate of an existing 30040/30041 to reference as a child of
	 *  the index, rather than authored content. Carried to the publish request
	 *  as `slot`; the engine emits an `a`-tag and mints no 30041. */
	slot?: string;
}

export type SyncMode = 'reactive' | 'explicit';
export type ButtonLabels = 'icon' | 'text';
export type EditorInsertMode = 'cursor' | 'append';
export type ComposeDefaultMode = 'full' | 'plain';

export interface ComposeState {
	title: string;
	tags: TagEntry[];
	sections: ContextItem[];
	/** When the draft was seeded from an existing publication, this is the
	 * source 30040 NAddr. Used to (a) detect structural change at publish
	 * time and (b) emit a fork-marker tag on the new 30040. */
	source_publication_addr?: NAddr | null;
	/** The 30040 event id we forked from — emitted as the `e` tag with
	 * `fork` marker when republished. */
	source_publication_event_id?: string | null;
	/** Original section order (list of source NAddrs) for structural-change
	 * detection. Indexes line up with the order of `sections` at seed time. */
	source_section_order?: NAddr[];
}

/** Per-section authorship/provenance state, derived from ContextItem.
 * Drives border color in DraftReader and ComposeView, and decides what
 * each section emits on publish. */
export type SectionState = 'imported' | 'claimed' | 'forked' | 'original';

export type ViewMode = 'outline' | 'continuous' | 'paginated';
export type DocMode = 'empty' | 'reading' | 'compose' | 'ignored' | 'claude-sessions' | 'profile';

export interface NostrEvent {
	id: string;
	pubkey: string;
	kind: number;
	created_at: number;
	content: string;
	tags: string[][];
	/** Schnorr signature, hex-encoded. The engine emits this on every event
	 *  returned from `/api/v1/query` and friends. Optional in the type because
	 *  callers that build NostrEvent literals (e.g. compose previews) don't
	 *  always populate it; a missing/empty sig signals "unsigned draft". */
	sig?: string;
	/** Relays the engine has seen this event on. Empty / missing = local-only
	 *  (written or not yet broadcast). Used by `<PoolStateBadges>` for the
	 *  draft / remote / relay-label provenance pill. */
	relays?: string[];
}

/** Treat a hex `sig` as signed iff it's non-empty and not the all-zeros
 *  placeholder the engine uses for unsigned drafts. Mirrors `publication.rs`
 *  `signed` derivation so the same rule lights up frontend pills. */
export function isEventSigned(sig: string | undefined | null): boolean {
	if (!sig) return false;
	return sig.length > 0 && !sig.split('').every((c) => c === '0');
}

export interface ClaudeSessionSummary {
	id: string;
	date: string;
	message_count: number;
	first_prompt: string;
	last_message: string;
	modified: number;
}

export interface ClaudeSessionBlock {
	type: 'text' | 'thinking' | 'tool_use' | 'tool_result';
	text?: string;
	thinking?: string;
	name?: string;
	input?: unknown;
	content?: string;
}

export interface ClaudeSessionMessage {
	role: string;
	blocks: ClaudeSessionBlock[];
	timestamp: string;
}

export interface DocumentFile {
	name: string;
	format: string;
	size: number;
	modified: number;
}

export interface ImportPage {
	page_num: number;
	title: string | null;
	content: string;
}

export interface ImportResult {
	filename: string;
	format: string;
	page_count: number;
	pages: ImportPage[];
}

export interface EmbeddingStatusResponse {
	enabled: boolean;
	indexed_count: number;
	total_events: number;
	stale_count: number;
	missing_sections: number;
	embedding_available: boolean;
	model: string | null;
	/** Kinds currently eligible for embedding (engine-persisted selection). */
	embed_kinds: number[];
	/** Full menu of embeddable kinds the UI offers as checkboxes. */
	available_kinds: number[];
	/** Whether retrieval + publishing auto-embed the configured kinds. */
	auto_embed: boolean;
}

/** GET /health — engine liveness + the running build's version. */
export interface HealthResponse {
	status: string;
	version: string;
	/** Git branch of the checkout the engine runs from; absent outside a repo. */
	branch?: string;
}

export type NetworkMode = 'auto' | 'confirm';

/** Pattern of a user-initiated fetch operation (mirrors the engine). */
export type FetchPattern = 'event' | 'publication' | 'thread' | 'search' | 'profile' | 'custom';

/** Which relay class a fetch/publish member targets — dot-notation
 *  values match the DSL surface (`indexer.default`, `search.fallback`). */
export type Phase =
	| 'read'
	| 'write'
	| 'publish'
	| 'broadcast'
	| 'search.default'
	| 'search.fallback'
	| 'indexer.default'
	| 'indexer.fallback';

/** Per-relay lifecycle status — drives the dots in the expanded toast. */
export type RelayStatusValue =
	| { kind: 'connecting' }
	| { kind: 'eose'; event_count: number }
	| { kind: 'error'; msg: string }
	| { kind: 'timeout' }
	| { kind: 'accepted' }
	| { kind: 'rejected'; msg: string };

/** Structural NIP-01 filter (subset; all fields optional). */
export interface NipFilter {
	kinds?: number[];
	authors?: string[];
	ids?: string[];
	since?: number;
	until?: number;
	limit?: number;
	search?: string;
	tags?: Record<string, string[]>;
}

/** One execution stage — members fire concurrently; stages run in order. */
export interface PhaseStage {
	label: string;
	members: Array<[Phase, string[]]>;
	start_delay_ms: number;
}

export interface CompositionShape {
	phases: PhaseStage[];
}

/** Structured summary of a relay request — the formal-language form. */
export interface RequestSummary {
	filters: NipFilter[];
	composition: CompositionShape;
	dsl: string;
}

/** One event row in a {@link PublishManifest} — what the publish confirm
 *  modal lists (collapsed by default). */
export interface PublishEntry {
	event_id: string;
	kind: number;
	title?: string;
	d_tag?: string;
}

/** Plain-language description of what a publish replicates — the
 *  "function / procedure" the publish confirm modal renders instead of
 *  the raw event JSON. Travels on a `publish_intent`. */
export interface PublishManifest {
	/** `[kind, count]` pairs, ascending by kind. */
	kind_counts: [number, number][];
	total: number;
	/** kind-30040 count (publication indices). */
	index_count: number;
	/** kind-30041 count (publication sections). */
	section_count: number;
	/** True when >1 index is present — a nested tree. */
	nested: boolean;
	entries: PublishEntry[];
}

/** Events streamed from the engine over /api/v1/network/fetch-events. */
export type FetchEvent =
	| {
			type: 'intent';
			operation_id: string;
			pattern: FetchPattern;
			label: string;
			steps: string[];
			relays: string[];
			needs_confirmation: boolean;
			summary?: RequestSummary;
	  }
	| {
			type: 'publish_intent';
			operation_id: string;
			label: string;
			relays: string[];
			event_ids: string[];
			needs_confirmation: boolean;
			summary?: RequestSummary;
			manifest?: PublishManifest;
	  }
	| { type: 'progress'; operation_id: string; label: string; done: number; total: number | null }
	| {
			type: 'relay_status';
			operation_id: string;
			relay: string;
			phase: Phase;
			status: RelayStatusValue;
	  }
	| { type: 'completed'; operation_id: string; event_count: number }
	| { type: 'failed'; operation_id: string; error: string };

export interface FetchRecord {
	id: number;
	relay: string;
	filter_summary: string;
	event_count: number;
	duration_ms: number;
	trigger: string;
	timestamp: number;
	success: boolean;
	error: string | null;
}

export interface NetworkStatus {
	mode: NetworkMode;
	/** False until the user makes an explicit first-run mode choice. Drives
	 *  the one-time "choose your default network mode" modal. */
	mode_chosen: boolean;
	active_fetches: number;
	total_events_fetched: number;
	last_fetch_timestamp: number;
	recent: FetchRecord[];
}

export type IdentityState = 'none' | 'locked' | 'unlocked';

export type IdentitySourceKind = 'engine' | 'nip07' | 'nip46';

export interface IdentityStatus {
	state: IdentityState;
	pubkey: string | null;
	npub: string | null;
	seconds_remaining: number | null;
	unsigned_count: number;
	lock_timeout_minutes: number;
	/** Active signing source. Always present (defaults to "engine"). */
	source: IdentitySourceKind;
	/** Set when source is nip07 / nip46. */
	signer_id?: string;
	/** Only present on the assistant identity status: whether the OS keyring
	 *  is usable for persistence. `false` ⇒ the key won't survive a restart. */
	keyring_available?: boolean;
}

/** One row in the multi-event JSON inspector (EventsJsonModal). */
export interface EventsModalItem {
	label: string;
	kind?: number;
	id?: string;
	json: unknown;
	/** Provenance banner for compose previews: forked vs linked original.
	 *  `addr` is the original's `kind:pubkey:d_tag` coordinate. */
	banner?: { status: 'forked' | 'linked'; text: string; addr?: string };
}

/** A section in the republish diff, matched/added/removed by `T` (title slug). */
export interface RepublishDiffSection {
	title: string;
	/** Title slug — the match key. */
	t: string;
	/** Existing d-tag (matched/removed only). */
	dTag?: string;
	/** Matched only: content differs from the published version. */
	contentChanged?: boolean;
}

/** Result of detecting that a same-title publication already exists. Drives
 *  ComparePublishModal. `matched` = same `T` (reuse d-tag → replace),
 *  `added` = new only, `removed` = existing only. */
export interface RepublishDiff {
	existingAddr: NAddr;
	existingTitle: string;
	/** Existing index d-tag to reuse so the 30040 replaces. */
	pubDTag: string;
	matched: RepublishDiffSection[];
	added: RepublishDiffSection[];
	removed: RepublishDiffSection[];
	/** Title-slug → existing d-tag, for reusing section identifiers. */
	sectionDTagByT: Record<string, string>;
}
