// Style spans → segments, as a post-pass over `buildSegments`' output.
//
// `buildSegments` (nostr/nostrdown.ts) owns the hard merge: nostrdown atoms
// replace their span, highlights wrap theirs. Style spans are deliberately
// NOT fed into that merge — they are a decoration that must survive any
// overlap (a highlight across a heading, a ref inside emphasis, a span
// straddling a mark boundary). So this pass runs afterwards and only ever
// SPLITS text runs at style boundaries and attaches class lists; it never
// drops, reorders, or re-slices anything. Atoms keep their extent and just
// pick up the classes covering them (a ref chip inside `# heading` sizes
// with the heading).
//
// Invariants kept: the concatenation of all segment texts is still the
// source, and every piece carries exact `srcStart`/`srcEnd`.

import type { ContentSegment } from '$lib/nostr/nostrdown';
import type { StyleSpan } from './registry';

function classesAt(spans: StyleSpan[], start: number, end: number): string[] | undefined {
	let out: string[] | undefined;
	for (const s of spans) {
		if (s.start > start) break; // sorted by start: nothing later can cover `start`
		if (s.end >= end && s.start <= start) {
			(out ??= []).push(s.cls);
		}
	}
	return out && [...new Set(out)];
}

function splitText(
	seg: Extract<ContentSegment, { type: 'text' }>,
	spans: StyleSpan[],
	cuts: number[]
): ContentSegment[] {
	const { srcStart, srcEnd } = seg;
	// Cut points strictly inside this run.
	let lo = 0;
	let hi = cuts.length;
	// cuts is sorted; binary-search the window (srcStart, srcEnd).
	while (lo < hi) {
		const mid = (lo + hi) >> 1;
		if (cuts[mid] <= srcStart) lo = mid + 1;
		else hi = mid;
	}
	const pieces: ContentSegment[] = [];
	let at = srcStart;
	for (let i = lo; i < cuts.length && cuts[i] < srcEnd; i++) {
		const c = cuts[i];
		if (c === at) continue;
		pieces.push(piece(seg, at, c, spans));
		at = c;
	}
	if (pieces.length === 0) {
		const styles = classesAt(spans, srcStart, srcEnd);
		return styles ? [{ ...seg, styles }] : [seg];
	}
	pieces.push(piece(seg, at, srcEnd, spans));
	return pieces;
}

function piece(
	seg: Extract<ContentSegment, { type: 'text' }>,
	start: number,
	end: number,
	spans: StyleSpan[]
): ContentSegment {
	const off = start - seg.srcStart;
	const out: ContentSegment = {
		type: 'text',
		text: seg.text.slice(off, off + (end - start)),
		srcStart: start,
		srcEnd: end
	};
	const styles = classesAt(spans, start, end);
	if (styles) out.styles = styles;
	return out;
}

/** Apply `styles` to `segments`. Returns the input untouched when there are
 *  no spans, so the no-layer path costs nothing. */
export function applyStyles(segments: ContentSegment[], styles: StyleSpan[]): ContentSegment[] {
	if (styles.length === 0) return segments;
	const spans = [...styles].sort((a, b) => a.start - b.start || b.end - a.end);
	const cutSet = new Set<number>();
	for (const s of spans) {
		cutSet.add(s.start);
		cutSet.add(s.end);
	}
	const cuts = [...cutSet].sort((a, b) => a - b);

	const walk = (segs: ContentSegment[]): ContentSegment[] => {
		const out: ContentSegment[] = [];
		for (const seg of segs) {
			if (seg.type === 'text') {
				out.push(...splitText(seg, spans, cuts));
			} else if (seg.type === 'highlight') {
				out.push({ ...seg, parts: walk(seg.parts) });
			} else {
				const cls = classesAt(spans, seg.srcStart, seg.srcEnd);
				out.push(cls ? { ...seg, styles: cls } : seg);
			}
		}
		return out;
	};
	return walk(segments);
}

/** `class` attribute value for a segment's styles (undefined when none). */
export function styleClass(styles: string[] | undefined): string | undefined {
	if (!styles || styles.length === 0) return undefined;
	let s = 'ly';
	for (const c of styles) s += ' ly-' + c;
	return s;
}
