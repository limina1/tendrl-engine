<script lang="ts">
	import type { ComposeState, ContextItem, TagEntry } from '$lib/types';
	import ComposeSection from './ComposeSection.svelte';
	import TagEditor from './TagEditor.svelte';

	type ComposeMode = 'full' | 'plain' | 'preview';

	let {
		compose,
		onupdate,
		oncancel,
		onsendtochat,
		onpublish,
		ondelete,
		ondeletepermanent
	}: {
		compose: ComposeState;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublish: (items: ContextItem[]) => void;
		ondelete: (items: ContextItem[]) => void;
		ondeletepermanent: (items: ContextItem[]) => void;
	} = $props();

	let checkedIds: Set<string> = $state(new Set());
	let mode: ComposeMode = $state('full');
	let delimiter = $state('');
	let plainBuffer = $state('');
	let prevDelim = $state('');
	let trashPending: ContextItem[] = $state([]);
	let trashTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let trashCountdown = $state(0);
	let countdownInterval: ReturnType<typeof setInterval> | null = $state(null);

	// --- Reactive delimiter: swap headers when delim changes ---

	function replaceDelimiters(text: string, oldD: string, newD: string): string {
		const oldC = oldD.trim() || '=';
		const newC = newD.trim() || '=';
		if (oldC === newC) return text;
		const oldH2 = `${oldC}${oldC} `;
		const newH2 = `${newC}${newC} `;
		const oldH1 = `${oldC} `;
		const newH1 = `${newC} `;
		return text
			.split('\n')
			.map((line) => {
				if (line.startsWith(oldH2)) return newH2 + line.slice(oldH2.length);
				if (line.startsWith(oldH1)) return newH1 + line.slice(oldH1.length);
				return line;
			})
			.join('\n');
	}

	$effect(() => {
		const d = delimiter;
		if (d === prevDelim) return;
		if (mode === 'plain') {
			plainBuffer = replaceDelimiters(plainBuffer, prevDelim, d);
		} else if (mode === 'preview') {
			plainBuffer = serializeState(compose);
		}
		prevDelim = d;
	});

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

	function serializeState(state: ComposeState): string {
		const [h1, h2] = headChars();
		let out = `${h1}${state.title}\n`;
		out += serializeTagBlock(state.tags);
		for (const s of state.sections) {
			out += `\n${h2}${s.title}\n`;
			out += serializeTagBlock(s.tags);
			out += `\n${s.content}\n`;
		}
		return out;
	}

	function parsePlain(text: string): {
		title: string;
		pubTags: TagEntry[];
		sections: { title: string; content: string; tags: TagEntry[] }[];
	} {
		const [h1, h2] = headChars();
		const lines = text.split('\n');
		let title = '';
		let pubTags: TagEntry[] = [];
		let afterTitle = false;
		const sections: { title: string; content: string; tags: TagEntry[] }[] = [];
		let current: { title: string; tags: TagEntry[]; lines: string[]; inTagBlock: boolean } | null =
			null;

		for (const line of lines) {
			if (line.startsWith(h2)) {
				if (current) {
					sections.push({
						title: current.title,
						content: current.lines.join('\n').trim(),
						tags: current.tags
					});
				}
				current = { title: line.slice(h2.length).trim(), tags: [], lines: [], inTagBlock: true };
				afterTitle = false;
			} else if (line.startsWith(h1) && !title) {
				title = line.slice(h1.length).trim();
				afterTitle = true;
			} else if (afterTitle) {
				const parsed = parseTagLine(line);
				if (parsed) {
					pubTags.push(...parsed);
				} else {
					afterTitle = false;
				}
			} else if (current) {
				if (current.inTagBlock) {
					const parsed = parseTagLine(line);
					if (parsed) {
						current.tags.push(...parsed);
						continue;
					}
					current.inTagBlock = false;
				}
				current.lines.push(line);
			}
		}

		if (current) {
			sections.push({
				title: current.title,
				content: current.lines.join('\n').trim(),
				tags: current.tags
			});
		}

		return { title, pubTags, sections };
	}

	function applyPlainToState(): ComposeState {
		const parsed = parsePlain(plainBuffer);
		const existingByTitle = new Map<string, ContextItem>();
		for (const s of compose.sections) {
			existingByTitle.set(s.title, s);
		}
		const sections: ContextItem[] = parsed.sections.map((s) => {
			const existing = existingByTitle.get(s.title);
			if (existing) {
				return {
					...existing,
					title: s.title,
					content: s.content,
					tags: s.tags,
					modified: s.content !== existing.original_content
				};
			}
			return {
				id: crypto.randomUUID(),
				title: s.title,
				content: s.content,
				tags: s.tags,
				original_content: s.content,
				modified: false,
				in_context: false,
				in_compose: true
			};
		});
		const newState: ComposeState = {
			...compose,
			title: parsed.title || compose.title,
			tags: parsed.pubTags.length > 0 ? parsed.pubTags : compose.tags,
			sections
		};
		onupdate(newState);
		return newState;
	}

	// --- Mode switching ---

	function setMode(m: ComposeMode) {
		let currentState = compose;
		if (mode === 'plain' && m !== 'plain') {
			currentState = applyPlainToState();
		}
		if (m === 'plain' || m === 'preview') {
			plainBuffer = serializeState(currentState);
		}
		mode = m;
	}

	// --- Full mode handlers ---

	function selectAll() {
		checkedIds = new Set(compose.sections.map((s) => s.id));
	}

	function invertSelection() {
		const next = new Set<string>();
		for (const s of compose.sections) {
			if (!checkedIds.has(s.id)) next.add(s.id);
		}
		checkedIds = next;
	}

	function toggleCheck(id: string) {
		const next = new Set(checkedIds);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		checkedIds = next;
		clearTrash();
	}

	function sendCheckedToChat() {
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length > 0) {
			onsendtochat(items);
			checkedIds = new Set();
		}
		clearTrash();
	}

	function publishChecked() {
		const items = compose.sections.filter((s) => checkedIds.has(s.id));
		if (items.length > 0) {
			onpublish(items);
			checkedIds = new Set();
		}
		clearTrash();
	}

	function clearTrash() {
		trashPending = [];
		trashCountdown = 0;
		if (trashTimer) clearTimeout(trashTimer);
		trashTimer = null;
		if (countdownInterval) clearInterval(countdownInterval);
		countdownInterval = null;
	}

	function handleTrash() {
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

	const trashActive = $derived(trashPending.length > 0);

	// Plain/preview: send all to chat
	function sendAllToChat() {
		let currentState = compose;
		if (mode === 'plain') {
			currentState = applyPlainToState();
		}
		if (currentState.sections.length > 0) {
			onsendtochat(currentState.sections);
		}
	}

	// Plain/preview: publish all
	function publishAll() {
		let currentState = compose;
		if (mode === 'plain') {
			currentState = applyPlainToState();
		}
		if (currentState.sections.length > 0) {
			onpublish(currentState.sections);
		}
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
			in_compose: true
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

	{#if mode === 'full'}
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
			<button class="sel-btn" onclick={selectAll} disabled={compose.sections.length === 0} title="Select all">All</button>
			<button class="sel-btn" onclick={invertSelection} disabled={compose.sections.length === 0} title="Invert selection">Inv</button>
			<button class="icon-btn" onclick={sendCheckedToChat} disabled={checkedIds.size === 0} title="Send to chat">◂</button>
			<button class="icon-btn" onclick={publishChecked} disabled={checkedIds.size === 0} title="Publish locally">▸</button>
			<button
				class="icon-btn trash-btn"
				class:trash-armed={trashActive}
				onclick={handleTrash}
				disabled={checkedIds.size === 0 && !trashActive}
				title={trashActive ? 'Delete everywhere' : 'Remove from compose'}
			>🗑</button>
			{#if trashActive}
				<span class="trash-warn" style:opacity={trashCountdown / 10}>delete everywhere ({trashCountdown}s)</span>
			{/if}
		</div>

		<div class="compose-sections">
			{#each compose.sections as section (section.id)}
				<ComposeSection
					{section}
					checked={checkedIds.has(section.id)}
					oncheck={toggleCheck}
					onupdate={updateSection}
					onupdatetags={updateSectionTags}
					onreset={resetSection}
					onremove={removeSection}
				/>
			{/each}
		</div>

		<div class="compose-actions">
			<button onclick={addSection}>+ Section</button>
			<button onclick={oncancel}>Cancel</button>
		</div>
	{:else if mode === 'plain'}
		<div class="compose-toolbar">
			<button class="icon-btn" onclick={sendAllToChat} title="Send all to chat">◂</button>
			<button class="icon-btn" onclick={publishAll} title="Publish locally">▸</button>
		</div>
		<textarea
			class="plain-editor"
			bind:value={plainBuffer}
			spellcheck="false"
		></textarea>
		<div class="compose-actions">
			<button onclick={oncancel}>Cancel</button>
		</div>
	{:else}
		<div class="compose-toolbar">
			<button class="icon-btn" onclick={sendAllToChat} title="Send all to chat">◂</button>
			<button class="icon-btn" onclick={publishAll} title="Publish locally">▸</button>
		</div>
		<pre class="preview-content">{plainBuffer}</pre>
		<div class="compose-actions">
			<button onclick={oncancel}>Cancel</button>
		</div>
	{/if}
</div>

<style>
	.compose-view {
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 16px;
	}

	.compose-mode-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
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

	.compose-sections {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.compose-actions {
		display: flex;
		gap: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
	}

	.plain-editor {
		flex: 1;
		min-height: 300px;
		font-family: var(--font-mono);
		font-size: 0.85rem;
		line-height: 1.6;
		resize: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 12px;
		outline: none;
	}

	.plain-editor:focus {
		border-color: var(--accent);
	}

	.preview-content {
		flex: 1;
		min-height: 300px;
		font-family: var(--font-mono);
		font-size: 0.85rem;
		line-height: 1.6;
		padding: 12px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		overflow: auto;
		white-space: pre-wrap;
		word-break: break-word;
		margin: 0;
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
