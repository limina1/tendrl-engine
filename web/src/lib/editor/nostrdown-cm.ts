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
	MatchDecorator,
	ViewPlugin,
	type ViewUpdate,
	showTooltip,
	type Tooltip
} from '@codemirror/view';
import { StateEffect, StateField, type Extension } from '@codemirror/state';
import {
	autocompletion,
	startCompletion,
	type Completion,
	type CompletionContext,
	type CompletionResult
} from '@codemirror/autocomplete';

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

/** Preview metadata the host resolves for the popover card. */
export interface NostrdownPreview {
	found: boolean;
	/** A ref/embed-slug pointing at a heading in the current (unpublished) draft. */
	inDraft?: boolean;
	eventKind?: number;
	title?: string;
	/** Cited work author — the `["author", …]` tag (e.g. "Plato"). */
	author?: string;
	/** Publishing pubkey (the "index author") + its resolved kind-0 name. */
	authorPubkey?: string;
	authorName?: string;
	summary?: string;
	createdAt?: number;
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
	},
	'.cm-tooltip.cm-nd-card': {
		background: 'var(--bg)',
		border: '1px solid var(--panel-border-strong, var(--panel-border))',
		borderRadius: 'var(--r-md, 6px)',
		boxShadow: 'var(--shadow-lg, 0 8px 30px rgba(0,0,0,0.4))',
		padding: '8px 10px',
		maxWidth: '340px',
		fontFamily: 'var(--font-sans)'
	},
	'.cm-nd-card__head': { display: 'flex', alignItems: 'baseline', gap: '6px' },
	'.cm-nd-card__badge': {
		fontFamily: 'var(--font-mono)',
		fontSize: 'var(--t-3xs, 0.69rem)',
		textTransform: 'uppercase',
		letterSpacing: '0.05em',
		color: 'var(--id-yours)',
		border: '1px solid var(--panel-border)',
		borderRadius: 'var(--r-sm, 3px)',
		padding: '0 4px',
		flex: '0 0 auto'
	},
	'.cm-nd-card__title': {
		fontWeight: '600',
		fontSize: 'var(--t-sm, 0.92rem)',
		color: 'var(--fg)'
	},
	'.cm-nd-card__meta': {
		marginTop: '3px',
		fontSize: 'var(--t-2xs, 0.77rem)',
		color: 'var(--fg-muted)'
	},
	'.cm-nd-card__summary': {
		marginTop: '5px',
		fontSize: 'var(--t-xs, 0.85rem)',
		color: 'var(--fg)',
		lineHeight: '1.45'
	},
	'.cm-nd-card__hint': {
		marginTop: '6px',
		fontSize: 'var(--t-3xs, 0.69rem)',
		color: 'var(--fg-muted)',
		fontStyle: 'italic'
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

// ── preview card rendering ────────────────────────────────────────────────

function elem(tag: string, cls?: string, text?: string): HTMLElement {
	const e = document.createElement(tag);
	if (cls) e.className = cls;
	if (text != null) e.textContent = text;
	return e;
}

function kindLabel(eventKind: number | undefined, prefix: NostrdownToken['kind']): string {
	switch (eventKind) {
		case 30040:
			return 'publication';
		case 30041:
			return 'section';
		case 30023:
			return 'article';
		case 30818:
			return 'wiki';
		default:
			return prefix;
	}
}

function shortPubkey(pk: string): string {
	return pk.length > 12 ? pk.slice(0, 8) + '…' + pk.slice(-4) : pk;
}

function fmtDate(ts: number): string {
	try {
		return new Date(ts * 1000).toLocaleDateString(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric'
		});
	} catch {
		return '';
	}
}

