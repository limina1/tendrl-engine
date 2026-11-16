// The composer's rendered editor: CodeMirror over the real source text, with
// the rendering layers' style spans (web/src/lib/layers/bases/*) turned into
// decorations. The document is always genuine Markdown / Org / AsciiDoc —
// nothing is converted — so every construct the bases do not model passes
// through untouched.
//
// Three views:
//   rendered — styled, and markup sigils (`**`, `## `, `#+begin_src`, …) are
//              hidden, except on the line(s) the cursor is on, so what you
//              type stays visible while you type it;
//   markers  — styled, sigils shown dimmed (live preview);
//   source   — plain monospace text, no decorations.
//
// Also here: read-only ranges (the sibling sections of a placed edit) and
// the toolbar's commands, which apply markup.ts transforms to the selection.

import {
	Decoration,
	type DecorationSet,
	EditorView,
	ViewPlugin,
	type ViewUpdate,
	WidgetType
} from '@codemirror/view';
import {
	EditorSelection,
	EditorState,
	type Extension,
	type Range,
	StateEffect,
	StateField
} from '@codemirror/state';
import { basesFor, type BaseImpl, type StyleSpan } from '$lib/layers/registry';
import {
	codeBlock,
	headingChar,
	type Inline,
	linkText,
	quoteBlock,
	setHeadingLevel,
	type Style,
	toggleInline,
	toggleList
} from './markup';

export type ViewMode = 'rendered' | 'markers' | 'source';

// --- style-span decorations ------------------------------------------------

class BulletWidget extends WidgetType {
	constructor(readonly text: string) {
		super();
	}
	eq(o: BulletWidget) {
		return o.text === this.text;
	}
	toDOM() {
		const s = document.createElement('span');
		s.className = 'cm-ly-bullet';
		s.textContent = this.text;
		return s;
	}
}

const LIST_MARK = /^\s*([-+*]|\d+[.)])$/;

function lineDeco(cls: string) {
	return Decoration.line({ class: `cm-ly-line-${cls}` });
}

function buildDecorations(
	view: EditorView,
	spans: StyleSpan[],
	mode: ViewMode
): { all: DecorationSet; hidden: DecorationSet } {
	const doc = view.state.doc;
	const out: Range<Decoration>[] = [];
	// Lines the selection touches keep their sigils visible in `rendered`.
	const live = new Set<number>();
	if (mode === 'rendered') {
		for (const r of view.state.selection.ranges) {
			const a = doc.lineAt(r.from).number;
			const b = doc.lineAt(r.to).number;
			for (let n = a; n <= b; n++) live.add(n);
		}
	}
	const headingLines = new Set<number>();
	const hidden: Range<Decoration>[] = [];
	// Lines that are headings — an Org `* Title` star is a heading sigil there,
	// never a list bullet.
	const headingAt = new Set<number>();
	for (const s of spans) {
		if (/^h[1-6]$/.test(s.cls) && s.start < doc.length) headingAt.add(doc.lineAt(s.start).number);
	}
	for (const s of spans) {
		if (s.end <= s.start || s.end > doc.length) continue;
		if (/^h[1-6]$/.test(s.cls)) {
			const ln = doc.lineAt(s.start);
			if (!headingLines.has(ln.number)) {
				headingLines.add(ln.number);
				out.push(lineDeco(s.cls).range(ln.from));
			}
		}
		if (s.cls === 'fence' || s.cls === 'quote') {
			const a = doc.lineAt(s.start).number;
			const b = doc.lineAt(Math.max(s.start, s.end - 1)).number;
			for (let n = a; n <= b; n++) out.push(lineDeco(s.cls).range(doc.line(n).from));
		}
		if (s.cls === 'url' && mode === 'rendered' && !live.has(doc.lineAt(s.start).number)) {
			// A labelled link's target hides behind its label:
			// md `[label](url)`, org `[[url][label]]`, adoc `url[label]`.
			const before = doc.sliceString(Math.max(0, s.start - 2), s.start);
			const after = doc.sliceString(s.end, Math.min(doc.length, s.end + 2));
			const labelled =
				before.endsWith('(') ||
				(before === '[[' && after === '][') ||
				(after.startsWith('[') && after !== '[]');
			if (labelled) {
				const d = Decoration.replace({}).range(s.start, s.end);
				out.push(d);
				hidden.push(d);
				continue;
			}
		}
		if (s.cls !== 'mark') {
			out.push(Decoration.mark({ class: `ly-${s.cls}` }).range(s.start, s.end));
			continue;
		}
		// A sigil.
		const text = doc.sliceString(s.start, s.end);
		const ln = doc.lineAt(s.start);
		const hide = mode === 'rendered' && !live.has(ln.number) && !text.includes('\n');
		if (!hide || text.includes('|')) {
			out.push(Decoration.mark({ class: 'ly-mark' }).range(s.start, s.end));
			continue;
		}
		const atLineStart = doc.sliceString(ln.from, s.start).trim() === '';
		if (atLineStart && LIST_MARK.test(text) && !headingAt.has(ln.number)) {
			// A bullet: show a rendered bullet in its place (numbers stay numbers).
			const glyph = /\d/.test(text) ? text.trim() : '•';
			const end = doc.sliceString(s.end, s.end + 1) === ' ' ? s.end + 1 : s.end;
			const d = Decoration.replace({ widget: new BulletWidget(glyph + ' ') }).range(s.start, end);
			out.push(d);
			hidden.push(d);
			continue;
		}
		// A leading sigil (heading `## `, quote `> `) takes its trailing space.
		const end = atLineStart && doc.sliceString(s.end, s.end + 1) === ' ' ? s.end + 1 : s.end;
		const d = Decoration.replace({}).range(s.start, end);
		out.push(d);
		hidden.push(d);
	}
	return { all: Decoration.set(out, true), hidden: Decoration.set(hidden, true) };
}

