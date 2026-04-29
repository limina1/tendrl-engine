<script lang="ts">
	import type { ComposeState, ContextItem, TagEntry, SyncMode } from '$lib/types';
	import ComposeSection from './ComposeSection.svelte';
	import ItemBadge from './ItemBadge.svelte';
	import TagEditor from './TagEditor.svelte';
	import DraftReader from '$lib/wm/renderers/DraftReader.svelte';

	type ComposeMode = 'full' | 'plain' | 'preview';

	let {
		compose,
		syncMode,
		canPublish = false,
		onupdate,
		oncancel,
		onsendtochat,
		onpublish,
		ondelete,
		ondeletepermanent,
		onsenditemtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy
	}: {
		compose: ComposeState;
		syncMode: SyncMode;
		canPublish?: boolean;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublish: (items: ContextItem[]) => void;
		ondelete: (items: ContextItem[]) => void;
		ondeletepermanent: (items: ContextItem[]) => void;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
	} = $props();

	let checkedIds: Set<string> = $state(new Set());
	let mode: ComposeMode = $state('full');
	let delimiter = $state('');
	let prevDelimiter = $state('');
	let trashPending: ContextItem[] = $state([]);
	let trashTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let trashCountdown = $state(0);
	let countdownInterval: ReturnType<typeof setInterval> | null = $state(null);

	// --- Serialize / Parse ---

	function effectiveDelim(): string {
		return delimiter.trim() || '=';
	}

	function headChars(): [string, string] {
		const d = effectiveDelim();
		return [`${d} `, `${d}${d} `];
	}

	function headCharsFor(d: string): [string, string] {
		return [`${d} `, `${d}${d} `];
	}

	// Reactively swap delimiters in plain text
	$effect(() => {
		const cur = effectiveDelim();
		if (mode === 'plain' && prevDelimiter && cur !== prevDelimiter) {
			// Replace old delimiter headings with new ones in the live text
			const [oldH1, oldH2] = headCharsFor(prevDelimiter);
			const [newH1, newH2] = headCharsFor(cur);
			plainText = plainText
				.split('\n')
				.map((line) => {
					if (line.startsWith(oldH2)) return newH2 + line.slice(oldH2.length);
					if (line.startsWith(oldH1)) return newH1 + line.slice(oldH1.length);
					return line;
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
		const [, h2] = headChars();
		let out = `${h2}${s.title}\n`;
		out += serializeTagBlock(s.tags);
		out += `\n${s.content}`;
		return out;
	}

	// Serialize entire document into one text blob
	function serializeAll(): string {
		const [h1, h2] = headChars();
		let out = `${h1}${compose.title}\n`;
		out += serializeTagBlock(compose.tags);
		for (const s of compose.sections) {
			out += `\n${h2}${s.title}\n`;
			out += serializeTagBlock(s.tags);
			out += `\n${s.content}\n`;
		}
		return out;
	}

	interface ParsedSection {
		title: string;
		tags: TagEntry[];
		content: string;
	}

	// Parse full text blob back into title/tags + sections
	function parseAll(text: string): { title: string; tags: TagEntry[]; sections: ParsedSection[] } {
		const [h1, h2] = headChars();
		const lines = text.split('\n');
		let docTitle = '';
		const docTags: TagEntry[] = [];
		const sections: ParsedSection[] = [];
		let current: { title: string; tags: TagEntry[]; contentLines: string[]; inTags: boolean } | null = null;
		let inDocHeader = true;
		let docInTags = true;

		for (const line of lines) {
			if (inDocHeader && !docTitle && line.startsWith(h1) && !line.startsWith(h2)) {
				docTitle = line.slice(h1.length).trim();
				continue;
			}
			if (line.startsWith(h2)) {
				// Finish previous section
				if (current) {
					sections.push({
						title: current.title,
						tags: current.tags,
						content: current.contentLines.join('\n').trim()
					});
				}
				inDocHeader = false;
				current = { title: line.slice(h2.length).trim(), tags: [], contentLines: [], inTags: true };
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
				current = { title: '', tags: [], contentLines: [line], inTags: false };
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
				content: current.contentLines.join('\n').trim()
			});
		}
		return { title: docTitle, tags: docTags, sections };
	}

	// Reconcile parsed sections with existing compose sections
	function handlePlainFullEdit(text: string) {
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
					modified: p.content !== existing.original_content
				};
			}
			return {
				id: crypto.randomUUID(),
				title: p.title,
				content: p.content,
				context_content: p.content,
				tags: p.tags,
				original_content: '',
				modified: true,
				in_context: false,
				in_compose: true,
				origin: 'compose' as const,
				readonly: false
			};
		});

		onupdate({ title: parsed.title, tags: parsed.tags, sections: newSections });
	}

	// Detected structure for plain mode sidebar
	let plainText = $state('');
	const detectedState = $derived.by(() => {
		if (mode !== 'plain') return { title: '', tags: [] as TagEntry[], sections: [] as { title: string; item: ContextItem | null; index: number }[] };
		const parsed = parseAll(plainText);
		const oldSections = compose.sections;
		return {
			title: parsed.title,
			tags: parsed.tags,
			sections: parsed.sections.map((p, i) => {
				const existing = i < oldSections.length ? oldSections[i] : null;
				return { title: p.title, item: existing, index: i };
			})
		};
	});
	const detectedSections = $derived(detectedState.sections);

	// Track known section IDs so we can detect external additions/removals
	let knownSectionIds: Set<string> = $state(new Set());

	function enterPlainMode() {
		plainText = serializeAll();
		knownSectionIds = new Set(compose.sections.map((s) => s.id));
		prevDelimiter = effectiveDelim();
		mode = 'plain';
	}

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

	function setMode(m: ComposeMode) {
		if (m === 'plain') {
			enterPlainMode();
		} else {
			if (mode === 'plain') {
				// Commit plain text edits before leaving
				handlePlainFullEdit(plainText);
			}
			mode = m;
		}
	}

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

	function handlePlainInput(e: Event) {
		plainText = (e.target as HTMLTextAreaElement).value;
	}

	function handlePlainBlur() {
		handlePlainFullEdit(plainText);
	}

	// Highlight backdrop: render same text with section headers styled
	let plainTextarea: HTMLTextAreaElement | undefined = $state();
	let backdropEl: HTMLPreElement | undefined = $state();

	function syncScroll() {
		if (backdropEl && plainTextarea) {
			backdropEl.scrollTop = plainTextarea.scrollTop;
			backdropEl.scrollLeft = plainTextarea.scrollLeft;
		}
	}

	// Build highlighted HTML from plain text
	const highlightedHtml = $derived.by(() => {
		const [h1, h2] = headChars();
		const lines = plainText.split('\n');
		// Figure out which lines belong to which section index (for checked highlighting)
		let sectionIdx = -1;
		const checkedSectionIndices = new Set<number>();
		const oldSections = compose.sections;
		for (let i = 0; i < detectedSections.length; i++) {
			const det = detectedSections[i];
			if (det.item && checkedIds.has(det.item.id)) {
				checkedSectionIndices.add(i);
			}
		}

		const htmlLines: string[] = [];
		let currentSectionIdx = -1;
		for (const line of lines) {
			const escaped = escapeHtml(line);
			if (line.startsWith(h2)) {
				currentSectionIdx++;
				const isChecked = checkedSectionIndices.has(currentSectionIdx);
				htmlLines.push(`<span class="hl-heading${isChecked ? ' hl-checked' : ''}">${escaped}</span>`);
			} else if (currentSectionIdx === -1 && line.startsWith(h1) && !line.startsWith(h2)) {
				htmlLines.push(`<span class="hl-title">${escaped}</span>`);
			} else if (line.match(/^:[^:]+:\s/)) {
				htmlLines.push(`<span class="hl-tag">${escaped}</span>`);
			} else {
				const isChecked = checkedSectionIndices.has(currentSectionIdx);
				htmlLines.push(isChecked ? `<span class="hl-checked">${escaped}</span>` : escaped);
			}
		}
		return htmlLines.join('\n') + '\n';
	});

	function escapeHtml(s: string): string {
		return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
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
			origin: 'compose' as const,
			readonly: false
		};
		onupdate({ ...compose, sections: [...compose.sections, item] });
	}

	function publishAll() {
		if (mode === 'plain') handlePlainFullEdit(plainText);
		onpublish(compose.sections);
	}

	function publishSelected() {
		if (mode === 'plain') handlePlainFullEdit(plainText);
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length > 0) {
			onpublish(items);
			checkedIds = new Set();
		}
	}
