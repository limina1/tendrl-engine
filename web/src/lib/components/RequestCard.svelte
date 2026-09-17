<script lang="ts">
	// A request ("ask for notes") shown on the page it was asked about — the
	// question sitting where the answer will land. Unsigned and signed alike
	// (a signed one also appears as a comment in the section's thread).
	// Open: the ask, its scope hints, the excerpt. Answered: the summary and
	// the written notes, one click each.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import RichContent from './RichContent.svelte';
	import type { ResolvedRef, ParsedToken } from '$lib/nostr/nostrdown';

	let {
		request,
		onchanged = undefined
	}: {
		request: api.AnnotationRequest;
		/** Deleted or reopened — the reader re-reads the store. */
		onchanged?: (() => void) | undefined;
	} = $props();

	const app = getAppState();
	const r = $derived(request);
	const answered = $derived(r.status === 'answered');
	let expanded = $state(false);
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
	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
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

<div class="rqc" class:rqc--answered={answered} class:rqc--open={expanded} data-request={r.id}>
	<button class="rqc__head" onclick={() => (expanded = !expanded)} aria-expanded={expanded}>
		<span class="rqc__cap">{answered ? '✓ answered' : '? ask'}</span>
		<span class="rqc__task">{r.title ? `${r.title} — ` : ''}{r.task}</span>
		<span class="rqc__meta">{r.event_id ? 'signed · ' : ''}{relTime(r.created_at)}</span>
		<span class="rqc__ptr" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
	</button>
	{#if expanded}
		<div class="rqc__body">
			{#if r.scope.length > 0}
				<div class="rqc__line"><span class="rqc__k">scope</span> {r.scope.join(' · ')} · {r.output}</div>
			{/if}
			{#if r.style}
				<div class="rqc__line">
					<span class="rqc__k">style</span>
					<button class="rqc__mini" onclick={() => app.handleSearch(`style:${r.style}`, { scopeToMe: false })} title="Everything written in this style — {r.style}">{r.style_title ?? r.style.split(':').slice(2).join(':')}</button>
				</div>
			{/if}
			{#if r.context}
				<blockquote class="rqc__context">{r.context}</blockquote>
			{/if}
			{#if r.answer}
				<div class="rqc__answer">
					<RichContent content={r.answer.summary} refs={ndRefs} tokens={ndTokens} />
				</div>
				{#if r.answer.coordinates.length > 0}
					<div class="rqc__coords">
						{#each r.answer.coordinates as c (c)}
							<button class="rqc__mini" onclick={() => openCoordinate(c)} title={c}>{c.startsWith('30040') ? '⊞' : '§'} {titles[c] ?? c.split(':').slice(2).join(':')}</button>
						{/each}
					</div>
				{/if}
			{:else}
				<div class="rqc__line rqc__line--muted">
					{r.event_id
						? 'Signed as a mention — waiting for the assistant\'s reply.'
						: 'Local record — say "check tendrl requests" in your agent to have it answered.'}
				</div>
			{/if}
			<div class="rqc__actions">
				{#if answered}
					<button class="rqc__mini" onclick={reopen} disabled={busy}>reopen</button>
				{/if}
				<span class="rqc__spacer"></span>
				<button class="rqc__mini" onclick={remove} disabled={busy} title="Delete the local record">delete</button>
			</div>
		</div>
	{/if}
</div>

<style>
	.rqc {
		--rq-accent: var(--id-remote);
		margin: 6px 0;
		border: 1px dashed var(--panel-border);
		border-left: 2px solid var(--rq-accent);
		border-radius: var(--r-sm, 3px);
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		background: transparent;
	}
	.rqc--answered {
		--rq-accent: var(--id-yours);
		border-style: solid;
	}
	.rqc__head {
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
	.rqc__cap {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--rq-accent);
		white-space: nowrap;
	}
	.rqc__task {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--fg-muted);
	}
	.rqc__meta,
	.rqc__ptr {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base5);
		white-space: nowrap;
	}
	.rqc__body {
		padding: 2px 10px 8px;
		border-top: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.rqc__line {
		color: var(--fg-muted);
	}
	.rqc__line--muted {
		color: var(--base5);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
	}
	.rqc__k {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-right: 4px;
	}
	.rqc__context {
		margin: 0;
		padding: 2px 8px;
		border-left: 2px solid var(--panel-border);
		color: var(--fg-muted);
		font-style: italic;
	}
	.rqc__answer {
		color: var(--fg);
	}
	.rqc__coords,
	.rqc__actions {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		align-items: center;
	}
	.rqc__spacer {
		flex: 1;
	}
	.rqc__mini {
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
	.rqc__mini:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
</style>
