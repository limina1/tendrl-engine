<script lang="ts">
	// Merge-request review — one work buffer per kind-818 (`review:<id>`),
	// to the ledger contract (docs/annotations-ledger-contract.org, "Review
	// buffer"). The engine pairs the source with the base and computes the
	// OWNER'S NEXT VERSIONS (docs/zettel/decision-merge-requests-818.org);
	// this buffer reads top to bottom: status · the ask · what changes · the
	// definitions being linked · the diff per section · the index · the
	// assistant's review · the thread on the 818 · a sticky footer with the
	// two verdicts (merge = sign the next versions under the owner's own
	// `d`, react `+`, broadcast separately; decline = react `-` with a
	// reason). Once settled the verdict replaces the buttons and the buffer
	// stays open. A requester or reader sees the same page read-only.
	// Opened from the ledger in a split it takes the reader's leaf; "← reader"
	// puts the reader back. No Svelte lifecycle hooks (buffer renderers
	// unmount on every switch).
	import type { Buffer } from '../types';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import { notifications } from '$lib/notifications.svelte';
	import { promptText } from '$lib/wm/text-prompt.svelte';
	import { getAuthorDisplayName, prefetchAuthors } from '$lib/discussions/authors.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import DiffBlock from '$lib/components/DiffBlock.svelte';
	import AnnotationThread from '$lib/components/annotations/AnnotationThread.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const app = getAppState();
	const store = getActiveStore();
	const mrId = $derived(buffer.id.startsWith('review:') ? buffer.id.slice('review:'.length) : buffer.id);

	let diff = $state<api.MergeRequestDiff | null>(null);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let wireOpen = $state(false);
	let armed = $state<'merge' | 'decline' | null>(null);
	let working = $state(false);
	// The definitions being linked, enriched from the root's loaded
	// insertions (author, age, first sentence) — the diff carries only
	// coordinate/term/promoted.
	let defRows = $state<Map<string, api.LoadedInsertion>>(new Map());
	// Plain bookkeeping (never rendered) — keeps the loaders out of $state so
	// the mount effect cannot re-fire on their own writes.
	const book = { seq: 0, defsFor: '' };

	async function load() {
		const id = mrId;
		const mySeq = ++book.seq;
		loading = true;
		error = null;
		try {
			const d = await api.getMergeRequestDiff(id);
			if (mySeq !== book.seq) return;
			diff = d;
			prefetchAuthors([d.merge_request.pubkey, d.merge_request.owner]);
			void loadDefs(d);
		} catch (e) {
			if (mySeq !== book.seq) return;
			error = e instanceof Error ? e.message : String(e);
		} finally {
			if (mySeq === book.seq) loading = false;
		}
	}
	async function loadDefs(d: api.MergeRequestDiff) {
		const root = rootOf(d);
		if (!root || !d.definitions.length || book.defsFor === root) return;
		book.defsFor = root;
		try {
			const resp = await api.loadInsertions({ root, policy: 'local_only' });
			const m = new Map<string, api.LoadedInsertion>();
			for (const ins of resp.insertions) m.set(ins.coordinate, ins);
			defRows = m;
			prefetchAuthors([...new Set(resp.insertions.map((i) => i.event.pubkey))]);
		} catch {
			// The cards fall back to the term alone.
		}
	}
	$effect(() => {
		void mrId;
		void load();
	});

	const mr = $derived(diff?.merge_request ?? null);
	const status = $derived(diff?.status ?? mr?.status ?? 'open');
	const isOwner = $derived(diff?.role === 'owner' || diff?.role === 'both');
	const isRequester = $derived(diff?.role === 'requester' || diff?.role === 'both');
	const open = $derived(status === 'open');
	const canAct = $derived(isOwner && open && !working);
	const from = $derived(mr ? getAuthorDisplayName(mr.pubkey) : '');
	const ownerName = $derived(mr ? getAuthorDisplayName(mr.owner) : '');
	function rootOf(d: api.MergeRequestDiff): string {
		const t = d.merge_request.target ?? '';
		return t.startsWith('30040:') ? t : (d.base.coordinate ?? t);
	}
	const rootCoord = $derived(diff ? rootOf(diff) : '');
	const targetLabel = $derived(dOf(mr?.target ?? ''));
	const addedGlossary = $derived.by(() => {
		if (!diff?.index) return null;
		return diff.index.diff.added.find((a) => isGlossaryCoord(a.coordinate)) ?? null;
	});
	const promoted = $derived((diff?.definitions ?? []).filter((d) => d.promoted_in_source));
	const nextCount = $derived((diff?.sections.length ?? 0) + (diff?.index ? 1 : 0));
	const hunkCount = $derived((diff?.sections ?? []).reduce((n, s) => n + s.hunks.length, 0));
	const indexAdded = $derived(diff?.index?.diff.added.length ?? 0);
	const indexRemoved = $derived((diff?.index?.diff.removed.length ?? 0) + (diff?.index?.diff.moved.length ?? 0));
	const untouched = $derived(diff?.index?.diff.unchanged ?? []);
	const touchedTitles = $derived((diff?.sections ?? []).map((s) => s.title || dOf(s.original)));
	const defsAuthor = $derived.by(() => {
		const first = diff?.definitions[0];
		const row = first ? defRows.get(first.coordinate) : undefined;
		return row ? getAuthorDisplayName(row.event.pubkey) : from;
	});
	// The sibling ledger in this leaf's split, when the review took the
	// reader's place — "← reader" restores `reader:<root>` there.
	const siblingLedger = $derived.by(() => {
		const pos = store.focusedSlot;
		const tree = store.treeFor(pos);
		if (!tree) return null;
		type Node = { type: 'leaf'; buffer: Buffer } | { type: 'split'; children: Node[] };
		const find = (n: Node, prefix: number[]): { path: number[]; ledger: Buffer } | null => {
			if (n.type === 'leaf') return null;
			const me = n.children.findIndex((c) => c.type === 'leaf' && c.buffer.id === buffer.id);
			if (me >= 0) {
				const led = n.children.find((c) => c.type === 'leaf' && c.buffer.id.startsWith('annotations:')) as { type: 'leaf'; buffer: Buffer } | undefined;
				return led ? { path: [...prefix, me], ledger: led.buffer } : null;
			}
			for (let i = 0; i < n.children.length; i++) {
				const r = find(n.children[i], [...prefix, i]);
				if (r) return r;
			}
			return null;
		};
		return find(tree as Node, []);
	});
	function back() {
		const sib = siblingLedger;
		if (sib) {
			const root = sib.ledger.id.slice('annotations:'.length);
			const pos = store.focusedSlot;
			store.focusedLeafPath = { ...store.focusedLeafPath, [pos]: sib.path };
			const known = store.openBuffers.find((b) => b.buffer.id === `reader:${root}`)?.buffer;
			store.setLeaf(pos, known ?? { id: `reader:${root}`, kind: 'reader', label: 'reader', kicker: sib.ledger.kicker ?? dOf(root) });
			return;
		}
		if (rootCoord) app.requestRevealAnnotation(rootCoord, `818:${mrId}`);
	}

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
	function firstSentence(text: string, max = 150): string {
		const t = text.replace(/\s+/g, ' ').trim();
		const m = t.match(/^(.+?[.!?])(\s|$)/);
		const s = m ? m[1] : t;
		return s.length > max ? s.slice(0, max).replace(/\s+\S*$/, '') + '…' : s;
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
	const aTagCount = (s: api.MergeRequestDiffSection) => s.tags_added.filter((t) => t[0] === 'a').length;

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
	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		void app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}