</script>

<div class="compose-view">
	<div class="compose-mode-bar">
		<div class="mode-group">
			<button class:active={mode === 'full'} onclick={() => setMode('full')}>Full</button>
			<button class:active={mode === 'plain'} onclick={() => setMode('plain')}>Plain</button>
			<button class:active={mode === 'preview'} onclick={() => setMode('preview')}>Preview</button>
		</div>
		<div class="delim-group">
			<span class="delim-label">delim</span>
			<input
				class="delim-input"
				bind:value={delimiter}
				placeholder="="
				maxlength="2"
			/>
		</div>
	</div>

	{#if mode !== 'plain'}
		<div class="compose-header">
			<input
				class="compose-title"
				value={compose.title}
				oninput={updateTitle}
				placeholder="Publication title"
			/>
			<TagEditor tags={compose.tags} onupdate={updateTags} />
		</div>
	{/if}

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
	</div>

	<div class="compose-content">
		{#if mode === 'full'}
			<div class="compose-sections">
				{#each compose.sections as section (section.id)}
					<ComposeSection
						{section}
						{syncMode}
						checked={checkedIds.has(section.id)}
						oncheck={toggleCheck}
						onupdate={updateSection}
						onupdatetags={updateSectionTags}
						onreset={resetSection}
						onremove={removeSection}
						onsendtochat={onsenditemtochat}
						{ontogglereadonly}
						{onlocksource}
						{oncrosspanelcopy}
					/>
				{/each}
			</div>
		{:else if mode === 'plain'}
			<div class="plain-layout">
				<div class="plain-editor-wrap">
					<pre class="plain-backdrop" bind:this={backdropEl}>{@html highlightedHtml}</pre>
					<textarea
						class="plain-editor"
						spellcheck="false"
						bind:this={plainTextarea}
						value={plainText}
						oninput={handlePlainInput}
						onblur={handlePlainBlur}
						onscroll={syncScroll}
					></textarea>
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
					{#each detectedSections as det (det.index)}
						<div class="detected-row">
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
						</div>
					{/each}
					{#if detectedSections.length === 0}
						<div class="detected-empty">Type == heading to create sections</div>
					{/if}
				</div>
			</div>
		{:else}
			<DraftReader {compose} {ontogglereadonly} />
		{/if}
	</div>

	<div class="compose-actions">
		{#if mode === 'full'}
			<button onclick={addSection}>+ Section</button>
		{/if}
		{#if canPublish}
			<button class="publish-btn" onclick={publishAll} disabled={compose.sections.length === 0}>Publish</button>
			{#if checkedIds.size > 0}
				<button class="publish-btn publish-selected" onclick={publishSelected}>Publish ({checkedIds.size})</button>
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
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
		flex-shrink: 0;
	}

	.mode-group {
		display: flex;
		gap: 4px;
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

	.compose-header {
		display: flex;
		flex-direction: column;
		gap: 8px;
		flex-shrink: 0;
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

	.plain-layout {
		display: flex;
		flex: 1;
		gap: 0;
		min-height: 0;
	}

	.plain-editor-wrap {
		flex: 1;
		position: relative;
		min-height: 0;
		overflow: hidden;
	}

	.plain-backdrop {
		position: absolute;
		inset: 0;
		font-family: var(--font-mono);
		font-size: 0.85rem;
		line-height: 1.6;
		padding: 12px;
		margin: 0;
		white-space: pre-wrap;
		word-break: break-word;
		overflow: hidden;
		color: transparent;
		pointer-events: none;
	}

	.plain-editor {
		position: relative;
		width: 100%;
		height: 100%;
		font-family: var(--font-mono);
		font-size: 0.85rem;
		line-height: 1.6;
		padding: 12px;
		margin: 0;
		border: none;
		outline: none;
		background: transparent;
		color: var(--fg);
		resize: none;
		white-space: pre-wrap;
		word-break: break-word;
		overflow-y: auto;
	}

	/* Highlight styles in backdrop */
	.plain-backdrop :global(.hl-title) {
		background: color-mix(in srgb, var(--accent) 15%, transparent);
		display: inline;
		border-radius: 2px;
	}

	.plain-backdrop :global(.hl-heading) {
		background: color-mix(in srgb, var(--badge-synced) 12%, transparent);
		display: inline;
		border-radius: 2px;
		border-left: 3px solid var(--badge-synced);
		padding-left: 4px;
		margin-left: -4px;
	}

	.plain-backdrop :global(.hl-tag) {
		background: color-mix(in srgb, var(--fg-muted) 8%, transparent);
		display: inline;
		border-radius: 2px;
	}

	.plain-backdrop :global(.hl-checked) {
		background: color-mix(in srgb, var(--accent) 8%, transparent);
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
		border-bottom: 1px solid var(--border);
		font-size: 0.75rem;
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
</style>
