// CodeMirror extension: recognize nostrdown `{{ }}` references as you type, and
// — in the composer — let you peek at and follow them. This is the *authoring*
// counterpart to the reader's resolution: purely editor-side view state, so it
// lives in TS (the "editor sliver" of the frontend/backend boundary). The token
// shape it recognizes mirrors the canonical Rust grammar (`src/nostrdown.rs`);
// the *data* and *actions* are supplied by the host (`onPreview`/`onActivate`),
// since resolving a ref depends on context the editor doesn't own (sibling
// sections in the current draft, or events in the db).
//
// Gestures over a recognized token:
//   - plain click   → a preview popover (title, author, publisher, summary)
//   - ⌘/Ctrl-click  → follow it (host opens the target in the reader)

import {
	Decoration,
	type DecorationSet,
	EditorView,
	keymap,
	MatchDecorator,
	ViewPlugin,
	type ViewUpdate
} from '@codemirror/view';
import { Prec, type Extension } from '@codemirror/state';
import {
	acceptCompletion,
	autocompletion,
	startCompletion,
	type Completion,
	type CompletionContext,
	type CompletionResult
} from '@codemirror/autocomplete';

/** Tier-1 token shape: `{{ref|wiki|embed|quote:target(#fragment)?(|modifier)?}}`. */
const TOKEN_RE = /\{\{(ref|wiki|embed|quote):([^}|#]+)(?:#([^}|]+))?(?:\|([^}]+))?\}\}/g;

export interface NostrdownToken {
	kind: 'ref' | 'wiki' | 'embed' | 'quote';
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
const decorator = new MatchDecorator({ regexp: TOKEN_RE, decoration: () => refMark });

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
	const re = new RegExp(TOKEN_RE.source, 'g'); // own lastIndex
	let m: RegExpExecArray | null;
	while ((m = re.exec(line.text))) {
		const from = line.from + m.index;
		const to = from + m[0].length;
		if (pos >= from && pos <= to) return tokenFromMatch(m, from);
	}
	return null;
}

// ── extension ─────────────────────────────────────────────────────────────

/** Viewport coordinates of a clicked token, for the host to position a card. */
export interface PreviewAnchor {
	left: number;
	bottom: number;
}

/**
 * Build the nostrdown editor extension.
 * - `onActivate(token, view)` — invoked on ⌘/Ctrl-click to *follow* a token.
 * - `onPreview(token, anchor, view)` — plain-click peek. The host renders the
 *   shared EmbedCard declaratively at `anchor` (a fixed-position screen rect);
 *   called with `(null, null)` to dismiss. (We hand the host coordinates rather
 *   than a DOM node so it can render the Svelte card itself — manual `mount()`
 *   into a CM tooltip is unavailable in the prerendered build.)
 */
export function nostrdownEditor(
	opts: {
		onActivate?: (token: NostrdownToken, view: EditorView) => void;
		onPreview?: (
			token: NostrdownToken | null,
			anchor: PreviewAnchor | null,
			view: EditorView
		) => void;
	} = {}
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

	// Plain-click peek: hand the host the token + its screen rect so it can float
	// the EmbedCard there (rendered declaratively — see the onPreview doc). Click
	// off a token dismisses. A doc edit can't leave a stale card because every
	// edit re-runs the host's preview state via a fresh click.
	const handlers = EditorView.domEventHandlers({
		mousedown(e, view) {
			const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
			const token = pos == null ? null : tokenAt(view, pos);
			if (e.metaKey || e.ctrlKey) {
				if (token && opts.onActivate) {
					e.preventDefault();
					opts.onActivate(token, view);
					return true;
				}
				return false;
			}
			// Plain click: peek a token, dismiss off one. The cursor still lands
			// where clicked (we don't preventDefault).
			if (token) {
				const c = view.coordsAtPos(token.from);
				opts.onPreview?.(token, c ? { left: c.left, bottom: c.bottom } : null, view);
			} else {
				opts.onPreview?.(null, null, view);
			}
			return false;
		},
		keydown(e, view) {
			if (e.key === 'Escape') {
				opts.onPreview?.(null, null, view);
				return false;
			}
			return false;
		}
	});

	return [highlighter, theme, handlers];
}

// ── inline autocomplete ─────────────────────────────────────────────────────
// Typing `{{` pops a dropdown at the cursor whose contents are detected from the
// prefix: `{{` → ref:/wiki:/embed:; `{{ref:` → sibling titles; `{{wiki:` → title
// search; `{{embed:` → opens the coordinate builder (a coordinate is too much to
// type inline). The host injects the data sources + the builder opener.

/** A neutral suggestion the host returns; the extension wraps insertion + `}}`. */
export interface NdSuggestion {
	label: string;
	detail?: string;
	/** Value to insert (ref/wiki) — the token target. */
	value: string;
}

export interface NostrdownCompletionSources {
	/** Gate — return false to disable the dropdown (the mode-bar toggle). */
	enabled: () => boolean;
	/** Sibling section titles in the current draft, filtered to `partial`. */
	ref: (partial: string) => NdSuggestion[];
	/** Wiki/article titles matching `partial` (async search). */
	wiki: (partial: string) => Promise<NdSuggestion[]>;
	/** Open the coordinate builder; `range` is the in-progress `{{embed:…` /
	 *  `{{slot:…` token to replace once the builder produces a token. `kind`
	 *  selects which prefix the builder emits (inline embed vs block-level slot). */
	openEmbedBuilder: (range: { from: number; to: number }, kind: 'embed' | 'slot') => void;
}

const PREFIXES = ['ref', 'wiki', 'embed', 'quote', 'slot'];
const CONTEXT_RE = /\{\{([a-zA-Z]*)(:?)([^}|]*)$/;

/** Insert `value` at [from,to], appending `}}` unless it's already there, and
 *  park the cursor after the value (before any `}}`). */
function applyValue(view: EditorView, from: number, to: number, value: string) {
	const hasClose = view.state.sliceDoc(to, to + 2) === '}}';
	const text = hasClose ? value : value + '}}';
	view.dispatch({
		changes: { from, to, insert: text },
		selection: { anchor: from + value.length + (hasClose ? 0 : 2) }
	});
}

export function nostrdownCompletion(sources: NostrdownCompletionSources): Extension {
	async function source(ctx: CompletionContext): Promise<CompletionResult | null> {
		if (!sources.enabled()) return null;
		const before = ctx.matchBefore(CONTEXT_RE);
		if (!before) return null;
		const m = CONTEXT_RE.exec(before.text);
		if (!m) return null;
		const [, word, colon, partial] = m;
		const openFrom = before.from + 2; // just past `{{`

		// Prefix stage: complete ref:/wiki:/embed:.
		if (!colon) {
			const w = word.toLowerCase();
			const options: Completion[] = PREFIXES.filter((p) => p.startsWith(w)).map((p) => ({
				label: `${p}:`,
				type: 'keyword',
				apply: (view, _c, from, to) => {
					view.dispatch({
						changes: { from, to, insert: `${p}:` },
						selection: { anchor: from + p.length + 1 }
					});
					startCompletion(view); // chain straight into the value suggestions
				}
			}));
			return options.length ? { from: openFrom, options, filter: false } : null;
		}

		const kind = word.toLowerCase();
		const valueFrom = openFrom + word.length + 1; // just past `{{prefix:`

		if (kind === 'embed' || kind === 'slot') {
			// A coordinate is too much to type — hand off to the builder form,
			// which replaces this whole in-progress `{{embed:…`/`{{slot:…` token.
			const range = { from: before.from, to: ctx.pos };
			return {
				from: valueFrom,
				filter: false,
				options: [
					{
						label: `build ${kind} coordinate…`,
						type: 'function',
						apply: () => sources.openEmbedBuilder(range, kind)
					}
				]
			};
		}

		let suggestions: NdSuggestion[] = [];
		if (kind === 'ref') suggestions = sources.ref(partial);
		else if (kind === 'wiki') suggestions = await sources.wiki(partial);
		else return null;

		const options: Completion[] = suggestions.map((s) => ({
			label: s.label,
			detail: s.detail,
			type: 'text',
			apply: (view, _c, from, to) => applyValue(view, from, to, s.value)
		}));
		return { from: valueFrom, options, filter: false };
	}

	return [
		autocompletion({ override: [source], activateOnTyping: true }),
		// Tab accepts the highlighted suggestion (same as Enter — for embed it
		// runs the "build coordinate…" hand-off). `acceptCompletion` returns false
		// when no dropdown is open, so Tab keeps its normal behaviour otherwise.
		Prec.highest(keymap.of([{ key: 'Tab', run: acceptCompletion }]))
	];
}

