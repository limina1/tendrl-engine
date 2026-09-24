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
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';

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

	// ---- answer mode: answer a request by hand, ONE TERM AT A TIME --------
	// vocabulary → the request's terms in a dropdown, one definition entry,
	// "sign ▸" signs that one 30041 (role definition, the request pointer)
	// and pages to the next undone term, closing after the last; "skip ▸"
	// pages without signing. ask → the question as a fixed title, one note,
	// sign closes. notes / kasten → the composer with the request pointer.
	// "define them myself ▸" (vocabulary mode, marked words) is the same
	// window over the marked terms with no request at all.
	type Format = 'markdown' | 'org' | 'asciidoc' | 'plain';
	type TermStatus = 'pending' | 'defined' | 'signed' | 'skipped';
	type TermEntry = { term: string; body: string; format: Format; status: TermStatus; by: string | null; fixed?: boolean };
	const FORMATS: Format[] = ['markdown', 'org', 'asciidoc', 'plain'];
	let answering = $state(!!open?.answer);
	let answer = $state<api.AnnotationRequest | null>(open?.answer ?? null);
	/** "define them myself": answer mode with no request. */
	let defineOnly = $state(false);
	let entries = $state<TermEntry[]>([]);
	let idx = $state(0);
	let newTerm = $state(false);
	let signedCount = $state(0);
	let signedCoords = $state<string[]>([]);
	let answerStyle = $state<string>('');
	const openHere = $derived(existing.filter((r) => r.status === 'open'));
	const isDef = $derived(defineOnly || answer?.output === 'vocabulary');
	const entry = $derived(entries[idx] ?? null);
	const todo = $derived(entries.filter((e) => e.status === 'pending').length);
	const skipped = $derived(entries.filter((e) => e.status === 'skipped').map((e) => e.term));
	const myPubkey = $derived(app.identityStatus?.pubkey ?? app.myPubkey ?? null);
	const requestBy = $derived.by(() => {
		const r = answer;
		if (!r) return 'you';
		const pk = r.coordinate?.split(':')[1];
		if (!pk || pk === myPubkey) return 'you';
		return getAuthorDisplayName(pk);
	});
	function whoName(pk: string | null): string {
		if (!pk) return 'someone';
		return pk === myPubkey ? 'you' : getAuthorDisplayName(pk);
	}
	function fresh(term: string): TermEntry {
		return { term, body: '', format: 'markdown', status: 'pending', by: null };
	}
	/** The request's terms, each with what already answers it (the
	 *  definitions loaded for this root that carry its pointer). */
	async function entriesFor(r: api.AnnotationRequest): Promise<TermEntry[]> {
		if (r.output !== 'vocabulary') {
			const t = r.title ?? `${r.task.slice(0, 60)}${r.task.length > 60 ? '…' : ''}`;
			return [{ ...fresh(t), fixed: true }];
		}
		const list = (r.terms ?? []).filter((t) => t.trim()).map(fresh);
		let defs: api.LoadedInsertion[] = [];
		try {
			const resp = await api.loadInsertions({ root: r.root, policy: 'local_only' });
			defs = resp.insertions.filter(
				(i) => i.role === 'definition' && (i.request === r.coordinate || (r.answer?.coordinates ?? []).includes(i.coordinate))
			);
		} catch {
			defs = [];
		}
		for (const d of defs) {
			const term = d.term ?? d.event.tags.find((t) => t[0] === 'title')?.[1] ?? '';
			const hit = list.find((e) => e.term.toLowerCase() === term.toLowerCase());
			if (hit) {
				hit.status = 'defined';
				hit.by = d.event.pubkey;
			} else if (term) list.push({ ...fresh(term), status: 'defined', by: d.event.pubkey });
		}
		return list.length ? list : [fresh('')];
	}
	function firstPending(list: TermEntry[]): number {
		const k = list.findIndex((e) => e.status === 'pending');
		return k < 0 ? 0 : k;
	}
	async function startAnswer(r: api.AnnotationRequest) {
		answer = r;
		defineOnly = false;
		answering = true;
		signedCount = 0;
		signedCoords = [];
		newTerm = false;
		answerStyle = r.style ?? style;
		entries = [fresh('')];
		idx = 0;
		const list = await entriesFor(r);
		if (answer !== r) return;
		entries = list;
		idx = firstPending(list);
		newTerm = !list[idx]?.term;
		scheduleAnswerPreview();
	}
	function enterAnswerMode() {
		answering = true;
		if (!answer && openHere.length === 1) void startAnswer(openHere[0]);
	}
	function leaveAnswerMode() {
		answering = false;
		answer = null;
		defineOnly = false;
	}
	/** Vocabulary mode → define the marked words yourself, no request. */
	function defineMyself() {
		if (terms.length === 0) return;
		answer = null;
		defineOnly = true;
		answering = true;
		signedCount = 0;
		signedCoords = [];
		answerStyle = style;
		entries = terms.map(fresh);
		idx = 0;
		newTerm = false;
		scheduleAnswerPreview();
	}
	function pickTerm(v: string) {
		if (v === '+') {
			entries = [...entries, fresh('')];
			idx = entries.length - 1;
			newTerm = true;
		} else {
			idx = Number(v);
			newTerm = false;
		}
		scheduleAnswerPreview();
	}
	function nextPending(from: number): number {
		const n = entries.length;
		for (let k = 1; k < n; k++) {
			const j = (from + k) % n;
			if (entries[j].status === 'pending') return j;
		}
		return -1;
	}
	function anchorFor(): api.InsertionRequest['anchor'] {
		const r = answer;
		if (r) return { root: r.root, parent: r.parent && r.parent !== r.root ? r.parent : undefined, parent_event_id: r.parent_event_id };
		return { root, parent: topLevel ? undefined : parent, parent_event_id: open?.parentEventId };
	}
	function entryRequest(e: TermEntry, broadcast = false): api.InsertionRequest {
		return {
			anchor: anchorFor(),
			title: e.term.trim(),
			content: e.body.trim(),
			format: e.format === 'markdown' ? undefined : e.format,
			role: isDef ? 'definition' : 'note',
			request: answer?.coordinate,
			tags: answerStyle ? [['style', answerStyle]] : undefined,
			sign_as: 'user',
			broadcast
		};
	}
	if (open?.answer) void startAnswer(open.answer);
	// event preview: the one unsigned template for the current entry, debounced
	let answerPreview = $state<string | null>(null);
	let answerPreviewTimer: ReturnType<typeof setTimeout> | null = null;
	let answerPreviewSeq = 0;
	function scheduleAnswerPreview() {
		if (answerPreviewTimer) clearTimeout(answerPreviewTimer);
		answerPreviewTimer = setTimeout(() => void refreshAnswerPreview(), 250);
	}
	async function refreshAnswerPreview() {
		const e = entry;
		const seq = ++answerPreviewSeq;
		if (!e || !e.term.trim() || !e.body.trim()) {
			answerPreview = null;
			return;
		}
		try {
			const r = await api.previewInsertion(entryRequest(e));
			if (seq === answerPreviewSeq)
				answerPreview = `{ "kind": ${r.event.kind},\n  "content": ${JSON.stringify(r.event.content)},\n  "tags": [\n${r.event.tags.map((t) => '    ' + tagLine(t)).join(',\n')}\n  ] }`;
		} catch {
			if (seq === answerPreviewSeq) answerPreview = null;
		}
	}
	const canSignTerm = $derived(
		canSign && !posting && !!entry && entry.term.trim().length > 0 && entry.body.trim().length > 0
	);
	/** Close the local record (an unsigned request) with what was written. */
	async function closeRecord() {
		const r = answer;
		if (!r || r.coordinate || signedCoords.length === 0) return;
		const what = isDef ? `${signedCount} definition${signedCount === 1 ? '' : 's'}` : 'an answer';
		const names = entries.filter((e) => e.status === 'signed').map((e) => e.term).join(', ');
		await api.answerRequest(r.id, {
			summary: `${what} written by hand${names ? `: ${names}` : ''}${skipped.length ? ` · skipped: ${skipped.join(', ')}` : ''}`,
			coordinates: signedCoords
		});
	}
	/** Done: report what was signed and skipped, close the window. */
	async function finishAnswer() {
		if (signedCount > 0) {
			try {
				await closeRecord();
			} catch (e) {
				app.pushToast(api.errorMessage(e, 'Closing the request failed'), 'error', 4000);
			}
			const what = isDef ? `${signedCount} definition${signedCount === 1 ? '' : 's'}` : 'answer';
			app.pushToast(
				`${what} signed as you${answer ? ', pointing at the request' : ' — no request, just definitions anchored here'}${skipped.length ? ` · skipped: ${skipped.join(', ')}` : ''}`,
				'success'
			);
			app.bumpInsertions();
			app.bumpRequests();
		}
		close();
	}
	/** Sign the current entry; page to the next undone term or close. */
	async function signTerm() {
		const e = entry;
		if (!canSignTerm || !e) return;
		posting = true;
		try {
			const resp = await api.publishInsertion(entryRequest(e));
			e.status = 'signed';
			e.by = myPubkey;
			signedCount += 1;
			signedCoords = [...signedCoords, resp.coordinate];
			if (signedCount === 1) {
				// The first signed event closes the record; the signed request
				// is already marked answered by the pointer itself.
				await closeRecord();
				app.bumpRequests();
			}
			app.bumpInsertions();
		} catch (err) {
			app.pushToast(api.errorMessage(err, 'Signing failed'), 'error', 5000);
			posting = false;
			return;
		}
		posting = false;
		const j = nextPending(idx);
		if (j < 0) {
			await finishAnswer();
			return;
		}
		app.pushToast(`signed “${e.term.trim()}”`, 'success');
		idx = j;
		newTerm = !entries[j].term;
		answerPreview = null;
		scheduleAnswerPreview();
	}
	/** Leave this term for the agent or for later; page on. */
	async function skipTerm() {
		const e = entry;
		if (!e || posting) return;
		if (e.status === 'pending') e.status = 'skipped';
		const j = nextPending(idx);
		if (j < 0) {
			await finishAnswer();
			return;
		}
		idx = j;
		newTerm = !entries[j].term;
		answerPreview = null;
		scheduleAnswerPreview();
	}
	/** notes / kasten: the composer, with the request pointer. */
	function openComposerFor(r: api.AnnotationRequest) {
		app.annotationRequest = null;
		app.insertionComposer = {
			root: r.root,
			rootTitle: open?.rootTitle ?? null,
			parent: r.parent && r.parent !== r.root ? r.parent : undefined,
			parentTitle: r.parent_title ?? open?.parentTitle ?? null,
			parentEventId: r.parent_event_id,
			context: r.context,
			request: r.coordinate,
			requestTitle: r.title ?? `${r.task.slice(0, 60)}${r.task.length > 60 ? '…' : ''}`
		};
	}
	const OUTPUT_LABEL: Record<string, string> = { notes: 'notes', kasten: 'kasten', vocabulary: 'vocabulary', ask: 'ask' };

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
			if (answering) void signTerm();
			else void submit();
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
				{answering ? (defineOnly ? 'define' : 'answer') : 'annotate'}
				{#if answering && answer}
					<span class="rq-title__on">— <em>{answer.title ?? answer.task}</em></span>
					<span class="rq-pick__badge">{answer.status === 'open' ? '?' : '✓'} {OUTPUT_LABEL[answer.output] ?? answer.output}</span>
					<span class="rq-title__on">by {requestBy} · {relTime(answer.created_at)}</span>
				{:else if answering && defineOnly}
					<span class="rq-title__on">— {entries.length} term{entries.length === 1 ? '' : 's'} on <em>{topLevel ? 'the publication' : (open?.parentTitle ?? short(parent))}</em></span>
				{:else if topLevel}
					<span class="rq-title__on">— on the publication</span>
				{:else if open?.parentTitle}
					<span class="rq-title__on">— on <em>{open.parentTitle}</em></span>
				{/if}
			</h3>
			{#if answering && isDef && entries.length > 0}
				<span class="rq-count" title="terms in this request">{idx + 1} of {entries.length} · {todo} to do</span>
			{/if}
			{#if answering}
				<button class="rq-mini rq-answer-back" onclick={leaveAnswerMode} disabled={posting} title="Back to asking your agent">← annotate</button>
			{:else if openHere.length > 0}
				<button class="rq-mini rq-answer-toggle" onclick={enterAnswerMode} disabled={posting} title="Answer one of the open requests here by hand instead of asking your agent">answer a request ({openHere.length}) ▸</button>
			{/if}
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
			{#if answering && answer?.coordinate}
				<span class="rq-chip rq-chip--request" title={answer.coordinate}><b>request</b> {short(answer.coordinate)}</span>
			{/if}
		</div>

		{#if !answering}
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
			{#if output === 'vocabulary' && terms.length > 0}
				<button class="rq-btn rq-btn--define" onclick={defineMyself} disabled={posting || !canSign} title={canSign ? 'Skip the agent: write the definitions yourself — one signed 30041 per term, no request event' : 'Sign in to write definitions yourself'}>define them myself ▸</button>
			{/if}
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
		{:else if !answer && !defineOnly}
			<div class="rq-label">open requests here <span class="rq-label__sub">· pick the one to answer by hand</span></div>
			<div class="rq-picks">
				{#each openHere as r (r.id)}
					<button class="rq-pick" onclick={() => startAnswer(r)} disabled={posting}>
						<span class="rq-pick__badge">{OUTPUT_LABEL[r.output] ?? r.output}</span>
						<span class="rq-pick__task">{r.title ? `${r.title} — ` : ''}{r.task}</span>
						{#if r.output === 'vocabulary' && r.terms?.length}
							<span class="rq-pick__terms">{r.terms.join(', ')}</span>
						{/if}
						<span class="rq-pick__meta">{r.coordinate ? 'signed · ' : ''}{relTime(r.created_at)}</span>
					</button>
				{/each}
			</div>
		{:else if answer && (answer.output === 'notes' || answer.output === 'kasten')}
			<div class="rq-ask-line">
				<span class="rq-pick__badge">{OUTPUT_LABEL[answer.output] ?? answer.output}</span>
				<span class="rq-ask-line__task">{answer.title ? `${answer.title} — ` : ''}{answer.task}</span>
			</div>
			{#if answer.context}
				<div class="rq-style-preview">{answer.context}</div>
			{/if}
			<div class="rq-help">
				{answer.output === 'kasten'
					? 'write it as a note here; a full kasten is the agent\'s job — the composer opens anchored to this place with the request pointer'
					: 'the composer opens anchored to this place with the request pointer — the note you write there answers it'}
			</div>
			<footer class="rq-foot">
				<span class="rq-spacer"></span>
				<button class="rq-btn" onclick={close} disabled={posting}>Cancel</button>
				<button class="rq-btn rq-btn--post rq-answer-compose" onclick={() => openComposerFor(answer!)} disabled={posting}>open the composer ▸</button>
			</footer>
		{:else if entry}
			{#if answer && answer.output !== 'vocabulary'}
				<div class="rq-ask-line">
					<span class="rq-pick__badge">{OUTPUT_LABEL[answer.output] ?? answer.output}</span>
					<span class="rq-ask-line__task">{answer.task}</span>
				</div>
				{#if answer.context}
					<div class="rq-style-preview">{answer.context}</div>
				{/if}
			{/if}
			{#if isDef}
				<div class="rq-label">term <span class="rq-label__sub">· one definition per term; pick from the request or add one</span></div>
				<div class="rq-term">
					{#if newTerm || !entry.term}
						<!-- svelte-ignore a11y_autofocus -->
						<input class="rq-input rq-term__new" data-entry autofocus bind:value={entry.term} oninput={scheduleAnswerPreview} disabled={posting} placeholder="new term, as written in the text" />
						{#if entries.some((e) => e.term)}
							<button class="rq-mini" onclick={() => { newTerm = false; if (!entry.term.trim()) { entries = entries.filter((_, k) => k !== idx); idx = Math.min(idx, entries.length - 1); } }} disabled={posting} title="Back to the request's terms">list ▾</button>
						{/if}
					{:else}
						<select class="rq-select rq-term__pick" value={String(idx)} onchange={(e) => pickTerm((e.currentTarget as HTMLSelectElement).value)} disabled={posting} title="Which term this entry defines">
							{#each entries as e, k (k)}
								<option value={String(k)}>{e.term || '(new term)'}{e.status === 'defined' ? ` · defined by ${whoName(e.by)}` : e.status === 'signed' ? ' · signed by you' : e.status === 'skipped' ? ' · skipped' : ''}</option>
							{/each}
							<option value="+">+ another term…</option>
						</select>
					{/if}
					<select class="rq-select rq-term__fmt" bind:value={entry.format} onchange={scheduleAnswerPreview} disabled={posting} title="Content format">
						{#each FORMATS as f (f)}<option value={f}>{f}</option>{/each}
					</select>
				</div>
				{#if entry.status === 'defined'}
					<div class="rq-warn">already defined by {whoName(entry.by)} — signing writes a second definition of “{entry.term}” under your key; the relink will ask which one to point at</div>
				{:else if entry.status === 'signed'}
					<div class="rq-warn">signed by you a moment ago — signing again writes a second definition</div>
				{/if}
			{:else}
				<div class="rq-label">title</div>
				<div class="rq-term">
					<span class="rq-term__fixed">{entry.term}</span>
					<select class="rq-select rq-term__fmt" bind:value={entry.format} onchange={scheduleAnswerPreview} disabled={posting} title="Content format">
						{#each FORMATS as f (f)}<option value={f}>{f}</option>{/each}
					</select>
				</div>
			{/if}
			<label class="rq-label" for="rq-def">
				{isDef ? 'definition' : 'your answer'}
				<span class="rq-label__sub">{isDef ? '· the sense in this text first; that sentence becomes the hover summary of every {{ref}} to it' : '· a signed note pointing at the request'}</span>
			</label>
			<textarea id="rq-def" class="rq-input rq-def" data-entry bind:value={entry.body} oninput={scheduleAnswerPreview} disabled={posting} placeholder={isDef ? 'the sense here first, then the background a reader needs, then where the word comes from' : 'the answer, in your words — cite with {{ref:…}} where it helps'}></textarea>
			<div class="rq-label">style</div>
			<div class="rq-style">
				<select class="rq-select" bind:value={answerStyle} onchange={scheduleAnswerPreview} disabled={posting}>
					<option value="">— none —</option>
					{#each stylesList() as s (s.coordinate)}
						<option value={s.coordinate}>{s.title}</option>
					{/each}
				</select>
			</div>
			<details class="rq-preview" bind:open={previewOpen}>
				<summary class="rq-label rq-preview__sum">event preview <span class="rq-label__sub">· the unsigned {isDef ? 'definition' : 'note'} “sign” will sign</span></summary>
				{#if answerPreview}
					<pre class="rq-preview__pre">{answerPreview}</pre>
				{:else}
					<div class="rq-help">fill the {isDef ? 'term and its definition' : 'answer'} — the engine builds the template from them</div>
				{/if}
			</details>
			<div class="rq-help">
				{#if !canSign}
					Sign in to sign — each event is signed as you{answer ? (answer.coordinate ? ', pointing at the signed request' : ', and the local record closes with it') : ''}.
				{:else if answer}
					Signed as you, kept local; broadcast is the separate step. {answer.coordinate ? 'Each event points at the request — that is what marks it answered.' : 'The local record closes with the first one.'}
				{:else}
					Signed as you, kept local, no request — plain definitions anchored here.
				{/if}
			</div>
			{#if signedCount > 0}
				<div class="rq-help rq-signed">{signedCount} signed so far{isDef ? ' — the rows are already in the ledger' : ''}</div>
			{/if}
			<footer class="rq-foot">
				<span class="rq-hint">Ctrl-Enter</span>
				<span class="rq-spacer"></span>
				<button class="rq-btn rq-btn--done" onclick={finishAnswer} disabled={posting}>{signedCount > 0 ? 'done' : 'Cancel'}</button>
				{#if isDef && entries.length > 1}
					<button class="rq-btn rq-btn--skip" onclick={skipTerm} disabled={posting} title="Leave this term for the agent or for later; move on">skip ▸</button>
				{/if}
				<button class="rq-btn rq-btn--sign" onclick={signTerm} disabled={!canSignTerm} title={isDef && nextPending(idx) >= 0 ? 'Signs this definition and pages to the next term' : 'Signs and closes'}>{posting ? 'signing…' : 'sign ▸'}</button>
			</footer>
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
	.rq-answer-toggle,
	.rq-answer-back {
		margin-left: auto;
		margin-right: var(--s-2);
	}
	.rq-answer-toggle {
		color: var(--ask);
		border-color: color-mix(in srgb, var(--ask) 50%, transparent);
	}
	.rq-picks {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.rq-pick {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 6px 10px;
		text-align: left;
		font: inherit;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--ask);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		cursor: pointer;
	}
	.rq-pick:hover:not(:disabled) {
		border-color: var(--panel-border-strong);
		border-left-color: var(--ask);
	}
	.rq-pick__badge {
		font: 600 calc(var(--t-xs) - 3px) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ask);
		border: 1px solid color-mix(in srgb, var(--ask) 50%, transparent);
		border-radius: 3px;
		padding: 1px 6px;
	}
	.rq-pick__task {
		flex: 1 1 60%;
		font-size: var(--t-xs);
	}
	.rq-pick__terms,
	.rq-pick__meta {
		font: calc(var(--t-xs) - 2px) / 1.3 var(--font-mono);
		color: var(--base5);
	}
	.rq-ask-line {
		display: flex;
		align-items: baseline;
		gap: 10px;
		font-family: var(--font-serif);
		font-size: var(--t-md);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--ask);
		border-radius: var(--r-sm);
		padding: 6px 10px;
	}
	.rq-count {
		margin-left: auto;
		margin-right: var(--s-2);
		font: calc(var(--t-xs) - 2px) / 1.3 var(--font-mono);
		color: var(--base5);
		white-space: nowrap;
	}
	.rq-count + .rq-answer-back {
		margin-left: 0;
	}
	.rq-chip--request b {
		color: var(--ask);
	}
	.rq-term {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	.rq-term__pick,
	.rq-term__new,
	.rq-term__fixed {
		flex: 1;
		min-width: 0;
		font: 600 var(--t-md) / 1.3 var(--font-sans);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--green);
		border-radius: var(--r-sm);
		padding: 5px 8px;
		min-height: 32px;
	}
	.rq-term__pick {
		cursor: pointer;
	}
	.rq-term__fixed {
		font-family: var(--font-serif);
		font-weight: 500;
	}
	.rq-term__fmt {
		flex: none;
	}
	.rq-warn {
		font: calc(var(--t-xs) - 2px) / 1.4 var(--font-mono);
		color: var(--yellow);
	}
	.rq-def {
		font-family: var(--font-serif);
		font-size: var(--t-md);
		min-height: 96px;
		resize: vertical;
	}
	.rq-signed {
		color: var(--green);
	}
	.rq-btn--sign,
	.rq-btn--define {
		color: var(--green);
	}
	.rq-btn--sign:hover:not(:disabled),
	.rq-btn--define:hover:not(:disabled) {
		border-color: var(--green);
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
