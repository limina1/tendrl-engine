// Nostrdown `{{ }}` reference rendering helpers. Pure — no engine, no DOM.
//
// PARSING + RESOLUTION (tokenizing `{{ref|wiki|embed:…}}` and looking each
// target up against sibling events / the db / relays) lives in the engine
// (`src/nostrdown.rs` + `PublicationEngine::resolve_refs`, via
// `POST /api/v1/nostrdown/resolve`), per the frontend/backend boundary — the
// same split as NIP-84 highlights. What remains here is the rendering side:
// the `ResolvedRef` type and `buildSegments`, which slices section text into
// renderable runs, merging nostrdown refs with highlight spans onto one
// non-overlapping segmentation.

import type { HighlightSpan } from '$lib/discussions/highlights';

/**
 * NIP-54 slug normalization — a JS mirror of the engine's `nostrdown::normalize`
 * (the Rust side is the source of truth for *stored* data; this is used only for
 * editor-side matching, e.g. resolving a `{{ref:slug}}` against a heading title
 * while drafting). Lowercase; whitespace and `-`/`_` collapse to a single `-`;
 * other punctuation dropped; trailing `-` trimmed; non-ASCII letters preserved.
 */
export function normalizeSlug(s: string): string {
	let out = '';
	let lastDash = false;
	for (const ch of s.toLowerCase()) {
		if (/[\p{L}\p{N}]/u.test(ch)) {
			out += ch;
			lastDash = false;
		} else if (/\s/.test(ch) || ch === '-' || ch === '_') {
			if (out && !lastDash) {
				out += '-';
				lastDash = true;
			}
		}
		// other punctuation/symbols are dropped without a separator
	}
	return out.replace(/-+$/, '');
}

/** A `{{ }}` reference after engine resolution. `start`/`end` are UTF-16
 *  code-unit offsets into the section text spanning the whole token, so the
 *  renderer can replace `content.slice(start, end)` wholesale. */
export interface ResolvedRef {
	kind: 'ref' | 'wiki' | 'embed';
	start: number;
	end: number;
	/** Canonical lookup target (normalized slug or bech32 entity). */
	target: string;
	/** Heading anchor to scroll to after navigation, if any. */
	fragment?: string;
	/** Text to render: explicit `|display`, else the resolved title, else the
	 *  raw target as written. */
	label: string;
	/** True when the target resolved to a known address/event. */
	found: boolean;
	/** NIP-19 entity to navigate to, when resolved. */
	naddr?: string;
	/** `"kind:pubkey:dtag"` coordinate for in-app navigation, when addressable. */
	coord?: string;
	/** Kind of the resolved event, when known. */
	event_kind?: number;
	/** Transcluded content for `embed` (depth-1). */
	content?: string;
}

/** One renderable run of a section's content. */
export type ContentSegment =
	| { type: 'text'; text: string }
	| {
			type: 'highlight';
			text: string;
			highlight: { id: string; pubkey: string; focused: boolean };
	  }
	| { type: 'ref'; ref: ResolvedRef };

interface Overlay {
	start: number;
	end: number;
	/** Higher wins when two overlays start at the same offset. */
	prio: number;
	make: () => ContentSegment;
}

/**
 * Slice section `content` into plain/highlighted/reference runs, merging the
 * engine's resolved highlight `spans` and nostrdown `refs` (both UTF-16 offsets
 * into the same text) onto one non-overlapping segmentation. On overlap a
 * nostrdown ref wins (it replaces its `{{…}}` token entirely); a highlight that
 * would intersect an already-claimed run is dropped.
 *
 * `focusedId` (typically the `?highlight=<id>` marker) flags its highlight's
 * segment so the renderer can add an emphasis ring.
 */
export function buildSegments(
	content: string,
	spans: HighlightSpan[],
	refs: ResolvedRef[],
	focusedId: string | null = null
): ContentSegment[] {
	if (!content) return [{ type: 'text', text: '' }];
	if (spans.length === 0 && refs.length === 0) return [{ type: 'text', text: content }];

	const focusedLower = focusedId ? focusedId.toLowerCase() : null;
	const overlays: Overlay[] = [];
	for (const ref of refs) {
		overlays.push({ start: ref.start, end: ref.end, prio: 2, make: () => ({ type: 'ref', ref }) });
	}
	for (const s of spans) {
		overlays.push({
			start: s.start,
			end: s.end,
			prio: 1,
			make: () => ({
				type: 'highlight',
				text: content.slice(s.start, s.end),
				highlight: {
					id: s.id,
					pubkey: s.pubkey,
					focused: focusedLower !== null && s.id.toLowerCase() === focusedLower
				}
			})
		});
	}
	// Earliest start first; on a tie the higher-priority overlay (ref) leads so
	// it claims the run and the highlight is dropped on the overlap check.
	overlays.sort((a, b) => a.start - b.start || b.prio - a.prio);

	const segments: ContentSegment[] = [];
	let cursor = 0;
	for (const o of overlays) {
		if (o.start < cursor) continue; // overlaps an already-emitted run → drop
		if (o.start > cursor) segments.push({ type: 'text', text: content.slice(cursor, o.start) });
		segments.push(o.make());
		cursor = o.end;
	}
	if (cursor < content.length) segments.push({ type: 'text', text: content.slice(cursor) });
	return segments;
}
