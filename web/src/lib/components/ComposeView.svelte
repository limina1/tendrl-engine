<script lang="ts">
	import { untrack } from 'svelte';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';
	import type { ComposeState, ContextItem, TagEntry, SyncMode } from '$lib/types';
	import type { DraftSummary } from '$lib/api';
	import type { EditorView } from '@codemirror/view';
	import ComposeSection from './ComposeSection.svelte';
	import ItemBadge from './ItemBadge.svelte';
	import TagEditor from './TagEditor.svelte';
	import DraftReader from '$lib/wm/renderers/DraftReader.svelte';
	import CodeMirrorEditor from './CodeMirrorEditor.svelte';
	import {
		hasStructuralChange,
		claimedUntouchedSections
	} from '$lib/compose/state';
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();

	type ComposeMode = 'full' | 'plain' | 'preview';

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
		vimMode = true
	}: {
		compose: ComposeState;
		syncMode: SyncMode;
		canPublish?: boolean;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublish: (items: ContextItem[], meta?: { title: string; tags: TagEntry[] }) => void;
		onpreview?: (items: ContextItem[], meta?: { title: string; tags: TagEntry[] }) => void;
		/** Save the current compose as a local draft (never signed). */
		onsavedraft?: (items: ContextItem[], meta?: { title: string; tags: TagEntry[] }) => void;
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
	 *  Counts leading delimiter chars on the line and requires a space. */
	function parseHeadingLine(line: string, d: string): { level: number; title: string } | null {
		if (line.length === 0 || line[0] !== d) return null;
		let i = 0;
		while (i < line.length && line[i] === d) i++;
		if (i < 1 || i > 6) return null;
		if (line[i] !== ' ') return null;
		const title = line.slice(i + 1).trimEnd();
		if (!title) return null;
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
		sections: { title: string; tags: TagEntry[]; content: string; level: number }[]
	): string {
		let out = `${headFor(1)}${title}\n`;
		out += serializeTagBlock(tags);
		for (const s of sections) {
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
	}

	// Parse full text blob back into title/tags + sections. Recognises
	// any heading level >= 2 as a section; level 1 is reserved for the
	// publication title. Per-section level rides through to compose so
	// the engine can emit the nested 30040/30041 graph.
	function parseAll(text: string): { title: string; tags: TagEntry[]; sections: ParsedSection[] } {
		const d = effectiveDelim();
		const lines = text.split('\n');
		let docTitle = '';
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
			if (inDocHeader && !docTitle && head && head.level === 1) {
				docTitle = head.title;
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
		return { title: docTitle, tags: docTags, sections };
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
				tags: [] as TagEntry[],
				sections: [] as {
					title: string;
					item: ContextItem | null;
					index: number;
					level: number;
				}[]
			};
		const parsed = parseAll(plainText);
		const oldSections = compose.sections;
		return {
			title: parsed.title,
			tags: parsed.tags,
			sections: parsed.sections.map((p, i) => {
				const existing = i < oldSections.length ? oldSections[i] : null;
				return { title: p.title, item: existing, index: i, level: p.level };
			})
		};
	});
	const detectedSections = $derived(detectedState.sections);

	// Track known section IDs so we can detect external additions/removals
	let knownSectionIds: Set<string> = $state(new Set());

	// Re-serialize when sections change externally (e.g. search → compose)
	$effect(() => {
		if (mode !== 'plain') return;
		const currentIds = new Set(compose.sections.map((s) => s.id));
		const changed =
			currentIds.size !== knownSectionIds.size ||
			[...currentIds].some((id) => !knownSectionIds.has(id)) ||
			[...knownSectionIds].some((id) => !currentIds.has(id));
		if (changed) {
			plainText = serializeAll();
			knownSectionIds = currentIds;
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
				knownSectionIds = new Set(compose.sections.map((s) => s.id));
				prevDelimiter = effectiveDelim();
			} else if (appliedMode === 'plain') {
				handlePlainFullEdit(plainText);
			}
			appliedMode = next;
		});
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
		// In plain mode `compose.sections`/`compose.title` only commit on
		// blur, so parse the live text here and publish title+tags+sections
		// directly — otherwise the prop is stale (empty title / no sections)
		// at click time.
		let sections: ContextItem[];
		let meta: { title: string; tags: TagEntry[] } | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags };
		} else {
			sections = compose.sections;
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

	let draftsOpen = $state(false);
	function formatDraftTime(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}

	// Inspect the would-be 30040/30041 events as JSON — no signing/publish.
	function previewEvents() {
		if (!onpreview) return;
		let sections: ContextItem[];
		let meta: { title: string; tags: TagEntry[] } | undefined;
		if (mode === 'plain') {
			const parsed = handlePlainFullEdit(plainText);
			sections = parsed.sections;
			meta = { title: parsed.title, tags: parsed.tags };
		} else {
			sections = compose.sections;
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
	<div class="compose-mode-bar">
		<!-- Mode is set by the user's compose-default setting and toggled
		     via h/l in normal mode; no visible toggle button. The current
		     mode is rendered as a static label so the user knows where
		     they are. -->
		<div class="mode-label">{mode}</div>
		<div class="delim-group">
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
		<span class="bar-sp"></span>
		<!-- Bulk lock/unlock mirrors ReaderBuffer's draft toolbar so the
		     read↔edit transition keeps the same affordances at the same
		     on-screen level. Gated on a source publication since there's
		     nothing to lock against in a from-scratch draft. -->
		{#if compose.source_publication_addr}
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
		<!-- Read mirrors ReaderBuffer's "Edit" button — same on-screen
		     position (toolbar far-right) so the Edit↔Read swap reads as
		     a single mode toggle. When a source pub exists we navigate to
		     its ReaderBuffer; for from-scratch drafts we fall back to
		     inline DraftReader. Green to signal "read view" symmetric to
		     Edit. -->
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
	</div>

	<!-- Edit chrome (title/tags + selection toolbar) is hidden in
	     preview so the read view fills the buffer. The mode bar above
	     stays so the user can flip back to Full/Plain. -->
	{#if mode !== 'plain' && mode !== 'preview'}
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
	{/if}

	{#if mode !== 'preview'}
		<div class="compose-toolbar">
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
	{/if}

	<div class="compose-content">
		{#if mode === 'full'}
			<div class="compose-sections" bind:this={sectionsListEl}>
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
		{:else if mode === 'plain'}
			<div class="plain-layout">
				<div class="plain-editor-wrap">
					<CodeMirrorEditor
						bind:value={plainText}
						bind:editorView={plainCmView}
						{lineNumbers}
						{vimMode}
						onBlur={handlePlainBlur}
					/>
				</div>
				<div class="detected-sections">
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
							style="--depth: {Math.max(0, det.level - 2)}"
						>
							{#if det.item}
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
		{:else}
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
		{/if}
	</div>

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
				<ul class="compose-drafts-list">
					{#each drafts as d (d.draft_id)}
						<li class="compose-draft-row">
							<button
								class="compose-draft-load"
								onclick={() => onloaddraft?.(d.draft_id)}
								title="Resume this draft (replaces current compose sections)"
							>
								<span class="compose-draft-title">{d.title || '[untitled]'}</span>
								<span class="compose-draft-meta"
									>{d.section_count} section{d.section_count === 1 ? '' : 's'} · {formatDraftTime(
										d.modified_at
									)}</span
								>
							</button>
							<button
								class="compose-draft-del"
								onclick={() => ondeletedraft?.(d.draft_id)}
								title="Delete this draft">✕</button
							>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}

	<div class="compose-actions">
		{#if mode === 'full'}
			<button onclick={addSection}>+ Section</button>
		{/if}
		{#if onpreview}
			<button
				class="preview-events-btn"
				onclick={previewEvents}
				disabled={mode === 'plain'
					? detectedSections.length === 0
					: compose.sections.length === 0}
				title="Inspect the 30040/30041 events this draft would publish, as JSON"
			>Preview events</button>
		{/if}
		{#if onsavedraft}
			<button
				class="save-draft-btn"
				onclick={saveDraftAction}
				disabled={mode === 'plain'
					? detectedSections.length === 0
					: compose.sections.length === 0}
				title="Save this draft locally — survives refresh; resume it from the Saved drafts list"
			>Save draft</button>
		{/if}
		{#if canPublish}
			<button
				class="publish-btn"
				onclick={publishAll}
				disabled={mode === 'plain'
					? detectedSections.length === 0
					: compose.sections.length === 0 || !structuralChange}
				title={mode === 'plain'
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

<style>
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

	.mode-label {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 2px 6px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
	}

	.delim-group {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.delim-label {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.delim-input {
		width: 36px;
		text-align: center;
		font-family: var(--font-mono);
		font-size: 0.85rem;
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
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.nest-select {
		font-family: var(--font-mono);
		font-size: 0.8rem;
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
		font-size: 0.75rem;
		padding: 0 4px;
		min-width: 18px;
		background: transparent;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.collapse-toggle:hover { color: var(--fg); }

	.header-tag-count {
		font-size: 0.7rem;
		color: var(--fg-muted);
		font-family: var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.compose-title {
		font-family: inherit;
		font-size: 1.1rem;
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
		font-size: 0.65rem;
		padding: 2px 6px;
		color: var(--fg-muted);
	}

	.icon-btn {
		padding: 4px 8px;
		font-size: 0.85rem;
		min-width: 28px;
	}

	.trash-btn {
		font-size: 0.75rem;
	}

	.trash-armed {
		background: #dc2626;
		border-color: #dc2626;
		color: white;
	}

	.trash-warn {
		font-size: 0.7rem;
		color: #dc2626;
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

	.compose-sections {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
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
		font-size: 0.7rem;
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
		font-size: 0.75rem;
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
		font-size: 0.75rem;
	}

	.detected-new {
		color: var(--fg-muted);
		font-style: italic;
	}

	.detected-label {
		font-size: 0.6rem;
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
		font-size: 0.75rem;
		font-style: italic;
	}

	.detected-row .check {
		display: flex;
		align-items: center;
	}

	.icon-btn-sm {
		padding: 2px 6px;
		font-size: 0.75rem;
		min-width: 22px;
	}

	.badge-new {
		font-size: 0.6rem;
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
		font-size: 0.75rem;
		flex-shrink: 0;
	}

	.checked-section {
		background: color-mix(in srgb, var(--accent) 8%, transparent);
	}

	.editor-pane {
		flex: 1;
		font-family: var(--font-mono);
		font-size: 0.85rem;
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
		font-size: 0.78rem;
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
	.compose-draft-row {
		display: flex;
		gap: 4px;
		align-items: stretch;
	}
	.compose-draft-load {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 2px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		padding: 5px 9px;
		cursor: pointer;
		color: var(--fg);
		text-align: left;
	}
	.compose-draft-load:hover {
		border-color: var(--accent, var(--id-yours));
		background: color-mix(in srgb, var(--accent, var(--id-yours)) 10%, transparent);
	}
	.compose-draft-title {
		font-size: 0.82rem;
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 100%;
	}
	.compose-draft-meta {
		font-size: 0.68rem;
		color: var(--fg-muted);
	}
	.compose-draft-del {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		color: var(--fg-muted);
		cursor: pointer;
		padding: 0 8px;
	}
	.compose-draft-del:hover {
		color: var(--id-draft, crimson);
		border-color: var(--id-draft, crimson);
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
</style>