function renderCard(token: NostrdownToken, data: NostrdownPreview | null): HTMLElement {
	const card = elem('div', 'cm-nd-card');
	if (!data) {
		card.appendChild(elem('div', 'cm-nd-card__meta', 'Resolving…'));
		return card;
	}
	if (data.inDraft) {
		const head = elem('div', 'cm-nd-card__head');
		head.appendChild(elem('span', 'cm-nd-card__badge', 'section'));
		head.appendChild(elem('span', 'cm-nd-card__title', data.title || token.target));
		card.appendChild(head);
		card.appendChild(elem('div', 'cm-nd-card__meta', 'in this draft · not published yet'));
		return card;
	}
	if (!data.found) {
		card.appendChild(
			elem('div', 'cm-nd-card__meta', `couldn't resolve ${token.kind}: ${token.target}`)
		);
		card.appendChild(elem('div', 'cm-nd-card__hint', '⌘/Ctrl-click to search relays in the reader'));
		return card;
	}
	const head = elem('div', 'cm-nd-card__head');
	head.appendChild(elem('span', 'cm-nd-card__badge', kindLabel(data.eventKind, token.kind)));
	head.appendChild(elem('span', 'cm-nd-card__title', data.title || token.target));
	card.appendChild(head);

	const meta: string[] = [];
	if (data.author) meta.push(data.author);
	const publisher = data.authorName || (data.authorPubkey ? shortPubkey(data.authorPubkey) : '');
	if (publisher) meta.push(`pub: ${publisher}`);
	if (data.createdAt) {
		const d = fmtDate(data.createdAt);
		if (d) meta.push(d);
	}
	if (meta.length) card.appendChild(elem('div', 'cm-nd-card__meta', meta.join(' · ')));
	if (data.summary) card.appendChild(elem('div', 'cm-nd-card__summary', data.summary));
	card.appendChild(elem('div', 'cm-nd-card__hint', '⌘/Ctrl-click to open in the reader'));
	return card;
}

// ── extension ─────────────────────────────────────────────────────────────

const setPreview = StateEffect.define<{ pos: number; token: NostrdownToken } | null>();

/**
 * Build the nostrdown editor extension.
 * - `onActivate(token, view)` — invoked on ⌘/Ctrl-click to *follow* a token.
 * - `onPreview(token)` — resolves the popover data shown on a plain click.
 */
export function nostrdownEditor(
	opts: {
		onActivate?: (token: NostrdownToken, view: EditorView) => void;
		onPreview?: (token: NostrdownToken) => Promise<NostrdownPreview | null>;
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

	const tooltipField = StateField.define<Tooltip | null>({
		create: () => null,
		update(value, tr) {
			for (const e of tr.effects) {
				if (e.is(setPreview)) {
					value = e.value ? makeTooltip(e.value.pos, e.value.token, opts) : null;
				}
			}
			// Any edit dismisses the popover so it can't dangle at a stale spot.
			if (value && tr.docChanged) value = null;
			return value;
		},
		provide: (f) => showTooltip.from(f)
	});

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
			// Plain click: open the preview over a token, dismiss it off one. The
			// cursor still lands where clicked (we don't preventDefault).
			view.dispatch({ effects: setPreview.of(token ? { pos: token.from, token } : null) });
			return false;
		},
		keydown(e, view) {
			if (e.key === 'Escape' && view.state.field(tooltipField, false)) {
				view.dispatch({ effects: setPreview.of(null) });
				return true;
			}
			return false;
		},
		blur(_e, view) {
			if (view.state.field(tooltipField, false)) {
				view.dispatch({ effects: setPreview.of(null) });
			}
			return false;
		}
	});

	return [highlighter, theme, tooltipField, handlers];
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
	/** Open the embed coordinate builder; `range` is the in-progress `{{embed:…`
	 *  token to replace once the builder produces a token. */
	openEmbedBuilder: (range: { from: number; to: number }) => void;
}

const PREFIXES = ['ref', 'wiki', 'embed'];
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

		if (kind === 'embed') {
			// A coordinate is too much to type — hand off to the builder form,
			// which replaces this whole in-progress `{{embed:…` token.
			const range = { from: before.from, to: ctx.pos };
			return {
				from: valueFrom,
				filter: false,
				options: [
					{
						label: 'build embed coordinate…',
						type: 'function',
						apply: () => sources.openEmbedBuilder(range)
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

	return autocompletion({ override: [source], activateOnTyping: true });
}

function makeTooltip(
	pos: number,
	token: NostrdownToken,
	opts: { onPreview?: (token: NostrdownToken) => Promise<NostrdownPreview | null> }
): Tooltip {
	return {
		pos,
		above: true,
		create() {
			const dom = elem('div', 'cm-nd-card');
			dom.append(...renderCard(token, null).childNodes);
			if (opts.onPreview) {
				opts
					.onPreview(token)
					.then((data) => dom.replaceChildren(...renderCard(token, data).childNodes))
					.catch(() => dom.replaceChildren(...renderCard(token, { found: false }).childNodes));
			}
			return { dom };
		}
	};
}