/** The rendered editor for one style + view. Reconfigure through a
 *  Compartment when either changes. */
export function wysiwyg(style: Style, mode: ViewMode): Extension {
	if (mode === 'source') return [EditorView.editorAttributes.of({ class: 'cm-ly-source' })];
	const layer = basesFor(style)[0];
	let impl: BaseImpl | null = null;

	const plugin = ViewPlugin.fromClass(
		class {
			decorations: DecorationSet = Decoration.none;
			hidden: DecorationSet = Decoration.none;
			constructor(readonly view: EditorView) {
				if (layer) {
					layer
						.load()
						.then((i) => {
							impl = i;
							this.compute();
							// An empty transaction makes CM pick the decorations up.
							view.dispatch({ effects: [] });
						})
						.catch(() => {});
				}
			}
			compute() {
				if (!impl) return;
				let spans: StyleSpan[] = [];
				try {
					spans = impl.spans(this.view.state.doc.toString());
				} catch {
					spans = [];
				}
				const d = buildDecorations(this.view, spans, mode);
				this.decorations = d.all;
				this.hidden = d.hidden;
			}
			update(u: ViewUpdate) {
				if (u.docChanged || u.selectionSet || u.viewportChanged || u.transactions.length) {
					this.compute();
				}
			}
		},
		{
			decorations: (v) => v.decorations,
			provide: (p) =>
				EditorView.atomicRanges.of((view) => view.plugin(p)?.hidden ?? Decoration.none)
		}
	);
	return [plugin, EditorView.editorAttributes.of({ class: `cm-ly-${mode}` })];
}

// --- read-only ranges --------------------------------------------------------

/** Replace the locked ranges (document offsets, half-open). */
export const setLocked = StateEffect.define<{ from: number; to: number }[]>();

const lockedField = StateField.define<DecorationSet>({
	create: () => Decoration.none,
	update(value, tr) {
		value = value.map(tr.changes);
		for (const e of tr.effects) {
			if (e.is(setLocked)) {
				const doc = tr.state.doc;
				const ranges: Range<Decoration>[] = [];
				for (const r of e.value) {
					if (r.to <= r.from || r.to > doc.length) continue;
					ranges.push(Decoration.mark({ class: 'cm-locked', inclusiveEnd: false }).range(r.from, r.to));
					const a = doc.lineAt(r.from).number;
					const b = doc.lineAt(Math.max(r.from, r.to - 1)).number;
					for (let n = a; n <= b; n++) {
						ranges.push(Decoration.line({ class: 'cm-locked-line' }).range(doc.line(n).from));
					}
				}
				value = Decoration.set(ranges, true);
			}
		}
		return value;
	},
	provide: (f) => EditorView.decorations.from(f)
});

