// Helpers for the tokenizer tests: read spans back as the text they cover.
import type { StyleSpan } from '../registry';

/** The source slices covered by every span of class `cls`, in span order. */
export function covered(text: string, spans: StyleSpan[], cls: string): string[] {
	return spans
		.filter((s) => s.cls === cls)
		.sort((a, b) => a.start - b.start || a.end - b.end)
		.map((s) => text.slice(s.start, s.end));
}

/** Classes covering offset `i` (a quick "what is this character styled as"). */
export function classesAt(spans: StyleSpan[], i: number): string[] {
	return spans
		.filter((s) => s.start <= i && i < s.end)
		.map((s) => s.cls)
		.sort();
}

/** Every span lies inside the text and is non-empty. */
export function wellFormed(text: string, spans: StyleSpan[]): boolean {
	return spans.every((s) => s.start >= 0 && s.end <= text.length && s.end > s.start);
}
