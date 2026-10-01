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
	const tree = parser.parse(text);
	tree.iterate({
		enter(node) {
			const cls = CLASS[node.name];
			if (cls && node.to > node.from) out.push({ start: node.from, end: node.to, cls });
		}
	});
	return out;
}

export const base: BaseImpl = { spans };
