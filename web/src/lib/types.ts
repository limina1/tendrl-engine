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
	children: TocEntry[];
}

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

export interface DocPageResult {
	filename: string;
	page_num: number;
	title: string | null;
	content: string;
	semantic_score: number;
}

export interface SearchResponse {
	results: SearchResult[];
	count: number;
	local_count: number;
	relay_count: number;
	doc_results?: DocPageResult[];
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

export type NetworkMode = 'online' | 'offline';

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

export interface IdentityStatus {
	state: IdentityState;
	pubkey: string | null;
	npub: string | null;
	seconds_remaining: number | null;
	unsigned_count: number;
	lock_timeout_minutes: number;
}
