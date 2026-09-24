<script lang="ts">
	// The expanded body of a ledger row (`.xb`): the full text, the meta
	// chips, the per-kind actions (docs/annotations-ledger-contract.org,
	// "Row actions") wired to today's handlers — app.relink, the glossary
	// preview + publish (lifted from InsertionCard), app.openMergeRequestReview,
	// app.openAddressableInModal, app.annotationRequest with `answer` —
	// and the thread. Rendered under its row (inline) or in the detail
	// pane (wide); the same component either way.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { identityCanSign } from '$lib/identity/signer';
	import { styleTitle } from '$lib/styles.svelte';
	import RichContent from '$lib/components/RichContent.svelte';
	import EventLinks from '$lib/components/EventLinks.svelte';
	import AnnotationThread from './AnnotationThread.svelte';
	import { glyphOf, type LedgerRow, type Section } from './ledger-model';

	let {
		row,
		answers = [],
		sections,
		root,
		rootTitle = null,
		ownsRoot = false,
		who,
		requestTitle = undefined,
		onopen,
		onreveal,
		onchanged
	}: {
		row: LedgerRow;
		/** Requests: the rows that answer it. */
		answers?: LedgerRow[];
		sections: Section[];
		root: string;
		rootTitle?: string | null;
		ownsRoot?: boolean;
		who: (pk: string) => string;
		/** Answering rows: the title of the request they answer. */
		requestTitle?: string | undefined;
		/** Open the note in its own reader buffer / the kasten as a publication. */
		onopen: (row: LedgerRow) => void;
		/** Reveal another row (the request this answers, a definition). */
		onreveal: (id: string) => void;
		/** Something was signed (a glossary change) — reload. */
		onchanged: () => void;
	} = $props();

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));
	const styleLabel = $derived(row.style ? (row.req?.style_title ?? styleTitle(row.style) ?? row.style.split(':').slice(2).join(':')) : null);
	const parentTitle = $derived(row.sec < 0 ? null : (sections[row.sec]?.title ?? null));
	const defAnswers = $derived(answers.filter((a) => a.kind === 'def'));
	const mine = $derived(!!app.identityStatus?.pubkey && row.author === app.identityStatus.pubkey);
	const mrStatus = $derived(row.note?.merge_request?.status ?? row.status ?? 'open');
	const coordLabel = $derived(
		row.kind === 'request' && row.req && !row.req.coordinate ? `record · ${row.req.id}` : row.kind === 'mr' ? `818 · ${row.id.slice(4, 12)}…` : row.id
	);

	function stop(e: Event) {
		e.stopPropagation();
	}
	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		void app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}
	function menu() {
		if (row.kind === 'mr') {
			const id = row.note?.merge_request?.id ?? row.id.slice(4);
			void app.getEventForModal?.(id);
			return;
		}
		if (row.kind === 'request') {
			if (row.req?.coordinate) openCoordinate(row.req.coordinate);
			else app.pushToast('Unsigned local record — nothing signed to open', 'info');
			return;
		}
		openCoordinate(row.id);
	}
	function answer() {
		if (!row.req) return;
		app.annotationRequest = {
			root,
			rootTitle,
			parent: row.parent !== root ? row.parent : undefined,
			parentTitle,
			parentEventId: row.req.parent_event_id,
			output: row.req.output,
			answer: row.req
		};
	}
	function relink(defs: LedgerRow[]) {
		if (!defs.length) return;
		app.relink = {
			root,
			rootTitle,
			definitions: defs.map((d) => d.id),
			promoted: defs.filter((d) => d.glossary).map((d) => d.id)
		};
	}
	function review() {
		const id = row.note?.merge_request?.id ?? row.id.slice(4);
		app.openMergeRequestReview(id);
	}
	function demoAgent() {
		app.pushToast('say “check tendrl requests” in your agent — it lists the open requests and answers them', 'info', 4000);
	}

	// --- Glossary (definitions): promote or remove ONE event ---------------
	// The same preview-then-one-armed-button shape InsertionCard has; the
	// ledger keeps its own copy so the card file stays with its track.
	let glossOpen = $state(false);
	let glossMode = $state<'add' | 'remove'>('add');
	let glossPreview = $state<api.GlossaryPreview | null>(null);
	let glossErr = $state<string | null>(null);
	let glossPreviewing = $state(false);
	let glossPublishing = $state(false);
	function glossaryBody(mode: 'add' | 'remove'): api.GlossaryRequest {
		return mode === 'add' ? { root, add: [row.id] } : { root, remove: [row.id] };
	}
	async function openGlossary(mode: 'add' | 'remove') {
		if (glossOpen && glossMode === mode) {
			glossOpen = false;
			return;
		}
		glossMode = mode;
		glossOpen = true;
		glossPreview = null;
		glossErr = null;
		glossPreviewing = true;
		try {
			glossPreview = await api.previewGlossary(glossaryBody(mode));
		} catch (e) {
			glossErr = api.errorMessage(e, 'preview failed');
		} finally {
			glossPreviewing = false;
		}
	}
	async function publishGlossaryChange() {
		if (!glossPreview || glossPublishing) return;
		glossPublishing = true;
		try {
			const resp = await api.publishGlossary(glossaryBody(glossMode));
			const n = resp.promoted.length;
			app.pushToast(
				glossMode === 'add'
					? `“${row.term ?? row.title}” promoted into the glossary (${n} definition${n === 1 ? '' : 's'})${resp.index ? ' — index republished' : ''}`
					: `“${row.term ?? row.title}” taken out of the glossary (${n} left)${resp.index ? ' — index republished' : ''}`,
				'success'
			);
			glossOpen = false;
			onchanged();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Glossary update failed'), 'error', 5000);
		} finally {
			glossPublishing = false;
		}
	}
	function tagValue(t: api.UnsignedTemplate | null | undefined, name: string): string | undefined {
		return t?.tags.find((x) => x[0] === name)?.[1];
	}
	function shortTag(t: string[]): string {
		return t.map((v) => (v.length > 40 ? v.slice(0, 18) + '…' + v.slice(-12) : v)).join(' ');
	}
	function wireText(p: api.GlossaryPreview): string {
		const rows = (t: api.UnsignedTemplate) => t.tags.map((x) => '  [' + shortTag(x) + ']').join('\n');
		let out = `glossary · kind ${p.glossary.kind}\n` + rows(p.glossary);
		if (p.index) out += `\n\nindex · kind ${p.index.kind}\n` + rows(p.index);
		return out;
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="xb" data-id={row.id} style="--au: {row.author ? pubkeyToColor(row.author) : 'var(--base5)'}" onclick={stop} onkeydown={stop}>
	{#if row.kind === 'request' && row.req}
		{@const r = row.req}
		<div class="meta">
			<span><span class="meta__k">output</span>{r.output}</span>
			{#if r.output === 'vocabulary'}
				<span><span class="meta__k">terms</span>{#if r.terms.length}{#each r.terms as t (t)}<span class="chip">{t}</span> {/each}{:else}<span class="meta__muted">find them</span>{/if}</span>
			{/if}
			{#if r.scope.length}<span><span class="meta__k">scope</span>{r.scope.join(' · ')}</span>{/if}
			{#if styleLabel}<span><span class="meta__k">style</span>✎ {styleLabel}</span>{/if}
			<span><span class="meta__k">{r.coordinate ? 'signed' : 'record'}</span>30041 · t: annotation-request</span>
		</div>
		{#if r.output !== 'vocabulary'}
			<div class="xb__txt"><p>{r.task}</p></div>
		{/if}
		{#if r.context}<blockquote class="xb__ctx">{r.context}</blockquote>{/if}
		<div class="xb__answers">
			{#if answers.length}
				answered by {answers.length} {answers.length === 1 ? 'event' : 'events'} —
				{#each answers as a, i (a.id)}{#if i > 0} · {/if}<button class="xb__link" onclick={() => onreveal(a.id)}><span class="g {glyphOf(a).cls}">{glyphOf(a).g}</span> {a.title}</button>{/each}
			{:else if r.answer}
				answered — {r.answer.summary}
			{:else if r.coordinate}
				open — answered by the first event that points at it
			{:else}
				local record — say “check tendrl requests” in your agent
			{/if}
		</div>
		<div class="acts">
			{#if r.status === 'open' || r.output === 'vocabulary'}
				<button
					class="act act--answer"
					onclick={answer}
					disabled={!canSign}
					title={!canSign
						? 'Sign in to answer'
						: r.output === 'vocabulary'
							? 'Write the definitions yourself — one signed entry per term, each pointing at this request'
							: r.output === 'ask'
								? 'Answer the question yourself — a signed note pointing at this request'
								: 'Write it yourself in the composer, pointing at this request'}
				>{r.status === 'open' ? 'answer it yourself ▸' : 'define a term yourself ▸'}</button>
			{/if}
			{#if r.status === 'open'}
				<button class="act act--demo" onclick={demoAgent} title="your own agent answers requests over MCP">let the agent answer</button>
			{/if}
			{#if r.output === 'vocabulary' && defAnswers.length}
				<button class="act act--relink" onclick={() => relink(defAnswers)} disabled={!canSign} title={ownsRoot ? 'Wrap the first occurrence of each term as a {{ref}} to its definition — diff first, then one armed sign' : 'Not your publication: a derivative with the refs in, plus a kind 818 to the owner'}>
					{ownsRoot ? `relink ${defAnswers.length} term${defAnswers.length === 1 ? '' : 's'} ▸` : `fork + merge request (${defAnswers.length}) ▸`}
				</button>
			{/if}
			<button class="act act--menu" onclick={menu} title="The signed request — JSON, comment, broadcast">menu</button>
			<span class="coord">{coordLabel}</span>
		</div>
	{:else if row.kind === 'mr'}
		{@const mr = row.note?.merge_request}
		<div class="meta">
			<span><span class="meta__k">kind</span>818 · a → {row.parent}</span>
			<span><span class="meta__k">source</span>index′ (derivative, E → {mine ? 'their' : ownsRoot ? 'your' : 'the owner’s'} version)</span>
		</div>
		<div class="xb__txt"><p>{mr?.content ?? row.body}</p></div>
		{#if mrStatus !== 'open'}
			<div class="xb__verdict">
				<span class="g {mrStatus}">{mrStatus === 'merged' ? '+' : '−'}</span>
				{mrStatus}{mine ? ' — the owner reacted on your 818' : ownsRoot ? ' — you reacted on the 818' : ''}
			</div>
		{/if}
		<div class="acts">
			{#if mrStatus === 'open' && ownsRoot && !mine}
				<button class="act act--review" onclick={review} title="Review the diff — the engine computes your next version; merge or decline there">review the diff ▸</button>
			{:else}
				<button class="act act--review" onclick={review} title={mine ? 'read-only: you are the requester' : ownsRoot ? 'Open the review' : 'not yours to merge — addressed to the owner'}>review{mrStatus === 'open' ? ' (read-only)' : ''}</button>
			{/if}
			<button class="act act--menu" onclick={menu} title="Event menu — JSON, comment, broadcast">menu</button>
			<span class="coord">{coordLabel}</span>
		</div>
	{:else if row.kind === 'kasten' && row.note}
		<div class="xb__txt"><p>A kasten of {row.note.children?.length ?? 0} entries, attached to {row.sec < 0 ? 'the whole publication' : `§${sections[row.sec]?.n ?? ''}`}.</p></div>
		<div class="acts">
			<button class="act act--open" onclick={() => onopen(row)} title="Open the kasten as a publication">open kasten ▸</button>
			{#if row.request}
				<button class="act act--request" onclick={() => onreveal(row.request!)} title="Written for this request">↳ answers {requestTitle ?? 'a request'}</button>
			{/if}
			<button class="act act--menu" onclick={menu} title="Event menu — JSON, comment, broadcast">menu</button>
			<span class="coord">{coordLabel}</span>
		</div>
	{:else if row.note}
		{@const ev = row.note.event}
		<div class="xb__txt">
			<RichContent content={ev.content} muted />
			<EventLinks tags={ev.tags} compact />
		</div>
		<div class="meta">
			{#if row.kind === 'def'}
				<span><span class="meta__k">term</span><span class="chip">{row.term ?? row.title}</span></span>
				<span><span class="meta__k">{row.glossary ? 'in glossary' : 'not in glossary'}</span></span>
			{:else}
				<span><span class="meta__k">placement</span>{row.placement === 'annotation' ? 'attached' : row.placement}</span>
			{/if}
			{#if styleLabel}<span><span class="meta__k">style</span>✎ {styleLabel}</span>{/if}
		</div>
		<div class="acts">
			{#if row.kind === 'def'}
				<button class="act act--relink" onclick={() => relink([row])} disabled={!canSign} title={ownsRoot ? 'Wrap the first occurrence of the term in each section as a {{ref}} to this definition — diff first, then one armed sign' : 'Not your publication: a derivative with the refs in, plus a kind 818 to the owner'}>{ownsRoot ? 'relink' : 'fork + merge request'}</button>
				<button
					class="act act--glossary"
					class:on={glossOpen}
					onclick={() => openGlossary(row.glossary ? 'remove' : 'add')}
					disabled={!canSign}
					title={row.glossary ? 'Take this definition out of your glossary bundle (the definition itself is untouched)' : ownsRoot ? 'Optional, per event: list this definition in your glossary bundle and republish your index with the glossary in' : 'Optional, per event: list this definition in your glossary bundle for this publication (the glossary stays under your key)'}
				>{row.glossary ? 'remove from glossary' : 'promote → glossary'}</button>
			{:else}
				<button class="act act--promote" onclick={() => openCoordinate(row.id)} disabled={!canSign} title="Fold this note into the publication — from the card in the reader, or the event menu">promote</button>
			{/if}
			{#if row.request}
				<button class="act act--request" onclick={() => onreveal(row.request!)} title="Written for this request">↳ answers {requestTitle ?? 'a request'}</button>
			{/if}
			{#if row.replyTo}
				<button class="act act--reply-of" onclick={() => onreveal(row.replyTo!)} title="Written in reply to this annotation">↩ in reply</button>
			{/if}
			<button class="act act--open" onclick={() => onopen(row)} title="Open this section in its own reader buffer">open ▸</button>
			<button class="act act--menu" onclick={menu} title="Event menu — JSON, comment, broadcast, pool">menu</button>
			<span class="coord">{coordLabel}</span>
		</div>
		{#if glossOpen}
			<div class="xb__promote" data-glossary={glossMode}>
				{#if glossPreviewing}
					<div class="xb__line">computing the glossary…</div>
				{:else if glossErr}
					<div class="xb__line xb__line--bad">{glossErr}</div>
				{:else if glossPreview}
					{@const n = glossPreview.promoted.length}
					<div class="xb__line"><b>{glossMode === 'add' ? `List “${row.term ?? row.title}” in your glossary` : `Take “${row.term ?? row.title}” out of your glossary`}</b> · <code>30040 · d:{tagValue(glossPreview.glossary, 'd') ?? '?'}</code> · {n} definition{n === 1 ? '' : 's'} after this{glossPreview.index ? ' · a new version of your index (same d)' : glossPreview.owner ? ' · your index already lists the glossary' : ' · the index is not yours; the glossary stays under your key'}</div>
					{#if glossPreview.note}<div class="xb__line">{glossPreview.note}</div>{/if}
					<details class="xb__wire">
						<summary>event preview — what “sign” will sign</summary>
						<pre class="xb__wire-pre">{wireText(glossPreview)}</pre>
					</details>
					<div class="acts">
						<button class="act" onclick={() => (glossOpen = false)} disabled={glossPublishing}>cancel</button>
						<button class="act act--go" onclick={publishGlossaryChange} disabled={glossPublishing}>{glossPublishing ? 'signing…' : glossMode === 'add' ? 'sign · promote' : 'sign · remove'}</button>
					</div>
				{/if}
			</div>
		{/if}
	{/if}
	<AnnotationThread
		{root}
		{rootTitle}
		coordinate={row.kind === 'mr' ? `818:${row.note?.merge_request?.id ?? row.id.slice(4)}` : row.id}
		author={row.author}
		title={row.title}
		count={row.replies}
		parent={row.parent}
		{parentTitle}
		onreplied={onchanged}
	/>
</div>

<style>
	.xb {
		border-bottom: 1px solid var(--panel-border);
		background: var(--base1);
		padding: 8px 12px 10px 49px;
		border-left: 3px solid var(--au);
		font: var(--t-xs) / 1.45 var(--font-sans);
	}
	:global(.abuf.phone) .xb { padding-left: 12px; }
	:global(.detail) .xb { border-bottom: 0; background: transparent; padding: 10px 12px 12px; }
	.xb__txt { font: var(--t-md) / 1.5 var(--font-serif); color: var(--fg); max-width: 62ch; }
	.xb__txt p { margin: 0 0 6px; }
	.meta { display: flex; flex-wrap: wrap; gap: 4px 10px; margin: 2px 0 6px; font: var(--t-2xs) / 1.4 var(--font-mono); color: var(--base6); }
	.meta__k {
		color: var(--base5);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		font-size: var(--t-3xs);
		font-weight: 600;
		margin-right: 3px;
	}
	.meta__muted { color: var(--base5); font-style: italic; }
	.chip {
		display: inline-block;
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--green);
		border: 1px solid rgba(180, 190, 130, 0.5);
		border-radius: 999px;
		padding: 0 8px;
	}
	.xb__ctx {
		margin: 4px 0 6px;
		padding: 2px 8px;
		border-left: 2px solid var(--panel-border-strong);
		color: var(--fg-alt);
		font: italic var(--t-base) / 1.45 var(--font-serif);
	}
	.xb__answers { margin-top: 6px; font: var(--t-2xs) / 1.4 var(--font-mono); color: var(--base5); }
	.xb__link { background: transparent; border: 0; padding: 0; color: var(--base6); font: inherit; cursor: pointer; }
	.xb__link:hover { color: var(--fg); }
	.xb__verdict { margin-top: 4px; font: var(--t-2xs) / 1.4 var(--font-mono); color: var(--base6); }
	.g { font-weight: 700; }
	.g.note { color: var(--base6); }
	.g.def { color: var(--green); }
	.g.req { color: var(--ask); }
	.g.ok { color: var(--blue); }
	.g.mr { color: var(--ask); }
	.g.kasten { color: var(--violet); }
	.g.merged { color: var(--green); }
	.g.declined { color: var(--red); }
	.acts { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; align-items: center; }
	.act {
		font: var(--t-3xs) / 1.2 var(--font-mono);
		min-height: 22px;
		padding: 1px 8px;
		color: var(--base6);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		background: transparent;
		cursor: pointer;
	}
	.act:hover { border-color: var(--base5); color: var(--fg); }
	.act:disabled { opacity: 0.45; cursor: default; }
	.act--answer,
	.act--review { color: var(--ask); }
	.act--relink,
	.act--glossary,
	.act--go { color: var(--green); }
	.act--glossary.on { border-color: var(--green); }
	.act--promote,
	.act--open { color: var(--yellow); }
	.act--demo { color: var(--base5); border-style: dashed; }
	.coord { margin-left: auto; font: var(--t-3xs) / 1.2 var(--font-mono); color: var(--base4); word-break: break-all; }
	.xb__promote { margin-top: 8px; border: 1px dashed var(--panel-border-strong); border-radius: var(--r-md); padding: 6px 10px; }
	.xb__line { font: var(--t-2xs) / 1.45 var(--font-mono); color: var(--base6); }
	.xb__line b { color: var(--fg); }
	.xb__line--bad { color: var(--red); }
	.xb__wire summary {
		cursor: pointer;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-top: 4px;
	}
	.xb__wire-pre { font: var(--t-2xs) / 1.5 var(--font-mono); color: var(--base7); white-space: pre-wrap; word-break: break-word; margin: 4px 0 0; }
</style>
