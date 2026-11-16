// The composer's per-style syntax: heading lines, code fences, and the text
// transforms behind the formatting toolbar. Pure — no CodeMirror, no state —
// so the editor commands (wysiwyg.ts) and the outline parser share one
// definition of "what a heading looks like in this style".
//
// Styles are the engine's canonical mimetypes. `text/plain` has no markup of
// its own, so the composer keeps its historical `=` heading lines for it
// (the same lines AsciiDoc uses); everything else is the style's own syntax.

export type Style = 'text/markdown' | 'text/org' | 'text/asciidoc' | 'text/plain';

export function asStyle(v: string | null | undefined): Style {
	switch (v) {
		case 'text/org':
		case 'text/asciidoc':
		case 'text/plain':
			return v;
		default:
			return 'text/markdown';
	}
}

/** The character a heading line repeats, per style. */
export function headingChar(style: Style): string {
	switch (style) {
		case 'text/markdown':
			return '#';
		case 'text/org':
			return '*';
		default:
			return '=';
	}
}

/** `## ` / `** ` / `== ` — the prefix of a level-N heading line. */
export function headingPrefix(style: Style, level: number): string {
	return headingChar(style).repeat(Math.max(1, Math.min(6, level))) + ' ';
}

/** A heading line's level and title, or null. An empty title still counts
 *  (`## ` alone is a heading being typed). */
export function parseHeading(line: string, style: Style): { level: number; title: string } | null {
	const c = headingChar(style);
	let i = 0;
	while (i < line.length && line[i] === c) i++;
	if (i < 1 || i > 6) return null;
	if (i === line.length) return null; // `##` with no space yet
	if (line[i] !== ' ') return null;
	return { level: i, title: line.slice(i + 1).trim() };
}

/** Opening / closing fence of a verbatim block, per style. Returns the token
 *  that closes it, or null when `line` opens nothing. Heading-looking lines
 *  inside a fence are content. */
export function fenceOpen(line: string, style: Style): string | null {
	const t = line.trim();
	if (style === 'text/markdown') {
		const m = t.match(/^(`{3,}|~{3,})/);
		return m ? m[1] : null;
	}
	if (style === 'text/org') {
		const m = t.match(/^#\+begin_(\w+)/i);
		return m ? `#+end_${m[1].toLowerCase()}` : null;
	}
	// AsciiDoc / plain: listing, literal, passthrough, example, sidebar, quote
	const m = t.match(/^(-{4,}|\.{4,}|\+{4,}|={4,}|\*{4,}|_{4,}|`{3,})$/);
	return m ? m[1] : null;
}

export function fenceCloses(line: string, close: string, style: Style): boolean {
	const t = line.trim();
	if (style === 'text/org') return t.toLowerCase().startsWith(close);
	if (style === 'text/markdown') return t.startsWith(close[0].repeat(close.length)) && /^(`+|~+)\s*$/.test(t);
	return t === close;
}

/** Heading lines of a document (outside fences), with their line indexes. */
export function headingLines(
	text: string,
	style: Style
): { line: number; level: number; title: string }[] {
	const out: { line: number; level: number; title: string }[] = [];
	let close: string | null = null;
	text.split('\n').forEach((l, i) => {
		if (close) {
			if (fenceCloses(l, close, style)) close = null;
			return;
		}
		const open = fenceOpen(l, style);
		if (open) {
			close = open;
			return;
		}
		const h = parseHeading(l, style);
		if (h) out.push({ line: i, ...h });
	});
	return out;
}

// --- Toolbar transforms ---------------------------------------------------

/** Set (or, when it already is that level, clear) a line's heading level. */
export function setHeadingLevel(line: string, style: Style, level: number): string {
	const h = parseHeading(line, style);
	const body = h ? line.slice(line.indexOf(' ') + 1) : line.replace(/^\s+/, '');
	if (h && h.level === level) return body;
	return headingPrefix(style, level) + body;
}

export type Inline = 'bold' | 'italic' | 'code' | 'strike';

/** The wrapping delimiters of an inline style. */
export function inlineMarks(style: Style, kind: Inline): [string, string] {
	const t: Record<Style, Record<Inline, string>> = {
		'text/markdown': { bold: '**', italic: '*', code: '`', strike: '~~' },
		'text/org': { bold: '*', italic: '/', code: '~', strike: '+' },
		'text/asciidoc': { bold: '*', italic: '_', code: '`', strike: '[line-through]#' },
		'text/plain': { bold: '*', italic: '_', code: '`', strike: '~' }
	};
	const open = t[style][kind];
	const close = style === 'text/asciidoc' && kind === 'strike' ? '#' : open;
	return [open, close];
}

/** Wrap `sel` in the inline marks, or unwrap it when it already is wrapped
 *  (marks inside the selection, or immediately around it in `before`/`after`). */
export function toggleInline(
	sel: string,
	before: string,
	after: string,
	style: Style,
	kind: Inline
): { text: string; unwrapOuter: boolean; open: string; close: string } {
	const [open, close] = inlineMarks(style, kind);
	if (sel.length >= open.length + close.length && sel.startsWith(open) && sel.endsWith(close)) {
		return { text: sel.slice(open.length, sel.length - close.length), unwrapOuter: false, open, close };
	}
	if (before.endsWith(open) && after.startsWith(close)) {
		return { text: sel, unwrapOuter: true, open, close };
	}
	return { text: open + sel + close, unwrapOuter: false, open, close };
}

/** A link in the style's syntax. */
export function linkText(style: Style, label: string, url: string): string {
	switch (style) {
		case 'text/org':
			return label ? `[[${url}][${label}]]` : `[[${url}]]`;
		case 'text/asciidoc':
			return `${/^[a-z]+:/i.test(url) ? url : `link:${url}`}[${label}]`;
		case 'text/plain':
			return label ? `${label} <${url}>` : url;
		default:
			return `[${label || url}](${url})`;
	}
}

/** The bullet a list line starts with, per style. */
export function bullet(style: Style): string {
	return style === 'text/asciidoc' ? '* ' : '- ';
}

const LIST_RE = /^(\s*)([-+*]|\d+[.)])\s+/;

/** Toggle every line in `lines` as a list item. */
export function toggleList(lines: string[], style: Style): string[] {
	const all = lines.filter((l) => l.trim()).every((l) => LIST_RE.test(l));
	return lines.map((l) => {
		if (!l.trim()) return l;
		return all ? l.replace(LIST_RE, '$1') : bullet(style) + l;
	});
}

/** Wrap `body` in a quote (Markdown: `> ` per line; Org / AsciiDoc: a block). */
export function quoteBlock(body: string, style: Style): string {
	if (style === 'text/markdown' || style === 'text/plain') {
		return body
			.split('\n')
			.map((l) => (l ? `> ${l}` : '>'))
			.join('\n');
	}
	if (style === 'text/org') return `#+begin_quote\n${body}\n#+end_quote`;
	return `____\n${body}\n____`;
}

/** Wrap `body` in a verbatim code block. */
export function codeBlock(body: string, style: Style, lang = ''): string {
	if (style === 'text/org') return `#+begin_src ${lang}`.trimEnd() + `\n${body}\n#+end_src`;
	if (style === 'text/asciidoc') return (lang ? `[source,${lang}]\n` : '') + `----\n${body}\n----`;
	return '```' + lang + `\n${body}\n` + '```';
}
