<script lang="ts">
	import { untrack } from 'svelte';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';
	import { shell } from '$lib/wm/shell.svelte';
	import { CANONICAL_MIMETYPES, FORMAT_LABEL } from '$lib/layers/registry';
	import type { ComposeState, ContextItem, TagEntry, SyncMode } from '$lib/types';
	import { wireTag } from '$lib/types';
	import type { DraftSummary } from '$lib/api';
	import { resolveNostrdown, search } from '$lib/api';
	import type * as api from '$lib/api';
	import { EditorView } from '@codemirror/view';
	import { Compartment } from '@codemirror/state';
	import {
		nostrdownEditor,
		nostrdownCompletion,
		type NostrdownToken,
		type NdSuggestion,
		type PreviewAnchor
	} from '$lib/editor/nostrdown-cm';
	import type { ResolvedRef } from '$lib/nostr/nostrdown';
	import { cachedSlug, ensureSlugs, slug } from '$lib/nostr/slugs';
	import EmbedCard from './EmbedCard.svelte';
	import ReferenceBuilderModal from './ReferenceBuilderModal.svelte';
	import ItemBadge from './ItemBadge.svelte';
	import TagsPanel from './TagsPanel.svelte';
	import FormatToolbar from './FormatToolbar.svelte';
	import {
		wysiwyg,
		wysiwygTheme,
		lockedRanges,
		setLocked,
		type ViewMode
	} from '$lib/editor/wysiwyg';
	import {
		asStyle,
		fenceCloses,
		fenceOpen,
		headingPrefix,
		parseHeading,
		type Style
	} from '$lib/editor/markup';
	import DraftReader from '$lib/wm/renderers/DraftReader.svelte';
	import DraftVersions from './DraftVersions.svelte';
	import PublishedDiffModal from './PublishedDiffModal.svelte';
	import CodeMirrorEditor from './CodeMirrorEditor.svelte';
	import {
		hasStructuralChange,
		claimedUntouchedSections,
		sectionDiverged
	} from '$lib/compose/state';
	import { getAppState } from '$lib/state.svelte';
	import { runTour, discovery, TIPS } from '$lib/wm/discovery.svelte';
	import { toursForKind } from '$lib/wm/tours';
	import { openComposeHelp } from '$lib/wm/compose-help.svelte';

	const app = getAppState();

	// ── Nostrdown: recognize {{ }} refs, preview on click, follow on mod-click ──
	/** The matching sibling heading in this draft (by title-slug): its char offset
	 *  and title, else null. */
	async function findHeading(
		doc: string,
		targetSlug: string
	): Promise<{ pos: number; title: string } | null> {
		const heads: { pos: number; title: string }[] = [];
		let offset = 0;
		for (const line of doc.split('\n')) {
			const head = parseHeading(line, style);
			if (head) heads.push({ pos: offset, title: head.title });
			offset += line.length + 1; // + newline
		}
		await ensureSlugs(heads.map((h) => h.title));
		return heads.find((h) => cachedSlug(h.title) === targetSlug) ?? null;
	}

	const NOSTR_ENTITY_RE = /^(nostr:)?(naddr1|nevent1|note1)/i;

	// Resolve a token to a ResolvedRef for the preview card: a sibling heading in
	// this draft (unpublished, no event yet) or the engine's resolution. A
	// `[[wikilink]]` checks the draft's own headings too — a topic naming a
	// sibling section is an internal link, same as `{{ref:}}`.
	async function previewRefFor(token: NostrdownToken, view: EditorView): Promise<ResolvedRef | null> {
		if (
			(token.kind === 'ref' || token.kind === 'embed' || token.kind === 'wiki') &&
			!NOSTR_ENTITY_RE.test(token.target)
		) {
			// Pinned-coordinate match first: an imported/resumed draft's refs
			// target section d-tags, which never appear in heading text — a
			// title-slug scan alone can't see them.
			const pinned = compose.sections.find((s) => s.d_tag === token.target);
			if (pinned) {
				return {
					kind: 'embed',
					start: 0,
					end: 0,
					target: token.target,
					label: pinned.title,
					found: true,
					event_kind: 30041,
					title: pinned.title
				} as ResolvedRef;
			}
			const hit = await findHeading(view.state.doc.toString(), await slug(token.target));
			if (hit) {
				return {
					kind: 'embed',
					start: 0,
					end: 0,
					target: token.target,
					label: hit.title,
					found: true,
					event_kind: 30041,
					title: hit.title
				} as ResolvedRef;
			}
		}
		try {
			// Engine fallback WITH the draft's sections as siblings, so
			// `ref:`/slug-`embed:` resolve pre-publish exactly as they do in
			// the draft-reader preview (d-tag / title-slug match engine-side).
			const siblings = compose.sections.map((s) => ({
				title: s.title || undefined,
				d_tag: s.d_tag ?? s.source_addr?.d_tag ?? s.id
			}));
			const m = await resolveNostrdown([{ key: 'k', content: token.raw, siblings }]);
			return m['k']?.[0] ?? null;
		} catch {
			return null;
		}
	}

	// Plain-click preview: float the shared EmbedCard beside the clicked token —
	// the same card the reader renders, but declared in the template (below)
	// rather than imperatively `mount()`ed, which is unavailable in the
	// prerendered build. The CM extension hands us the token + its screen rect;
	// we resolve it and drop a fixed-position card there.
	let preview = $state<{ ref: ResolvedRef; x: number; y: number } | null>(null);
	let previewSeq = 0;
	let previewHideTimer: ReturnType<typeof setTimeout> | undefined;
	async function showPreview(
		token: NostrdownToken | null,
		anchor: PreviewAnchor | null,
		view: EditorView
	) {
		clearTimeout(previewHideTimer);
		const seq = ++previewSeq;
		if (!token || !anchor) {
			preview = null;
			return;
		}
		const ref = await previewRefFor(token, view);
		if (seq !== previewSeq) return; // a newer click superseded this resolve
		if (!ref || !ref.found) {
			preview = null;
			return;
		}
		preview = {
			ref,
			x: Math.max(8, Math.min(anchor.left, window.innerWidth - 348)),
			y: anchor.bottom + 4
		};
	}
	function openPreview(r: ResolvedRef) {
		if (r.coord) app.openCoord(r.coord);
		else if (r.event_kind === 0 && r.author_pubkey) app.navigateToProfile(r.author_pubkey);
		else if (r.event_id) app.getEventForModal(r.event_id);
		preview = null;
	}
	function scheduleHidePreview() {
		clearTimeout(previewHideTimer);
		previewHideTimer = setTimeout(() => (preview = null), 140);
	}
	// Portal the card to <body> so the editor's scroll/transform ancestors can't
	// clip the fixed-position popover (mirrors RichContent's reader preview).
	function previewPortal(node: HTMLElement) {
		document.body.appendChild(node);
		return { destroy: () => node.remove() };
	}

	// mod-click on a recognized token: jump to a sibling heading in this buffer
	// (works while drafting, before anything is published — wikilinks included:
	// a topic naming a sibling heading is an internal link), else resolve
	// against the db and open the target event.
	async function followNostrdown(token: NostrdownToken, view: EditorView) {
		if (
			(token.kind === 'ref' || token.kind === 'embed' || token.kind === 'wiki') &&
			!NOSTR_ENTITY_RE.test(token.target)
		) {
			const hit = await findHeading(view.state.doc.toString(), await slug(token.target));
			if (hit) {
				view.dispatch({ selection: { anchor: hit.pos }, scrollIntoView: true });
				view.focus();
				return;
			}
		}
		try {
			const resolved = await resolveNostrdown([{ key: 'k', content: token.raw }]);
			const r = resolved['k']?.[0];
			if (r?.found && r.coord) {
				app.openCoord(r.coord);
				return;
			}
			// A profile mention (`{{@npub…}}`) resolves found + kind-0 but has no
			// addressable coord — follow it to the profile, same as the reader.
			if (r?.found && r.event_kind === 0 && r.author_pubkey) {
				app.navigateToProfile(r.author_pubkey);
				return;
			}
			// An nevent/note target has no coordinate — open the event modal.
			if (r?.found && r.event_id) {
				app.getEventForModal(r.event_id);
				return;
			}
			// An unresolved wiki reference: don't dead-end on a toast — open the
			// search frame seeded with the topic so the user can find (or, in Auto
			// mode, auto-fetch) the defining event. Confirm mode searches local with
			// the relay-fetch option, per the standing network-intent pattern.
			// (Topic form only — a bech32 entity target isn't a d-tag to search.)
			if (token.kind === 'wiki' && !NOSTR_ENTITY_RE.test(token.target)) {
				app.openSearchFor(`k:30818 d:${token.target}`, token.target);
				return;
			}
		} catch {
			/* fall through to the toast */
		}
		app.pushToast(`Unresolved ${token.kind}: ${token.target}`, 'info');
	}

	// The reference-builder modal behind the mode-bar {{ }} button, plus the
	// inline-autocomplete checkbox beside it. The CM extension array
	// (`nostrdownExt`) is assembled below, after `refSectionTitles`/
	// `insertNostrdownToken` are in scope.
	let autocompleteOn = $state(true);
	let refBuilderOpen = $state(false);
	// Tab the builder opens on — `ref` from the toolbar button; `embed`/`slot`
	// when autocomplete hands an in-progress token off to the builder.
	let builderTab = $state<'ref' | 'wiki' | 'embed' | 'slot' | 'quote' | 'mention'>('ref');
	// When the builder is opened mid-token from autocomplete, the in-progress
	// `{{embed:…` range it should replace on insert.
	let embedRange = $state<{ from: number; to: number } | null>(null);

	// Composer walkthrough dropdown. The in-chrome `W` lists every composer
	// tutorial (registry's `composer.tours`); picking one switches the editor to
	// the tour's view (when it has one) and runs it. Mirrors the logo `W`'s
	// guide menu, scoped to this buffer.
	const composerTours = toursForKind('composer');
	let walkMenuOpen = $state(false);
	function runComposeTour(t: { key: string; mode?: 'plain' }) {
		// A `mode`-tagged tour walks a specific view — switch to it first so its
		// anchors are mounted when the tip resolves. View-agnostic tours leave the
		// current view untouched. (No-op under an atomic kind, which has no views.)
		if (t.mode && !isAtomic) mode = t.mode;
		walkMenuOpen = false;
		runTour(t.key);
	}

	type ComposeMode = 'plain' | 'preview';

	/** Publish/preview/save payload. `kind`/`content` are set only for atomic
	 *  kinds (NIP-23 blog, NIP-54 wiki, custom) — then the whole editor body is
	 *  one event and the section graph is bypassed. */
	type PublishMeta = {
		title: string;
		tags: TagEntry[];
		kind?: number;
		content?: string;
		/** Notes mode — publish each section as a standalone 30041, no index. */
		notes?: boolean;
	};

	// Output-kind presets for the mode dropdown. 30040 (Publication) keeps the
	// section-parsing path; the rest publish a single atomic event of that kind.
	// `-1` is the "Custom…" sentinel that reveals a free numeric kind input.
	// `notes` = standalone 30041s (no index); `-1` = "Custom…" (a free kind).
	const KIND_PRESETS = [
		{ kind: 30040, label: 'Publication', value: 'pub' },
		{ kind: 30040, label: 'Notes', value: 'notes' },
		{ kind: 30818, label: 'Wiki · 30818', value: '30818' },
		{ kind: 30817, label: 'Spec · 30817', value: '30817' },
		{ kind: 30023, label: 'Blog · 30023', value: '30023' },
		{ kind: -1, label: 'Custom kind…', value: 'custom' }
	];
	/** The style a kind fixes by spec (NIP-23 Markdown, NIP-54 AsciiDoc). */
	const KIND_STYLE: Record<number, string> = { 30023: 'text/markdown', 30818: 'text/asciidoc' };

	let {
		compose,
		syncMode,
		canPublish = false,
		onupdate,
		oncancel,
		onsendtochat,
		onpublish,
		onpreview,
		onsavedraft,
		drafts = [],
		onloaddraft,
		ondeletedraft,
		ondelete,
		ondeletepermanent,
		onsenditemtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy,
		onreorder,
		mode = $bindable<ComposeMode>('plain'),
		cursor = -1,
		sectionsListEl = $bindable<HTMLDivElement | undefined>(undefined),
		plainCmView = $bindable<EditorView | null>(null),
		lineNumbers = false,
		vimMode = false
	}: {
		compose: ComposeState;
		syncMode: SyncMode;
		canPublish?: boolean;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublish: (items: ContextItem[], meta?: PublishMeta) => void;
		onpreview?: (items: ContextItem[], meta?: PublishMeta) => void;
		/** Save the current compose as a local draft (never signed). */
		onsavedraft?: (items: ContextItem[], meta?: PublishMeta) => void;
		/** Saved drafts to list for resuming, newest first. */
		drafts?: DraftSummary[];
		onloaddraft?: (draftId: string) => void;
		ondeletedraft?: (draftId: string) => void;
		ondelete: (items: ContextItem[]) => void;
		ondeletepermanent: (items: ContextItem[]) => void;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
		onreorder?: (id: string, dir: 'up' | 'down') => void;
		mode?: ComposeMode;
		cursor?: number;
		sectionsListEl?: HTMLDivElement;
		plainCmView?: EditorView | null;
		lineNumbers?: boolean;
		vimMode?: boolean;
	} = $props();


	// --- Output kind (Publication vs atomic blog/wiki/custom) ---
	// Owned by app state so resuming a draft can restore the editor mode. The
	// select reflects it; a non-preset kind shows the free numeric input.
	const PRESET_KINDS = new Set([30040, 30818, 30817, 30023]);
	const composeKind = $derived(app.composeKind);
	// Default content format for the sections — the engine stamps it as each
	// 30041's `m` tag; a section's own select overrides it.
	const composeFormat = $derived(app.composeFormat);
	const isAtomic = $derived(composeKind !== 30040);
	const isCustomKind = $derived(!PRESET_KINDS.has(composeKind));

	const typeValue = $derived(
		isCustomKind ? 'custom' : composeKind === 30040 ? (app.composeNotes ? 'notes' : 'pub') : String(composeKind)
	);
	function onKindSelect(e: Event) {
		const v = (e.currentTarget as HTMLSelectElement).value;
		if (v === 'pub' || v === 'notes') {
			app.composeKind = 30040;
			app.composeNotes = v === 'notes';
			return;
		}
		app.composeNotes = false;
		// "Custom…": keep the current kind if it's already custom, else seed
		// a non-preset default (1 = a plain note) the user can edit.
		const k = v === 'custom' ? (isCustomKind ? composeKind : 1) : +v;
		app.composeKind = k;
		// A kind that fixes its style by spec sets it (still changeable).
		if (KIND_STYLE[k]) app.composeFormat = KIND_STYLE[k];
	}
	function onCustomKind(e: Event) {
		app.composeKind = +(e.currentTarget as HTMLInputElement).value || 0;
	}
	// Body for atomic kinds. Seeded once from the section contents the first
	// time the user switches to an atomic kind, so a part-written publication
	// isn't lost; thereafter it's the independent source of truth. Lives in
	// app state (composeAtomicBody) so a buffer switch doesn't drop it.
	let atomicCmView = $state<EditorView | null>(null);

	$effect(() => {
		if (!isAtomic) return;
		untrack(() => {
			if (app.composeAtomicSeeded) return;
			if (!app.composeAtomicBody.trim()) {
				// The whole document carries over — its title line, headings
				// and text — so switching Publication → Wiki loses nothing.
				app.composeAtomicBody = plainText.trim() ? plainText : serializeAll();
			}
			app.composeAtomicSeeded = true;
		});
	});

	/** The style the editor reads and the toolbar writes (the `m` tag). */
	const style: Style = $derived(asStyle(composeFormat));

	/** An atomic body's title is its leading level-1 heading (`# Title`,
	 *  `* Title`, `= Title`); the rest is the event content. */
	function splitAtomic(text: string): { title: string; content: string } {
		const lines = text.split('\n');
		let i = 0;
		while (i < lines.length && lines[i].trim() === '') i++;
		const h = i < lines.length ? parseHeading(lines[i], style) : null;
		if (h && h.level === 1) {
			let j = i + 1;
			while (j < lines.length && lines[j].trim() === '') j++;
			return { title: h.title, content: lines.slice(j).join('\n') };
		}
		return { title: compose.title, content: text };
	}
	const atomicParsed = $derived(
		isAtomic ? splitAtomic(app.composeAtomicBody) : { title: '', content: '' }
	);
	const atomicCanPublish = $derived(
		isAtomic && atomicParsed.title.trim().length > 0 && atomicParsed.content.trim().length > 0
	);

	function atomicMeta(): PublishMeta {
		return {
			title: atomicParsed.title,
			tags: compose.tags,
			kind: composeKind,
			content: atomicParsed.content
		};
	}

	// Sections are H2 (and H3, which turns its H2 into a nested index);
	// H4 and deeper stay inside the section text. H1 is the title.
	const SECTION_DEPTH = 3;

	// --- Serialize / Parse ---

	/** Heading prefix for level N in the current style. Level 1 = the
	 *  publication title, 2 = a section, 3 = a nested section. */
	function headFor(level: number): string {
		return headingPrefix(style, level);
	}

	// Serialize entire document into one text blob. Tags live in the tags
	// panel (compose state), never in the text.
	function serializeAll(): string {
		// Blank sections (a fresh draft's placeholder) write nothing, and the
		// title line is written only when there is a title — so a new draft
		// opens empty and the first `# ` the author types is the title.
		const sections = compose.sections.filter((s) => s.slot || s.title.trim() || s.content.trim());
		let out = compose.title ? `${headFor(1)}${compose.title}\n` : '';
		for (const s of sections) {
			if (s.slot) {
				// A slot serializes back as its `{{slot:…}}` line — so a resumed
				// draft / mode switch reconstructs it as a block, not a section.
				out += `\n{{slot:${s.slot}}}\n`;
				continue;
			}
			const level = s.level ?? 2;
			out += `\n${headFor(level)}${s.title}\n`;
			out += `\n${s.content}\n`;
		}
		return out;
	}

	// Serialize a *parsed* document (used by outline reorder: parse → swap
	// sections → write back without round-tripping through compose.sections).
	function serializeParsed(title: string, hasTitle: boolean, sections: ParsedSection[]): string {
		let out = hasTitle ? `${headFor(1)}${title}\n` : '';
		for (const s of sections) {
			if (s.slot) {
				out += `\n{{slot:${s.slot}}}\n`;
				continue;
			}
			out += `\n${headFor(s.level)}${s.title}\n`;
			out += `\n${s.content}\n`;
		}
		return out;
	}

	function reorderInPlain(index: number, dir: 'up' | 'down') {
		const parsed = parseAll(plainText);
		const swap = dir === 'up' ? index - 1 : index + 1;
		if (swap < 0 || swap >= parsed.sections.length) return;
		const next = parsed.sections.slice();
		[next[index], next[swap]] = [next[swap], next[index]];
		// Section tags follow their section.
		const items = compose.sections.slice();
		if (items[index] && items[swap]) {
			[items[index], items[swap]] = [items[swap], items[index]];
			onupdate({ ...compose, sections: items });
		}
		plainText = serializeParsed(parsed.title, parsed.hasTitle, next);
	}

	interface ParsedSection {
		title: string;
		content: string;
		/** Heading depth — 2 for a section, 3 for a nested one. */
		level: number;
		/** First line (0-based) of the section — its heading, or its slot line. */
		line: number;
		/** Set when this item is a block-level `{{slot:target}}` transclude —
		 *  the naddr/coordinate of an existing 30040/30041 to reference as a
		 *  child of the index (no title/content/heading of its own). */
		slot?: string;
	}

	// Parse the text into title + sections, in the current style. Headings
	// inside code fences are content. Level 1 is the title; levels
	// 2..SECTION_DEPTH are sections; deeper headings stay in the text.
	function parseAll(text: string): {
		title: string;
		hasTitle: boolean;
		sections: ParsedSection[];
	} {
		const lines = text.split('\n');
		let docTitle = '';
		// Whether a level-1 heading LINE was seen — an explicit but empty
		// `# ` still counts as a title line.
		let hasTitle = false;
		const sections: ParsedSection[] = [];
		let current: { title: string; contentLines: string[]; level: number; line: number } | null =
			null;
		let inDocHeader = true;
		let fence: string | null = null;
		const finish = () => {
			if (current)
				sections.push({
					title: current.title,
					content: current.contentLines.join('\n').trim(),
					level: current.level,
					line: current.line
				});
			current = null;
		};

		lines.forEach((line, ln) => {
			if (fence) {
				if (fenceCloses(line, fence, style)) fence = null;
				if (current) current.contentLines.push(line);
				else if (line.trim()) {
					inDocHeader = false;
					current = { title: '', contentLines: [line], level: 2, line: ln };
				}
				return;
			}
			const head = parseHeading(line, style);
			if (inDocHeader && !hasTitle && head && head.level === 1) {
				docTitle = head.title;
				hasTitle = true;
				return;
			}
			if (head && head.level >= 2 && head.level <= SECTION_DEPTH) {
				finish();
				inDocHeader = false;
				current = { title: head.title, contentLines: [], level: head.level, line: ln };
				return;
			}
			// A standalone `{{slot:target}}` line is a block-level transclude.
			const slotMatch = line.trim().match(/^\{\{slot:([^}|#]+)\}\}$/);
			if (slotMatch) {
				finish();
				inDocHeader = false;
				sections.push({ title: '', content: '', level: 2, line: ln, slot: slotMatch[1].trim() });
				return;
			}
			const open = fenceOpen(line, style);
			if (open) fence = open;
			if (inDocHeader) {
				if (line.trim() === '') return;
				// Content before any section — an untitled section.
				inDocHeader = false;
				current = { title: '', contentLines: [line], level: 2, line: ln };
			} else if (current) {
				current.contentLines.push(line);
			}
		});
		finish();
		return { title: docTitle, hasTitle, sections };
	}

	// Reconcile parsed sections with existing compose sections. Carries
	// the parsed heading level through to each ContextItem so the engine's
	// publish path sees the nested-outline shape. Returns the parsed
	// sections so the publish path can use them directly without waiting
	// for the reactive commit.
	function handlePlainFullEdit(
		text: string
	): { title: string; format: string; tags: TagEntry[]; sections: ContextItem[] } {
		const parsed = parseAll(text);
		const oldSections = compose.sections;

		// Match parsed sections to existing by position, create new for extras.
		// Tags live in the tags panel, on the item — they follow by position.
		const newSections: ContextItem[] = parsed.sections.map((p, i) => {
			const existing = i < oldSections.length ? oldSections[i] : null;
			if (existing) {
				return {
					...existing,
					title: p.title,
					content: p.content,
					level: p.level,
					slot: p.slot,
					modified: p.content !== existing.original_content
				};
			}
			return {
				id: crypto.randomUUID(),
				title: p.title,
				content: p.content,
				context_content: p.content,
				tags: [],
				level: p.level,
				slot: p.slot,
				original_content: '',
				modified: true,
				in_context: false,
				in_compose: true,
				held: false,
				origin: 'compose' as const,
				readonly: false
			};
		});

		const next = { ...compose, title: parsed.title, tags: compose.tags, sections: newSections };
		// Our own commit: the text already says this — don't re-serialize it.
		knownSectionFp = sectionFp(newSections);
		onupdate(next);
		return { title: parsed.title, format: compose.format, tags: compose.tags, sections: newSections };
	}

	// Detected structure for plain mode sidebar
	let plainText = $state('');
	// The detected panel collapses behind a caret so it stops squeezing the
	// editor — at 390px the fixed 200px sidebar left ~half the width for
	// text. Default: open where there's room (desktop), closed on mobile.
	let detectedOpen = $state(shell.mode !== 'mobile');
	const detectedState = $derived.by(() => {
		if (mode !== 'plain')
			return {
				title: '',
				hasTitle: false,
				sections: [] as {
					title: string;
					item: ContextItem | null;
					index: number;
					level: number;
					line: number;
					slot?: string;
				}[]
			};
		const parsed = parseAll(plainText);
		const oldSections = compose.sections;
		return {
			title: parsed.title,
			hasTitle: parsed.hasTitle,
			sections: parsed.sections.map((p, i) => {
				const existing = i < oldSections.length ? oldSections[i] : null;
				return {
					title: p.title,
					item: existing,
					index: i,
					level: p.level,
					line: p.line,
					slot: p.slot
				};
			})
		};
	});
	const detectedSections = $derived(detectedState.sections);

	// Compact, readable label for a slot target in the outline. A coordinate
	// (kind:pubkey:d-tag) shows `kind · d-tag`; an naddr/other shows a truncation.
	function slotLabel(slot: string): string {
		const parts = slot.split(':');
		if (parts.length >= 3 && /^\d+$/.test(parts[0])) {
			return `${parts[0]} · ${parts.slice(2).join(':')}`;
		}
		return slot.length > 26 ? `${slot.slice(0, 14)}…${slot.slice(-6)}` : slot;
	}

	// Reference builder: titles of the other sections in the current draft (the
	// `{{ref:}}` candidates) + insertion into the active CodeMirror surface
	// (plain mode or the atomic body; Full-mode section textareas aren't CM).
	const refSectionTitles = $derived(
		(mode === 'plain' ? detectedSections.map((s) => s.title) : compose.sections.map((s) => s.title))
			.map((t) => (t ?? '').trim())
			.filter((t) => t.length > 0)
	);

	function activeCmView(): EditorView | null {
		if (isAtomic) return atomicCmView ?? null;
		if (mode === 'plain') return plainCmView ?? null;
		return null;
	}

	// The toolbar {{ }} button — open the builder fresh, inserting at the cursor.
	// Guard up front: only the CM editors (plain mode / atomic body) can take the
	// token, so don't let the user build one that has nowhere to land.
	function openRefBuilder() {
		if (!activeCmView()) {
			app.pushToast('Switch to Plain mode (or an atomic body) to insert a reference', 'info');
			return;
		}
		embedRange = null;
		builderTab = 'ref';
		refBuilderOpen = true;
	}

	function insertNostrdownToken(token: string) {
		const view = activeCmView();
		if (!view) {
			app.pushToast('Switch to Plain mode (or an atomic body) to insert a reference', 'info');
			refBuilderOpen = false;
			embedRange = null;
			return;
		}
		// Replace the in-progress `{{embed:…` range when the builder was opened
		// mid-token from autocomplete; otherwise insert at the cursor.
		const sel = view.state.selection.main;
		const range = embedRange ?? { from: sel.from, to: sel.to };
		view.dispatch({
			changes: { from: range.from, to: range.to, insert: token },
			selection: { anchor: range.from + token.length }
		});
		view.focus();
		embedRange = null;
	}

	// Inline autocomplete data sources + the CM extension array passed to both
	// editors. `ref:` completes the draft's own section titles instantly; `wiki:`
	// searches existing titles; `embed:` hands off to the builder modal.
	async function wikiSuggestions(partial: string): Promise<NdSuggestion[]> {
		const q = partial.trim();
		if (!q) return [];
		try {
			const resp = await search(`k:30818 k:30023 ${q}`, 12);
			const results = (resp.results ?? []).filter((r) => r.addr && r.title);
			await ensureSlugs(results.map((r) => r.title as string));
			return results.map((r) => ({
				label: r.title as string,
				detail: r.kind === 30818 ? 'wiki' : 'article',
				value: r.addr?.d_tag ?? cachedSlug(r.title as string)
			}));
		} catch {
			return [];
		}
	}

	const nostrdownExt = [
		nostrdownEditor({ onActivate: followNostrdown, onPreview: showPreview }),
		nostrdownCompletion({
			enabled: () => autocompleteOn,
			ref: async (partial) => {
				const q = await slug(partial);
				await ensureSlugs(refSectionTitles);
				return refSectionTitles
					.filter((t) => !q || cachedSlug(t).includes(q))
					.map((t) => ({ label: t, value: t }));
			},
			wiki: wikiSuggestions,
			openEmbedBuilder: (range, kind) => {
				embedRange = range;
				builderTab = kind;
				refBuilderOpen = true;
			}
		})
	];

	// "Nothing to act on" gate for Preview/Save: atomic needs a title + body;
	// plain needs a detected section; full needs a section card.
	const noContent = $derived(
		isAtomic
			? !atomicCanPublish
			: mode === 'plain'
				? detectedSections.length === 0
				: compose.sections.length === 0
	);

	// Publication vs Notes is AUTO-DETECTED from the document title (the level-1
	// `=` heading in plain mode, the title field in full mode). With a title the
	// sections bind into one Publication (30040 index + 30041s); with no title
	// they're scattered Notes — each section a standalone 30041, no index. The
	// mode-bar shows which, so the user sees what a Sign will publish.
	const hasSections = $derived(
		mode === 'plain' ? detectedSections.length > 0 : compose.sections.length > 0
	);
	// Keyed on the title LINE's presence, not its text: an explicit but empty
	// `= ` heading still counts as a title (→ Publication), so a fresh plain
	// draft (just `=`) doesn't read as Notes. Notes is the genuine no-`=`-line
	// case (only `==` sections), or an empty title field in full mode.
	const hasDocTitle = $derived(
		mode === 'plain' ? detectedState.hasTitle : compose.title.trim().length > 0
	);
	// Notes: picked in the Type select, or a publication with no title line.
	const isNotes = $derived(!isAtomic && hasSections && (app.composeNotes || !hasDocTitle));

	// Fingerprint of section identity + lock/divergence state, so we can
	// detect external changes. readonly/modified flips cover the badge
	// lock-to-source cycle, which resets content to the original — the
	// editor text must follow.
	let knownSectionFp = $state('');
	function sectionFp(sections: ContextItem[] = compose.sections): string {
		return sections
			.map((s) => `${s.id}:${s.readonly ? 1 : 0}:${sectionDiverged(s) ? 1 : 0}`)
			.join('|');
	}

	// Re-serialize when sections change externally (e.g. search → compose,
	// badge lock/unlock)
	$effect(() => {
		if (mode !== 'plain') return;
		const fp = sectionFp();
		if (fp !== knownSectionFp) {
			plainText = serializeAll();
			knownSectionFp = fp;
		}
	});

	// --- Mode switching ---
	// `mode` is bindable. ComposerBuffer's normal-mode h/l toggle writes
	// directly into it; the settings-driven default also lands here at
	// mount. We watch it via $effect and run the appropriate transition
	// (serialize on entering plain, commit on leaving) so the prop write
	// stays the single source of truth. `appliedMode` starts as null so
	// the first run always serializes when mounting in plain mode.
	let appliedMode = $state<ComposeMode | null>(null);
	$effect(() => {
		const next = mode;
		untrack(() => {
			if (next === appliedMode) return;
			if (next === 'plain') {
				plainText = serializeAll();
				knownSectionFp = sectionFp();
			} else if (appliedMode === 'plain') {
				handlePlainFullEdit(plainText);
			}
			appliedMode = next;
		});
	});

	// A buffer switch unmounts the composer with no blur or mode transition —
	// commit any uncommitted plain-mode text so it survives the round trip.
	// (Full/atomic modes commit per keystroke; plain only commits on blur or
	// on leaving the mode.)
	$effect(() => {
		return () => {
			if (appliedMode === 'plain') handlePlainFullEdit(plainText);
		};
	});

	// --- Section handlers ---

	function handlePlainBlur() {
		handlePlainFullEdit(plainText);
	}

	function updateTags(tags: TagEntry[]) {
		onupdate({ ...compose, tags });
	}

	function updateSectionTags(index: number, tags: TagEntry[]) {
		// Commit the live text first so a section typed since the last blur
		// has its item, then set the tags on it.
		const { sections } = handlePlainFullEdit(plainText);
		const target = sections[index];
		if (!target) return;
		onupdate({
			...compose,
			sections: compose.sections.map((s) => (s.id === target.id ? { ...s, tags } : s))
		});
	}

	function removeSection(id: string) {
		onupdate({ ...compose, sections: compose.sections.filter((s) => s.id !== id) });
	}

	function unlockAllImported() {
		const sections = compose.sections.map((s) =>
			s.source_addr && s.readonly ? { ...s, readonly: false } : s
		);
		onupdate({ ...compose, sections });
	}

	function lockAllUnlocked() {
		const sections = compose.sections.map((s) =>
			s.source_addr && !s.readonly && s.content === s.original_content
				? { ...s, readonly: true }
				: s
		);
		onupdate({ ...compose, sections });
	}

	const structuralChange = $derived(hasStructuralChange(compose));
	const claimedUntouched = $derived(claimedUntouchedSections(compose));

	// Mirror ReaderBuffer's bulk-lock affordance: a section is "lockable"
	// when imported and currently locked (can be unlocked), and "unlocked"
	// when imported, unlocked, and unmodified (can be re-locked cleanly).
	const anyLockable = $derived(compose.sections.some((s) => s.source_addr && s.readonly));
	const anyUnlocked = $derived(
		compose.sections.some((s) => s.source_addr && !s.readonly && s.content === s.original_content)
	);

	// --- The rendered editor ---

	/** Rendered / Show markers / Source — per-viewer, remembered. */
	const VIEW_KEY = 'tendrl.compose.view';
	function loadView(): ViewMode {
		try {
			const v = localStorage.getItem(VIEW_KEY);
			if (v === 'rendered' || v === 'markers' || v === 'source') return v;
		} catch {
			/* storage blocked */
		}
		return 'rendered';
	}
	let viewMode = $state<ViewMode>(loadView());
	function setViewMode(v: ViewMode) {
		viewMode = v;
		try {
			localStorage.setItem(VIEW_KEY, v);
		} catch {
			/* storage blocked */
		}
	}

	// One compartment per editor surface (plain + atomic), reconfigured when
	// the style or the view changes; the document is never rewritten.
	const plainLook = new Compartment();
	const atomicLook = new Compartment();
	/** Bumped on every editor update — the toolbar re-reads the heading level,
	 *  the rail re-reads the cursor's section. */
	let edVersion = $state(0);
	let cursorLine = $state(0);
	const tracker = EditorView.updateListener.of((u) => {
		if (u.selectionSet || u.docChanged) {
			cursorLine = u.state.doc.lineAt(u.state.selection.main.head).number - 1;
			edVersion++;
		}
	});
	const plainExt = [
		...nostrdownExt,
		plainLook.of(wysiwyg(asStyle(app.composeFormat), loadView())),
		lockedRanges(),
		wysiwygTheme,
		tracker
	];
	const atomicExt = [
		...nostrdownExt,
		atomicLook.of(wysiwyg(asStyle(app.composeFormat), loadView())),
		wysiwygTheme,
		tracker
	];
	$effect(() => {
		const ext = wysiwyg(style, viewMode);
		untrack(() => {
			plainCmView?.dispatch({ effects: plainLook.reconfigure(ext) });
			atomicCmView?.dispatch({ effects: atomicLook.reconfigure(ext) });
		});
	});

	/** The section the cursor is in (outline index), or -1 before the first. */
	const cursorSection = $derived.by(() => {
		let at = -1;
		for (const d of detectedSections) if (d.line <= cursorLine) at = d.index;
		return at;
	});

	// Locked sections (a placed edit's siblings, imported sections not yet
	// claimed) are read-only ranges in the editor: their heading line through
	// the line before the next section.
	$effect(() => {
		const view = plainCmView;
		if (!view || mode !== 'plain') return;
		void knownSectionFp;
		// Re-run on every editor update (edVersion), and read the editor's own
		// doc: the host's text reaches the editor in a later effect, so
		// ranges computed from `plainText` could land on the old doc.
		void edVersion;
		void plainText;
		untrack(() => {
			const text = view.state.doc.toString();
			const parsed = parseAll(text);
			const lineStarts: number[] = [0];
			for (let i = 0; i < text.length; i++) if (text[i] === '\n') lineStarts.push(i + 1);
			const ranges: { from: number; to: number }[] = [];
			parsed.sections.forEach((p, i) => {
				const item = compose.sections[i];
				if (!item?.readonly || !item.source_addr) return;
				const from = lineStarts[p.line] ?? text.length;
				const nextLine = parsed.sections[i + 1]?.line;
				const to = nextLine !== undefined ? (lineStarts[nextLine] ?? text.length) : text.length;
				ranges.push({ from, to });
			});
			// Re-sent on every change (a whole-doc sync collapses the old
			// ranges); skipped only while there is nothing to lock.
			if (ranges.length === 0 && !hadLocked) return;
			hadLocked = ranges.length > 0;
			view.dispatch({ effects: setLocked.of(ranges) });
		});
	});
	let hadLocked = false;

	/** Jump the editor to a section from the outline. */
	function jumpTo(line: number) {
		const view = plainCmView;
		if (!view) return;
		const pos = view.state.doc.line(Math.min(line + 1, view.state.doc.lines)).from;
		view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
		view.focus();
	}

	/** Tags panel tab: the publication's (or the event's) tags, or the
	 *  cursor's section. */
	let railTab = $state<'doc' | 'section'>('doc');
	const autoDocTags = $derived(
		isAtomic
			? ['d', 'title', `m ${composeFormat}`]
			: isNotes
				? []
				: ['d', 'title', 'a × ' + detectedSections.length, 'auto-update']
	);

	// --- Edit from an event menu: Sign opens the review (EditReviewModal) ---

	const edit = $derived(app.composeEdit);

	/** The section-edit requests a Sign makes for the current edit, or null
	 *  when the edit is better published as a whole (own publication with a
	 *  changed structure — the normal republish path handles that). */
	function editRequests(): api.SectionEditRequest[] | null {
		const e = edit;
		if (!e) return null;
		if (e.mode === 'atomic') {
			if (!e.baseId) return null;
			return [
				{
					section: e.baseId,
					content: atomicParsed.content,
					title: atomicParsed.title,
					format: composeFormat,
					tags: compose.tags.map(wireTag)
				}
			];
		}
		const { sections } = handlePlainFullEdit(plainText);
		const placed = sections.filter((s) => s.source_event_id);
		const added = sections.filter((s) => !s.source_event_id && !s.slot);
		const reordered =
			placed.map((s) => s.source_event_id).join() !==
			compose.sections.filter((s) => s.source_event_id).map((s) => s.source_event_id).join();
		if (e.own && (added.length > 0 || reordered)) return null;
		if (!e.own && added.length > 0) {
			app.pushToast(
				"New sections can't go into someone else's publication yet — only the edited sections are sent",
				'info'
			);
		}
		const tagKey = (ts: TagEntry[] | undefined) => JSON.stringify((ts ?? []).map(wireTag));
		return placed
			.filter(
				(s) =>
					s.content !== s.original_content ||
					s.title !== (s.original_title ?? s.title) ||
					tagKey(s.tags) !== tagKey(s.original_tags)
			)
			.map((s) => ({
				section: s.source_event_id!,
				index: e.indexCoord,
				content: s.content,
				title: s.title,
				format: s.format ?? composeFormat,
				tags: s.tags.map(wireTag)
			}));
	}

	function publishAll() {
		if (edit) {
			const reqs = editRequests();
			if (reqs) {
				if (reqs.length === 0) {
					app.pushToast('Nothing changed yet — edit a section, then sign', 'info');
					return;
				}
				app.editReview = reqs;
				return;
			}
		}
		if (isAtomic) {
			onpublish([], atomicMeta());
			return;
		}
		// In plain mode `compose.sections`/`compose.title` only commit on
		// blur, so parse the live text here and publish title+tags+sections
		// directly — otherwise the prop is stale (empty title / no sections)
		// at click time.
		let sections: ContextItem[];
		let meta: PublishMeta | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags, notes: isNotes };
		} else {
			sections = compose.sections;
			meta = isNotes ? { title: compose.title, tags: compose.tags, notes: true } : undefined;
		}
		if (claimedUntouched.length > 0) {
			const n = claimedUntouched.length;
			const ok = confirm(
				`You've left ${n} section${n === 1 ? '' : 's'} unlocked but haven't modified ${n === 1 ? 'it' : 'them'}. ` +
					`Unlocked-but-untouched sections still publish as transclusions of the original. Publish anyway?`
			);
			if (!ok) return;
		}
		onpublish(sections, meta);
	}

	// Save the current compose as a local draft. Same section/meta resolution
	// as publish, but never signs — so it's available regardless of identity.
	function saveDraftAction() {
		if (!onsavedraft) return;
		if (isAtomic) {
			onsavedraft([], atomicMeta());
			return;
		}
		let sections: ContextItem[];
		let meta: { title: string; tags: TagEntry[] } | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags };
		} else {
			sections = compose.sections;
		}
		onsavedraft(sections, meta);
	}

	// Diff the current compose against the last published version of this
	// article. Same section/meta resolution as Sign/Save; the result opens the
	// PublishedDiffModal (state owns it via app.publishedDiff).
	function diffPublishedAction() {
		let sections: ContextItem[];
		let meta: { title: string; tags: TagEntry[] } | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags };
		} else {
			sections = compose.sections;
		}
		app.handleComposeDiffPublished(sections, meta);
	}

	let draftsOpen = $state(false);

	// Inspect the would-be 30040/30041 events as JSON — no signing/publish.
	function previewEvents() {
		if (!onpreview) return;
		if (isAtomic) {
			onpreview([], atomicMeta());
			return;
		}
		let sections: ContextItem[];
		let meta: PublishMeta | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags, notes: isNotes };
		} else {
			sections = compose.sections;
			meta = isNotes ? { title: compose.title, tags: compose.tags, notes: true } : undefined;
		}
		onpreview(sections, meta);
	}
