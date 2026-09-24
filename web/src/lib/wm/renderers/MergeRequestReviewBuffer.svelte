<script lang="ts">
	// Merge-request review — one work buffer per kind-818 (`review:<id>`).
	// The engine pairs the source with the base and computes the OWNER'S
	// NEXT VERSIONS (docs/zettel/decision-merge-requests-818.org); this
	// buffer renders that diff and offers the two verdicts. Merge = sign the
	// next versions under the owner's own `d`, react `+` on the 818,
	// broadcast (the separate, Confirm-gated step); decline = react `-`
	// with an optional reason. A requester sees the same diff read-only.
	// No Svelte lifecycle hooks (buffer renderers unmount on every switch).
	import type { Buffer } from '../types';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { notifications } from '$lib/notifications.svelte';
	import { promptText } from '$lib/wm/text-prompt.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import DiffBlock from '$lib/components/DiffBlock.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const app = getAppState();
	const mrId = $derived(buffer.id.startsWith('review:') ? buffer.id.slice('review:'.length) : buffer.id);

	let diff = $state<api.MergeRequestDiff | null>(null);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let wireOpen = $state(false);
	let armed = $state<'merge' | 'decline' | null>(null);
	let working = $state(false);
	// Plain bookkeeping (never rendered) — keeps the loader out of $state so
	// the mount effect cannot re-fire on its own writes.
	const book = { seq: 0 };

	async function load() {
		const id = mrId;
		const mySeq = ++book.seq;
		loading = true;
		error = null;
		try {
			const d = await api.getMergeRequestDiff(id);
			if (mySeq !== book.seq) return;
			diff = d;
		} catch (e) {
			if (mySeq !== book.seq) return;
			error = e instanceof Error ? e.message : String(e);
		} finally {
			if (mySeq === book.seq) loading = false;
		}
	}
	$effect(() => {
		void mrId;
		void load();
	});

	const mr = $derived(diff?.merge_request ?? null);
	const status = $derived(diff?.status ?? mr?.status ?? 'open');
	const isOwner = $derived(diff?.role === 'owner' || diff?.role === 'both');
	const open = $derived(status === 'open');
	const canAct = $derived(isOwner && open && !working);
	const from = $derived(mr ? getAuthorDisplayName(mr.pubkey) : '');
	const rootCoord = $derived.by(() => {
		const t = mr?.target ?? '';
		return t.startsWith('30040:') ? t : (diff?.base.coordinate ?? t);
	});
	const targetLabel = $derived(dOf(mr?.target ?? ''));
	const addedGlossary = $derived.by(() => {
		if (!diff?.index) return null;
		const g = diff.index.diff.added.find((a) => isGlossaryCoord(a.coordinate));
		return g ?? null;
	});
	const promotedTerms = $derived((diff?.definitions ?? []).filter((d) => d.promoted_in_source).map((d) => d.term));
	const nextCount = $derived((diff?.sections.length ?? 0) + (diff?.index ? 1 : 0));

	function dOf(coord: string): string {
		return coord.split(':').slice(2).join(':');
	}
	// `{/…/}` in markup reads as a block close to the Svelte parser — keep
	// regexes in script.
	const isGlossaryCoord = (coord: string) => /-glossary$/.test(dOf(coord));
	function relTime(ts: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000 - ts));
		if (s < 60) return 'now';
		if (s < 3600) return `${Math.floor(s / 60)}m`;
		if (s < 86400) return `${Math.floor(s / 3600)}h`;
		if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
		return new Date(ts * 1000).toLocaleDateString();
	}
	function short(id: string | null | undefined, n = 8): string {
		return id ? id.slice(0, n) + '…' : '—';
	}
	function shortTag(t: string[]): string {
		return t.map((v) => JSON.stringify(v.length > 72 ? v.slice(0, 72) + '…' : v)).join(', ');
	}
	function wireText(d: api.MergeRequestDiff): string {
		const rows = (t: api.UnsignedTemplate) => t.tags.map((x) => '  [' + shortTag(x) + ']').join('\n');
		const parts: string[] = [];
		for (const s of d.sections) parts.push(`§ ${s.title || dOf(s.original)} · kind ${s.next.kind} · same d, your next version\n` + rows(s.next));
		if (d.index) parts.push(`index · kind ${d.index.next.kind} · same d, your next version\n` + rows(d.index.next));
		return parts.join('\n\n') || '(nothing to sign)';
	}
	function verdictReason(): string | null {
		const c = (mr?.verdict?.content ?? '').trim();
		return c.length > 1 ? c.slice(1).trim() : null;
	}

	function afterVerdict(next: api.MergeStatus) {
		if (diff) diff = { ...diff, status: next };
		void notifications.refresh('open');
		app.bumpInsertions?.();
		if (rootCoord) app.markRelinkDone(rootCoord);
	}
	async function merge() {
		if (!canAct || !diff) return;
		if (armed !== 'merge') {
			armed = 'merge';
			return;
		}
		working = true;
		try {
			const r = await api.mergeMergeRequest(mrId, { broadcast: false });
			app.pushToast(`Merged — signed ${r.sections.length} section${r.sections.length === 1 ? '' : 's'}${r.index ? ' + index' : ''} under your d · reacted + on the 818 (broadcast is the separate step)`, 'success', 4000);
			afterVerdict('merged');
		} catch (e) {
			app.pushToast(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			working = false;
			armed = null;
		}
	}
	async function decline() {
		if (!canAct) return;
		if (armed !== 'decline') {
			armed = 'decline';
			return;
		}
		const reason = await promptText({ title: 'decline — a reason for the requester?', placeholder: 'optional' });
		if (reason === null) {
			armed = null;
			return;
		}
		working = true;
		try {
			await api.declineMergeRequest(mrId, { reason: reason.trim() || undefined, broadcast: false });
			app.pushToast('Declined — reacted − on the 818. Their derivative stays as their fork.', 'info', 3500);
			afterVerdict('declined');
		} catch (e) {
			app.pushToast(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			working = false;
			armed = null;
		}
	}
	function openTarget() {
		const parts = rootCoord.split(':');
		if (parts.length >= 3 && parts[0] === '30040') void app.openPublication(parts[1], parts.slice(2).join(':'));
	}
</script>

<div class="mr" data-mr={mrId} data-status={status} data-role={diff?.role ?? ''}>
	{#if loading && !diff}
		<div class="mr__note">computing the diff…</div>
	{:else if error && !diff}
		<div class="mr__note mr__note--bad">{error}</div>
	{:else if diff && mr}
		<div class="mr__scroll">
			<div class="mr__hdr" style="--au:{pubkeyToColor(mr.pubkey)}">
				<span class="mr__badge" class:mr__badge--ask={open} class:mr__badge--ok={status === 'merged'} class:mr__badge--bad={status === 'declined'}>
					{open ? '? merge request' : status === 'merged' ? '✓ merge request · merged' : '− merge request · declined'}
				</span>
				<b class="mr__from"><span class="mr__dot"></span>from {from}</b>
				<span class="mr__meta">{relTime(mr.created_at)} ago</span>
				<span class="mr__spacer"></span>
				<button class="mr__link" onclick={openTarget} title="Open the target publication">target <code>{targetLabel}</code></button>
				<span class="mr__meta mr__base" class:mr__base--ok={!diff.base.moved_on} class:mr__base--warn={diff.base.moved_on}>
					base {short(diff.base.base_id)} · your current {short(diff.base.current_id)}
					{#if diff.base.moved_on}· (the target moved on since){:else}· (applies cleanly){/if}
				</span>
			</div>
			{#if diff.note}
				<div class="mr__note" class:mr__note--warn={diff.base.moved_on}>{diff.note}</div>
			{/if}

			<div class="mr__ask">{mr.content || 'merge request'}</div>
			{#if !open && verdictReason()}
				<div class="mr__note">your reason: “{verdictReason()}”</div>
			{/if}
			<div class="mr__help">the engine paired source with base and computed <b>your next version</b> — that is what the diff shows and what the button signs.</div>

			<div class="mr__block" data-block="index">
				<div class="mr__block-head"><b>index</b><span>30040 · {diff.index ? 'same d, new version' : 'unchanged'}</span></div>
				{#if diff.index}
					{#each diff.index.diff.added as a (a.coordinate)}
						<div class="mr__struct"><span class="mr__plus">+ a → {isGlossaryCoord(a.coordinate) ? 'Glossary' : dOf(a.coordinate)}</span><span>slot {a.slot}{addedGlossary && a.coordinate === addedGlossary.coordinate && promotedTerms.length ? ` · ${promotedTerms.length} definition${promotedTerms.length === 1 ? '' : 's'}: ${promotedTerms.join(', ')}` : ''}</span></div>
					{/each}
					{#each diff.index.diff.replaced as r (r.original)}
						<div class="mr__struct"><span class="mr__plus">replaced</span><span>{dOf(r.original)} ← derivative {dOf(r.derivative)} (its content becomes your next version, same d)</span></div>
					{/each}
					{#each diff.index.diff.moved as m (m.coordinate)}
						<div class="mr__struct"><span class="mr__plus">moved</span><span>{dOf(m.coordinate)} · {m.from} → {m.to}</span></div>
					{/each}
					{#if diff.index.diff.removed.length}
						<div class="mr__struct"><span class="mr__minus">− removed</span><span>{diff.index.diff.removed.map(dOf).join(', ')}</span></div>
					{/if}
					{#if diff.index.diff.unchanged.length}
						<div class="mr__struct"><span class="mr__same">· unchanged</span><span>{diff.index.diff.unchanged.map(dOf).join(', ')}</span></div>
					{/if}
				{:else}
					<div class="mr__struct"><span class="mr__same">· the index needs no new version</span></div>
				{/if}
			</div>

			{#each diff.sections as s (s.original)}
				<DiffBlock title={s.title || dOf(s.original)} sub={`30041 · d:${dOf(s.original)} · same d, your next version`} hunks={s.hunks} tagsAdded={s.tags_added} tagsRemoved={s.tags_removed} coordinate={s.original} />
			{/each}
			{#if !diff.sections.length}
				<div class="mr__note">no section changes — the index alone.</div>
			{/if}

			{#if addedGlossary}
				<div class="mr__struct mr__struct--gl"><span class="mr__plus">glossary (new)</span><span>{promotedTerms.length ? promotedTerms.join(' · ') : 'the requester’s glossary bundle'} — their definition events, referenced by coordinate; they stay under their key</span></div>
			{/if}

			<details class="mr__wire" bind:open={wireOpen}>
				<summary class="mr__label">event preview <span class="mr__label-sub">· what “merge +” signs — {nextCount} next version{nextCount === 1 ? '' : 's'} under your d</span></summary>
				<pre class="mr__wire-pre">{wireText(diff)}</pre>
			</details>

			<div class="mr-agent" data-slot="assistant-review">
				<div class="mr-agent__h">assistant review <span class="mr__spacer"></span><button class="mr__btn mr__btn--mini" disabled title="your own agent writes it: “check tendrl” in your harness runs list_asks → get_ask_diff → review_ask; it reads this same diff and cannot merge">ask the assistant</button></div>
				{#if diff?.review}
					<ul class="mr-agent__findings">
						{#each diff.review.findings as f}<li>{f}</li>{/each}
					</ul>
					<div class="mr-agent__rec" data-recommendation={diff.review.recommendation}>↳ recommend: {diff.review.recommendation}{#if diff.review.note} — {diff.review.note}{/if}</div>
				{:else}
					<div class="mr__note">optional — a structured review kept as a local record, never an event. None yet.</div>
				{/if}
			</div>

			{#if isOwner}
				<div class="mr__foot" data-open={open ? 'true' : 'false'}>
					{#if open}
						<button class="mr__btn mr__btn--decline" class:mr__btn--armed-bad={armed === 'decline'} onclick={decline} disabled={!canAct}>{armed === 'decline' ? 'sure? react − with a reason' : 'decline −'}</button>
						<button class="mr__btn" disabled title="later — edit the merged result before signing">open merged result in composer</button>
						<span class="mr__spacer"></span>
						<button class="mr__btn mr__btn--merge" class:mr__btn--armed={armed === 'merge'} onclick={merge} disabled={!canAct}>{working ? 'signing…' : armed === 'merge' ? `sure? sign ${nextCount} next version${nextCount === 1 ? '' : 's'} · react + · broadcast` : 'merge + ▸'}</button>
					{:else}
						<span class="mr__badge" class:mr__badge--ok={status === 'merged'} class:mr__badge--bad={status === 'declined'}>{status === 'merged' ? '✓ merged' : '− declined'}</span>
						<span class="mr__meta">{status === 'merged' ? 'your next versions are signed and local until broadcast' : 'their derivative stays as their fork'}</span>
					{/if}
				</div>
				{#if open}
					<div class="mr__steps">merge = sign {diff.sections.length} section{diff.sections.length === 1 ? '' : 's'}{diff.index ? ' + index' : ''} (same d, your key) → kind 7 “+” on the 818 → broadcast (Confirm-gated, the separate step) · the definitions stay the requester’s events, referenced by coordinate</div>
				{/if}
			{:else}
				<div class="mr__foot" data-open="false">
					<span class="mr__badge" class:mr__badge--ask={open} class:mr__badge--ok={status === 'merged'} class:mr__badge--bad={status === 'declined'}>{open ? 'open — waiting for the owner' : status === 'merged' ? '✓ merged' : '− declined'}</span>
					<span class="mr__meta">read-only — you sent this</span>
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.mr {
		display: flex;
		flex-direction: column;
		min-height: 0;
		height: 100%;
		background: var(--panel-bg);
		color: var(--fg);
	}
	.mr__scroll {
		flex: 1;
		overflow: auto;
		padding: var(--s-3) var(--s-3) var(--s-10);
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		font: var(--t-xs) / 1.45 var(--font-sans);
	}
	.mr__hdr {
		display: flex;
		gap: var(--s-2);
		align-items: baseline;
		flex-wrap: wrap;
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--base6);
	}
	.mr__from {
		color: var(--fg);
		font: 600 var(--t-sm) / 1.3 var(--font-sans);
		display: inline-flex;
		align-items: center;
		gap: 5px;
	}
	.mr__dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au, var(--yellow));
		flex: none;
	}
	.mr__meta {
		color: var(--base6);
	}
	.mr__base--ok {
		color: var(--green);
	}
	.mr__base--warn {
		color: var(--yellow);
	}
	.mr__spacer {
		flex: 1;
	}
	.mr__link {
		font: var(--t-2xs) / 1.2 var(--font-mono);
		min-height: 22px;
		padding: 0 var(--s-2);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--base6);
		cursor: pointer;
	}
	.mr__link:hover {
		color: var(--fg);
		border-color: var(--base5);
	}
	.mr__badge {
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base6);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 1px 6px;
		white-space: nowrap;
	}
	.mr__badge--ask {
		color: var(--orange);
		border-color: rgba(226, 164, 120, 0.5);
	}
	.mr__badge--ok {
		color: var(--green);
		border-color: rgba(180, 190, 130, 0.5);
	}
	.mr__badge--bad {
		color: var(--red);
		border-color: rgba(226, 120, 120, 0.5);
	}
	.mr__ask {
		font: var(--t-md) / 1.5 var(--font-serif);
		color: var(--fg);
		border-left: 3px solid var(--orange);
		padding: var(--s-1) var(--s-3);
		background: var(--panel-bg-soft);
	}
	.mr__help,
	.mr__note {
		font: var(--t-2xs) / 1.4 var(--font-mono);
		color: var(--base5);
	}
	.mr__help b {
		color: var(--base7);
	}
	.mr__note--bad {
		color: var(--red);
	}
	.mr__note--warn {
		color: var(--yellow);
	}
	.mr__block {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		overflow: hidden;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		line-height: 1.5;
	}
	.mr__block-head {
		background: var(--bg-surface);
		padding: 4px 10px;
		display: flex;
		gap: 10px;
		align-items: baseline;
		color: var(--base6);
		border-bottom: 1px solid var(--panel-border);
	}
	.mr__block-head b {
		color: var(--fg);
	}
	.mr__struct {
		padding: 3px 10px;
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
		border-top: 1px solid var(--panel-border);
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
	}
	.mr__block .mr__struct:first-of-type {
		border-top: 0;
	}
	.mr__struct--gl {
		border: 1px dashed var(--panel-border);
		border-radius: 3px;
	}
	.mr__plus {
		color: var(--green);
	}
	.mr__minus {
		color: var(--red);
	}
	.mr__same {
		color: var(--base5);
	}
	.mr__label {
		cursor: pointer;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
	}
	.mr__label-sub {
		text-transform: none;
		letter-spacing: 0;
		font-weight: 400;
	}
	.mr__wire-pre {
		margin: 4px 0 0;
		font: calc(var(--t-xs) - 1px) / 1.5 var(--font-mono);
		color: var(--base7);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 6px 10px;
		max-height: 260px;
		overflow: auto;
		white-space: pre-wrap;
		word-break: break-word;
	}
	.mr-agent {
		border: 1px dashed var(--panel-border-strong);
		border-radius: 3px;
		padding: var(--s-2) var(--s-3);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.mr-agent__findings {
		margin: 2px 0 0 var(--s-4);
		padding: 0;
		font: var(--t-xs) / 1.45 var(--font-sans);
		color: var(--fg);
	}
	.mr-agent__rec {
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--green);
	}
	.mr-agent__h {
		display: flex;
		gap: var(--s-2);
		align-items: center;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
	}
	.mr__foot {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		flex-wrap: wrap;
		margin-top: var(--s-1);
	}
	.mr__steps {
		font: var(--t-3xs) / 1.5 var(--font-mono);
		color: var(--base5);
	}
	.mr__btn {
		font: var(--t-xs) / 1.2 var(--font-mono);
		min-height: 30px;
		padding: 3px 10px;
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.mr__btn:hover:not(:disabled) {
		border-color: var(--base5);
	}
	.mr__btn:disabled {
		opacity: 0.45;
		cursor: default;
	}
	.mr__btn--mini {
		min-height: 22px;
		padding: 0 var(--s-2);
		font-size: var(--t-3xs);
		letter-spacing: 0;
		text-transform: none;
	}
	.mr__btn--merge {
		color: var(--green);
	}
	.mr__btn--armed {
		background: var(--green);
		color: var(--bg);
		border-color: var(--green);
	}
	.mr__btn--decline {
		color: var(--red);
	}
	.mr__btn--armed-bad {
		background: var(--red);
		color: var(--bg);
		border-color: var(--red);
	}
</style>