</script>

<div class="mr" data-mr={mrId} data-status={status} data-role={diff?.role ?? ''}>
	<div class="rv__bar">
		<span class="rv__bar-t">⑂ 818{#if mr} · {dOf(rootCoord) || targetLabel}{/if}</span>
		<span class="rv__spacer"></span>
		<button class="pane-head__back" onclick={back} title={siblingLedger ? 'Put the reader back in this window' : 'Show this merge request in the ledger'}>{siblingLedger ? '← reader' : '← ledger'}</button>
	</div>
	{#if loading && !diff}
		<div class="rv__note">computing the diff…</div>
	{:else if error && !diff}
		<div class="rv__note rv__note--bad">{error}</div>
	{:else if diff && mr}
		<div class="rv">
			<div class="rv__status">
				<span class="mr__badge" class:mr__badge--ask={open} class:mr__badge--ok={status === 'merged'} class:mr__badge--bad={status === 'declined'}>⑂ merge · {status}</span>
				<span><span class="rv__k">role</span>{diff.role === 'both' ? 'owner · requester' : diff.role}</span>
				<button class="rv__link" onclick={openTarget} title="Open the target publication"><span class="rv__k">target</span><code>{targetLabel}</code></button>
				<span class="mr__base" class:mr__base--ok={!diff.base.moved_on && open} class:mr__base--warn={diff.base.moved_on && open}>
					<span class="rv__k">base</span>{short(diff.base.base_id)} · {isOwner ? 'your' : 'their'} current {short(diff.base.current_id)} ·
					{#if status === 'merged'}<span class="rv__ok">merged as the next version</span>{:else if status === 'declined'}<span class="rv__muted">declined · nothing signed</span>{:else if diff.base.moved_on}<span class="rv__warn">the target moved on — rebased onto the current version</span>{:else}<span class="rv__ok">applies cleanly</span>{/if}
				</span>
				{#if mr.target_relay_hint}<span><span class="rv__k">hint</span>{mr.target_relay_hint}</span>{/if}
			</div>
			{#if diff.note}
				<div class="rv__note" class:rv__note--warn={diff.base.moved_on}>{diff.note}</div>
			{/if}

			<div class="rv__ask" style="--au:{pubkeyToColor(mr.pubkey)}">
				<span class="rv__ask-rail"></span>
				<div class="rv__ask-body">
					<div class="rv__ask-who"><span class="rv__dot"></span>{from} · {relTime(mr.created_at)}</div>
					<div class="rv__ask-txt">{mr.content || 'merge request'}</div>
					<div class="rv__ask-who rv__ask-src">source · derivative <code>{short(diff.source.id)}</code> (E → {isOwner ? 'your' : 'the owner’s'} base) · the definitions stay {defsAuthor}’s events, referenced by coordinate</div>
					{#if !open && verdictReason()}
						<div class="rv__ask-who">{isOwner ? 'your' : 'the owner’s'} reason: “{verdictReason()}”</div>
					{/if}
				</div>
			</div>

			<h4 class="rv__h">what changes <span class="rv__hn">· the engine paired source with base and computed {isOwner ? 'your' : 'the owner’s'} {status === 'merged' ? 'next version, now signed' : 'next version; that is what the diff shows and what the button signs'}</span></h4>
			<div class="rv__sum">
				<span class="tile" class:plus={diff.sections.length > 0} class:same={diff.sections.length === 0} data-tile="sections"><b>{diff.sections.length}</b>section{diff.sections.length === 1 ? '' : 's'} relinked{touchedTitles.length ? ` · ${touchedTitles.join(', ')}` : ''}</span>
				<span class="tile" class:plus={hunkCount > 0} class:same={hunkCount === 0} data-tile="hunks"><b>{hunkCount}</b>hunk{hunkCount === 1 ? '' : 's'} · first occurrence per term</span>
				<span class="tile" class:plus={indexAdded > 0} class:same={indexAdded === 0} data-tile="index"><b>{indexAdded > 0 ? `+${indexAdded}` : '0'}</b>index change{indexAdded === 1 ? '' : 's'}{addedGlossary ? ` · glossary (${promoted.length}) in slot ${addedGlossary.slot}` : diff.index ? '' : ' · the index needs no new version'}</span>
				<span class="tile" class:plus={indexRemoved > 0} class:same={indexRemoved === 0} data-tile="removed"><b>{indexRemoved}</b>section{indexRemoved === 1 ? '' : 's'} removed or moved</span>
				<span class="tile same" data-tile="untouched"><b>{untouched.length}</b>untouched{untouched.length ? ` · ${untouched.map(dOf).join(', ')}` : ''}</span>
			</div>

			{#if diff.definitions.length}
				<h4 class="rv__h">the definitions being linked <span class="rv__hn">· {diff.definitions.length} · read these; they become the hover text on every ref</span></h4>
				<div class="rv__defs">
					{#each diff.definitions as d (d.coordinate)}
						{@const row = defRows.get(d.coordinate)}
						<div class="dcard" data-coord={d.coordinate} style="--au:{row ? pubkeyToColor(row.event.pubkey) : pubkeyToColor(mr.pubkey)}">
							<div class="dcard__h">
								<span class="mr__badge mr__badge--def">def</span>
								{#if row}<span class="rv__dot"></span>{getAuthorDisplayName(row.event.pubkey)} · {relTime(row.event.created_at)}{/if}
								{#if d.promoted_in_source}<span class="dcard__flag" title="listed in the requester's glossary bundle">→ glossary</span>{/if}
								<span class="rv__spacer"></span>
								<button class="dcard__row" onclick={() => (rootCoord ? app.requestRevealAnnotation(rootCoord, d.coordinate) : openCoordinate(d.coordinate))} title="its row in the ledger">≡</button>
							</div>
							<div class="dcard__t">{row?.term ?? d.term}</div>
							{#if row}<div class="dcard__e">{firstSentence(row.event.content)}</div>{/if}
						</div>
					{/each}
				</div>
			{/if}

			<h4 class="rv__h">the diff <span class="rv__hn">· {isOwner ? 'your' : 'the owner’s'} current → next version, same d</span></h4>
			{#each diff.sections as s (s.original)}
				<details class="rv__sec" data-coord={s.original} open>
					<summary><b>{s.title || dOf(s.original)}</b><span>30041 · d:{dOf(s.original)}</span><span class="rv__sec-n">{s.hunks.length} hunk{s.hunks.length === 1 ? '' : 's'} · +{aTagCount(s)} a tag{aTagCount(s) === 1 ? '' : 's'}</span></summary>
					<DiffBlock title={s.title || dOf(s.original)} hunks={s.hunks} tagsAdded={s.tags_added} tagsRemoved={s.tags_removed} coordinate={s.original} bare />
				</details>
			{/each}
			{#if !diff.sections.length}
				<div class="rv__note">no section changes — the index alone.</div>
			{/if}
			<details class="rv__index" open>
				<summary><b>index</b><span>30040 · {diff.index ? 'same d, new version' : 'unchanged'}</span><span class="rv__sec-n">{diff.index ? `${indexAdded ? `+${indexAdded} a tag${indexAdded === 1 ? '' : 's'}` : 'structure'}` : 'no new version'}</span></summary>
				<div class="rv__struct-box">
					{#if diff.index}
						{#each diff.index.diff.added as a (a.coordinate)}
							<div class="mr__struct"><span class="mr__plus">+ a → {isGlossaryCoord(a.coordinate) ? 'Glossary' : dOf(a.coordinate)}</span><span>slot {a.slot}{addedGlossary && a.coordinate === addedGlossary.coordinate && promoted.length ? ` · 30040 bundle · ${promoted.length}: ${promoted.map((p) => p.term).join(', ')} · by ${defsAuthor} (stays theirs)` : ''}</span></div>
						{/each}
						{#each diff.index.diff.replaced as r (r.original)}
							<div class="mr__struct"><span class="mr__plus">replaced</span><span>{dOf(r.original)} ← derivative {dOf(r.derivative)} (its content becomes the next version, same d)</span></div>
						{/each}
						{#each diff.index.diff.moved as m (m.coordinate)}
							<div class="mr__struct"><span class="mr__plus">moved</span><span>{dOf(m.coordinate)} · {m.from} → {m.to}</span></div>
						{/each}
						{#if diff.index.diff.removed.length}
							<div class="mr__struct"><span class="mr__minus">− removed</span><span>{diff.index.diff.removed.map(dOf).join(', ')}</span></div>
						{/if}
						<div class="mr__struct"><span class="mr__same">· unchanged</span><span>{untouched.length ? untouched.map(dOf).join(', ') : 'none'} · order kept</span></div>
					{:else}
						<div class="mr__struct"><span class="mr__same">· the index needs no new version</span></div>
					{/if}
				</div>
			</details>

			<details class="mr__wire" bind:open={wireOpen}>
				<summary class="mr__label">event preview <span class="mr__label-sub">· what “merge +” signs — {nextCount} next version{nextCount === 1 ? '' : 's'} under {isOwner ? 'your' : 'the owner’s'} d</span></summary>
				<pre class="mr__wire-pre">{wireText(diff)}</pre>
			</details>

			<div class="mr-agent" data-slot="assistant-review">
				<div class="mr-agent__h">assistant review <span class="rv__spacer"></span><button class="mr__btn mr__btn--mini" disabled title="your own agent writes it: “check tendrl” in your harness runs list_asks → get_ask_diff → review_ask; it reads this same diff and cannot merge">ask the assistant</button></div>
				{#if diff?.review}
					<ul class="mr-agent__findings">
						{#each diff.review.findings as f}<li>{f}</li>{/each}
					</ul>
					<div class="mr-agent__rec" data-recommendation={diff.review.recommendation}>↳ recommend: {diff.review.recommendation}{#if diff.review.note} — {diff.review.note}{/if}</div>
				{:else}
					<div class="rv__note">optional — a structured review kept as a local record, never an event. None yet.</div>
				{/if}
			</div>

			<AnnotationThread root={rootCoord} coordinate={`818:${mrId}`} author={mr.pubkey} title={mr.content || 'merge request'} compact={false} count={undefined} onreplied={() => {}} />
		</div>

		<div class="rvfoot">
			{#if !open}
				<div class="rvfoot__verdict">
					<span class="rv__g" class:rv__g--ok={status === 'merged'} class:rv__g--bad={status === 'declined'}>{status === 'merged' ? '+' : '−'}</span>
					{status}{#if mr.verdict} {relTime(mr.verdict.created_at)} ago{/if}{#if verdictReason()} — “{verdictReason()}”{/if} · kind 7 on the 818 ·
					{status === 'merged' ? (isOwner ? 'your next versions are signed, local until broadcast' : 'the owner republished under their d; your derivative stays as provenance') : isRequester ? 'your derivative stays as your fork' : 'their derivative stays as their fork'}
				</div>
			{:else if isOwner}
				<div class="rvfoot__foot" data-open="true">
					<button class="mr__btn mr__btn--decline" class:mr__btn--armed-bad={armed === 'decline'} onclick={decline} disabled={!canAct}>{armed === 'decline' ? 'sure? react − with a reason' : 'decline −'}</button>
					<button class="mr__btn" disabled title="later — edit the merged result before signing">open merged result in composer</button>
					<span class="rv__spacer"></span>
					<button class="mr__btn mr__btn--merge" class:mr__btn--armed={armed === 'merge'} onclick={merge} disabled={!canAct}>{working ? 'signing…' : armed === 'merge' ? `sure? sign ${nextCount} next version${nextCount === 1 ? '' : 's'} · react + · broadcast` : 'merge + ▸'}</button>
				</div>
				<div class="rvfoot__steps">merge = sign {diff.sections.length} section{diff.sections.length === 1 ? '' : 's'}{diff.index ? ' + index' : ''} (same d, your key) → kind 7 “+” on the 818 → broadcast (Confirm-gated, the separate step) · the definitions stay {defsAuthor}’s events, referenced by coordinate</div>
			{:else}
				<div class="rvfoot__verdict rv__muted">{isRequester ? `waiting for ${ownerName} — you can read the diff; only the owner merges` : `read-only — addressed to ${ownerName}, not to you`}</div>
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
	.rv__bar {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: 3px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--base6);
		flex-shrink: 0;
	}
	.rv__bar-t {
		color: var(--base6);
	}
	.pane-head__back {
		font: var(--t-2xs) / 1 var(--font-mono);
		min-height: 22px;
		padding: 0 var(--s-2);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--base6);
		cursor: pointer;
	}
	.pane-head__back:hover {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rv {
		flex: 1;
		overflow: auto;
		padding: var(--s-3) var(--s-3) var(--s-6);
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
		font: var(--t-xs) / 1.45 var(--font-sans);
	}
	.rv__spacer {
		flex: 1;
	}
	.rv__k {
		color: var(--base5);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		font-size: var(--t-3xs);
		font-weight: 600;
		margin-right: 4px;
	}
	.rv__ok {
		color: var(--green);
	}
	.rv__warn {
		color: var(--yellow);
	}
	.rv__muted {
		color: var(--base5);
	}
	.rv__dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au, var(--yellow));
		display: inline-block;
		flex: none;
	}
	.rv__status {
		display: flex;
		flex-wrap: wrap;
		gap: 6px 12px;
		align-items: center;
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--base6);
	}
	.rv__link {
		font: var(--t-2xs) / 1.2 var(--font-mono);
		min-height: 22px;
		padding: 0 var(--s-2);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--base6);
		cursor: pointer;
	}
	.rv__link:hover {
		color: var(--fg);
		border-color: var(--base5);
	}
	.mr__base--ok {
		color: var(--base6);
	}
	.mr__base--warn {
		color: var(--base6);
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
		color: var(--ask);
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
	.mr__badge--def {
		color: var(--green);
		border-color: rgba(180, 190, 130, 0.5);
	}
	.rv__note {
		font: var(--t-2xs) / 1.4 var(--font-mono);
		color: var(--base5);
		padding: 0 var(--s-3);
	}
	.rv .rv__note {
		padding: 0;
	}
	.rv__note--bad {
		color: var(--red);
	}
	.rv__note--warn {
		color: var(--yellow);
	}
	/* the ask */
	.rv__ask {
		display: grid;
		grid-template-columns: 3px minmax(0, 1fr);
		gap: 0 10px;
		background: var(--panel-bg-soft);
		border-radius: 3px;
	}
	.rv__ask-rail {
		background: var(--au, var(--ask));
		border-radius: 3px 0 0 3px;
	}
	.rv__ask-body {
		padding: 6px 10px 8px 0;
	}
	.rv__ask-who {
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--base5);
		display: flex;
		gap: 6px;
		align-items: center;
		flex-wrap: wrap;
		margin-bottom: 2px;
	}
	.rv__ask-src {
		margin-top: 4px;
	}
	.rv__ask-txt {
		font: var(--t-md) / 1.5 var(--font-serif);
		color: var(--fg);
	}
	/* section headings */
	.rv__h {
		margin: var(--s-1) 0 0;
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
	}
	.rv__hn {
		color: var(--base4);
		font-weight: 400;
		letter-spacing: 0;
		text-transform: none;
	}
	/* what changes */
	.rv__sum {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.tile {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 4px 9px;
		font: var(--t-2xs) / 1.3 var(--font-mono);
		color: var(--base6);
		display: flex;
		flex-direction: column;
		max-width: 100%;
	}
	.tile b {
		font: 600 var(--t-md) / 1.2 var(--font-sans);
		color: var(--fg);
	}
	.tile.plus b {
		color: var(--green);
	}
	.tile.same b {
		color: var(--base5);
	}
	/* the definitions being linked */
	.rv__defs {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
		gap: 6px;
	}
	.dcard {
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--green));
		border-radius: 3px;
		padding: 5px 9px 6px;
		background: var(--panel-bg-soft);
		min-width: 0;
	}
	.dcard__h {
		display: flex;
		gap: 6px;
		align-items: center;
		font: var(--t-3xs) / 1.2 var(--font-mono);
		color: var(--base5);
		flex-wrap: wrap;
	}
	.dcard__flag {
		font: var(--t-3xs) / 1 var(--font-mono);
		color: var(--green);
		border: 1px solid rgba(180, 190, 130, 0.4);
		border-radius: 3px;
		padding: 1px 4px;
	}
	.dcard__row {
		min-height: 18px;
		padding: 0 6px;
		font: var(--t-3xs) / 1 var(--font-mono);
		border: 1px solid transparent;
		border-radius: 3px;
		background: transparent;
		color: var(--base5);
		cursor: pointer;
	}
	.dcard__row:hover {
		color: var(--fg);
		border-color: var(--panel-border);
	}
	.dcard__t {
		font: 600 var(--t-sm) / 1.3 var(--font-sans);
		color: var(--fg);
		margin: 2px 0;
	}
	.dcard__e {
		font: var(--t-sm) / 1.4 var(--font-serif);
		color: var(--fg-alt);
	}
	/* the diff, per section */
	.rv__sec > summary,
	.rv__index > summary {
		cursor: pointer;
		list-style: none;
		display: flex;
		gap: 10px;
		align-items: baseline;
		flex-wrap: wrap;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 4px 10px;
		font: var(--t-2xs) / 1.4 var(--font-mono);
		color: var(--base6);
	}
	.rv__sec > summary::-webkit-details-marker,
	.rv__index > summary::-webkit-details-marker {
		display: none;
	}
	.rv__sec > summary b,
	.rv__index > summary b {
		color: var(--fg);
		font-weight: 600;
	}
	.rv__sec[open] > summary,
	.rv__index[open] > summary {
		border-radius: 3px 3px 0 0;
		border-bottom: 0;
	}
	.rv__sec-n {
		margin-left: auto;
		color: var(--base5);
	}
	.rv__sec[open] > :global(.diff) {
		border-radius: 0 0 3px 3px;
	}
	.rv__struct-box {
		border: 1px solid var(--panel-border);
		border-radius: 0 0 3px 3px;
		overflow: hidden;
	}
	.mr__struct {
		padding: 3px 10px;
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
		border-top: 1px solid var(--panel-border);
		color: var(--base6);
		font: calc(var(--t-xs) - 1px) / 1.5 var(--font-mono);
	}
	.rv__struct-box .mr__struct:first-child {
		border-top: 0;
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
	/* the sticky footer */
	.rvfoot {
		flex-shrink: 0;
		background: var(--panel-bg);
		border-top: 1px solid var(--panel-border);
		padding: 6px var(--s-3);
		display: flex;
		flex-direction: column;
		gap: 3px;
	}
	.rvfoot__foot {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		flex-wrap: wrap;
	}
	.rvfoot__steps {
		font: var(--t-3xs) / 1.5 var(--font-mono);
		color: var(--base5);
	}
	.rvfoot__verdict {
		font: var(--t-xs) / 1.4 var(--font-mono);
		color: var(--fg);
	}
	.rv__g {
		font-weight: 700;
		font-family: var(--font-mono);
	}
	.rv__g--ok {
		color: var(--green);
	}
	.rv__g--bad {
		color: var(--red);
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
