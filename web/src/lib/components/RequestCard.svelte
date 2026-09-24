<script lang="ts">
	// A request ("ask for notes") shown on the page it was asked about — the
	// question sitting where the answer will land. Unsigned records and
	// signed requests alike (a signed one is a kind-30041 annotation on the
	// section with a `coordinate`; the notes that answer it point back with
	// `request`, and the engine derives "answered" from those pointers).
	// Open: the ask, its scope hints, the excerpt. Answered: the summary and
	// the written notes, one click each.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import RichContent from './RichContent.svelte';
	import InsertionCard from './InsertionCard.svelte';
	import AnnotationThread from './annotations/AnnotationThread.svelte';
	import type { ResolvedRef, ParsedToken } from '$lib/nostr/nostrdown';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';

	let {
		request,
		onchanged = undefined, answers = [], onopen = undefined, onpromoted = undefined }: {
		request: api.AnnotationRequest;
		/** Deleted or reopened — the reader re-reads the store. */
		onchanged?: (() => void) | undefined;
		/** The notes written for this request: they fold with the card. */
		answers?: api.LoadedInsertion[];
		onopen?: ((ins: api.LoadedInsertion) => void) | undefined;
		onpromoted?: ((resp: api.PromotionResponse) => void) | undefined;
	} = $props();

	const app = getAppState();
	const r = $derived(request);
	const notes = $derived(answers ?? []);
	const answered = $derived(r.status === 'answered');
	/** The definition events written for a vocabulary request, by term. */
	const definitions = $derived(notes.filter((n) => n.role === 'definition'));
	/** A local record (`rq_…`) can be deleted or reopened; a request read
	 *  off its signed event alone (`ev_…`) has no record to change. */
	const isRecord = $derived(r.id.startsWith('rq_'));
	// Whose ask this is: a signed request carries its signer in the
	// coordinate; an unsigned local record is the user's own, coloured with
	// their key when one is known. `--au` is the highlight recipe, the hue
	// the annotations table gives the same author.
	const authorPubkey = $derived.by(() => {
		if (r.coordinate) return r.coordinate.split(':')[1] ?? null;
		return app.identityStatus?.pubkey ?? app.myPubkey ?? null;
	});
	const au = $derived(authorPubkey ? pubkeyToColor(authorPubkey) : 'var(--base5)');
	const mine = $derived(!!app.identityStatus?.pubkey && authorPubkey === app.identityStatus.pubkey);
	const who = $derived(
		!authorPubkey || mine || !r.coordinate ? 'you' : getAuthorDisplayName(authorPubkey)
	);
	// Starts open: the ask and what was written for it read as one; the
	// head folds both away.
	let expanded = $state(true);
	let busy = $state(false);

	// The answer's summary is nostrdown too (the agent cites sections and
	// notes by `{{ref:…}}`); resolve it against the request's root. Titles
	// for the written notes come from the local store, keyed on the `T`
	// tag's display twin, so a card never shows a raw d-tag.
	let ndRefs = $state<ResolvedRef[]>([]);
	let ndTokens = $state<ParsedToken[]>([]);
	let titles = $state<Record<string, string>>({});
	$effect(() => {
		if (!expanded || !r.answer) return;
		const text = r.answer.summary;
		const key = r.id;
		let cancelled = false;
		api.parseNostrdown([{ key, content: text }])
			.then((m) => {
				if (!cancelled) ndTokens = m[key] ?? [];
			})
			.catch(() => {});
		api.resolveNostrdownProgressive(
			[{ key, content: text, publication: r.root, author: r.answer.by }],
			(m) => {
				if (!cancelled) ndRefs = m[key] ?? [];
			},
			{ fetch: false }
		).catch(() => {});
		for (const c of r.answer.coordinates) {
			if (titles[c]) continue;
			const [k, pk, ...d] = c.split(':');
			api.queryEvents([{ kinds: [Number(k)], authors: [pk], '#d': [d.join(':')] }], 'local_only')
				.then((resp) => {
					if (cancelled) return;
					const evs = (resp.events ?? []) as { created_at: number; tags: string[][] }[];
					evs.sort((a, b) => b.created_at - a.created_at);
					const t = evs[0]?.tags.find((x) => x[0] === 'title')?.[1];
					if (t) titles = { ...titles, [c]: t };
				})
				.catch(() => {});
		}
		return () => {
			cancelled = true;
		};
	});

	function relTime(ts: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
		if (s < 60) return 'now';
		if (s < 3600) return `${Math.floor(s / 60)}m`;
		if (s < 86400) return `${Math.floor(s / 3600)}h`;
		if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
		return new Date(ts * 1000).toLocaleDateString();
	}
	/** The relink window: every definition written for this request, with
	 *  the ones already in the glossary pre-checked. */
	function openRelink() {
		if (!definitions.length) return;
		app.relink = {
			root: r.root,
			definitions: definitions.map((d) => d.coordinate),
			promoted: definitions.filter((d) => d.promoted).map((d) => d.coordinate)
		};
	}

	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}
	/** Its row in the ledger (opens or focuses the ledger first). */
	function revealRow() {
		app.requestRevealAnnotation(r.root, r.coordinate ?? r.id);
	}
	/** Answer this request by hand: the annotate window in answer mode —
	 *  one term at a time; an answered vocabulary request still takes
	 *  your definitions. */
	function answerYourself() {
		app.annotationRequest = {
			root: r.root,
			parent: r.parent && r.parent !== r.root ? r.parent : undefined,
			parentTitle: r.parent_title ?? null,
			parentEventId: r.parent_event_id,
			answer: r
		};
	}
	async function remove() {
		busy = true;
		try {
			await api.deleteRequest(r.id);
			app.pushToast('Request deleted', 'success');
			onchanged?.();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Delete failed'), 'error', 4000);
		} finally {
			busy = false;
		}
	}
	async function reopen() {
		busy = true;
		try {
			await api.reopenRequest(r.id);
			onchanged?.();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Reopen failed'), 'error', 4000);
		} finally {
			busy = false;
		}
	}
