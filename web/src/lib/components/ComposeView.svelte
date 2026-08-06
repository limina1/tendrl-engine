<script lang="ts">
	import { untrack } from 'svelte';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';
	import type { ComposeState, ContextItem, TagEntry, SyncMode } from '$lib/types';
	import type { DraftSummary } from '$lib/api';
	import { resolveNostrdown, search } from '$lib/api';
	import type { EditorView } from '@codemirror/view';
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
	import ComposeSection from './ComposeSection.svelte';
	import ReferenceBuilderModal from './ReferenceBuilderModal.svelte';
	import ItemBadge from './ItemBadge.svelte';
	import TagEditor from './TagEditor.svelte';
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
		const d = effectiveDelim();
		const heads: { pos: number; title: string }[] = [];
		let offset = 0;
		for (const line of doc.split('\n')) {
			const head = parseHeadingLine(line, d);
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
	function runComposeTour(t: { key: string; mode?: 'full' | 'plain' }) {
		// A `mode`-tagged tour walks a specific view — switch to it first so its
		// anchors are mounted when the tip resolves. View-agnostic tours leave the
		// current view untouched. (No-op under an atomic kind, which has no views.)
		if (t.mode && !isAtomic) mode = t.mode;
		walkMenuOpen = false;
		runTour(t.key);
	}

	type ComposeMode = 'full' | 'plain' | 'preview';

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
	const KIND_PRESETS = [
		{ kind: 30040, label: 'Publication' },
		{ kind: 30023, label: 'Blog' },
		{ kind: 30818, label: 'Wiki' },
		{ kind: -1, label: 'Custom…' }
	];

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
		mode = $bindable<ComposeMode>('full'),
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

	let checkedIds: Set<string> = $state(new Set());
	let collapsedIds: Set<string> = $state(new Set());
	let headerCollapsed = $state(false);

	// --- Output kind (Publication vs atomic blog/wiki/custom) ---
	// Owned by app state so resuming a draft can restore the editor mode. The
	// select reflects it; a non-preset kind shows the free numeric input.
	const PRESET_KINDS = new Set(KIND_PRESETS.filter((p) => p.kind !== -1).map((p) => p.kind));
	const composeKind = $derived(app.composeKind);
	const isAtomic = $derived(composeKind !== 30040);
	const isCustomKind = $derived(!PRESET_KINDS.has(composeKind));

	function onKindSelect(e: Event) {
		const v = +(e.currentTarget as HTMLSelectElement).value;
		// -1 = "Custom…": keep the current kind if it's already custom, else
		// seed a non-preset default (1 = a plain note) the user can edit.
		app.composeKind = v === -1 ? (isCustomKind ? composeKind : 1) : v;
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
				app.composeAtomicBody = compose.sections
					.map((s) => s.content)
					.filter((c) => c.trim().length > 0)
					.join('\n\n');
			}
			app.composeAtomicSeeded = true;
		});
	});

	const atomicCanPublish = $derived(
		isAtomic && compose.title.trim().length > 0 && app.composeAtomicBody.trim().length > 0
	);

	/** Pull a leading run of `:name: value` tag lines off the atomic body — the
	 *  same `:tag:` / `:tags:` syntax plain mode uses for its doc header — so an
	 *  atomic blog/wiki can carry tags inline. `:tags: a, b` expands to `t`
	 *  tags (via parseTagLine). Returns the parsed tags and the body with the
	 *  block (and its trailing blank separator) stripped. */
	function parseAtomicBody(text: string): { tags: TagEntry[]; content: string } {
		const lines = text.split('\n');
		const tags: TagEntry[] = [];
		let i = 0;
		for (; i < lines.length; i++) {
			const parsed = parseTagLine(lines[i]);
			if (!parsed) break;
			tags.push(...parsed);
		}
		if (tags.length > 0) {
			while (i < lines.length && lines[i].trim() === '') i++;
		}
		return { tags, content: lines.slice(i).join('\n') };
	}

	/** Header (TagEditor) tags plus body-parsed tags, de-duped by name+value. */
	function mergeTags(base: TagEntry[], extra: TagEntry[]): TagEntry[] {
		const seen = new Set(base.map((t) => `${t.name} ${t.value}`));
		const out = [...base];
		for (const t of extra) {
			const key = `${t.name} ${t.value}`;
			if (!seen.has(key)) {
				seen.add(key);
				out.push(t);
			}
		}
		return out;
	}

	// Live view of what the body's leading tag block parses to — drives the
	// "detected tags" hint and the publish payload so they can't drift.
	const atomicParsed = $derived(
		isAtomic ? parseAtomicBody(app.composeAtomicBody) : { tags: [] as TagEntry[], content: '' }
	);

	function atomicMeta(): PublishMeta {
		return {
			title: compose.title,
			tags: mergeTags(compose.tags, atomicParsed.tags),
			kind: composeKind,
			content: atomicParsed.content
		};
	}

	let delimiter = $state('');
	// Parse level — the heading depth at which sections stop being their
	// own 30040 indices and fold into the nearest ancestor's 30041 content.
	// 2 = flat (one index over a flat list of sections); each higher level
	// turns one more heading tier into nested sub-indices and folds anything
	// deeper into content. Mirrors Alexandria's parseLevel
	// (docs/publication_creation.md §1.2). Range 2–5.
	let parseLevel = $state(2);
	let prevDelimiter = $state('');
	let trashPending: ContextItem[] = $state([]);
	let trashTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let trashCountdown = $state(0);
	let countdownInterval: ReturnType<typeof setInterval> | null = $state(null);

	// --- Serialize / Parse ---

	function effectiveDelim(): string {
		return delimiter.trim() || '=';
	}

	/** Heading prefix for level N — N copies of the active delimiter
	 *  followed by a single space. Level 1 = publication root, level 2 =
	 *  top-level section, level 3+ = nested. */
	function headFor(level: number): string {
		return effectiveDelim().repeat(level) + ' ';
	}

	function headForWith(d: string, level: number): string {
		return d.repeat(level) + ' ';
	}

	/** If `line` starts with one of the heading prefixes for the active
	 *  delimiter (level 1..6), return that level and the title; else null.
	 *  Counts leading delimiter chars and requires a trailing space. An *empty*
	 *  title (`= ` with nothing after) is still a heading — so a fresh draft's
	 *  lone `= ` round-trips through Full↔Plain instead of being misread as
	 *  content (which would explode into a stray `=`/`==`/`=` on re-serialize). */
	function parseHeadingLine(line: string, d: string): { level: number; title: string } | null {
		if (line.length === 0 || line[0] !== d) return null;
		let i = 0;
		while (i < line.length && line[i] === d) i++;
		if (i < 1 || i > 6) return null;
		if (line[i] !== ' ') return null;
		const title = line.slice(i + 1).trimEnd();
		return { level: i, title };
	}

	// Reactively swap delimiters in plain text — preserves the level
	// count per heading line by counting the old delim and replacing
	// with the same count of the new delim.
	$effect(() => {
		const cur = effectiveDelim();
		if (mode === 'plain' && prevDelimiter && cur !== prevDelimiter) {
			plainText = plainText
				.split('\n')
				.map((line) => {
					const old = parseHeadingLine(line, prevDelimiter);
					if (!old) return line;
					return headForWith(cur, old.level) + old.title;
				})
				.join('\n');
		}
		prevDelimiter = cur;
	});

	function serializeTagBlock(tags: TagEntry[]): string {
		if (tags.length === 0) return '';
		const tValues: string[] = [];
		const lines: string[] = [];
		for (const tag of tags) {
			if (tag.name === 't') {
				tValues.push(tag.value);
			} else {
				lines.push(`:${tag.name}: ${tag.value}`);
			}
		}
		if (tValues.length > 0) {
			lines.push(`:tags: ${tValues.join(', ')}`);
		}
		return lines.join('\n') + '\n';
	}

	function parseTagLine(line: string): TagEntry[] | null {
		const match = line.match(/^:([^:]+):\s*(.*)$/);
		if (!match) return null;
		const name = match[1].trim();
		const value = match[2].trim();
		if (name === 'tags') {
			return value
				.split(',')
				.map((s) => s.trim())
				.filter(Boolean)
				.map((v) => ({ name: 't', value: v }));
		}
		return [{ name, value }];
	}

	function serializeSection(s: ContextItem): string {
		const level = s.level ?? 2;
		let out = `${headFor(level)}${s.title}\n`;
		out += serializeTagBlock(s.tags);
		out += `\n${s.content}`;
		return out;
	}

	// Serialize entire document into one text blob
	function serializeAll(): string {
		let out = `${headFor(1)}${compose.title}\n`;
		out += serializeTagBlock(compose.tags);
		for (const s of compose.sections) {
			if (s.slot) {
				// A slot serializes back as its `{{slot:…}}` line — so a resumed
				// draft / mode switch reconstructs it as a block, not a section.
				out += `\n{{slot:${s.slot}}}\n`;
				continue;
			}
			const level = s.level ?? 2;
			out += `\n${headFor(level)}${s.title}\n`;
			out += serializeTagBlock(s.tags);
			out += `\n${s.content}\n`;
		}
		return out;
	}

	// Serialize a *parsed* document (used by plain-mode reorder: parse →
	// swap sections in the array → write back without round-tripping
	// through compose.sections). Carries per-section level so heading
	// depth round-trips through the parser.
	function serializeParsed(
		title: string,
		tags: TagEntry[],
		sections: ParsedSection[]
	): string {
		let out = `${headFor(1)}${title}\n`;
		out += serializeTagBlock(tags);
		for (const s of sections) {
			if (s.slot) {
				// A slot round-trips as its own `{{slot:…}}` line — no heading,
				// tags, or body — so reorder/move preserves it like any block.
				out += `\n{{slot:${s.slot}}}\n`;
				continue;
			}
			out += `\n${headFor(s.level)}${s.title}\n`;
			out += serializeTagBlock(s.tags);
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
		plainText = serializeParsed(parsed.title, parsed.tags, next);
	}

	interface ParsedSection {
		title: string;
		tags: TagEntry[];
		content: string;
		/** Heading depth — 2 for `== Title`, 3 for `=== Subtitle`, … */
		level: number;
		/** Set when this item is a block-level `{{slot:target}}` transclude —
		 *  the naddr/coordinate of an existing 30040/30041 to reference as a
		 *  child of the index (no title/content/heading of its own). */
		slot?: string;
	}

	// Parse full text blob back into title/tags + sections. Recognises
	// any heading level >= 2 as a section; level 1 is reserved for the
	// publication title. Per-section level rides through to compose so
	// the engine can emit the nested 30040/30041 graph.
	function parseAll(text: string): {
		title: string;
		hasTitle: boolean;
		tags: TagEntry[];
		sections: ParsedSection[];
	} {
		const d = effectiveDelim();
		const lines = text.split('\n');
		let docTitle = '';
		// Whether a level-1 `=` heading LINE was seen — distinct from `docTitle`
		// being non-empty. An explicit but empty `= ` sets this true (→ Publication
		// shape); a doc with only `==` sections and no `=` line leaves it false
		// (→ Notes). Drives the auto-detected shape, not the title text.
		let hasTitle = false;
		const docTags: TagEntry[] = [];
		const sections: ParsedSection[] = [];
		let current: {
			title: string;
			tags: TagEntry[];
			contentLines: string[];
			inTags: boolean;
			level: number;
		} | null = null;
		let inDocHeader = true;
		let docInTags = true;

		for (const line of lines) {
			const head = parseHeadingLine(line, d);
			if (inDocHeader && !hasTitle && head && head.level === 1) {
				docTitle = head.title;
				hasTitle = true;
				continue;
			}
			// Sections at levels 2..parseLevel become their own segments;
			// anything deeper falls through and folds into the nearest
			// ancestor's content (header text and all).
			if (head && head.level >= 2 && head.level <= parseLevel) {
				// Finish previous section
				if (current) {
					sections.push({
						title: current.title,
						tags: current.tags,
						content: current.contentLines.join('\n').trim(),
						level: current.level
					});
				}
				inDocHeader = false;
				current = {
					title: head.title,
					tags: [],
					contentLines: [],
					inTags: true,
					level: head.level
				};
				continue;
			}
			// A standalone `{{slot:target}}` line is a block-level transclude
			// slot: it ends the current section and becomes its own ordered
			// item (no heading, no body). Its identity is the target, in the
			// text — so it reorders / round-trips like any other block.
			const slotMatch = line.trim().match(/^\{\{slot:([^}|#]+)\}\}$/);
			if (slotMatch) {
				if (current) {
					sections.push({
						title: current.title,
						tags: current.tags,
						content: current.contentLines.join('\n').trim(),
						level: current.level
					});
					current = null;
				}
				inDocHeader = false;
				sections.push({ title: '', tags: [], content: '', level: 2, slot: slotMatch[1].trim() });
				continue;
			}
			if (inDocHeader) {
				if (docInTags) {
					const parsed = parseTagLine(line);
					if (parsed) {
						docTags.push(...parsed);
						continue;
					}
					docInTags = false;
				}
				// Skip blank lines in header before first section
				if (line.trim() === '') continue;
				// Non-heading content before any section — start an untitled section
				inDocHeader = false;
				current = { title: '', tags: [], contentLines: [line], inTags: false, level: 2 };
			} else if (current) {
				if (current.inTags) {
					const parsed = parseTagLine(line);
					if (parsed) {
						current.tags.push(...parsed);
					} else {
						current.inTags = false;
						current.contentLines.push(line);
					}
				} else {
					current.contentLines.push(line);
				}
			}
		}
		// Finish last section
		if (current) {
			sections.push({
				title: current.title,
				tags: current.tags,
				content: current.contentLines.join('\n').trim(),
				level: current.level
			});
		}
		return { title: docTitle, hasTitle, tags: docTags, sections };
	}

	// Reconcile parsed sections with existing compose sections. Carries
	// the parsed heading level through to each ContextItem so the engine's
	// publish path sees the nested-outline shape. Returns the parsed
	// sections so the publish path can use them directly without waiting
	// for the reactive commit.
	function handlePlainFullEdit(
		text: string
	): { title: string; tags: TagEntry[]; sections: ContextItem[] } {
		const parsed = parseAll(text);
		const oldSections = compose.sections;

		// Match parsed sections to existing by position, create new for extras
		const newSections: ContextItem[] = parsed.sections.map((p, i) => {
			const existing = i < oldSections.length ? oldSections[i] : null;
			if (existing) {
				return {
					...existing,
					title: p.title,
					content: p.content,
					tags: p.tags,
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
				tags: p.tags,
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

		onupdate({ title: parsed.title, tags: parsed.tags, sections: newSections });
		return { title: parsed.title, tags: parsed.tags, sections: newSections };
	}

	// Detected structure for plain mode sidebar
	let plainText = $state('');
	const detectedState = $derived.by(() => {
		if (mode !== 'plain')
			return {
				title: '',
				hasTitle: false,
				tags: [] as TagEntry[],
				sections: [] as {
					title: string;
					item: ContextItem | null;
					index: number;
					level: number;
					slot?: string;
				}[]
			};
		const parsed = parseAll(plainText);
		const oldSections = compose.sections;
		return {
			title: parsed.title,
			hasTitle: parsed.hasTitle,
			tags: parsed.tags,
			sections: parsed.sections.map((p, i) => {
				const existing = i < oldSections.length ? oldSections[i] : null;
				return { title: p.title, item: existing, index: i, level: p.level, slot: p.slot };
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
	const isNotes = $derived(!isAtomic && hasSections && !hasDocTitle);

	// Fingerprint of section identity + lock/divergence state, so we can
	// detect external changes. readonly/modified flips cover the badge
	// lock-to-source cycle, which resets content to the original — the
	// editor text must follow.
	let knownSectionFp = $state('');
	function sectionFp(): string {
		return compose.sections
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
				prevDelimiter = effectiveDelim();
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

	// --- Trash state ---

	function clearTrash() {
		trashPending = [];
		trashCountdown = 0;
		if (trashTimer) clearTimeout(trashTimer);
		trashTimer = null;
		if (countdownInterval) clearInterval(countdownInterval);
		countdownInterval = null;
	}

	const trashActive = $derived(trashPending.length > 0);

	// --- Section handlers ---

	function toggleCheck(id: string) {
		const next = new Set(checkedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedIds = next;
		clearTrash();
	}

	function toggleCollapse(id: string) {
		const next = new Set(collapsedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		collapsedIds = next;
	}

	function toggleAllCollapsed() {
		if (allCollapsed) {
			collapsedIds = new Set();
		} else {
			collapsedIds = new Set(compose.sections.map((s) => s.id));
		}
	}

	const allCollapsed = $derived(
		compose.sections.length > 0 && compose.sections.every((s) => collapsedIds.has(s.id))
	);

	// Toolbar actions work across all modes via compose.sections
	function toolbarSendToChat() {
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length > 0) {
			onsendtochat(items);
			checkedIds = new Set();
		}
		clearTrash();
	}

	function toolbarPublish() {
		publishSelected();
		clearTrash();
	}

	function toolbarTrash() {
		if (trashPending.length > 0) {
			ondeletepermanent(trashPending);
			checkedIds = new Set();
			clearTrash();
			return;
		}
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length === 0) return;
		ondelete(items);
		trashPending = items;
		checkedIds = new Set();
		trashCountdown = 10;
		trashTimer = setTimeout(clearTrash, 10000);
		countdownInterval = setInterval(() => {
			trashCountdown--;
			if (trashCountdown <= 0) clearTrash();
		}, 1000);
	}

	function toolbarSelectAll() {
		checkedIds = new Set(compose.sections.map((s) => s.id));
	}

	function toolbarInvert() {
		const next = new Set<string>();
		for (const s of compose.sections) {
			if (!checkedIds.has(s.id)) next.add(s.id);
		}
		checkedIds = next;
	}

	function handlePlainBlur() {
		handlePlainFullEdit(plainText);
	}

	function updateTitle(e: Event) {
		onupdate({ ...compose, title: (e.target as HTMLInputElement).value });
	}

	function updateTags(tags: TagEntry[]) {
		onupdate({ ...compose, tags });
	}

	function updateSection(id: string, title: string, content: string) {
		const sections = compose.sections.map((s) =>
			s.id === id ? { ...s, title, content, modified: content !== s.original_content } : s
		);
		onupdate({ ...compose, sections });
	}

	function updateSectionTags(id: string, tags: TagEntry[]) {
		const sections = compose.sections.map((s) => (s.id === id ? { ...s, tags } : s));
		onupdate({ ...compose, sections });
	}

	function resetSection(id: string) {
		const sections = compose.sections.map((s) =>
			s.id === id ? { ...s, content: s.original_content, modified: false } : s
		);
		onupdate({ ...compose, sections });
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
	const anyLockable = $derived(
		compose.sections.some((s) => s.source_addr && s.readonly)
	);
	const anyUnlocked = $derived(
		compose.sections.some((s) => s.source_addr && !s.readonly && s.content === s.original_content)
	);

	function addSection() {
		const item: ContextItem = {
			id: crypto.randomUUID(),
			title: '',
			content: '',
			context_content: '',
			tags: [],
			original_content: '',
			modified: false,
			in_context: false,
			in_compose: true,
			held: false,
			origin: 'compose' as const,
			// Default-locked. The user explicitly unlocks (yellow) before
			// editing, matching the model used for transcluded sections.
			readonly: true
		};
		onupdate({ ...compose, sections: [...compose.sections, item] });
	}

	function publishAll() {
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

	function publishSelected() {
		let all: ContextItem[];
		let meta: { title: string; tags: TagEntry[] } | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			all = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags };
		} else {
			all = compose.sections;
		}
		const items = all.filter((s) => checkedIds.has(s.id));
		if (items.length === 0) return;
		const claimedInSelection = items.filter(
			(s) => s.source_addr && !s.readonly && s.content === s.original_content
		);
		if (claimedInSelection.length > 0) {
			const n = claimedInSelection.length;
			const ok = confirm(
				`You've left ${n} of the selected section${n === 1 ? '' : 's'} unlocked but haven't modified ${n === 1 ? 'it' : 'them'}. Publish anyway?`
			);
			if (!ok) return;
		}
		onpublish(items, meta);
		checkedIds = new Set();
	}
</script>

<div class="compose-view">
	<div class="compose-mode-bar" data-tour="compose-modebar">
		<!-- Output kind. Publication parses the editor into a 30040/30041
		     section graph; Blog/Wiki/Custom publish the whole body as a single
		     atomic event of that kind. The numeric input lets the user pick any
		     replaceable kind directly. -->
		<label class="kind-group" data-tour="compose-kind" title="Output kind — Publication (30040/41 section graph) or a single atomic event (blog/wiki/custom)">
			<span class="kind-label">kind</span>
			<select
				class="kind-select"
				value={isCustomKind ? -1 : composeKind}
				onchange={onKindSelect}
			>
				{#each KIND_PRESETS as preset (preset.kind)}
					<option value={preset.kind}>{preset.label}</option>
				{/each}
			</select>
			{#if isCustomKind}
				<input
					class="kind-input"
					type="number"
					min="0"
					value={composeKind}
					oninput={onCustomKind}
					title="Event kind number"
				/>
			{/if}
		</label>
		{#if !isAtomic}
			<!-- Output shape, AUTO-DETECTED from the document title and shown read-
			     only so the user can see what a Sign will publish: a title binds
			     the sections into one Publication (30040 index + 30041s); no title
			     means scattered Notes (each section a standalone 30041, no index).
			     Same parsed sections either way. -->
			<span
				class="pub-shape"
				data-tour="compose-shape"
				class:pub-shape--notes={isNotes}
				title={isNotes
					? 'Notes — no document title, so each section publishes as a standalone 30041 (no 30040 index). Add a title to bind them into one Publication.'
					: 'Publication — one 30040 index over the parsed 30041 sections. Remove the title to publish them as standalone Notes instead.'}
			>{isNotes ? 'Notes' : 'Publication'}</span>
			<!-- The view starts on the user's compose-default setting (full/plain)
			     and stays switchable here. Normal-mode h/l still toggles it; this
			     segmented control gives a click target that doesn't depend on vim
			     mode. Preview is its own toggle (the Read button on the right). -->
			<div class="mode-toggle" role="group" aria-label="Editor view" data-tour="compose-view">
				<button
					class="mode-seg"
					class:mode-seg--on={mode === 'full'}
					onclick={() => (mode = 'full')}
					title="Full view — structured section cards"
				>full</button>
				<button
					class="mode-seg"
					class:mode-seg--on={mode === 'plain'}
					onclick={() => (mode = 'plain')}
					title="Plain view — one plain-text editor with a live detected-section outline"
				>plain</button>
			</div>
			<div class="delim-group" data-tour="compose-nest">
				<span class="delim-label">delim</span>
				<input
					class="delim-input"
					bind:value={delimiter}
					placeholder="="
					maxlength="2"
				/>
			</div>
			<label
				class="nest-group"
				title="Parse level — how deep the outline nests into 30040 indices. flat = one index over a flat list of sections (deeper headings stay as content); each higher level turns one more heading tier into nested sub-indices and folds anything below it into content."
			>
				<span class="nest-label">nest</span>
				<select class="nest-select" bind:value={parseLevel}>
					<option value={2}>flat</option>
					<option value={3}>1 tier</option>
					<option value={4}>2 tiers</option>
					<option value={5}>3 tiers</option>
				</select>
			</label>
		{/if}
		<span class="bar-sp"></span>
		<!-- Bulk lock/unlock mirrors ReaderBuffer's draft toolbar so the
		     read↔edit transition keeps the same affordances at the same
		     on-screen level. Gated on a source publication since there's
		     nothing to lock against in a from-scratch draft. -->
		{#if compose.source_publication_addr && !isAtomic}
			<button
				class="bulk-btn"
				onclick={unlockAllImported}
				disabled={!anyLockable}
				title="Unlock all imported sections (yellow — claimed for reorder/edit)"
			>Unlock all</button>
			<button
				class="bulk-btn"
				onclick={lockAllUnlocked}
				disabled={!anyUnlocked}
				title="Re-lock unlocked sections that haven't been modified"
			>Lock all</button>
		{/if}
		<!-- Reference builder + autocomplete checkbox — lead the affordance cluster. -->
		<button
			class="affordance affordance--ref"
			onclick={openRefBuilder}
			title="Insert a reference — build a ref / wiki / embed / slot / quote / mention token at the cursor"
			aria-label="Insert reference"
			data-tour="compose-ref"
		>&lbrace;&lbrace; &rbrace;&rbrace;</button>
		<label
			class="ref-auto"
			title="Inline autocomplete — suggest ref / wiki / embed / quote as you type the brace syntax in the editor"
		>
			<input type="checkbox" bind:checked={autocompleteOn} />
			auto
		</label>
		<!-- Read mirrors ReaderBuffer's "Edit" button — same on-screen
		     position (toolbar far-right) so the Edit↔Read swap reads as
		     a single mode toggle. When a source pub exists we navigate to
		     its ReaderBuffer; for from-scratch drafts we fall back to
		     inline DraftReader. Green to signal "read view" symmetric to
		     Edit. Publication-only: an atomic event has no section tree to read. -->
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
		{/if}
		<!-- Composer's own affordances, mirroring the mode-line's W / ? pair: W
		     opens this buffer's walkthrough menu (every composer tutorial), ?
		     opens the flat reference. Each menu row carries a plain/full tag and,
		     when picked, switches the editor to that view before running. -->
		{#if !isAtomic}
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
								{#if t.mode}<span class="walk-mode">{t.mode}</span>{/if}
							</button>
						{/each}
					</div>
				{/if}
			</div>
		{/if}
		<button
			class="affordance affordance--help"
			onclick={openComposeHelp}
			title="Composer reference — modes, the section model, and draft → sign → broadcast"
			aria-label="Composer help"
		>?</button>
	</div>

	<!-- Title + tags and the selection toolbar are factored into snippets so
	     full mode can anchor them (sticky) at the top of its single scroll
	     region (feed-style), while plain mode renders just the toolbar and
	     preview renders neither. -->
	{#snippet composeHeader()}
		<div class="compose-header" class:compose-header--collapsed={headerCollapsed}>
			<div class="compose-title-row">
				<button
					class="collapse-toggle"
					onclick={() => (headerCollapsed = !headerCollapsed)}
					title={headerCollapsed ? 'Expand publication tags' : 'Collapse to title only'}
					aria-expanded={!headerCollapsed}
				>{headerCollapsed ? '▸' : '▾'}</button>
				<input
					class="compose-title"
					value={compose.title}
					oninput={updateTitle}
					placeholder="Publication title"
				/>
				{#if headerCollapsed && compose.tags.length > 0}
					<span class="header-tag-count" title="{compose.tags.length} publication tag{compose.tags.length === 1 ? '' : 's'}">{compose.tags.length} tag{compose.tags.length === 1 ? '' : 's'}</span>
				{/if}
			</div>
			{#if !headerCollapsed}
				<TagEditor tags={compose.tags} onupdate={updateTags} />
			{/if}
		</div>
	{/snippet}

	{#snippet composeToolbar()}
		<div class="compose-toolbar" data-tour="compose-toolbar">
			<button class="sel-btn" onclick={toolbarSelectAll} disabled={compose.sections.length === 0} title="Select all">All</button>
			<button class="sel-btn" onclick={toolbarInvert} disabled={compose.sections.length === 0} title="Invert selection">Inv</button>
			<button class="icon-btn" onclick={toolbarSendToChat} disabled={checkedIds.size === 0} title="Send to chat">◂</button>
			<button class="icon-btn" onclick={toolbarPublish} disabled={checkedIds.size === 0} title="Publish locally">▸</button>
			<button
				class="icon-btn trash-btn"
				class:trash-armed={trashActive}
				onclick={toolbarTrash}
				disabled={checkedIds.size === 0 && !trashActive}
				title={trashActive ? 'Delete everywhere' : 'Remove from compose'}
			>🗑</button>
			{#if trashActive}
				<span class="trash-warn" style:opacity={trashCountdown / 10}>delete everywhere ({trashCountdown}s)</span>
			{/if}
			<span class="toolbar-sp"></span>
			<button
				class="sel-btn"
				onclick={toggleAllCollapsed}
				disabled={compose.sections.length === 0}
				title={allCollapsed ? 'Expand all sections' : 'Collapse all sections to titles'}
			>{allCollapsed ? '▾ all' : '▸ all'}</button>
		</div>
	{/snippet}

	<!-- Atomic mode = one title + tags header over a single body editor. No
	     section parsing, delimiter, or nesting — the whole body is one event of
	     the selected kind (blog/wiki/custom). -->
	{#if isAtomic}
		<div class="compose-content compose-content--scroll">
			<div class="compose-stick">
				{@render composeHeader()}
			</div>
			<div class="atomic-editor-wrap" data-tour="compose-atomic">
				{#if atomicParsed.tags.length > 0}
					<div class="atomic-detected" title="Parsed from leading :tag: lines in the body and merged with the header tags">
						<span class="atomic-detected-label">tags from body</span>
						{#each atomicParsed.tags as t, i (i)}
							<span class="atomic-tag-chip">{t.name === 't' ? `#${t.value}` : `${t.name}: ${t.value}`}</span>
						{/each}
					</div>
				{/if}
				<CodeMirrorEditor
					bind:value={app.composeAtomicBody}
					bind:editorView={atomicCmView}
					{lineNumbers}
					{vimMode}
					extensions={nostrdownExt}
				/>
			</div>
		</div>
	<!-- Full mode = the feed's shape: title + tags and the selection toolbar
	     anchored (sticky) at the top of one scroll region; sections appended
	     below and scrolling under them. One window, not two competing panes. -->
	{:else if mode === 'full'}
		<div class="compose-content compose-content--scroll">
			<div class="compose-stick">
				{@render composeHeader()}
				{@render composeToolbar()}
			</div>
			<div class="compose-sections" data-tour="compose-sections" bind:this={sectionsListEl}>
				{#each compose.sections as section, i (section.id)}
					<div
						class="compose-section-row"
						class:compose-section-row--cursor={i === cursor}
						data-cursor={i}
					>
						<ComposeSection
							{section}
							{syncMode}
							checked={checkedIds.has(section.id)}
							collapsed={collapsedIds.has(section.id)}
							oncheck={toggleCheck}
							oncollapse={toggleCollapse}
							onupdate={updateSection}
							onupdatetags={updateSectionTags}
							onreset={resetSection}
							onremove={removeSection}
							onsendtochat={onsenditemtochat}
							{ontogglereadonly}
							{onlocksource}
							{oncrosspanelcopy}
							onreorder={onreorder}
							isFirst={i === 0}
							isLast={i === compose.sections.length - 1}
						/>
					</div>
				{/each}
			</div>
		</div>
	{:else if mode === 'plain'}
		{@render composeToolbar()}
		<div class="compose-content">
			<div class="plain-layout" data-tour="compose-plain">
				<div class="plain-editor-wrap">
					<CodeMirrorEditor
						bind:value={plainText}
						bind:editorView={plainCmView}
						{lineNumbers}
						{vimMode}
						onBlur={handlePlainBlur}
						extensions={nostrdownExt}
					/>
				</div>
				<div class="detected-sections" data-tour="compose-detected">
					<div class="detected-header">Detected</div>
					<div class="detected-row detected-doc-title">
						<span class="detected-label">title</span>
						<span class="detected-title">{detectedState.title || '[No title]'}</span>
					</div>
					{#if detectedState.tags.length > 0}
						<div class="detected-row">
							<span class="detected-label">tags</span>
							<span class="detected-title">{detectedState.tags.length}</span>
						</div>
					{/if}
					{#each detectedSections as det, di (det.index)}
						<div
							class="detected-row"
							class:detected-row--nested={det.level > 2}
							class:detected-row--slot={!!det.slot}
							style="--depth: {Math.max(0, det.level - 2)}"
						>
							{#if det.slot}
								<span class="detected-title detected-slot" title={det.slot}>⧉ slot · {slotLabel(det.slot)}</span>
							{:else if det.item}
								<label class="check">
									<input
										type="checkbox"
										checked={checkedIds.has(det.item.id)}
										onchange={() => toggleCheck(det.item!.id)}
									/>
								</label>
								<span class="detected-title">{det.title || '[Untitled]'}</span>
								<ItemBadge item={det.item} {syncMode} panel="compose" {ontogglereadonly} {onlocksource} {oncrosspanelcopy} />
								<button class="icon-btn-sm" onclick={() => onsenditemtochat(det.item!.id)} title="Send to chat">◂</button>
							{:else}
								<span class="detected-title detected-new">{det.title || '[Untitled]'}</span>
								<span class="badge badge-new">new</span>
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
							Type {effectiveDelim()}{effectiveDelim()} heading to create a
							section ({effectiveDelim()}{effectiveDelim()}{effectiveDelim()}+
							for nested sub-sections)
						</div>
					{/if}
				</div>
			</div>
		</div>
	{:else}
		<div class="compose-content">
			<!-- Preview = the read view of the current draft. Same lock/
			     unlock/reorder affordances as ReaderBuffer; the surrounding
			     edit chrome (title/tags + selection toolbar) is hidden so
			     this reads as the read-mode equivalent of the draft. -->
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
		{#if mode === 'full' && !isAtomic}
			<button onclick={addSection}>+ Section</button>
		{/if}
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
			>Sign</button>
			{#if checkedIds.size > 0}
				<button class="publish-btn publish-selected" onclick={publishSelected}>Sign ({checkedIds.size})</button>
			{/if}
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
	.mode-toggle {
		display: inline-flex;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		overflow: hidden;
	}
	.mode-seg {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 2px 8px;
		background: transparent;
		border: none;
		cursor: pointer;
	}
	.mode-seg + .mode-seg {
		border-left: 1px solid var(--border);
	}
	.mode-seg:hover {
		color: var(--fg);
	}
	.mode-seg--on {
		background: color-mix(in srgb, var(--id-yours) 22%, transparent);
		color: var(--id-yours);
	}

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
	.pub-shape {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		font-weight: 600;
		padding: 2px 8px;
		border-radius: var(--radius);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		cursor: default;
		user-select: none;
		white-space: nowrap;
	}
	.pub-shape--notes {
		color: var(--accent);
		border-color: var(--accent);
		background: color-mix(in srgb, var(--accent) 14%, transparent);
	}

	/* Atomic body editor — fills the scroll region under the sticky header,
	   same framing as the plain editor wrap. */
	.atomic-editor-wrap {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		padding: 12px;
		gap: 8px;
	}

	/* Mirrors plain mode's "Detected" affordance: shows what the body's leading
	   :tag: block parses to, so the stripped tags aren't invisible. */
	.atomic-detected {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		flex-shrink: 0;
	}
	.atomic-detected-label {
		font-size: var(--t-3xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.atomic-tag-chip {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		padding: 1px 6px;
		border-radius: 4px;
		background: color-mix(in srgb, var(--accent) 16%, transparent);
		color: var(--accent);
	}

	.delim-group {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.delim-label {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.delim-input {
		width: 36px;
		text-align: center;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 700;
		padding: 4px 6px;
	}

	.nest-group {
		display: flex;
		align-items: center;
		gap: 6px;
		cursor: pointer;
		user-select: none;
	}
	.nest-label {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.nest-select {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: 3px 6px;
		cursor: pointer;
	}

	.compose-header {
		display: flex;
		flex-direction: column;
		gap: 8px;
		flex-shrink: 0;
		max-height: 28vh;
		overflow-y: auto;
		padding-right: 4px;
	}

	.compose-header--collapsed {
		max-height: none;
		overflow-y: visible;
	}

	.compose-title-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.compose-title-row .compose-title { flex: 1; }

	.collapse-toggle {
		font-size: var(--t-2xs);
		padding: 0 4px;
		min-width: 18px;
		background: transparent;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.collapse-toggle:hover { color: var(--fg); }

	.header-tag-count {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.compose-title {
		font-family: inherit;
		font-size: var(--t-md);
		font-weight: 700;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 10px 12px;
		outline: none;
	}

	.compose-title:focus {
		border-color: var(--accent);
	}

	.compose-toolbar {
		display: flex;
		gap: 6px;
		align-items: center;
		flex-shrink: 0;
	}

	.sel-btn {
		font-size: var(--t-3xs);
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		padding: 4px 8px;
		font-size: var(--t-xs);
		min-width: 28px;
	}

	.trash-btn {
		font-size: var(--t-2xs);
	}

	.trash-armed {
		background: var(--danger-strong);
		border-color: var(--danger-strong);
		color: white;
	}

	.trash-warn {
		font-size: var(--t-3xs);
		color: var(--danger-strong);
		font-weight: 600;
		white-space: nowrap;
	}

	.toolbar-sp { flex: 1; }

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

	/* Full mode: the content box itself scrolls (feed pattern) so the title +
	   toolbar can stick to its top and the sections fill the rest. */
	.compose-content--scroll {
		overflow-y: auto;
	}
	.compose-stick {
		position: sticky;
		top: 0;
		z-index: 2;
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 12px 12px 8px;
		background: var(--bg-surface);
		border-bottom: 1px solid var(--border);
	}
	/* Inside the sticky header the title/tags no longer compete with the
	   sections for height — drop the viewport cap, keep a generous scroll
	   ceiling only so a huge tag list can't dominate. */
	.compose-stick .compose-header {
		max-height: 40vh;
	}

	.compose-sections {
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 12px;
	}

	/* Ranger-style cursor over compose sections — same inset bar +
	   tinted background as feed/reader/search. Wraps each
	   ComposeSection so the section's own provenance border is
	   preserved alongside the cursor highlight. */
	.compose-section-row {
		border-radius: 4px;
	}
	.compose-section-row--cursor {
		box-shadow: inset 4px 0 0 var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
	}

	.plain-layout {
		display: flex;
		flex: 1;
		gap: 0;
		min-height: 0;
	}

	.plain-editor-wrap {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.detected-sections {
		width: 200px;
		flex-shrink: 0;
		border-left: 1px solid var(--border);
		overflow-y: auto;
		background: var(--bg);
	}

	.detected-header {
		font-size: var(--t-3xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 8px 10px 4px;
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

	.detected-row .check {
		display: flex;
		align-items: center;
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

	.preview-sections {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 0;
	}

	.preview-section-bar {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 4px 10px;
		background: var(--bg);
		border-bottom: 1px solid var(--border);
		font-size: var(--t-2xs);
		flex-shrink: 0;
	}

	.checked-section {
		background: color-mix(in srgb, var(--accent) 8%, transparent);
	}

	.editor-pane {
		flex: 1;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1.6;
		padding: 4px 0;
		margin: 0;
		white-space: pre-wrap;
		word-break: break-word;
		color: var(--fg);
		outline: none;
		background: transparent;
		border: none;
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

	.publish-selected {
		background: transparent;
		color: var(--accent);
		border: 1px solid var(--accent);
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
	.walk-mode {
		flex: 0 0 auto;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--fg-muted);
		background: color-mix(in srgb, var(--fg-muted) 16%, transparent);
		padding: 0 5px;
		border-radius: var(--r-sm, 4px);
	}
</style>
