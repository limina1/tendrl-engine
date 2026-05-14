// Multi-highlight rendering. Pure logic — no engine, no DOM.
//
// A section may carry many NIP-84 highlights: per-section ones tagging
// the 30041 addr directly, plus publication-level ones tagging the
// 30040 root that cascade down to whichever section's content they
// match. Each must render as its own <mark> span in the author's hue.
//
// Today we substring-match (no offset tags yet — see the plan doc's
// gap #5). When two highlights overlap, the longer one wins; the
// shorter is dropped. Composite stacks (the plan's stacked left-border
// stripes for overlapping authors) belong with the offset-aware pass.

export interface Highlight {
	/** kind-9802 event id. */
	id: string;
	/** Highlighted text — the `content` field of the 9802 event. */
	content: string;
	/** Author pubkey, for per-author color. */
	pubkey: string;
}

export interface HighlightSegment {
	text: string;
	/** Non-null means this slice should render inside a <mark>. */
	highlight: { id: string; pubkey: string; focused: boolean } | null;
}

/**
 * Split section content into a list of plain/highlighted segments
 * suitable for rendering as a <pre> with inline <mark> spans.
 *
 * `focusedId` (typically the `?highlight=<id>` marker) gets the
 * `focused` flag on its segment so the renderer can add an emphasis
 * ring without disturbing other overlays.
 */
export function computeHighlightSegments(
	content: string,
	highlights: Highlight[],
	focusedId: string | null = null
): HighlightSegment[] {
	if (!content) return [{ text: '', highlight: null }];
	if (highlights.length === 0) return [{ text: content, highlight: null }];

	const lower = content.toLowerCase();
	const focusedLower = focusedId ? focusedId.toLowerCase() : null;

	type Match = {
		start: number;
		end: number;
		id: string;
		pubkey: string;
		focused: boolean;
		// Length is used for overlap arbitration; longer match wins.
		len: number;
	};

	// Longer highlights win against shorter ones when they overlap.
	// Sort DESC by length first; ties broken by id for determinism.
	const ordered = [...highlights].sort((a, b) => {
		const da = b.content.length - a.content.length;
		return da !== 0 ? da : a.id.localeCompare(b.id);
	});

	const matches: Match[] = [];
	for (const hl of ordered) {
		const needle = hl.content.trim();
		if (!needle) continue;
		const needleLower = needle.toLowerCase();
		// Take the first non-overlapping occurrence. If a later highlight
		// would overlap one we've already claimed, skip it — this avoids
		// the visual mess of nested <mark>s without losing the longer
		// (more informative) match.
		let from = 0;
		while (true) {
			const idx = lower.indexOf(needleLower, from);
			if (idx < 0) break;
			const end = idx + needle.length;
			const overlaps = matches.some((m) => m.start < end && idx < m.end);
			if (!overlaps) {
				matches.push({
					start: idx,
					end,
					id: hl.id,
					pubkey: hl.pubkey,
					focused: focusedLower !== null && hl.id.toLowerCase() === focusedLower,
					len: needle.length
				});
				break;
			}
			from = idx + 1;
		}
	}

	matches.sort((a, b) => a.start - b.start);

	const segments: HighlightSegment[] = [];
	let cursor = 0;
	for (const m of matches) {
		if (m.start > cursor) {
			segments.push({ text: content.slice(cursor, m.start), highlight: null });
		}
		segments.push({
			text: content.slice(m.start, m.end),
			highlight: { id: m.id, pubkey: m.pubkey, focused: m.focused }
		});
		cursor = m.end;
	}
	if (cursor < content.length) {
		segments.push({ text: content.slice(cursor), highlight: null });
	}

	return segments;
}