</script>

<div
	class="rcard"
	class:rcard--answered={answered}
	class:rcard--open={expanded}
	data-request={r.id}
	style="--au: {au}"
>
	<button class="rcard__head" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
		<span class="badge" class:answered>{answered ? '✓ request · answered' : '? request'}</span>
		<span class="rcard__task">{r.title ? `${r.title} — ` : ''}{r.task}</span>
		<span class="rcard__by"><span class="rcard__dot" aria-hidden="true"></span>{who}</span>
		{#if notes.length > 0}
			<span class="rcard__count" title="{notes.length} note(s) written for this request — fold with the card">{notes.length} {notes.length === 1 ? 'note' : 'notes'}</span>
		{/if}
		<span class="rcard__meta">{r.coordinate ? 'signed · ' : ''}{relTime(r.created_at)}</span>
		<span class="rcard__ptr" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
	</button>
	{#if expanded}
		<div class="rcard__body">
			{#if r.scope.length > 0}
				<div class="rcard__line"><span class="rcard__k">scope</span> {r.scope.join(' · ')} · {r.output}</div>
				{#if r.output === 'vocabulary'}
					<div class="rcard__line"><span class="rcard__k">terms</span> {r.terms?.length ? r.terms.join(', ') : 'find them'}</div>
					{#if definitions.length > 0}
						<div class="rcard__line rcard__defs"><span class="rcard__k">defined</span>
							{#each definitions as d (d.coordinate)}
								<button class="rcard__mini rcard__mini--def" onclick={() => openCoordinate(d.coordinate)} title={d.coordinate}>{d.term ?? d.event.tags.find((t) => t[0] === 'title')?.[1] ?? d.coordinate.split(':').slice(2).join(':')}{d.promoted ? ' · in glossary' : ''}</button>
							{/each}
							<button
								class="rcard__mini rcard__mini--go"
								onclick={openRelink}
								title="Wrap the first occurrence of each term in the text as a {'{{ref}}'} to its definition — a diff of the new section versions first, then one armed sign"
							>relink {definitions.length} term{definitions.length === 1 ? '' : 's'} ▸</button>
						</div>
					{/if}
				{/if}
			{/if}
			{#if r.style}
				<div class="rcard__line">
					<span class="rcard__k">style</span>
					<button class="rcard__mini" onclick={() => app.handleSearch(`style:${r.style}`, { scopeToMe: false })} title="Everything written in this style — {r.style}">{r.style_title ?? r.style.split(':').slice(2).join(':')}</button>
				</div>
			{/if}
			{#if r.context}
				<blockquote class="rcard__context">{r.context}</blockquote>
			{/if}
			{#if r.answer}
				<div class="rcard__answer">
					<RichContent content={r.answer.summary} refs={ndRefs} tokens={ndTokens} />
				</div>
				{#if r.answer.coordinates.length > 0}
					<div class="rcard__coords">
						{#each r.answer.coordinates as c (c)}
							<button class="rcard__mini" onclick={() => openCoordinate(c)} title={c}>{c.startsWith('30040') ? '⊞' : '§'} {titles[c] ?? c.split(':').slice(2).join(':')}</button>
						{/each}
					</div>
				{/if}
			{:else}
				<div class="rcard__line rcard__line--muted">
					{r.coordinate
						? 'Signed as an annotation — answered by the first note that points at it.'
						: 'Local record — say "check tendrl requests" in your agent to have it answered.'}
				</div>
			{/if}
			<div class="rcard__actions">
				{#if !answered}
					<button class="rcard__mini rcard__mini--answer" onclick={answerYourself} disabled={busy} title={r.output === 'vocabulary' ? 'Write the definitions yourself — one term at a time, each signed and pointing at this request' : r.output === 'ask' ? 'Answer the question yourself — a signed note pointing at this request' : 'Write the note yourself in the composer, pointing at this request'}>answer it yourself ▸</button>
				{:else if r.output === 'vocabulary'}
					<button class="rcard__mini rcard__mini--answer" onclick={answerYourself} disabled={busy} title="Add or replace a definition yourself — one term at a time, each signed and pointing at this request">define a term yourself ▸</button>
				{/if}
				<button class="rcard__mini rcard__mini--row" onclick={revealRow} title="This request's row in the annotations ledger">≡ row</button>
				{#if answered && isRecord}
					<button class="rcard__mini" onclick={reopen} disabled={busy}>reopen</button>
				{/if}
				{#if r.coordinate}
					<button class="rcard__mini" onclick={() => openCoordinate(r.coordinate!)} title="The signed request — JSON, comment, broadcast, pool">menu</button>
				{/if}
				<span class="rcard__spacer"></span>
				{#if isRecord}
					<button class="rcard__mini" onclick={remove} disabled={busy} title={r.coordinate ? 'Delete the local record — the signed annotation stays' : 'Delete the local record'}>delete</button>
				{/if}
			</div>
			{#if r.coordinate}
				<!-- Replies (kind 1111) threaded under the signed request; an
				     unsigned local record is not an event, so no thread. -->
				<AnnotationThread root={r.root} coordinate={r.coordinate} author={authorPubkey ?? ''} title={r.title ?? r.task} compact count={undefined} onreplied={() => onchanged?.()} />
			{/if}
		</div>
		{#if notes.length > 0}
			<!-- What was written for the ask, folded with it: a request and
			     its answers read as one thing, and hide as one thing. -->
			<div class="rcard__answers" role="group" aria-label="Notes written for this request">
				{#each notes as ins (ins.coordinate)}
					<InsertionCard insertion={ins} {onopen} {onpromoted} />
				{/each}
			</div>
		{/if}
	{/if}
</div>

<style>
	/* A request looks like any other card — author rail, author dot — and
	   the badge says what it is. No dashed borders. */
	.rcard {
		--rq-accent: var(--ask);
		margin: 8px 0 6px;
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--rq-accent));
		border-radius: var(--r-sm, 3px);
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		background: var(--bg-surface);
	}
	.rcard--answered {
		--rq-accent: var(--id-yours);
	}
	.badge {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		font-weight: 600;
		line-height: 1.2;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ask);
		border: 1px solid color-mix(in srgb, var(--ask) 50%, transparent);
		border-radius: 3px;
		padding: 1px 6px;
		white-space: nowrap;
	}
	.badge.answered {
		color: var(--id-yours);
		border-color: color-mix(in srgb, var(--id-yours) 50%, transparent);
	}
	.rcard__by {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base5);
		white-space: nowrap;
		max-width: 14ch;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.rcard__dot {
		display: inline-block;
		flex: none;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au, var(--rq-accent));
	}
	.rcard__head {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		min-height: 34px;
		padding: 4px 10px;
		background: transparent;
		border: none;
		color: var(--fg);
		font: inherit;
		text-align: left;
		cursor: pointer;
	}
	.rcard__task {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--fg-muted);
	}
	.rcard__meta,
	.rcard__ptr {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base5);
		white-space: nowrap;
	}
	.rcard__body {
		padding: 2px 10px 8px;
		border-top: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.rcard__line {
		color: var(--fg-muted);
	}
	.rcard__line--muted {
		color: var(--base5);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
	}
	.rcard__k {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-right: 4px;
	}
	.rcard__context {
		margin: 0;
		padding: 2px 8px;
		border-left: 2px solid var(--panel-border);
		color: var(--fg-muted);
		font-style: italic;
	}
	.rcard__answer {
		color: var(--fg);
	}
	.rcard__coords,
	.rcard__actions {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		align-items: center;
	}
	.rcard__spacer {
		flex: 1;
	}
	.rcard__mini {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base6);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 1px 6px;
		min-height: 22px;
		cursor: pointer;
	}
	.rcard__mini--answer {
		color: var(--ask);
		border-color: color-mix(in srgb, var(--ask) 50%, transparent);
	}
	.rcard__mini:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rcard__count {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--base6);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 0 6px;
		white-space: nowrap;
	}
	.rcard__defs {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		align-items: center;
	}
	.rcard__mini--row {
		color: var(--base5);
	}
	.rcard__mini--def {
		color: var(--green);
		border-color: color-mix(in srgb, var(--green) 50%, transparent);
	}
	.rcard__mini--go {
		color: var(--id-local);
		border-color: color-mix(in srgb, var(--id-local) 45%, transparent);
	}
	.rcard__answers {
		display: flex;
		flex-direction: column;
		gap: var(--s-1);
		padding: 0 var(--s-2) var(--s-2) var(--s-3);
		border-top: 1px dashed var(--panel-border);
		margin-top: 2px;
	}
	.rcard__answers :global(.icard) {
		margin: var(--s-1) 0 0;
	}
</style>
