// Rendering layers — the opt-in registry.
//
// A *layer* is something the reader may put over a section's text once the
// user has switched it on. Nothing in this registry is active by default:
// `tendrl.layers` (store.svelte.ts) holds the enabled set and the user's
// bindings, and `resolve.ts` turns a section's declared format + those
// preferences into "which base, if any".
//
// Milestone 1 ships one kind, `base`: a *decorated-source* renderer that
// emits STYLE SPANS over UTF-16 offsets of the unchanged section text. The
// text stays one `pre-wrap` run (see RichContent), so highlight capture and
// nostrdown overlays keep their 1:1 DOM-offset ↔ content mapping. `grammar`
// (code-fence syntax colouring) and `block` (fence → figure replacement, e.g.
// Graphviz) are reserved for the next milestone and slot in beside `BaseImpl`
// without touching the reader.
//
// Leaf module: imports nothing from wm/, state, or components.

/** A style span over the section text, UTF-16 offsets (the same basis as
 *  highlight spans and nostrdown refs). `cls` is a short token; RichContent
 *  renders it as `ly-<cls>`. Spans may nest and overlap freely. */
export type StyleSpan = { start: number; end: number; cls: string };

/** What a loaded base contributes: spans over the text. Pure + synchronous
 *  (the parse runs inside a `$derived`). */
export interface BaseImpl {
	spans(text: string): StyleSpan[];
}

export type LayerKind = 'base' | 'grammar' | 'block';

export interface Layer {
	id: string;
	kind: LayerKind;
	/** Settings / picker label. */
	label: string;
	/** Two-or-three-letter toolbar label. */
	short: string;
	/** Canonical mimetypes this base renders (engine vocabulary). */
	mimetypes: string[];
	/** Dynamic import — the implementation lands in its own chunk and is only
	 *  fetched once a user enables the layer and a matching section renders. */
	load(): Promise<BaseImpl>;
}

/** Reserved pseudo-layer id meaning "explicitly none": usable as a binding
 *  value ("never decorate text/markdown") and as the per-buffer pick. Never
 *  appears in `enabled`. */
export const SOURCE_ID = 'source';

/** The engine's canonical content mimetypes (src/mimetype.rs). */
export const CANONICAL_MIMETYPES = [
	'text/markdown',
	'text/org',
	'text/asciidoc',
	'text/plain'
] as const;
export type CanonicalMimetype = (typeof CANONICAL_MIMETYPES)[number];

/** Short names the composer / publish API accept, keyed by mimetype. */
export const FORMAT_SHORT: Record<CanonicalMimetype, string> = {
	'text/markdown': 'markdown',
	'text/org': 'org',
	'text/asciidoc': 'asciidoc',
	'text/plain': 'plain'
};
export const FORMAT_LABEL: Record<CanonicalMimetype, string> = {
	'text/markdown': 'Markdown',
	'text/org': 'Org',
	'text/asciidoc': 'AsciiDoc',
	'text/plain': 'Plain text'
};

/** The composer's default: its own heading syntax is Markdown. */
export const DEFAULT_COMPOSE_FORMAT: CanonicalMimetype = 'text/markdown';

export function isCanonicalMimetype(v: unknown): v is CanonicalMimetype {
	return typeof v === 'string' && (CANONICAL_MIMETYPES as readonly string[]).includes(v);
}

/** Human label for a mimetype (falls back to the raw string). */
export function formatLabel(mimetype: string | null | undefined): string {
	if (!mimetype) return 'no format';
	return isCanonicalMimetype(mimetype) ? FORMAT_LABEL[mimetype] : mimetype;
}

const memo = new Map<string, Promise<BaseImpl>>();
function once(id: string, load: () => Promise<BaseImpl>): () => Promise<BaseImpl> {
	return () => {
		let p = memo.get(id);
		if (!p) {
			p = load();
			memo.set(id, p);
		}
		return p;
	};
}

export const LAYERS: readonly Layer[] = [
	{
		id: 'md-decorated',
		kind: 'base',
		label: 'Markdown (decorated source)',
		short: 'md',
		mimetypes: ['text/markdown'],
		load: once('md-decorated', () => import('./bases/markdown').then((m) => m.base))
	}
];

export function layerById(id: string | null | undefined): Layer | undefined {
	if (!id || id === SOURCE_ID) return undefined;
	return LAYERS.find((l) => l.id === id);
}

export function isLayerId(id: unknown): id is string {
	return typeof id === 'string' && LAYERS.some((l) => l.id === id);
}

/** Bases registered for a mimetype, in registry order. */
export function basesFor(mimetype: string | null | undefined): Layer[] {
	if (!mimetype) return [];
	return LAYERS.filter((l) => l.kind === 'base' && l.mimetypes.includes(mimetype));
}