/** Edits that touch a locked range are dropped — the sibling sections of a
 *  placed edit stay as published until the user unlocks them. */
export function lockedRanges(): Extension {
	return [
		lockedField,
		EditorState.changeFilter.of((tr) => {
			const set = tr.startState.field(lockedField, false);
			if (!set || set.size === 0) return true;
			let blocked = false;
			tr.changes.iterChangedRanges((fromA, toA) => {
				set.between(fromA, toA, (from, to, deco) => {
					if (deco.spec.class !== 'cm-locked') return;
					// Inserting exactly at a locked range's edge is allowed.
					if (toA > from && fromA < to) blocked = true;
					else if (fromA === toA && fromA > from && fromA < to) blocked = true;
				});
			});
			return !blocked;
		})
	];
}

// --- toolbar commands ----------------------------------------------------------

function selectedLines(view: EditorView): { from: number; to: number; lines: string[] } {
	const { state } = view;
	const sel = state.selection.main;
	const a = state.doc.lineAt(sel.from);
	const b = state.doc.lineAt(sel.to);
	return { from: a.from, to: b.to, lines: state.sliceDoc(a.from, b.to).split('\n') };
}

function replaceLines(view: EditorView, from: number, to: number, text: string) {
	view.dispatch({
		changes: { from, to, insert: text },
		selection: EditorSelection.cursor(from + text.length),
		scrollIntoView: true,
		userEvent: 'input.format'
	});
	view.focus();
}

export function cmdHeading(view: EditorView, style: Style, level: number) {
	const { from, to, lines } = selectedLines(view);
	replaceLines(view, from, to, lines.map((l) => setHeadingLevel(l, style, level)).join('\n'));
}

export function cmdInline(view: EditorView, style: Style, kind: Inline) {
	const { state } = view;
	let { from, to } = state.selection.main;
	if (from === to) {
		const w = state.wordAt(from);
		if (w) ({ from, to } = w);
	}
	const sel = state.sliceDoc(from, to);
	const before = state.sliceDoc(Math.max(0, from - 20), from);
	const after = state.sliceDoc(to, Math.min(state.doc.length, to + 20));
	const r = toggleInline(sel, before, after, style, kind);
	if (r.unwrapOuter) {
		view.dispatch({
			changes: [
				{ from: from - r.open.length, to: from, insert: '' },
				{ from: to, to: to + r.close.length, insert: '' }
			],
			selection: EditorSelection.range(from - r.open.length, to - r.open.length),
			userEvent: 'input.format'
		});
	} else {
		const empty = from === to;
		view.dispatch({
			changes: { from, to, insert: r.text },
			selection: empty
				? EditorSelection.cursor(from + r.open.length)
				: EditorSelection.range(from, from + r.text.length),
			userEvent: 'input.format'
		});
	}
	view.focus();
}

export function cmdLink(view: EditorView, style: Style, url: string, label?: string) {
	const { from, to } = view.state.selection.main;
	const text = linkText(style, label ?? view.state.sliceDoc(from, to), url);
	view.dispatch({
		changes: { from, to, insert: text },
		selection: EditorSelection.cursor(from + text.length),
		userEvent: 'input.format'
	});
	view.focus();
}

export function cmdList(view: EditorView, style: Style) {
	const { from, to, lines } = selectedLines(view);
	replaceLines(view, from, to, toggleList(lines, style).join('\n'));
}

export function cmdQuote(view: EditorView, style: Style) {
	const { from, to, lines } = selectedLines(view);
	replaceLines(view, from, to, quoteBlock(lines.join('\n'), style));
}

