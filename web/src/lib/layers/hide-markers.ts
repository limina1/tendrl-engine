// Hidden markers — the reader's twin of the composer's `rendered` view
// (editor/wysiwyg.ts `buildDecorations`, minus the cursor-line exception).
//
// Given a base's style spans over the text, emit EXTRA spans that hide the
// markup sigils: `hide` (display: none), `bullet` (a `-`/`*`/`+` list mark
// drawn as •) and `listnum` (a numbered mark, shown at full strength). The
// text itself is never touched — the hidden runs stay in the DOM with their
// `data-src-*` offsets, so highlight capture and nostrdown refs keep their
// 1:1 mapping. Pure; leaf module.

import type { StyleSpan } from './registry';

const LIST_MARK = /^\s*([-+*]|\d+[.)])$/;
const HEADING = /^h[1-6]$/;

function lineStart(text: string, at: number): number {
	return text.lastIndexOf('\n', at - 1) + 1;
}

export function hiddenMarkerSpans(text: string, spans: StyleSpan[]): StyleSpan[] {
	const out: StyleSpan[] = [];
	// Lines that are headings — an Org `* Title` star is a heading sigil
	// there, never a list bullet.
	const headingLines = new Set<number>();
	for (const s of spans) {
		if (HEADING.test(s.cls) && s.start < text.length) headingLines.add(lineStart(text, s.start));
	}
	for (const s of spans) {
		if (s.end <= s.start || s.end > text.length) continue;
		if (s.cls === 'url') {
			// A labelled link's target hides behind its label:
			// md `[label](url)`, org `[[url][label]]`, adoc `url[label]`.
			const before = text.slice(Math.max(0, s.start - 2), s.start);
			const after = text.slice(s.end, s.end + 2);
			const labelled =
				before.endsWith('(') ||
				(before === '[[' && after === '][') ||
				(after.startsWith('[') && after !== '[]');
			if (labelled) out.push({ start: s.start, end: s.end, cls: 'hide' });
			continue;
		}
		if (s.cls !== 'mark') continue;
		const sigil = text.slice(s.start, s.end);
		// Multi-line runs and table pipes stay (dimmed): hiding them would
		// collapse the structure they carry.
		if (sigil.includes('\n') || sigil.includes('|')) continue;
		const ls = lineStart(text, s.start);
		const atLineStart = text.slice(ls, s.start).trim() === '';
		if (atLineStart && LIST_MARK.test(sigil) && !headingLines.has(ls)) {
			out.push({ start: s.start, end: s.end, cls: /\d/.test(sigil) ? 'listnum' : 'bullet' });
			continue;
		}
		// A leading sigil (heading `## `, quote `> `) takes its trailing space.
		const end = atLineStart && text[s.end] === ' ' ? s.end + 1 : s.end;
		out.push({ start: s.start, end, cls: 'hide' });
	}
	return out;
}
