export interface Fragment {
	id: number;
	role: string;
	content: string;
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

export interface SearchResponse {
	results: SearchResult[];
	count: number;
	local_count: number;
	relay_count: number;
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
	origin: 'chat' | 'search' | 'compose';
	readonly: boolean;
}

export type SyncMode = 'reactive' | 'explicit';
export type ButtonLabels = 'icon' | 'text';

export interface ComposeState {
	title: string;
	tags: TagEntry[];
	sections: ContextItem[];
}

export type ViewMode = 'outline' | 'continuous' | 'paginated';
export type DocMode = 'empty' | 'reading' | 'compose';

export interface EmbeddingStatusResponse {
	enabled: boolean;
	indexed_count: number;
	total_events: number;
	sidecar_available: boolean;
	model: string | null;
}
