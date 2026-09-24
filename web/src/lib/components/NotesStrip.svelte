<script lang="ts">
	// The reader's notes strip (docs/annotations-ledger-contract.org,
	// "Reader"): under a section's text, one stub per request / insertion /
	// merge request anchored there — glyph, title, answer count — and the
	// full card (RequestCard / InsertionCard) under the strip for the one
	// stub that is open. Which stub is open is view state of this strip;
	// everything about the items themselves is engine-derived on the rows.
	import * as api from '$lib/api';
	import RequestCard from './RequestCard.svelte';
	import InsertionCard from './InsertionCard.svelte';
	import { groupAnswers } from './request-group';
	import { getAppState } from '$lib/state.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';

	let {
		requests = [],
		insertions = [],
		caption = null,
		onopeninsertion = undefined,
		onrequestschanged = undefined,
		oninsertionpromoted = undefined
	}: {
		/** The annotation requests anchored here (from the merged list). */
		requests?: api.AnnotationRequest[];
		/** The loaded insertions anchored here — answers fold under their
		 *  request and count on its stub; the rest get a stub each. */
		insertions?: api.LoadedInsertion[];
		/** A leading caption ("⌂ whole publication"). */
		caption?: string | null;
		onopeninsertion?: ((ins: api.LoadedInsertion) => void) | undefined;
		onrequestschanged?: (() => void) | undefined;
		oninsertionpromoted?: ((resp: api.PromotionResponse) => void) | undefined;
	} = $props();

	const app = getAppState();
	const groups = $derived(groupAnswers(requests, insertions));

	type Stub = {
		id: string;
		glyph: string;
		cls: 'req' | 'ok' | 'def' | 'mr' | 'kasten' | 'note';
		title: string;
		answers: number;
		author: string;
		req?: api.AnnotationRequest;
		ins?: api.LoadedInsertion;
	};
	const me = $derived(app.identityStatus?.pubkey ?? '');
	const stubs = $derived.by((): Stub[] => {
		const out: Stub[] = [];
		for (const r of requests) {
			const answered = r.status === 'answered';
			out.push({
				id: r.id,
				glyph: answered ? '✓' : '?',
				cls: answered ? 'ok' : 'req',
				title: r.output === 'vocabulary' && r.terms?.length ? r.terms.join(', ') : (r.title || r.task),
				answers: groups.answers.get(r.id)?.length ?? 0,
				author: r.coordinate ? r.coordinate.split(':')[1] : me,
				req: r
			});
		}
		for (const ins of groups.rest) {
			if (ins.role === 'request') continue;
			const ev = ins.event;
			const title = ev.tags.find((t) => t[0] === 'title')?.[1] ?? ins.term ?? '';
			let glyph = '◂';
			let cls: Stub['cls'] = 'note';
			let label = title;
			if (ins.role === 'definition') {
				glyph = 'def';
				cls = 'def';
				label = ins.term ?? title;
			} else if (ins.role === 'ask') {
				glyph = '⑂';
				cls = 'mr';
				label = ins.merge_request?.content || title || 'merge request';
			} else if (ins.role === 'bundle') {
				glyph = '⊞';
				cls = 'kasten';
			} else if (ins.anchor.placement === 'before') glyph = '▴';
			else if (ins.anchor.placement === 'after') glyph = '▾';
			out.push({ id: ins.coordinate, glyph, cls, title: label || ins.coordinate.split(':').slice(2).join(':'), answers: 0, author: ev.pubkey, ins });
		}
		return out;
	});

	let open = $state<string | null>(null);
	const openStub = $derived(open ? (stubs.find((s) => s.id === open) ?? null) : null);
	function toggle(id: string) {
		open = open === id ? null : id;
	}
	const who = (pk: string) => (pk && pk === me ? 'you' : pk ? getAuthorDisplayName(pk) : 'unsigned');
</script>

{#if stubs.length > 0}
	<div class="strip" role="group" aria-label="Notes here">
		{#if caption}<span class="strip__cap">{caption}</span>{/if}
		{#each stubs as s (s.id)}
			<button
				class="stub"
				class:on={open === s.id}
				data-id={s.id}
				style="--au:{s.author ? pubkeyToColor(s.author) : 'var(--base5)'}"
				onclick={() => toggle(s.id)}
				title="{who(s.author)} — {open === s.id ? 'fold' : 'open'} this card"
				aria-expanded={open === s.id}
			>
				<span class="stub__g {s.cls}">{s.glyph}</span>
				<span class="stub__t">{s.title}</span>
				{#if s.answers > 0}<span class="stub__n">{s.answers}↳</span>{/if}
			</button>
		{/each}
	</div>
	{#if openStub}
		<div class="strip__open">
			{#if openStub.req}
				<RequestCard request={openStub.req} onchanged={onrequestschanged} answers={groups.answers.get(openStub.req.id) ?? []} onopen={onopeninsertion} onpromoted={oninsertionpromoted} />
			{:else if openStub.ins}
				<InsertionCard insertion={openStub.ins} onopen={onopeninsertion} onpromoted={oninsertionpromoted} />
			{/if}
		</div>
	{/if}
{/if}

<style>
	.strip {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
		margin: 6px 0 0;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		line-height: 1.3;
	}
	.strip__cap {
		color: var(--base5);
		font-size: var(--t-3xs);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		margin-right: 4px;
	}
	.stub {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		max-width: 100%;
		min-height: 22px;
		padding: 2px 7px 2px 6px;
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au);
		border-radius: var(--r-md);
		background: var(--panel-bg-soft);
		color: var(--base6);
		font: inherit;
		cursor: pointer;
	}
	.stub:hover {
		color: var(--fg);
		border-color: var(--base5);
		border-left-color: var(--au);
	}
	.stub.on {
		color: var(--fg);
		background: var(--panel-header-bg);
	}
	.stub__g {
		font-weight: 700;
		flex: none;
	}
	.stub__g.req { color: var(--ask); }
	.stub__g.ok { color: var(--id-yours); }
	.stub__g.def { color: var(--green); }
	.stub__g.mr { color: var(--ask); }
	.stub__g.kasten { color: var(--violet); }
	.stub__g.note { color: var(--base6); }
	.stub__t {
		max-width: 180px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
	}
	.stub__n {
		color: var(--base5);
	}
	.strip__open {
		margin-top: 4px;
	}
	.strip__open :global(.icard),
	.strip__open :global(.rcard) {
		margin-left: 0;
		margin-right: 0;
	}
</style>
