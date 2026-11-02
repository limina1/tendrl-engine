// Markdown decorated-source base — @lezer/markdown (GFM) → style spans.
//
// The ONLY file that imports the parser, so it lands in its own chunk and is
// fetched the first time a user with this layer enabled opens a Markdown
// section. Lezer positions are JS string indices (UTF-16 code units) — the
// same basis `buildSegments` and highlight capture use, so no conversion.
//
// The text is never rewritten: markup sigils (`#`, `**`, backticks, `>`,
// list bullets) get the `mark` class and are dimmed by CSS, never hidden,
// so every source character stays selectable at its true offset.

import { parser as md, GFM } from '@lezer/markdown';
import type { BaseImpl, StyleSpan } from '../registry';

const parser = md.configure(GFM);

/** Lezer node name → style class (rendered as `ly-<cls>`). Unlisted nodes
 *  (Paragraph, Document, ListItem, …) contribute nothing. */
const CLASS: Record<string, string> = {
	ATXHeading1: 'h1',
	ATXHeading2: 'h2',
	ATXHeading3: 'h3',
	ATXHeading4: 'h4',
	ATXHeading5: 'h5',
	ATXHeading6: 'h6',
	SetextHeading1: 'h1',
	SetextHeading2: 'h2',
	Emphasis: 'em',
	StrongEmphasis: 'strong',
	InlineCode: 'code',
	FencedCode: 'fence',
	CodeBlock: 'fence',
	CodeInfo: 'lang',
	Link: 'link',
	Image: 'link',
	URL: 'url',
	Autolink: 'url',
	Blockquote: 'quote',
	HorizontalRule: 'hr',
	Strikethrough: 'strike',
	Table: 'table',
	TableHeader: 'th',
	// Sigils — dimmed, never removed.
	HeaderMark: 'mark',
	QuoteMark: 'mark',
	ListMark: 'mark',
	LinkMark: 'mark',
	EmphasisMark: 'mark',
	CodeMark: 'mark',
	StrikethroughMark: 'mark',
	TableDelimiter: 'mark',
	TaskMarker: 'mark',
	HTMLTag: 'mark',
	HTMLBlock: 'mark'
};

function spans(text: string): StyleSpan[] {
	const out: StyleSpan[] = [];
	const paragraphs: [number, number][] = [];
	const hard: [number, number][] = [];
	const tree = parser.parse(text);
	tree.iterate({
		enter(node) {
			const cls = CLASS[node.name];
			if (cls && node.to > node.from) out.push({ start: node.from, end: node.to, cls });
			if (node.name === 'Paragraph') paragraphs.push([node.from, node.to]);
			else if (node.name === 'HardBreak') hard.push([node.from, node.to]);
		}
	});
	// Soft line breaks: a newline inside a paragraph is a space in
	// CommonMark, not a line break. The character stays (offsets!) and is
	// rendered collapsible via `.ly-soft`; a HardBreak (`  \n` / `\\\n`)
	// keeps its newline.
	for (const [from, to] of paragraphs) {
		for (let i = text.indexOf('\n', from); i >= 0 && i < to; i = text.indexOf('\n', i + 1)) {
			if (hard.some(([a, b]) => a <= i && i < b)) continue;
			// Cover the continuation line's indentation too (wrapped list items).
			let e = i + 1;
			while (e < to && (text[e] === ' ' || text[e] === '\t')) e++;
			out.push({ start: i, end: e, cls: 'soft' });
		}
	}
	return out;
}

export const base: BaseImpl = { spans };
