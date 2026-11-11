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

import type { HighlightSpan, Highlighter } from '$lib/discussions/highlights';

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
	/** Rendering-layer style classes covering this run (see lib/layers —
	 *  applied as a post-pass, never part of the merge below). */
	styles?: string[];
}

/** One renderable run of a section's content. */
export type ContentSegment = SegmentSource &
	(
		| { type: 'text'; text: string }
		| {
				type: 'highlight';
				text: string;
				/** `id`/`pubkey`/`highlighter` = the top layer; `layers` = every
				 *  highlight covering this run, bottom → top (overlapping highlights
				 *  split into runs, each carrying all of its covers). */
				highlight: {
					id: string;
					pubkey: string;
					highlighter?: Highlighter;
					focused: boolean;
					layers: HighlightLayer[];
				};
				/** The runs *inside* the mark — plain text plus any nostrdown ref /
				 *  token the highlight spans. A highlight is a decoration over the
				 *  text, not a replacement of it, so a `[[wikilink]]` under a
				 *  highlight stays a live link. Never contains another highlight. */
				parts: ContentSegment[];
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

/** One highlight under a run of text. */
export interface HighlightLayer {
	id: string;
	pubkey: string;
	highlighter?: Highlighter;
}

/** Split possibly-overlapping highlight spans into non-overlapping runs, each
 *  carrying every highlight that covers it (bottom → top: outer, earlier-
 *  starting spans first). Adjacent runs with the same cover set merge. */
export function flattenHighlightSpans(
	spans: HighlightSpan[]
): { start: number; end: number; layers: HighlightLayer[] }[] {
	const live = spans
		.filter((s) => s.end > s.start)
		.sort((a, b) => a.start - b.start || b.end - a.end || a.id.localeCompare(b.id));
	if (live.length === 0) return [];
	const cuts = [...new Set(live.flatMap((s) => [s.start, s.end]))].sort((a, b) => a - b);
	const out: { start: number; end: number; layers: HighlightLayer[] }[] = [];
	for (let k = 0; k + 1 < cuts.length; k++) {
		const [a, b] = [cuts[k], cuts[k + 1]];
		const covers = live.filter((s) => s.start <= a && s.end >= b);
		if (covers.length === 0) continue;
		const layers = covers.map(({ id, pubkey, highlighter }) => ({ id, pubkey, highlighter }));
		const prev = out[out.length - 1];
		if (
			prev &&
			prev.end === a &&
			prev.layers.length === layers.length &&
			prev.layers.every((l, i) => l.id === layers[i].id)
		) {
			prev.end = b;
		} else {
			out.push({ start: a, end: b, layers });
		}
	}
	return out;
}

type Overlay = {
	start: number;
	end: number;
	/** Higher wins when two overlays start at the same offset. */
	prio: number;
} & (
	| { kind: 'atom'; make: () => ContentSegment }
	| { kind: 'highlight'; layers: HighlightLayer[] }
);

/**
 * Slice section `content` into plain/highlighted/reference runs, merging the
 * engine's resolved highlight `spans` and nostrdown `refs` (both UTF-16 offsets
 * into the same text) onto one segmentation. Refs and tokens are *atoms*:
 * each replaces its `{{…}}` / `[[…]]` span entirely and never nests. A
 * highlight is a *decoration*: its segment carries `parts` — the text runs and
 * atoms it covers — so a wikilink under a highlight is still a link. A
 * highlight that starts inside an atom begins after it; one that ends inside an
 * atom extends to the atom's end (atoms don't split). Highlights don't nest:
 * a highlight overlapping an already-emitted one is dropped.
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
			kind: 'atom',
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
			kind: 'atom',
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
	// Overlapping highlights are pre-split into non-overlapping runs, each
	// carrying all of its covering highlights — the merge below then never
	// sees two highlights over the same text.
	for (const run of flattenHighlightSpans(spans)) {
		overlays.push({ kind: 'highlight', start: run.start, end: run.end, prio: 1, layers: run.layers });
	}
	// The no-overlay early-out sits AFTER the spans loop deliberately: it used
	// to sit before it, which silently dropped highlight spans on any section
	// with no nostrdown refs/tokens — highlights only rendered next to refs.
	if (overlays.length === 0)
		return [{ type: 'text', text: content, srcStart: 0, srcEnd: content.length }];

	// Earliest start first. On a tie a highlight leads (it opens the mark and
	// the atom at the same offset becomes its first part — a highlight that
	// is exactly a wikilink still wraps it); between atoms the higher priority
	// wins, so a resolved ref beats the raw token it replaces.
	overlays.sort(
		(a, b) =>
			a.start - b.start ||
			Number(b.kind === 'highlight') - Number(a.kind === 'highlight') ||
			b.prio - a.prio
	);

	const text = (start: number, end: number): ContentSegment => ({
		type: 'text',
		text: content.slice(start, end),
		srcStart: start,
		srcEnd: end
	});

	const segments: ContentSegment[] = [];
	let cursor = 0;
	let i = 0;
	while (i < overlays.length) {
		const o = overlays[i];
		if (o.kind === 'atom') {
			i++;
			if (o.start < cursor) continue; // inside an already-emitted run → drop
			if (o.start > cursor) segments.push(text(cursor, o.start));
			segments.push(o.make());
			cursor = o.end;
			continue;
		}
		// A highlight. Its mark starts where the text is still unclaimed (a
		// highlight beginning inside an atom starts after it) and ends at its
		// own end — or at the end of the last atom it reaches into.
		const start = Math.max(o.start, cursor);
		let end = o.end;
		i++;
		if (start >= end) continue; // wholly inside a claimed run (another highlight / atom) → drop
		const parts: ContentSegment[] = [];
		let pc = start;
		while (i < overlays.length && overlays[i].start < end) {
			const inner = overlays[i];
			i++;
			// A highlight overlapping this one is dropped — marks don't nest.
			if (inner.kind !== 'atom') continue;
			if (inner.start < pc) continue; // overlaps an atom already inside the mark
			if (inner.start > pc) parts.push(text(pc, inner.start));
			parts.push(inner.make());
			pc = inner.end;
			if (pc > end) end = pc; // the atom straddles the highlight's end — extend
		}
		if (pc < end) parts.push(text(pc, end));
		if (start > cursor) segments.push(text(cursor, start));
		const top = o.layers[o.layers.length - 1];
		segments.push({
			type: 'highlight',
			text: content.slice(start, end),
			parts,
			highlight: {
				id: top.id,
				pubkey: top.pubkey,
				highlighter: top.highlighter,
				focused:
					focusedLower !== null && o.layers.some((l) => l.id.toLowerCase() === focusedLower),
				layers: o.layers
			},
			srcStart: start,
			srcEnd: end
		});
		cursor = end;
	}
	if (cursor < content.length) segments.push(text(cursor, content.length));
	return segments;
}