export function cmdCodeBlock(view: EditorView, style: Style) {
	const { from, to, lines } = selectedLines(view);
	replaceLines(view, from, to, codeBlock(lines.join('\n'), style));
}

/** Insert text at the cursor (the `{{ }}` / `[[ ]]` buttons). `caret` is the
 *  cursor offset inside `text` after the insert. */
export function cmdInsert(view: EditorView, text: string, caret = text.length) {
	const { from, to } = view.state.selection.main;
	view.dispatch({
		changes: { from, to, insert: text },
		selection: EditorSelection.cursor(from + caret),
		userEvent: 'input'
	});
	view.focus();
}

/** The heading level of the line the cursor is on (0 = body text). */
export function headingLevelAt(state: EditorState, style: Style): number {
	const line = state.doc.lineAt(state.selection.main.head).text;
	const c = headingChar(style);
	let i = 0;
	while (i < line.length && line[i] === c) i++;
	return i >= 1 && i <= 6 && line[i] === ' ' ? i : 0;
}

/** Styling for the rendered views — kept beside the extension so every host
 *  gets the same look. Mirrors RichContent's `ly-*` reader styles. */
export const wysiwygTheme = EditorView.baseTheme({
	'&.cm-ly-rendered .cm-content, &.cm-ly-markers .cm-content': {
		fontFamily: 'var(--font-reading, var(--font-sans))',
		lineHeight: '1.65'
	},
	'&.cm-ly-source .cm-content': { fontFamily: 'var(--font-mono)' },
	'.cm-ly-line-h1': { fontSize: '1.6em', fontWeight: '650', lineHeight: '1.3', paddingTop: '0.3em' },
	'.cm-ly-line-h2': { fontSize: '1.35em', fontWeight: '650', lineHeight: '1.35', paddingTop: '0.5em' },
	'.cm-ly-line-h3': { fontSize: '1.18em', fontWeight: '600', paddingTop: '0.3em' },
	'.cm-ly-line-h4': { fontSize: '1.06em', fontWeight: '600' },
	'.cm-ly-line-fence': {
		fontFamily: 'var(--font-mono)',
		fontSize: '0.9em',
		background: 'color-mix(in srgb, var(--fg) 5%, transparent)'
	},
	'.cm-ly-line-quote': {
		borderLeft: '3px solid color-mix(in srgb, var(--fg-muted) 50%, transparent)',
		paddingLeft: '10px',
		color: 'var(--fg-muted)',
		fontStyle: 'italic'
	},
	'.ly-em': { fontStyle: 'italic' },
	'.ly-strong': { fontWeight: '700' },
	'.ly-strike': { textDecoration: 'line-through' },
	'.ly-code': {
		fontFamily: 'var(--font-mono)',
		fontSize: '0.88em',
		background: 'color-mix(in srgb, var(--fg) 7%, transparent)',
		borderRadius: '3px',
		padding: '0 2px'
	},
	'.ly-lang': { color: 'var(--fg-muted)', fontSize: '0.85em' },
	'.ly-link': { color: 'var(--accent)', textDecoration: 'underline', textUnderlineOffset: '0.15em' },
	'.ly-url': { color: 'var(--fg-muted)' },
	'.ly-hl': { background: 'color-mix(in srgb, var(--accent) 22%, transparent)' },
	'.ly-underline': { textDecoration: 'underline' },
	'.ly-comment': { color: 'var(--fg-muted)', opacity: '0.7' },
	'.ly-kw, .ly-tag': { color: 'var(--fg-muted)', fontFamily: 'var(--font-mono)', fontSize: '0.88em' },
	'.ly-admon, .ly-todo': { color: 'var(--accent)', fontWeight: '700', fontFamily: 'var(--font-mono)', fontSize: '0.85em' },
	'.ly-mark': { opacity: '0.45', fontWeight: '400', fontStyle: 'normal' },
	'.cm-ly-bullet': { color: 'var(--id-yours)' },
	'.cm-locked': { color: 'var(--fg-muted)' },
	'.cm-locked-line': {
		background: 'color-mix(in srgb, var(--fg) 3%, transparent)',
		borderLeft: '2px dashed var(--border)'
	}
});
