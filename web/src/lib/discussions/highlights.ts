// NIP-84 highlight rendering helpers. Pure — no engine, no DOM.
//
// A section may carry many NIP-84 highlights: per-section ones tagging the
// 30041 addr directly, plus publication-level ones tagging the 30040 root that
// cascade down to whichever section's content they match. Each renders as its
// own <mark> span in the author's hue.
//
// RESOLUTION — finding *where* each highlight sits and arbitrating overlaps —
// now lives in the engine (`src/discussions.rs::resolve_highlight_spans`, via
// `POST /api/v1/highlights/resolve`), per the frontend/backend boundary. What
// remains here is the types + `segmentsFromSpans`: slicing the text by the
// engine's offset spans into renderable runs, plus applying focus (a view
// concern). The former `computeHighlightSegments` substring-matcher was deleted.

export interface Highlight {
	/** kind-9802 event id. */
	id: string;
	/** Highlighted text — the `content` field of the 9802 event. */
	content: string;
	/** Author pubkey, for per-author color. */
	pubkey: string;
	/** The event's `offset` tag (tendrl extension): UTF-16 `[start, end]`
	 *  into the pinned version's content. The engine verifies it against
	 *  the current text before trusting it — send when present. */
	offset?: [number, number];
	/** The event's `context` tag — disambiguates repeated phrases. */
	context?: string;
}

/** Build a resolver input from a raw kind-9802 event, carrying the
 *  offset/context tags through so the engine can pin exact positions
 *  (mirrors Rust `discussions::highlight_from_event`). */
export function highlightFromEvent(e: {
	id: string;
	content: string;
	pubkey: string;
	tags: string[][];
}): Highlight {
	let offset: [number, number] | undefined;
	let context: string | undefined;
	for (const t of e.tags) {
		if (t[0] === 'offset' && t.length >= 3) {
			const start = Number.parseInt(t[1], 10);
			const end = Number.parseInt(t[2], 10);
			if (Number.isFinite(start) && Number.isFinite(end)) offset = [start, end];
		} else if (t[0] === 'context' && t[1]) {
			context = t[1];
		}
	}
	return { id: e.id, content: e.content ?? '', pubkey: e.pubkey, offset, context };
}

/** A resolved highlight position from the engine. `start`/`end` are UTF-16
 *  code-unit offsets into the section text, so `content.slice(start, end)`
 *  yields the highlighted run exactly. */
export interface HighlightSpan {
	start: number;
	end: number;
	id: string;
	pubkey: string;
}

export interface HighlightSegment {
	text: string;
	/** Non-null means this slice should render inside a <mark>. */
	highlight: { id: string; pubkey: string; focused: boolean } | null;
}

/**
 * Slice section `content` into plain/highlighted segments using the engine's
 * resolved `spans` (non-overlapping, sorted by start — but we re-sort
 * defensively). Suitable for rendering as a <pre> with inline <mark> spans.
 *
 * `focusedId` (typically the `?highlight=<id>` marker) flags its span's segment
 * so the renderer can add an emphasis ring without disturbing other overlays.
 */
export function segmentsFromSpans(
	content: string,
	spans: HighlightSpan[],
	focusedId: string | null = null
): HighlightSegment[] {
	if (!content) return [{ text: '', highlight: null }];
	if (spans.length === 0) return [{ text: content, highlight: null }];

	const focusedLower = focusedId ? focusedId.toLowerCase() : null;
	const ordered = [...spans].sort((a, b) => a.start - b.start);

	const segments: HighlightSegment[] = [];
	let cursor = 0;
	for (const s of ordered) {
		// Skip a span that would overlap what we've already emitted (the engine
		// guarantees non-overlap, but stay defensive against bad input).
		if (s.start < cursor) continue;
		if (s.start > cursor) {
			segments.push({ text: content.slice(cursor, s.start), highlight: null });
		}
		segments.push({
			text: content.slice(s.start, s.end),
			highlight: {
				id: s.id,
				pubkey: s.pubkey,
				focused: focusedLower !== null && s.id.toLowerCase() === focusedLower
			}
		});
		cursor = s.end;
	}
	if (cursor < content.length) {
		segments.push({ text: content.slice(cursor), highlight: null });
	}

	return segments;
}
