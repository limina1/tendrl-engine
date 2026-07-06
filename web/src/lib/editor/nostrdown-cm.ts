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
	ViewPlugin,
	type ViewUpdate
} from '@codemirror/view';
import { Prec, StateEffect, StateField, type Extension } from '@codemirror/state';
import { parseNostrdown } from '$lib/api';
import {
	acceptCompletion,
	autocompletion,
	startCompletion,
	type Completion,
	type CompletionContext,
	type CompletionResult
} from '@codemirror/autocomplete';

export interface NostrdownToken {
	kind: 'ref' | 'wiki' | 'embed' | 'quote' | 'mention';
	/** Normalized target (engine-side NIP-54 slug or bech32 entity). */
	target: string;
	display?: string;
	/** The full `{{…}}` / `[[…]]` text, for handing to the engine resolver verbatim. */
	raw: string;
	/** Document offsets of the token (delimiters included). CM positions are
	 *  UTF-16 code units, matching the engine's `ParsedToken` span unit. */
	from: number;
	to: number;
}

const refMark = Decoration.mark({ class: 'cm-nd' });

// Async decorations: the grammar lives in the engine (`/api/v1/nostrdown/parse`),
// so we can't decorate synchronously as the doc changes. Instead a debounced
// ViewPlugin fetches token spans and dispatches them via `setTokens`; a StateField
// holds them, remapping positions through every edit so decorations + click
// metadata stay aligned between fetches. Stale fetches (doc moved on) are dropped.
const setTokens = StateEffect.define<NostrdownToken[]>();

interface TokenState {
	tokens: NostrdownToken[];
	deco: DecorationSet;
}

function decoOf(tokens: NostrdownToken[]): DecorationSet {
	return Decoration.set(
		tokens.filter((t) => t.to > t.from).map((t) => refMark.range(t.from, t.to)),
		true
	);
}

const tokenField = StateField.define<TokenState>({
	create: () => ({ tokens: [], deco: Decoration.none }),
	update(value, tr) {
		if (tr.docChanged) {
			// Keep spans aligned with the text until the next fetch lands.
			const tokens = value.tokens
				.map((t) => ({ ...t, from: tr.changes.mapPos(t.from), to: tr.changes.mapPos(t.to, 1) }))
				.filter((t) => t.to > t.from);
			value = { tokens, deco: value.deco.map(tr.changes) };
		}
		for (const e of tr.effects) {
			if (e.is(setTokens)) value = { tokens: e.value, deco: decoOf(e.value) };
		}
		return value;
	},
	provide: (f) => EditorView.decorations.from(f, (v) => v.deco)
});

/** Debounced engine-parse of the whole doc → token spans. Skips docs with no
 *  `{{`/`[[`, and drops a result if the doc changed while the request was in
 *  flight (a newer fetch is already scheduled). */
function tokenFetcher() {
	return ViewPlugin.fromClass(
		class {
			timer: ReturnType<typeof setTimeout> | undefined;
			constructor(view: EditorView) {
				this.schedule(view);
			}
			update(u: ViewUpdate) {
				if (u.docChanged) this.schedule(u.view);
			}
			schedule(view: EditorView) {
				clearTimeout(this.timer);
				this.timer = setTimeout(() => void this.run(view), 150);
			}
			async run(view: EditorView) {
				const text = view.state.doc.toString();
				if (!(text.includes('{{') || text.includes('[['))) {
					if (view.state.field(tokenField).tokens.length)
						view.dispatch({ effects: setTokens.of([]) });
					return;
				}
				let parsed: Awaited<ReturnType<typeof parseNostrdown>>;
				try {
					parsed = await parseNostrdown([{ key: 'doc', content: text }]);
				} catch {
					return;
				}
				if (view.state.doc.toString() !== text) return; // stale; a newer fetch is pending
				const spans = parsed['doc'] ?? [];
				const tokens: NostrdownToken[] = spans.map((s) => ({
					kind: s.kind,
					target: s.target,
					display: s.display,
					raw: text.slice(s.start, s.end),
					from: s.start,
					to: s.end
				}));
				view.dispatch({ effects: setTokens.of(tokens) });
			}
			destroy() {
				clearTimeout(this.timer);
			}
		}
	);
}

const theme = EditorView.baseTheme({
	'.cm-nd': {
		color: 'var(--id-yours)',
		textDecoration: 'underline',
		textDecorationStyle: 'dotted',
		textUnderlineOffset: '2px',
		cursor: 'pointer'
	}
});

/** The token under document position `pos`, if any — read from the last engine
 *  parse (remapped through edits by `tokenField`), no local re-scan. */
function tokenAt(view: EditorView, pos: number): NostrdownToken | null {
	const { tokens } = view.state.field(tokenField);
	return tokens.find((t) => pos >= t.from && pos <= t.to) ?? null;
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

	return [tokenField, tokenFetcher(), theme, handlers];
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
	/** Sibling section titles in the current draft, filtered to `partial`. Async
	 *  because slug normalization is engine-side (cached). */
	ref: (partial: string) => NdSuggestion[] | Promise<NdSuggestion[]>;
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
		if (kind === 'ref') suggestions = await sources.ref(partial);
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