</script>

<div class="compose-view">
	<div class="compose-mode-bar" data-tour="compose-modebar">
		<!-- Type: what a Sign publishes. Publication parses the editor into a
		     30040/30041 graph; Notes publishes each section as a standalone
		     30041; Wiki / Spec / Blog / Custom publish the whole body as one
		     event of that kind. -->
		<label class="kind-group" data-tour="compose-kind" title="Type — what a Sign publishes">
			<span class="kind-label">Type</span>
			<select class="kind-select" value={typeValue} onchange={onKindSelect}>
				{#each KIND_PRESETS as preset (preset.value)}
					<option value={preset.value}>{preset.label}</option>
				{/each}
			</select>
			{#if isCustomKind}
				<input
					class="kind-input"
					type="number"
					min="0"
					value={composeKind}
					oninput={onCustomKind}
					aria-label="Event kind number"
					title="Event kind number"
				/>
			{/if}
		</label>
		<!-- Style: the markup the editor reads and the toolbar writes, stamped
		     as every section's `m` tag. Changing it re-reads the text as the new
		     style; nothing is converted. -->
		<label class="kind-group" data-tour="compose-format" title="Style — the markup of the text (its m tag). Changing it re-reads the text; nothing is converted.">
			<span class="kind-label">Style</span>
			<select
				class="kind-select"
				value={composeFormat}
				onchange={(e) => (app.composeFormat = e.currentTarget.value)}
			>
				{#each CANONICAL_MIMETYPES as mt (mt)}
					<option value={mt}
						>{FORMAT_LABEL[mt]}{KIND_STYLE[composeKind] === mt ? ' · kind default' : ''}</option
					>
				{/each}
			</select>
		</label>
		<label class="kind-group" data-tour="compose-view" title="View — Rendered hides the markup; Show markers dims it; Source is the plain text">
			<span class="kind-label">View</span>
			<select
				class="kind-select"
				value={viewMode}
				onchange={(e) => setViewMode(e.currentTarget.value as ViewMode)}
			>
				<option value="rendered">Rendered</option>
				<option value="markers">Show markers</option>
				<option value="source">Source</option>
			</select>
		</label>
		<span class="bar-sp"></span>
		{#if compose.source_publication_addr && !isAtomic}
			<button
				class="bulk-btn"
				onclick={unlockAllImported}
				disabled={!anyLockable}
				title="Unlock every section of the publication for editing"
			>Unlock all</button>
			<button
				class="bulk-btn"
				onclick={lockAllUnlocked}
				disabled={!anyUnlocked}
				title="Re-lock unlocked sections that haven't been modified"
			>Lock all</button>
		{/if}
		<label
			class="ref-auto"
			title="Inline autocomplete — suggest ref / wiki / embed / quote as you type the brace syntax in the editor"
		>
			<input type="checkbox" bind:checked={autocompleteOn} />
			auto refs
		</label>
		{#if !isAtomic}
			<button
				class="read-btn"
				onclick={() => {
					if (mode === 'plain') handlePlainFullEdit(plainText);
					try {
						const store = getActiveStore();
						store.openBuffer({
							className: 'work',
							buffer: {
								id: 'draft-reader:current',
								kind: 'draft-reader',
								label: 'draft',
								kicker: compose.title || 'preview'
							}
						});
					} catch {
						// No WM store — inline preview as fallback.
						mode = 'preview';
					}
				}}
				class:active={mode === 'preview'}
				title="Preview the draft in a separate buffer"
			>Read</button>
			<div class="compose-walk">
				<button
					class="affordance affordance--walkthrough"
					onclick={() => (walkMenuOpen = !walkMenuOpen)}
					title="Composer walkthroughs — pick a guided tour"
					aria-label="Composer walkthroughs"
					aria-expanded={walkMenuOpen}
				>W</button>
				{#if walkMenuOpen}
					<!-- svelte-ignore a11y_click_events_have_key_events -->
					<div class="walk-backdrop" onclick={() => (walkMenuOpen = false)} role="presentation"></div>
					<div class="walk-menu" role="menu">
						<div class="walk-head">Composer walkthroughs</div>
						{#each composerTours as t (t.key)}
							<button class="walk-row" role="menuitem" onclick={() => runComposeTour(t)}>
								<span
									class="walk-check"
									class:walk-check--on={discovery.seen.includes(t.key)}
									title={discovery.seen.includes(t.key) ? 'Run before' : 'Not run yet'}
								>{discovery.seen.includes(t.key) ? '✓' : '·'}</span>
								<span class="walk-title">{TIPS[t.key]?.title ?? t.key}</span>
							</button>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
		<button
			class="affordance affordance--help"
			onclick={openComposeHelp}
			title="Composer reference — the editor, sections, tags, and draft → sign → broadcast"
			aria-label="Composer help"
		>?</button>
	</div>

	{#if edit}
		<div class="edit-banner" class:edit-banner--fork={!edit.own} data-tour="compose-edit">
			<span class="edit-pill">{edit.own ? 'yours' : 'fork'}</span>
			<span>
				{#if edit.mode === 'section'}
					Editing “{edit.title || 'section'}” in its publication — the other sections are locked.
				{:else if edit.mode === 'publication'}
					Editing the whole publication.
				{:else}
					Editing “{edit.title}”.
				{/if}
				{edit.own
					? 'Sign republishes your changes in place (same d).'
					: 'Sign publishes your copy and sends the author a merge request; their version is untouched until they merge.'}
			</span>
		</div>
	{/if}

	{#snippet tagsRail(sectionTabs: boolean)}
		<div class="rail-tags" data-tour="compose-tags">
			{#if sectionTabs}
				<div class="rail-tabs" role="tablist" aria-label="Tags of">
					<button
						role="tab"
						aria-selected={railTab === 'doc'}
						class:on={railTab === 'doc'}
						onclick={() => (railTab = 'doc')}>{isNotes ? 'All notes' : 'Publication'}</button
					>
					<button
						role="tab"
						aria-selected={railTab === 'section'}
						class:on={railTab === 'section'}
						disabled={cursorSection < 0}
						title={cursorSection < 0 ? 'Put the cursor in a section' : ''}
						onclick={() => (railTab = 'section')}
						>{cursorSection >= 0
							? detectedSections[cursorSection]?.title || '[Untitled]'
							: 'Section'}</button
					>
				</div>
			{:else}
				<div class="rail-head">Tags</div>
			{/if}
			{#if sectionTabs && railTab === 'section' && cursorSection >= 0}
				{@const det = detectedSections[cursorSection]}
				<TagsPanel
					tags={det?.item?.tags ?? []}
					disabled={!!det?.item?.readonly && !!det?.item?.source_addr}
					auto={['d', 'title', `m ${det?.item?.format ?? composeFormat}`]}
					empty="No tags on this section."
					onupdate={(t) => updateSectionTags(cursorSection, t)}
				/>
			{:else}
				<TagsPanel tags={compose.tags} auto={autoDocTags} onupdate={updateTags} />
			{/if}
		</div>
	{/snippet}

	{#if isAtomic}
		<div class="compose-content">
			<div class="plain-layout" class:plain-layout--closed={!detectedOpen}>
				<div class="plain-editor-wrap editor-col" data-tour="compose-atomic">
					{#if viewMode !== 'source'}
						<FormatToolbar view={atomicCmView} {style} version={edVersion} onref={openRefBuilder} />
					{/if}
					<div class="editor-host">
						<CodeMirrorEditor
							bind:value={app.composeAtomicBody}
							bind:editorView={atomicCmView}
							{lineNumbers}
							{vimMode}
							extensions={atomicExt}
						/>
					</div>
				</div>
				<div class="detected-sections">
					<button
						class="detected-toggle"
						onclick={() => (detectedOpen = !detectedOpen)}
						aria-expanded={detectedOpen}
					>
						<span class="ptr">{detectedOpen ? '▾' : '▸'}</span>
						{atomicParsed.title || '[No title — start with a level-1 heading]'}
					</button>
					{#if detectedOpen}
						{@render tagsRail(false)}
					{/if}
				</div>
			</div>
		</div>
	{:else if mode === 'plain'}
		<div class="compose-content">
			<div class="plain-layout" class:plain-layout--closed={!detectedOpen} data-tour="compose-plain">
				<div class="plain-editor-wrap editor-col">
					{#if viewMode !== 'source'}
						<FormatToolbar view={plainCmView} {style} version={edVersion} onref={openRefBuilder} />
					{/if}
					<div class="editor-host">
						<CodeMirrorEditor
							bind:value={plainText}
							bind:editorView={plainCmView}
							{lineNumbers}
							{vimMode}
							onBlur={handlePlainBlur}
							extensions={plainExt}
						/>
					</div>
				</div>
				<div class="detected-sections" data-tour="compose-detected">
					<button
						class="detected-toggle"
						onclick={() => (detectedOpen = !detectedOpen)}
						aria-expanded={detectedOpen}
						title={detectedOpen
							? 'Hide the outline — the editor takes the full width'
							: 'Show the outline and tags'}
					>
						<span class="ptr">{detectedOpen ? '▾' : '▸'}</span>
						Outline ({detectedSections.length})
					</button>
					{#if detectedOpen}
					<div class="detected-row detected-doc-title">
						<span class="detected-label">{isNotes ? 'notes' : 'title'}</span>
						<span class="detected-title">{isNotes ? 'no index — each section is a note' : detectedState.title || '[No title]'}</span>
					</div>
					{#each detectedSections as det, di (det.index)}
						<div
							class="detected-row"
							class:detected-row--nested={det.level > 2}
							class:detected-row--slot={!!det.slot}
							class:detected-row--cur={di === cursorSection}
							style="--depth: {Math.max(0, det.level - 2)}"
						>
							{#if det.slot}
								<span class="detected-title detected-slot" title={det.slot}>⧉ slot · {slotLabel(det.slot)}</span>
							{:else}
								<button
									class="detected-title detected-jump"
									class:detected-new={!det.item}
									onclick={() => jumpTo(det.line)}
									title="Go to this section">{det.title || '[Untitled]'}</button
								>
								{#if det.item}
									<ItemBadge item={det.item} {syncMode} panel="compose" {ontogglereadonly} {onlocksource} {oncrosspanelcopy} />
									<button class="icon-btn-sm" onclick={() => onsenditemtochat(det.item!.id)} title="Send to chat" aria-label="Send to chat">◂</button>
								{:else}
									<span class="badge badge-new">new</span>
								{/if}
							{/if}
							<button
								class="icon-btn-sm"
								onclick={() => reorderInPlain(di, 'up')}
								disabled={di === 0}
								title="Move section up"
								aria-label="Move section up"
							>↑</button>
							<button
								class="icon-btn-sm"
								onclick={() => reorderInPlain(di, 'down')}
								disabled={di === detectedSections.length - 1}
								title="Move section down"
								aria-label="Move section down"
							>↓</button>
						</div>
					{/each}
					{#if detectedSections.length === 0}
						<div class="detected-empty">
							Type <code>{headFor(2)}</code> (or press H2) to start a section;
							<code>{headFor(3)}</code> nests one under it.
						</div>
					{/if}
					{@render tagsRail(true)}
					{/if}
				</div>
			</div>
		</div>
	{:else}
		<div class="compose-content">
			<!-- Preview = the read view of the current draft. -->
			<DraftReader
				{compose}
				{ontogglereadonly}
				onremove={removeSection}
				onunlockall={unlockAllImported}
				onlockall={lockAllUnlocked}
			/>
		</div>
	{/if}

	{#if drafts.length > 0}
		<div class="compose-drafts">
			<button
				class="compose-drafts-head"
				onclick={() => (draftsOpen = !draftsOpen)}
				aria-expanded={draftsOpen}
			>
				<span class="ptr">{draftsOpen ? '▾' : '▸'}</span>
				Saved drafts <span class="compose-drafts-count">({drafts.length})</span>
			</button>
			{#if draftsOpen}
				<div class="compose-drafts-list">
					<DraftVersions
						{drafts}
						onload={(id) => onloaddraft?.(id)}
						ondelete={(id) => ondeletedraft?.(id)}
					/>
				</div>
			{/if}
		</div>
	{/if}

	<div class="compose-actions" data-tour="compose-actions">
		{#if onpreview}
			<button
				class="preview-events-btn"
				onclick={previewEvents}
				disabled={noContent}
				title={isAtomic
					? `Inspect the kind ${composeKind} event this draft would publish, as JSON`
					: 'Inspect the 30040/30041 events this draft would publish, as JSON'}
			>Preview events</button>
		{/if}
		{#if onsavedraft}
			<button
				class="save-draft-btn"
				onclick={saveDraftAction}
				disabled={noContent}
				title="Save this draft locally — survives refresh; resume it from the Saved drafts list"
			>Save draft</button>
		{/if}
		{#if !isAtomic && !isNotes}
			<button
				class="diff-published-btn"
				onclick={diffPublishedAction}
				disabled={mode === 'plain'
					? detectedSections.length === 0
					: compose.sections.length === 0}
				title="Diff the current draft against the last published version of this article"
			>Diff vs published</button>
		{/if}
		{#if canPublish}
			<button
				class="publish-btn"
				onclick={publishAll}
				disabled={isAtomic
					? !atomicCanPublish
					: mode === 'plain'
						? detectedSections.length === 0
						: compose.sections.length === 0 || !structuralChange}
				title={isAtomic
					? atomicCanPublish
						? 'Sign a local snapshot (broadcast it separately when ready)'
						: 'Add a title and body to enable signing'
					: mode === 'plain'
						? detectedSections.length === 0
							? 'Type a heading line to detect a section'
							: 'Sign a local snapshot (broadcast it separately when ready)'
						: structuralChange
							? 'Sign a local snapshot (broadcast it separately when ready)'
							: compose.source_publication_addr
								? 'No structural change since the source publication — nothing to sign'
								: 'Add or modify a section to enable signing'}
			>{edit ? (edit.own ? 'Republish…' : 'Fork + request merge…') : 'Sign'}</button>
		{/if}
		<button onclick={oncancel}>Cancel</button>
	</div>
</div>

{#if app.publishedDiff}
	<PublishedDiffModal diff={app.publishedDiff} onclose={app.closePublishedDiff} />
{/if}

<ReferenceBuilderModal
	open={refBuilderOpen}
	initialTab={builderTab}
	sectionTitles={refSectionTitles}
	oninsert={insertNostrdownToken}
	onclose={() => {
		refBuilderOpen = false;
		embedRange = null;
	}}
/>

{#if preview}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="nd-preview"
		style="left:{preview.x}px; top:{preview.y}px"
		use:previewPortal
		onmouseenter={() => clearTimeout(previewHideTimer)}
		onmouseleave={scheduleHidePreview}
		role="tooltip"
	><EmbedCard ref={preview.ref} onopen={openPreview} /></div>
{/if}

<style>
	/* Floating wrapper for the click-preview card (portaled to <body>). The card
	   itself is EmbedCard; this frames + lifts it. Mirrors RichContent's reader
	   preview so the composer peek looks identical. */
	.nd-preview {
		position: fixed;
		z-index: 200;
		width: min(340px, 90vw);
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		overflow: hidden;
	}
	.nd-preview :global(.nd-embed) {
		margin: 0;
	}

	.compose-view {
		flex: 1;
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 16px;
		min-height: 0;
		overflow: hidden;
	}

	.compose-mode-bar {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
		flex-shrink: 0;
	}

	/* Segmented full/plain switch — a click target independent of vim mode. */

	.kind-group {
		display: flex;
		align-items: center;
		gap: 6px;
		cursor: pointer;
		user-select: none;
	}
	.kind-label {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.kind-select {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: 3px 6px;
		cursor: pointer;
	}
	.kind-input {
		width: 72px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: 3px 6px;
	}

	/* Read-only output-shape indicator — auto-detected Publication vs Notes.
	   Notes is the attention state (no index), so it's tinted accent. */

	/* Atomic body editor — fills the scroll region under the sticky header,
	   same framing as the plain editor wrap. */

	/* Mirrors plain mode's "Detected" affordance: shows what the body's leading
	   :tag: block parses to, so the stripped tags aren't invisible. */




















	.compose-content {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
	}

	.plain-layout {
		display: flex;
		flex: 1;
		gap: 0;
		min-height: 0;
	}
	/* Closed: the panel folds to a slim full-width bar under the editor —
	   the editor takes the whole width, the caret stays reachable. */
	.plain-layout--closed {
		flex-direction: column;
	}
	.plain-layout--closed .detected-sections {
		width: auto;
		border-left: none;
		border-top: 1px solid var(--border);
	}

	.plain-editor-wrap {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.detected-sections {
		width: 260px;
		flex-shrink: 0;
		border-left: 1px solid var(--border);
		overflow-y: auto;
		background: var(--bg);
	}
	/* Narrow widths: even the open panel stacks below the editor as a
	   bottom drawer instead of stealing half the line length. */
	@media (max-width: 768px) {
		.plain-layout {
			flex-direction: column;
		}
		.detected-sections {
			width: auto;
			border-left: none;
			border-top: 1px solid var(--border);
			max-height: 45%;
		}
	}

	.detected-toggle {
		display: flex;
		align-items: center;
		gap: 4px;
		width: 100%;
		background: none;
		border: none;
		cursor: pointer;
		text-align: left;
		font-size: var(--t-3xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 8px 10px 4px;
	}
	.detected-toggle:hover {
		color: var(--fg);
	}
	.plain-layout--closed .detected-toggle {
		padding-bottom: 8px;
	}

	.detected-row {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 4px 10px;
		padding-left: calc(10px + var(--depth, 0) * 14px);
		border-bottom: 1px solid var(--border);
		font-size: var(--t-2xs);
	}

	.detected-row--nested {
		/* Nested sections (level >= 3) sit visually under their shallower
		   sibling. The actual indent is driven by the inline --depth css
		   variable so any level renders with the right offset. */
		color: var(--fg-muted);
	}

	.detected-row--nested .detected-title {
		font-weight: 500;
	}

	.detected-title {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-weight: 600;
		font-size: var(--t-2xs);
	}

	.detected-new {
		color: var(--fg-muted);
		font-style: italic;
	}
	.detected-jump {
		background: none;
		border: none;
		color: inherit;
		text-align: left;
		cursor: pointer;
		padding: 2px 0;
		font-family: inherit;
	}
	.detected-jump:hover {
		color: var(--id-yours);
	}
	.detected-row--cur {
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
	}
	.detected-empty code {
		font-family: var(--font-mono);
	}
	.editor-col {
		min-width: 0;
	}
	.editor-host {
		flex: 1;
		min-height: 0;
		overflow: hidden;
		display: flex;
		flex-direction: column;
	}
	.edit-banner {
		display: flex;
		gap: 10px;
		align-items: baseline;
		padding: 8px 12px;
		margin-bottom: 8px;
		font-size: var(--t-sm);
		border: 1px solid color-mix(in srgb, var(--id-yours) 45%, var(--border));
		background: color-mix(in srgb, var(--id-yours) 8%, transparent);
		border-radius: var(--radius);
	}
	.edit-banner--fork {
		border-color: color-mix(in srgb, var(--id-forked, #b294bb) 55%, var(--border));
		background: color-mix(in srgb, var(--id-forked, #b294bb) 8%, transparent);
	}
	.edit-pill {
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid currentColor;
		color: var(--id-yours);
		flex-shrink: 0;
	}
	.edit-banner--fork .edit-pill {
		color: var(--id-forked, #b294bb);
	}
	.rail-tags {
		padding: 10px;
		border-top: 1px solid var(--border);
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.rail-head {
		font-size: var(--t-3xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.rail-tabs {
		display: flex;
		gap: 4px;
		flex-wrap: wrap;
	}
	.rail-tabs button {
		font: inherit;
		font-size: var(--t-2xs);
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		border-radius: var(--r-sm);
		padding: 3px 8px;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		cursor: pointer;
	}
	.rail-tabs button.on {
		border-color: var(--id-yours);
		color: var(--fg);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.rail-tabs button:disabled {
		opacity: 0.45;
		cursor: default;
	}

	/* A transclude slot in the outline — an existing event referenced by the
	   index, not an authored section. Tinted to read as "borrowed". */
	.detected-slot {
		color: var(--id-yours);
		font-weight: 600;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
	}
	.detected-row--slot {
		background: color-mix(in srgb, var(--id-yours) 7%, transparent);
		border-radius: var(--r-sm, 3px);
	}

	.detected-label {
		font-size: var(--t-3xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.03em;
		flex-shrink: 0;
	}

	.detected-doc-title {
		font-weight: 700;
	}

	.detected-empty {
		padding: 12px 10px;
		color: var(--fg-muted);
		font-size: var(--t-2xs);
		font-style: italic;
	}


	.icon-btn-sm {
		padding: 2px 6px;
		font-size: var(--t-2xs);
		min-width: 22px;
	}

	.badge-new {
		font-size: var(--t-3xs);
		padding: 0 5px;
		border-radius: 4px;
		font-weight: 600;
		line-height: 1.6;
		background: color-mix(in srgb, var(--accent) 20%, transparent);
		color: var(--accent);
	}





	.compose-actions {
		display: flex;
		gap: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
		flex-shrink: 0;
	}
	/* Narrow (mobile) widths: wrap the action row instead of clipping the
	   trailing buttons off the right edge. */
	@media (max-width: 600px) {
		.compose-actions {
			flex-wrap: wrap;
		}
	}

	/* Saved-drafts list — collapsible, sits just above the action row. */
	.compose-drafts {
		flex-shrink: 0;
		border-top: 1px solid var(--border);
		padding-top: 8px;
	}
	.compose-drafts-head {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-2xs);
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 0;
	}
	.compose-drafts-head:hover {
		color: var(--fg);
	}
	.compose-drafts-count {
		color: var(--fg-muted);
	}
	.compose-drafts-list {
		list-style: none;
		margin: 6px 0 0;
		padding: 0;
		max-height: 180px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.publish-btn {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
		font-weight: 600;
	}

	.publish-btn:disabled {
		opacity: 0.4;
	}


	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}

	.bar-sp { flex: 1; }

	/* Match ReaderBuffer's `.toolbar .bulk` look so the right-cluster of
	   the compose mode-bar reads as the same control row as the read view. */
	.bulk-btn {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.bulk-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--id-yours);
	}
	.bulk-btn:disabled { opacity: 0.4; cursor: not-allowed; }

	/* Symmetric round-trip with ReaderBuffer's Edit button: Read = green,
	   sits at the far right of the same toolbar row as Unlock/Lock all. */
	.read-btn {
		background: color-mix(in srgb, var(--green) 18%, transparent);
		color: var(--green);
		border-color: var(--green);
		font-weight: 600;
	}
	.read-btn:hover {
		background: color-mix(in srgb, var(--green) 28%, transparent);
	}
	.read-btn.active {
		background: var(--green);
		color: var(--bg);
		border-color: var(--green);
	}

	/* Composer walkthrough dropdown — the in-chrome W's menu of every composer
	   tutorial. The W button itself uses the global `.affordance` styling shared
	   with the mode-line; only the menu is local. */
	.compose-walk {
		position: relative;
		display: inline-flex;
	}
	/* Autocomplete checkbox — the inline `{{` suggestion mode, beside the
	   reference-builder button (which shares the global `.affordance` base
	   with W / ?). */
	.ref-auto {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		cursor: pointer;
		user-select: none;
	}
	.ref-auto:hover {
		color: var(--fg);
	}
	.ref-auto input {
		accent-color: var(--id-yours);
		margin: 0;
		cursor: pointer;
	}
	.walk-backdrop {
		position: fixed;
		inset: 0;
		z-index: 40;
	}
	.walk-menu {
		position: absolute;
		top: calc(100% + 4px);
		right: 0;
		z-index: 41;
		min-width: 232px;
		max-width: 300px;
		padding: 4px;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--border));
		border-radius: var(--radius);
		box-shadow: var(--shadow-md);
		display: flex;
		flex-direction: column;
		gap: 1px;
	}
	.walk-head {
		font-size: var(--t-3xs);
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--fg-muted);
		padding: 4px 8px 5px;
	}
	.walk-row {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		text-align: left;
		padding: 5px 8px;
		background: transparent;
		border: none;
		border-radius: var(--r-sm, 4px);
		color: var(--fg);
		font-size: var(--t-2xs);
		cursor: pointer;
	}
	.walk-row:hover {
		background: color-mix(in srgb, var(--accent) 14%, transparent);
	}
	.walk-check {
		flex: 0 0 auto;
		width: 12px;
		text-align: center;
		color: var(--fg-muted);
		opacity: 0.5;
	}
	.walk-check--on {
		color: var(--green);
		opacity: 1;
	}
	.walk-title {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	/* Opaque view tag — which editor view the tour applies to (plain/full).
	   Absent for view-agnostic tours. */
</style>
