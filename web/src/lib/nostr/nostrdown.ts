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

/** A `{{ }}` reference after engine resolution. `start`/`end` are UTF-16
 *  code-unit offsets into the section text spanning the whole token, so the
 *  renderer can replace `content.slice(start, end)` wholesale. */
export interface ResolvedRef {
	kind: 'ref' | 'wiki' | 'embed' | 'quote' | 'mention' | 'slot';
	start: number;
	end: number;
	/** Canonical lookup target (normalized slug or bech32 entity). */
	target: string;
	/** Text to render: explicit `|display`, else the resolved title, else the
	 *  raw target as written. */
	label: string;
	/** True when the target resolved to a known address/event. */
	found: boolean;
	/** True when the address is valid (`found`) but the event isn't local yet —
	 *  the card offers a relay fetch (auto in Auto mode, a click in Confirm). */
	pending?: boolean;
	/** NIP-19 entity to navigate to, when resolved. */
	naddr?: string;
	/** `"kind:pubkey:dtag"` coordinate for in-app navigation, when addressable. */
	coord?: string;
	/** Hex event id for an `nevent`/`note` target — navigation for
	 *  non-addressable events (the event modal), which have no coordinate. */
	event_id?: string;
	/** Kind of the resolved event, when known. */
	event_kind?: number;
	/** Transcluded content for `embed` (depth-1). */
	content?: string;

	// Preview metadata for an editor hover/click card.
	/** The resolved event's own `title` tag (distinct from `label`). */
	title?: string;
	/** The resolved event's `summary`/`description`, capped. */
	summary?: string;
	/** Embed-card image: a document's image/thumb tag, or a profile picture. */
	image?: string;
	/** Cited work author — the `["author", …]` tag. */
	author?: string;
	/** Publishing pubkey (the "index author"). */
	author_pubkey?: string;
	/** The resolved event's `created_at`. */
	created_at?: number;
}

/** Where a segment came from in the source `content` (UTF-16 offsets).
 *  Text/highlight segments render the exact `content.slice(srcStart, srcEnd)`;
 *  ref/token segments render something else in that span's place (a chip or
 *  card). Selection capture maps DOM positions back to content offsets through
 *  these — the rendered DOM is NOT offset-faithful on its own. */
export interface SegmentSource {
	srcStart: number;
	srcEnd: number;
}

/** One renderable run of a section's content. */
export type ContentSegment = SegmentSource &
	(
		| { type: 'text'; text: string }
		| {
				type: 'highlight';
				text: string;
				highlight: { id: string; pubkey: string; focused: boolean };
		  }
		| { type: 'ref'; ref: ResolvedRef }
		/** A raw `{{ }}` token the engine hasn't resolved yet (resolution is async,
		 *  or hasn't run). Rendered as a "resolving" reference chip so the syntax
		 *  reads as a reference, not plain text, before the resolved `ref` lands. */
		| { type: 'token'; kind: string; target: string; display?: string; raw: string }
	);

/** A parsed `{{ }}`/`[[ ]]` token from the engine (`POST /api/v1/nostrdown/parse`)
 *  — the locate-and-classify surface, UTF-16 offsets. The engine grammar is the
 *  single source; the reader marks these as "resolving" chips before `/resolve`
 *  lands, the editor decorates them. Mirrors Rust `nostrdown::ParsedToken`. */
export interface ParsedToken {
	kind: 'ref' | 'wiki' | 'embed' | 'quote' | 'mention' | 'slot';
	/** Normalized lookup target (NIP-54 slug or bech32 entity). */
	target: string;
	/** Target exactly as written (trimmed), pre-normalization. */
	raw_target: string;
	display?: string;
	/** UTF-16 offsets spanning the whole token, delimiters included. */
	start: number;
	end: number;
}

/** Split an addressable coordinate `"kind:pubkey:d_tag"` (the d-tag may itself
 *  contain `:`), or null if it isn't one. */
export function splitCoord(
	coord: string
): { kind: string; pubkey: string; d_tag: string } | null {
	const i1 = coord.indexOf(':');
	const i2 = coord.indexOf(':', i1 + 1);
	if (i1 < 0 || i2 < 0) return null;
	return { kind: coord.slice(0, i1), pubkey: coord.slice(i1 + 1, i2), d_tag: coord.slice(i2 + 1) };
}

/** Does a resolved ref's `coord` address this section? Exact match, or — when
 *  either side lacks a pubkey (a draft sibling's synthetic coordinate is
 *  `"30041::<id>"`) — a kind + d-tag match. Used for in-document navigation:
 *  a ref that lands on a section of the same view scrolls/pages to it. */
export function coordMatchesAddr(
	coord: string,
	addr: { kind: number; pubkey: string; d_tag: string }
): boolean {
	const c = splitCoord(coord);
	if (!c || c.kind !== String(addr.kind) || c.d_tag !== addr.d_tag) return false;
	return c.pubkey === addr.pubkey || c.pubkey === '' || addr.pubkey === '';
}

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
	tokens: ParsedToken[] = [],
	focusedId: string | null = null
): ContentSegment[] {
	if (!content) return [{ type: 'text', text: '', srcStart: 0, srcEnd: 0 }];

	const focusedLower = focusedId ? focusedId.toLowerCase() : null;
	const overlays: Overlay[] = [];
	for (const ref of refs) {
		overlays.push({
			start: ref.start,
			end: ref.end,
			prio: 2,
			make: () => ({ type: 'ref', ref, srcStart: ref.start, srcEnd: ref.end })
		});
	}
	// Engine-parsed `{{ }}`/`[[ ]]` tokens the resolver hasn't returned yet —
	// lowest priority, so a resolved `ref` (or highlight) covering the same span
	// always wins and these only surface in the pre-resolution window. Offsets are
	// the same UTF-16 unit refs use, so the merge below is uniform.
	for (const t of tokens) {
		overlays.push({
			start: t.start,
			end: t.end,
			prio: 0,
			make: () => ({
				type: 'token',
				kind: t.kind,
				target: t.target,
				display: t.display,
				raw: content.slice(t.start, t.end),
				srcStart: t.start,
				srcEnd: t.end
			})
		});
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
				},
				srcStart: s.start,
				srcEnd: s.end
			})
		});
	}
	// The no-overlay early-out sits AFTER the spans loop deliberately: it used
	// to sit before it, which silently dropped highlight spans on any section
	// with no nostrdown refs/tokens — highlights only rendered next to refs.
	if (overlays.length === 0)
		return [{ type: 'text', text: content, srcStart: 0, srcEnd: content.length }];

	// Earliest start first; on a tie the higher-priority overlay (ref) leads so
	// it claims the run and the highlight is dropped on the overlap check.
	overlays.sort((a, b) => a.start - b.start || b.prio - a.prio);

	const segments: ContentSegment[] = [];
	let cursor = 0;
	for (const o of overlays) {
		if (o.start < cursor) continue; // overlaps an already-emitted run → drop
		if (o.start > cursor)
			segments.push({
				type: 'text',
				text: content.slice(cursor, o.start),
				srcStart: cursor,
				srcEnd: o.start
			});
		segments.push(o.make());
		cursor = o.end;
	}
	if (cursor < content.length)
		segments.push({
			type: 'text',
			text: content.slice(cursor),
			srcStart: cursor,
			srcEnd: content.length
		});
	return segments;
}
