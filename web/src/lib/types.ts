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
	/** False = unsigned draft (placeholder signature). */
	signed: boolean;
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
	  }
	| {
			type: 'leaf';
			addr: NAddr;
			depth: number;
			title: string | null;
			content: string | null;
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
	modified: boolean;
	in_context: boolean;
	in_compose: boolean;
	origin: 'chat' | 'search' | 'compose' | 'import';
	readonly: boolean;
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
	sidecar_available: boolean;
	model: string | null;
}

export type NetworkMode = 'auto' | 'confirm';

/** Pattern of a user-initiated fetch operation (mirrors the engine). */
export type FetchPattern = 'event' | 'publication' | 'thread' | 'search' | 'profile' | 'custom';

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
	  }
	| { type: 'progress'; operation_id: string; label: string; done: number; total: number | null }
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
}
