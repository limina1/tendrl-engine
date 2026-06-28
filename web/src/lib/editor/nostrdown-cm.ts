// CodeMirror extension: recognize nostrdown `{{ }}` references as you type and
// make them activatable (mod-click to follow). This is the *authoring* counter-
// part to the reader's resolution — purely editor-side view state, so it lives
// in TS (the "editor sliver" of the frontend/backend boundary). The token shape
// it recognizes mirrors the canonical Rust grammar (`src/nostrdown.rs`); the
// *action* taken on activation is supplied by the host via `onActivate`, since
// resolving a ref depends on context the editor doesn't own (sibling sections in
// the current compose, or events in the db).

import {
	Decoration,
	type DecorationSet,
	EditorView,
	MatchDecorator,
	ViewPlugin,
	type ViewUpdate
} from '@codemirror/view';
import { type Extension } from '@codemirror/state';

/** Tier-1 token shape: `{{ref|wiki|embed:target(#fragment)?(|display)?}}`. */
const TOKEN_RE = /\{\{(ref|wiki|embed):([^}|#]+)(?:#([^}|]+))?(?:\|([^}]+))?\}\}/g;

export interface NostrdownToken {
	kind: 'ref' | 'wiki' | 'embed';
	/** Target as written (trimmed) — normalize before matching slugs. */
	target: string;
	fragment?: string;
	display?: string;
	/** The full `{{…}}` text, for handing to the engine resolver verbatim. */
	raw: string;
	/** Document offsets of the token (braces included). */
	from: number;
	to: number;
}

function tokenFromMatch(m: RegExpMatchArray, from: number): NostrdownToken {
	return {
		kind: m[1] as NostrdownToken['kind'],
		target: m[2].trim(),
		fragment: m[3]?.trim() || undefined,
		display: m[4]?.trim() || undefined,
		raw: m[0],
		from,
		to: from + m[0].length
	};
}

const refMark = Decoration.mark({ class: 'cm-nd' });

const decorator = new MatchDecorator({
	regexp: TOKEN_RE,
	decoration: () => refMark
});

const theme = EditorView.baseTheme({
	'.cm-nd': {
		color: 'var(--id-yours)',
		textDecoration: 'underline',
		textDecorationStyle: 'dotted',
		textUnderlineOffset: '2px',
		cursor: 'pointer'
	}
});

/** The token under document position `pos`, if any (re-scans just that line). */
function tokenAt(view: EditorView, pos: number): NostrdownToken | null {
	const line = view.state.doc.lineAt(pos);
	// Fresh regex instance — TOKEN_RE's lastIndex is owned by the decorator.
	const re = new RegExp(TOKEN_RE.source, 'g');
	let m: RegExpExecArray | null;
	while ((m = re.exec(line.text))) {
		const from = line.from + m.index;
		const to = from + m[0].length;
		if (pos >= from && pos <= to) return tokenFromMatch(m, from);
	}
	return null;
}

/**
 * Build the nostrdown editor extension. `onActivate` is invoked on a
 * mod-click (Ctrl/Cmd) over a recognized token — the host decides what
 * "follow" means (jump within the buffer, open the resolved event, …).
 */
export function nostrdownEditor(
	opts: { onActivate?: (token: NostrdownToken, view: EditorView) => void } = {}
): Extension {
	const highlighter = ViewPlugin.fromClass(
		class {
			decorations: DecorationSet;
			constructor(view: EditorView) {
				this.decorations = decorator.createDeco(view);
			}
			update(u: ViewUpdate) {
				this.decorations = decorator.updateDeco(u, this.decorations);
			}
		},
		{ decorations: (v) => v.decorations }
	);

	const clicker = EditorView.domEventHandlers({
		mousedown(e, view) {
			if (!opts.onActivate || !(e.metaKey || e.ctrlKey)) return false;
			const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
			if (pos == null) return false;
			const token = tokenAt(view, pos);
			if (!token) return false;
			e.preventDefault();
			opts.onActivate(token, view);
			return true;
		}
	});

	return [highlighter, theme, clicker];
}
