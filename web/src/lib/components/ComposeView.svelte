<script lang="ts">
	import type { ComposeState, ContextItem, TagEntry, SyncMode } from '$lib/types';
	import ComposeSection from './ComposeSection.svelte';
	import TagEditor from './TagEditor.svelte';

	type ComposeMode = 'full' | 'plain' | 'preview';

	let {
		compose,
		syncMode,
		onupdate,
		oncancel,
		onsendtochat,
		onpublish,
		ondelete,
		ondeletepermanent,
		onsenditemtochat,
		ontogglereadonly
	}: {
		compose: ComposeState;
		syncMode: SyncMode;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublish: (items: ContextItem[]) => void;
		ondelete: (items: ContextItem[]) => void;
		ondeletepermanent: (items: ContextItem[]) => void;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
	} = $props();

	let checkedIds: Set<string> = $state(new Set());
	let mode: ComposeMode = $state('full');
	let delimiter = $state('');
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

	// --- Mode switching ---

	function setMode(m: ComposeMode) {
		mode = m;
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
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length > 0) {
			onpublish(items);
			checkedIds = new Set();
		}
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

	function handlePlainSectionEdit(id: string, text: string) {
		const [, h2] = headChars();
		const lines = text.split('\n');
		let title = '';
		const tags: TagEntry[] = [];
		const contentLines: string[] = [];
		let inTags = true;

		for (const line of lines) {
			if (!title && line.startsWith(h2)) {
				title = line.slice(h2.length).trim();
			} else if (inTags) {
				const parsed = parseTagLine(line);
				if (parsed) {
					tags.push(...parsed);
				} else {
					inTags = false;
					contentLines.push(line);
				}
			} else {
				contentLines.push(line);
			}
		}

		const content = contentLines.join('\n').trim();
		const sections = compose.sections.map((s) =>
			s.id === id
				? { ...s, title: title || s.title, content, tags, modified: content !== s.original_content }
				: s
		);
		onupdate({ ...compose, sections });
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

	<div class="compose-header">
		<input
			class="compose-title"
			value={compose.title}
			oninput={updateTitle}
			placeholder="Publication title"
		/>
		<TagEditor tags={compose.tags} onupdate={updateTags} />
	</div>

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
					/>
				{/each}
			</div>
		{:else}
			<div class="text-sections">
				{#each compose.sections as section (section.id)}
					<div class="text-section" class:checked-section={checkedIds.has(section.id)}>
						<label class="section-check">
							<input
								type="checkbox"
								checked={checkedIds.has(section.id)}
								onchange={() => toggleCheck(section.id)}
							/>
						</label>
						{#if mode === 'plain'}
							<textarea
								class="editor-pane editor-textarea"
								spellcheck="false"
								value={serializeSection(section)}
								onblur={(e) => handlePlainSectionEdit(section.id, e.currentTarget.value)}
							></textarea>
						{:else}
							<pre class="editor-pane">{serializeSection(section)}</pre>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</div>

	<div class="compose-actions">
		{#if mode === 'full'}
			<button onclick={addSection}>+ Section</button>
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
		overflow-y: auto;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
	}

	.compose-sections {
		display: flex;
		flex-direction: column;
		gap: 12px;
		padding: 12px;
	}

	.text-sections {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.text-section {
		display: flex;
		gap: 6px;
		padding: 4px 8px;
		border-bottom: 1px solid var(--border);
	}

	.text-section:last-child {
		border-bottom: none;
	}

	.checked-section {
		background: color-mix(in srgb, var(--accent) 8%, transparent);
	}

	.section-check {
		display: flex;
		align-items: flex-start;
		padding-top: 4px;
		flex-shrink: 0;
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

	.editor-textarea {
		resize: none;
		min-height: 80px;
		field-sizing: content;
	}

	.compose-actions {
		display: flex;
		gap: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
		flex-shrink: 0;
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
