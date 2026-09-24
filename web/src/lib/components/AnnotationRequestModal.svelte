<script lang="ts">
	// Annotate mode's window: ask for notes about a place. The request is a
	// local record by default (nothing signed, nothing on the wire) that an
	// answering agent reads through the `list_requests` tool — the user's own
	// Claude Code over MCP, at their prompt; tendrl never calls the model on
	// their behalf. Optionally the request is also signed by the user as a
	// kind-30041 annotation on the section (the answers point back at it),
	// and optionally broadcast. The same window
	// lists the requests already made on this place and how they were
	// answered, with the written notes one click away.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { confirmState } from '$lib/network/fetch-events.svelte';
	import { stylesList, refreshStyles } from '$lib/styles.svelte';
	import { onStoreEvent } from '$lib/store-events.svelte';

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));
	// svelte-ignore state_referenced_locally
	const open = app.annotationRequest;

	const SCOPES: { id: string; label: string; hint: string }[] = [
		{ id: 'section', label: 'this section', hint: 'read the anchored section' },
		{ id: 'siblings', label: 'siblings', hint: 'read the neighbouring sections too' },
		{ id: 'publication', label: 'whole publication', hint: 'read the entire tree' },
		{ id: 'neighbours', label: 'neighbours', hint: 'semantic search over the local store' },
		{ id: 'relays', label: 'relays', hint: 'other Nostr notes (Confirm-gated fetches)' },
		{ id: 'web', label: 'web', hint: "the answering agent's own web search" }
	];

	let root = $state(open?.root ?? '');
	const roots = $derived(open?.roots ?? []);
	const parent = $derived(open?.parent ?? root);
	const topLevel = $derived(parent === root);
	let task = $state('');
	let title = $state('');
	let context = $state(open?.context ?? '');
	let scope = $state<string[]>(['section']);
	let output = $state<'notes' | 'kasten' | 'vocabulary' | 'ask'>(app.annotationRequest?.output ?? 'notes');
	// A vocabulary preset (from the composer's segment) needs the section text at once.
	let presetApplied = false;
	$effect(() => {
		if (presetApplied) return;
		presetApplied = true;
		if (output === 'vocabulary') setOutput('vocabulary');
	});
	let sign = $state(false);
	let broadcast = $state(false);
	let posting = $state(false);

	// ---- vocabulary: mark words in the section's own content -------------
	// The words to define are highlights on the text, not a typed list. The
	// engine generates the request's content line from them; this window
	// shows the same line so what is signed is what is seen.
	type Word = { w: string; pi: number };
	type Mark = { from: number; to: number };
	const clean = (w: string) => w.replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, '');
	function paragraphsOf(content: string): string[] {
		return content
			.split(/\n\s*\n/)
			.map((p) => p.replace(/\s+/g, ' ').trim())
			.filter(Boolean);
	}
	function wordsOf(content: string): Word[] {
		const out: Word[] = [];
		paragraphsOf(content).forEach((p, pi) => p.split(' ').forEach((w) => out.push({ w, pi })));
		return out;
	}
	let sectionContent = $state<string | null>(open?.content ?? null);
	let loadingContent = $state(false);
	const words = $derived(sectionContent ? wordsOf(sectionContent) : []);
	let marks = $state<Mark[]>([]);
	let drag = $state<number | null>(null);
	let dragTo = $state<number | null>(null);
	const terms = $derived(
		marks
			.slice()
			.sort((x, y) => x.from - y.from)
			.map((m) =>
				words
					.slice(m.from, m.to + 1)
					.map((x) => clean(x.w))
					.join(' ')
			)
			.filter(Boolean)
	);
	/** The paragraphs the marks fall in — the sense in this text. */
	const markedContext = $derived.by(() => {
		if (!sectionContent) return '';
		const paras = paragraphsOf(sectionContent);
		const pis = new Set(marks.map((m) => words[m.from]?.pi).filter((x) => x !== undefined));
		if (pis.size === 0) return '';
		return [...pis].sort((a, b) => a - b).map((pi) => paras[pi]).join('\n\n');
	});
	/** What the engine will put in `content` — mirrored here, never typed. */
	// Mirrors the engine's `requests::generated_task` — the display only;
	// the engine generates the real one when `task` is sent empty.
	const generatedTask = $derived(
		output === 'vocabulary'
			? terms.length
				? `define these words in the context of these events: ${terms.join(', ')}`
				: 'find the words these events use but never define, and define them in their context'
			: output === 'kasten'
				? 'write a kasten on these events: one idea per note, native links between them, one index over them'
				: output === 'notes'
					? 'write notes on these events, in their context'
					: ''
	);
	function isMarked(i: number): boolean {
		return marks.some((m) => i >= m.from && i <= m.to);
	}
	function inDrag(i: number): boolean {
		if (drag === null || dragTo === null) return false;
		return i >= Math.min(drag, dragTo) && i <= Math.max(drag, dragTo);
	}
	function markDown(i: number) {
		if (posting) return;
		drag = i;
		dragTo = i;
	}
	function markOver(i: number) {
		if (drag !== null) dragTo = i;
	}
	function markUp(i: number) {
		if (drag === null) return;
		const from = Math.min(drag, i);
		const to = Math.max(drag, i);
		drag = null;
		dragTo = null;
		const hit = marks.findIndex((m) => !(to < m.from || from > m.to));
		if (hit >= 0 && from === to) {
			// A click on a highlighted word removes that highlight.
			marks = marks.filter((_, k) => k !== hit);
		} else {
			// A new phrase replaces any it overlaps.
			marks = [...marks.filter((m) => to < m.from || from > m.to), { from, to }];
		}
		schedulePreview();
	}
	function dragCancel() {
		drag = null;
		dragTo = null;
	}
	function clearMarks() {
		marks = [];
		schedulePreview();
	}
	/** Pre-highlight a short selection (≤ 4 words) as one phrase. */
	function premark(phrase: string) {
		const parts = phrase
			.trim()
			.split(/\s+/)
			.map((x) => clean(x).toLowerCase())
			.filter(Boolean);
		if (parts.length === 0 || parts.length > 4) return;
		for (let i = 0; i + parts.length <= words.length; i++) {
			if (parts.every((pt, k) => clean(words[i + k].w).toLowerCase() === pt)) {
				marks = [{ from: i, to: i + parts.length - 1 }];
				return;
			}
		}
	}
	async function ensureContent() {
		if (sectionContent !== null || loadingContent) return;
		const coord = topLevel ? root : parent;
		const [k, pk, ...d] = coord.split(':');
		if (!k || !pk || d.length === 0) return;
		loadingContent = true;
		try {
			const r = await api.getAddressable(Number(k), pk, d.join(':'), 'local_first');
			const ev = r.event as { content?: string } | null;
			sectionContent = ev?.content ?? '';
		} catch {
			sectionContent = '';
		} finally {
			loadingContent = false;
		}
		if (open?.context) premark(open.context);
		schedulePreview();
	}
	function setOutput(o: 'notes' | 'kasten' | 'vocabulary' | 'ask') {
		output = o;
		if (o === 'vocabulary') {
			if (sectionContent === null) void ensureContent();
			else if (marks.length === 0 && open?.context) premark(open.context);
		}
		schedulePreview();
	}

	// ---- event preview: the unsigned 30041 this window would sign --------
	// Refreshed from the handlers that change the form (debounced), not from
	// an $effect over the whole form — an async read of $state inside an
	// effect loops (project_effect_async_state_loop).
	let preview = $state<api.RequestPreview['event'] | null>(null);
	let previewOpen = $state(true);
	let previewTimer: ReturnType<typeof setTimeout> | null = null;
	let previewSeq = 0;
	function previewBody() {
		return {
			root,
			parent: topLevel ? undefined : parent,
			parent_event_id: open?.parentEventId,
			parent_title: open?.parentTitle ?? undefined,
			title: title.trim() || undefined,
			task: task.trim() || undefined,
			scope,
			context: output === 'vocabulary' ? markedContext || undefined : context.trim() || undefined,
			output,
			terms: output === 'vocabulary' ? terms : undefined,
			style: style || undefined
		};
	}
	function schedulePreview() {
		if (previewTimer) clearTimeout(previewTimer);
		previewTimer = setTimeout(() => void refreshPreview(), 250);
	}
	async function refreshPreview() {
		if (!root) return;
		const seq = ++previewSeq;
		try {
			const r = await api.previewRequest(previewBody());
			if (seq === previewSeq) preview = r.event;
		} catch {
			if (seq === previewSeq) preview = null;
		}
	}
	function tagLine(t: string[]): string {
		return JSON.stringify(t);
	}
	schedulePreview();

	// Writing style: one of the local style events, or a new one written
	// here (signed by you when you can sign, else by the assistant — a style
	// is an event either way, so it travels with the knowledge base).
	let style = $state<string>('');
	let newStyleOpen = $state(false);
	let newStyleTitle = $state('');
	let newStyleText = $state('');
	let savingStyle = $state(false);
	void refreshStyles();
	async function saveStyle() {
		if (savingStyle || !newStyleTitle.trim() || !newStyleText.trim()) return;
		savingStyle = true;
		try {
			const s = await api.createStyle({
				title: newStyleTitle.trim(),
				content: newStyleText.trim(),
				sign_as: canSign ? 'user' : 'assistant'
			});
			await refreshStyles();
			style = s.coordinate;
			newStyleOpen = false;
			newStyleTitle = '';
			newStyleText = '';
			app.pushToast(`Style "${s.title}" saved`, 'success');
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Saving the style failed'), 'error', 5000);
		} finally {
			savingStyle = false;
		}
	}
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');
	const canPost = $derived(
		!posting && !!root && (output !== 'ask' || task.trim().length > 0) && (!sign || canSign)
	);

	// Requests already on this place.
	let existing = $state<api.AnnotationRequest[]>([]);
	let loadingExisting = $state(false);
	async function loadExisting() {
		if (!root) return;
		loadingExisting = true;
		try {
			const r = await api.listRequests({ root, parent: topLevel ? undefined : parent });
			existing = r.requests;
		} catch {
			existing = [];
		} finally {
			loadingExisting = false;
		}
	}
	void loadExisting();
	// Answers and new asks on this place show up while the window is open.
	$effect(() =>
		onStoreEvent((ev) => {
			if (ev.type === 'request' && ev.root === root) void loadExisting();
		})
	);

	function short(coord: string): string {
		const [k, pk, ...d] = coord.split(':');
		return `${k}:${pk?.slice(0, 8)}…:${d.join(':')}`;
	}
	function toggleScope(id: string) {
		scope = scope.includes(id) ? scope.filter((s) => s !== id) : [...scope, id];
		if (scope.length === 0) scope = ['section'];
		schedulePreview();
	}
	function relTime(ts: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
		if (s < 60) return 'now';
		if (s < 3600) return `${Math.floor(s / 60)}m`;
		if (s < 86400) return `${Math.floor(s / 3600)}h`;
		return `${Math.floor(s / 86400)}d`;
	}

	function close() {
		app.annotationRequest = null;
	}

	async function submit() {
		if (!canPost) return;
		posting = true;
		try {
			await api.createRequest({
				root,
				parent: topLevel ? undefined : parent,
				parent_event_id: open?.parentEventId,
				parent_title: open?.parentTitle ?? undefined,
				title: title.trim() || undefined,
				// vocabulary: the engine generates the content line from `terms`
				task: task.trim() || generatedTask,
				scope,
				context: output === 'vocabulary' ? markedContext || undefined : context.trim() || undefined,
				output,
				terms: output === 'vocabulary' ? terms : undefined,
				style: style || undefined,
				sign,
				broadcast: sign && broadcast
			});
			app.pushToast(
				sign
					? broadcast
						? 'Request signed as an annotation and sent'
						: 'Request signed as a local annotation'
					: 'Request saved — ask your agent to check tendrl requests',
				'success'
			);
			app.bumpRequests();
			close();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Request failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	async function remove(r: api.AnnotationRequest) {
		try {
			await api.deleteRequest(r.id);
			existing = existing.filter((x) => x.id !== r.id);
			app.bumpRequests();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Delete failed'), 'error', 4000);
		}
	}

	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}

	function handleKeydown(e: KeyboardEvent) {
		if (posting) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			void submit();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="rq-backdrop {yielding ? 'rq-backdrop--yield' : ''}"
	onclick={() => {
		if (!posting) close();
	}}
	role="presentation"
>
	<div
		class="rq-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Request annotations"
		tabindex="-1"
	>
		<header class="rq-header">
			<h3 class="rq-title">
				<span class="rq-dot" aria-hidden="true">●</span>
				annotate
				{#if topLevel}
					<span class="rq-title__on">— on the publication</span>
				{:else if open?.parentTitle}
					<span class="rq-title__on">— on <em>{open.parentTitle}</em></span>
				{/if}
			</h3>
			<button class="rq-close" onclick={close} aria-label="Close">×</button>
		</header>

		<div class="rq-label">anchor</div>
		<div class="rq-anchor">
			{#if !topLevel}
				<span class="rq-chip" title={parent}><b>a</b> {short(parent)}</span>
			{/if}
			{#if roots.length > 1}
				<label class="rq-chip rq-chip--select">
					<b>A</b>
					<select bind:value={root} disabled={posting} onchange={loadExisting}>
						{#each roots as r (r.coord)}
							<option value={r.coord}>{r.title ?? short(r.coord)} · {r.coord.split(':')[1]?.slice(0, 8)}…</option>
						{/each}
					</select>
				</label>
			{:else}
				<span class="rq-chip" title={root}><b>A</b> {open?.rootTitle ?? short(root)}</span>
			{/if}
		</div>

		<div class="rq-label">mode <span class="rq-label__sub">— what your agent answers with: notes · a kasten · definitions · an answer to your question</span></div>
		<div class="rq-seg" role="radiogroup" aria-label="Mode">
			<button class="rq-seg__opt" class:on={output === 'notes'} role="radio" aria-checked={output === 'notes'} onclick={() => setOutput('notes')} disabled={posting} title="Loose notes attached to the place">notes</button>
			<button class="rq-seg__opt" class:on={output === 'kasten'} role="radio" aria-checked={output === 'kasten'} onclick={() => setOutput('kasten')} disabled={posting} title="Several notes plus an index grouping them — promotable as a sub-chapter">kasten</button>
			<button class="rq-seg__opt" class:on={output === 'vocabulary'} role="radio" aria-checked={output === 'vocabulary'} onclick={() => setOutput('vocabulary')} disabled={posting} title="One definition event per word, anchored here — highlight the words in the text">vocabulary</button>
			<button class="rq-seg__opt" class:on={output === 'ask'} role="radio" aria-checked={output === 'ask'} onclick={() => setOutput('ask')} disabled={posting} title="A question in your own words about this place — answered as notes that point at it">ask</button>
		</div>

		{#if output === 'vocabulary'}
			<div class="rq-label">
				words
				<span class="rq-label__sub">· drag over a word or phrase to highlight it, click a highlight to remove it · {terms.length ? `${terms.length} highlighted` : 'none = find the vocabulary this section leaves undefined'}</span>
				{#if terms.length}
					<button class="rq-mini rq-mini--inline" onclick={clearMarks} disabled={posting}>clear</button>
				{/if}
			</div>
			{#if loadingContent}
				<div class="rq-help">loading the section…</div>
			{:else if words.length === 0}
				<div class="rq-help">no section content to mark — the request will ask for the undefined vocabulary of this place</div>
			{:else}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div class="rq-words" onmouseleave={dragCancel} role="group" aria-label="Section content — highlight the words to define">
					{#each paragraphsOf(sectionContent ?? '') as _p, pi (pi)}
						<p>
							{#each words as w, i (i)}
								{#if w.pi === pi}
									<!-- svelte-ignore a11y_no_static_element_interactions -->
									<!-- svelte-ignore a11y_mouse_events_have_key_events -->
									<span
										class="rq-word"
										class:on={isMarked(i)}
										class:drag={inDrag(i)}
										onmousedown={(e) => {
											e.preventDefault();
											markDown(i);
										}}
										onmouseover={() => markOver(i)}
										onmouseup={() => markUp(i)}
									>{w.w}</span>{' '}
								{/if}
							{/each}
						</p>
					{/each}
				</div>
			{/if}
			<div class="rq-label">content <span class="rq-label__sub">· generated from the highlights; “these events” = the anchor and scope</span></div>
			<div class="rq-gen" class:rq-gen--empty={terms.length === 0}>
				{#if terms.length}
					define these words in the context of these events: <span class="rq-gen__words">{terms.join(', ')}</span>
				{:else}
					{generatedTask}
				{/if}
			</div>
		{:else if output === 'ask'}
			<label class="rq-label" for="rq-task">your question</label>
			<!-- svelte-ignore a11y_autofocus -->
			<textarea
				id="rq-task"
				class="rq-input rq-task"
				data-entry
				autofocus
				placeholder="What do you want to know about this place? e.g. where does this argument first appear, and who answered it?"
				bind:value={task}
				oninput={schedulePreview}
				disabled={posting}
			></textarea>
		{:else}
			<label class="rq-label" for="rq-task">the ask <span class="rq-label__sub">· optional — leave it and the content below is what gets signed</span></label>
			<textarea
				id="rq-task"
				class="rq-input rq-task"
				data-entry
				placeholder={output === 'kasten' ? 'e.g. one note per fable, an index by moral' : 'e.g. derive this two ways — closed form and iterative — with the fields that use each'}
				bind:value={task}
				oninput={schedulePreview}
				disabled={posting}
			></textarea>
			{#if !task.trim()}
				<div class="rq-label">content <span class="rq-label__sub">· generated; “these events” = the anchor and scope</span></div>
				<div class="rq-gen">{generatedTask}</div>
			{/if}
		{/if}

		<label class="rq-label" for="rq-title">title <span class="rq-label__sub">· optional, for what gets written</span></label>
		<input id="rq-title" class="rq-input" data-entry bind:value={title} oninput={schedulePreview} disabled={posting} placeholder="Linear regression, two ways" />

		<div class="rq-label">scope <span class="rq-label__sub">· hints for whoever answers</span></div>
		<div class="rq-chips">
			{#each SCOPES as s (s.id)}
				<button
					class="rq-scope"
					class:on={scope.includes(s.id)}
					onclick={() => toggleScope(s.id)}
					disabled={posting}
					title={s.hint}
				>{s.label}</button>
			{/each}
		</div>

		<div class="rq-label">style <span class="rq-label__sub">· how the notes should be written</span></div>
		<div class="rq-style">
			<select class="rq-select" bind:value={style} onchange={schedulePreview} disabled={posting}>
				<option value="">— none —</option>
				{#each stylesList() as s (s.coordinate)}
					<option value={s.coordinate}>{s.title}</option>
				{/each}
			</select>
			<button class="rq-mini" onclick={() => (newStyleOpen = !newStyleOpen)} disabled={posting}>{newStyleOpen ? 'cancel' : '+ new style'}</button>
		</div>
		{#if style}
			{@const chosen = stylesList().find((s) => s.coordinate === style)}
			{#if chosen}
				<div class="rq-style-preview">{chosen.content}</div>
			{/if}
		{/if}
		{#if newStyleOpen}
			<input class="rq-input" data-entry placeholder="Style name — e.g. Equations and code, built up" bind:value={newStyleTitle} disabled={savingStyle} />
			<textarea class="rq-input rq-style-text" data-entry placeholder="How to write: what to lead with, what to cite, what to leave out. The answering agent reads this verbatim." bind:value={newStyleText} disabled={savingStyle}></textarea>
			<div class="rq-style">
				<span class="rq-help">signed {canSign ? 'by you' : 'by the assistant (sign in to sign it yourself)'} · local</span>
				<span class="rq-spacer"></span>
				<button class="rq-btn" onclick={saveStyle} disabled={savingStyle || !newStyleTitle.trim() || !newStyleText.trim()}>{savingStyle ? 'saving…' : 'save style'}</button>
			</div>
		{/if}

		{#if output === 'vocabulary'}
			{#if markedContext}
				<div class="rq-label">context <span class="rq-label__sub">· the paragraphs the highlights fall in travel with the request</span></div>
				<div class="rq-style-preview">{markedContext}</div>
			{/if}
		{:else if context.trim()}
			<label class="rq-label" for="rq-context">from your selection</label>
			<textarea id="rq-context" class="rq-input rq-context" data-entry bind:value={context} oninput={schedulePreview} disabled={posting}></textarea>
		{/if}

		<div class="rq-sign">
			<label class="rq-check" title={canSign ? 'Sign the request as a kind-30041 annotation on the section — the notes that answer it point at it' : 'Sign in to sign the request — unsigned requests work without an identity'}>
				<input type="checkbox" bind:checked={sign} disabled={posting || !canSign} />
				sign as an annotation (kind 30041)
			</label>
			{#if sign}
				<label class="rq-check" title="Send the annotation to your publish relays — for an agent that reads relays rather than this engine">
					<input type="checkbox" bind:checked={broadcast} disabled={posting} />
					also broadcast
				</label>
			{/if}
		</div>
		<div class="rq-help">
			{#if sign}
				Signed by you, the same shape as the notes that will answer it; each answer points back at it. {broadcast ? 'Sent to relays.' : 'Stays local.'}
			{:else}
				Local record only — nothing signed. Your agent reads it with <code>list_requests</code> over MCP.
			{/if}
		</div>

		<details class="rq-preview" bind:open={previewOpen}>
			<summary class="rq-label rq-preview__sum">event preview <span class="rq-label__sub">· the unsigned kind-30041 “sign” will sign — tags as they will be emitted</span></summary>
			{#if preview}
				<pre class="rq-preview__pre">{`{ "kind": ${preview.kind},\n  "content": ${JSON.stringify(preview.content)},\n  "tags": [\n${preview.tags.map((t) => '    ' + tagLine(t)).join(',\n')}\n  ] }`}</pre>
			{:else}
				<div class="rq-help">preview unavailable — the engine builds this from the same fields the request is saved with</div>
			{/if}
		</details>

		<footer class="rq-foot">
			<span class="rq-hint">Ctrl-Enter</span>
			<span class="rq-spacer"></span>
			<button class="rq-btn" onclick={close} disabled={posting}>Cancel</button>
			<button class="rq-btn rq-btn--post" onclick={submit} disabled={!canPost}>
				{posting ? 'Saving…' : sign ? 'sign · keep local' : 'save request'}
			</button>
		</footer>

		{#if loadingExisting}
			<div class="rq-help">loading earlier requests…</div>
		{:else if existing.length > 0}
			<div class="rq-label">earlier requests here</div>
			<ul class="rq-list">
				{#each existing as r (r.id)}
					<li class="rq-item" class:rq-item--answered={r.status === 'answered'}>
						<div class="rq-item__head">
							<span class="rq-item__status">{r.status === 'answered' ? '✓ answered' : '○ open'}</span>
							<span class="rq-item__when">{relTime(r.created_at)}{r.event_id ? ' · signed' : ''}</span>
							<span class="rq-spacer"></span>
							<button class="rq-mini" onclick={() => remove(r)} title="Delete this request (the local record)">delete</button>
						</div>
						<div class="rq-item__task">{r.title ? `${r.title} — ` : ''}{r.task}</div>
						{#if r.answer}
							<div class="rq-item__answer">{r.answer.summary}</div>
							{#if r.answer.coordinates.length > 0}
								<div class="rq-item__coords">
									{#each r.answer.coordinates as c (c)}
										<button class="rq-mini" onclick={() => openCoordinate(c)} title={c}>{c.startsWith('30040') ? '⊞' : '§'} {c.split(':').slice(2).join(':')}</button>
									{/each}
								</div>
							{/if}
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

<style>
	.rq-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 8dvh;
	}
	.rq-backdrop--yield {
		visibility: hidden;
	}
	.rq-modal {
		width: min(600px, 94vw);
		max-height: 88dvh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}
	.rq-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.rq-title {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--fg);
		display: flex;
		align-items: baseline;
		gap: 6px;
		flex-wrap: wrap;
	}
	/* the window's name and its anchors are titles: the theme's green, not the ask accent */
	.rq-dot {
		color: var(--green);
	}
	.rq-title__on {
		color: var(--base5);
		font-weight: 400;
	}
	.rq-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
	}
	.rq-label {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-top: 6px;
	}
	.rq-label__sub {
		text-transform: none;
		letter-spacing: 0;
	}
	.rq-anchor,
	.rq-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.rq-chip {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--fg-muted);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 2px 8px;
		display: inline-flex;
		align-items: center;
		gap: 6px;
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.rq-chip b {
		color: var(--green);
	}
	.rq-chip--select select {
		font: inherit;
		color: var(--fg);
		background: transparent;
		border: none;
		max-width: 40ch;
	}
	.rq-input {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 5px 8px;
	}
	.rq-input:focus {
		outline: none;
		border-color: var(--id-remote);
	}
	.rq-task {
		min-height: 88px;
		resize: vertical;
	}
	.rq-context {
		min-height: 44px;
		resize: vertical;
		color: var(--fg-muted);
	}
	.rq-scope {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base6);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: 999px;
		padding: 2px 10px;
		min-height: 26px;
		cursor: pointer;
	}
	.rq-scope.on {
		background: var(--id-remote);
		color: var(--bg);
		border-color: var(--id-remote);
	}
	/* the window is a scrolling flex column: its children must not shrink
	   when the content outgrows it, or the segment collapses to a line */
	.rq-modal > * {
		flex-shrink: 0;
	}
	.rq-seg {
		display: inline-flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		overflow: hidden;
		align-self: flex-start;
	}
	.rq-seg__opt {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: transparent;
		border: none;
		border-right: 1px solid var(--panel-border);
		padding: 4px 12px;
		min-height: 30px;
		cursor: pointer;
	}
	.rq-seg__opt:last-child {
		border-right: none;
	}
	.rq-seg__opt.on {
		background: var(--ask);
		color: var(--bg);
	}
	.rq-style {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.rq-select {
		flex: 1;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 8px;
		min-height: 30px;
	}
	.rq-style-preview {
		font-size: calc(var(--t-xs) - 1px);
		color: var(--fg-muted);
		border-left: 2px solid var(--panel-border);
		padding: 2px 8px;
		max-height: 96px;
		overflow: auto;
		white-space: pre-wrap;
	}
	.rq-style-text {
		min-height: 88px;
		resize: vertical;
	}
	.rq-sign {
		display: flex;
		gap: 14px;
		flex-wrap: wrap;
		margin-top: 6px;
	}
	.rq-check {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--fg-muted);
	}
	.rq-help {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	.rq-help code {
		color: var(--fg-muted);
	}
	.rq-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 6px;
	}
	.rq-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.rq-spacer {
		flex: 1;
	}
	.rq-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		min-height: 30px;
		cursor: pointer;
	}
	.rq-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rq-btn--post {
		color: var(--ask);
	}
	.rq-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.rq-list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.rq-item {
		border: 1px solid var(--panel-border);
		border-left: 2px solid var(--id-remote);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		font-size: var(--t-xs);
	}
	.rq-item--answered {
		border-left-color: var(--id-yours);
	}
	.rq-item__head {
		display: flex;
		align-items: center;
		gap: 8px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	.rq-item__status {
		color: var(--fg-muted);
	}
	.rq-item__task {
		margin-top: 2px;
		color: var(--fg);
	}
	.rq-item__answer {
		margin-top: 4px;
		color: var(--fg-muted);
		font-style: italic;
	}
	.rq-item__coords {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		margin-top: 4px;
	}
	.rq-mini {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base6);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 1px 6px;
		cursor: pointer;
	}
	.rq-mini:hover {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rq-mini--inline {
		margin-left: 6px;
		text-transform: none;
		letter-spacing: 0;
	}
	/* vocabulary: the section's content, word by word */
	.rq-words {
		font-family: var(--font-serif);
		font-size: var(--t-md);
		line-height: 1.6;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 8px 12px;
		max-height: 240px;
		overflow: auto;
		user-select: none;
		cursor: crosshair;
	}
	.rq-words p {
		margin: 0 0 8px;
	}
	.rq-words p:last-child {
		margin-bottom: 0;
	}
	.rq-word {
		border-radius: 2px;
		padding: 0 1px;
		transition: background 0.08s;
	}
	.rq-word:hover {
		background: color-mix(in srgb, var(--ask) 28%, transparent);
	}
	.rq-word.drag {
		background: color-mix(in srgb, var(--ask) 28%, transparent);
	}
	.rq-word.on {
		background: color-mix(in srgb, var(--green) 32%, transparent);
		color: var(--base8);
		box-shadow: 0 1px 0 var(--green);
	}
	.rq-word.on:hover {
		background: color-mix(in srgb, var(--red) 30%, transparent);
		box-shadow: 0 1px 0 var(--red);
	}
	.rq-gen {
		font-family: var(--font-serif);
		font-size: var(--t-md);
		line-height: 1.5;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--ask);
		border-radius: var(--r-sm);
		padding: 6px 10px;
	}
	.rq-gen--empty {
		color: var(--base5);
		font-style: italic;
	}
	.rq-gen__words {
		color: var(--green);
	}
	.rq-preview {
		margin-top: 6px;
	}
	.rq-preview__sum {
		cursor: pointer;
		margin-top: 0;
	}
	.rq-preview__pre {
		margin: 4px 0 0;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		line-height: 1.5;
		color: var(--base7);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		max-height: 260px;
		overflow: auto;
		white-space: pre-wrap;
		word-break: break-word;
	}
</style>
