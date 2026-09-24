<script lang="ts">
	// Relink: the vocabulary loop's last step for a publication you own.
	// The engine rewrote the first occurrence of each defined term per
	// section as `{{ref:<definition coordinate>|term as written}}` (code,
	// headings, native links and existing tokens skipped) and hands back the
	// hunks plus the unsigned templates it would sign: new versions of the
	// changed sections under their own `d`, and — when the per-definition
	// glossary checkboxes change the selection — the glossary bundle and the
	// index. This window is the diff gate: every event it will sign is shown
	// before the armed button, and nothing is broadcast (the separate step).
	// A publication that is not yours takes the other door (plan Phase 5c
	// step 4): the same diff, but the button signs a *derivative* under your
	// key (index′ + copies of the changed sections, `E`/`p` back-pointers)
	// and one kind-818 merge request addressed to the owner, sent to the
	// destination set the engine computed — their inbox ∪ where the target
	// was found ∪ hints ∪ mine — each relay shown with its reason and
	// uncheckable. Undeliverable (no inbox, no provenance) never falls back
	// to the publish set silently: the button stays off and the note says why.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));
	// svelte-ignore state_referenced_locally
	const open = app.relink;
	const root = open?.root ?? '';
	const definitions = open?.definitions ?? [];
	const initialPromoted = new Set(open?.promoted ?? []);

	// Per-definition glossary selection (plain object bookkeeping; the
	// checkboxes write it and a debounced re-preview reads it).
	let glossarySel = $state<Record<string, boolean>>(
		Object.fromEntries(definitions.map((c) => [c, initialPromoted.has(c)]))
	);
	let preview = $state<api.RelinkPreview | null>(null);
	let previewing = $state(true);
	let previewErr = $state<string | null>(null);
	let armed = $state(false);
	let publishing = $state(false);
	let wireOpen = $state(false);
	// Foreign root: the destination set, editable (unchecked = dropped) and
	// refreshable with a Confirm-gated kind-10002 fetch of the owner's inbox.
	let destSel = $state<Record<string, boolean>>({});
	let destExtra = $state<api.DestinationSet | null>(null);
	let fetchingInbox = $state(false);
	const yielding = $derived((publishing && confirmState.intent?.type === 'publish_intent') || (fetchingInbox && confirmState.intent !== null));
	const foreign = $derived(!!preview && !preview.owner);
	const ownerPk = $derived(root.split(':')[1] ?? '');
	/** The destination rows shown: the preview's set, with a later 10002
	 *  fetch merged in (reasons unioned, order inbox → found on → hint → mine). */
	const destinations = $derived.by((): api.Destination[] => {
		const base = preview?.derivative?.destinations.destinations ?? [];
		const extra = destExtra?.destinations ?? [];
		const byUrl = new Map<string, api.Destination>();
		for (const d of [...base, ...extra]) {
			const cur = byUrl.get(d.url);
			if (cur) byUrl.set(d.url, { url: d.url, reasons: [...new Set([...cur.reasons, ...d.reasons])] });
			else byUrl.set(d.url, { url: d.url, reasons: [...d.reasons] });
		}
		const rank: Record<api.DestinationReason, number> = { inbox: 0, found_on: 1, hint: 2, mine: 3 };
		const best = (d: api.Destination) => Math.min(...d.reasons.map((r) => rank[r] ?? 9));
		return [...byUrl.values()].sort((a, b) => best(a) - best(b));
	});
	const destSet = $derived.by((): api.DestinationSet | null => {
		const ds = preview?.derivative?.destinations;
		if (!ds) return null;
		if (!destExtra) return ds;
		return {
			...ds,
			deliverable: ds.deliverable || destExtra.deliverable,
			inbox_known: ds.inbox_known || destExtra.inbox_known,
			provenance_known: ds.provenance_known || destExtra.provenance_known,
			note: ds.deliverable || destExtra.deliverable ? undefined : (destExtra.note ?? ds.note)
		};
	});
	const checkedDestinations = $derived(destinations.filter((d) => destSel[d.url] !== false).map((d) => d.url));
	/** Relays that actually reach the owner: checked, with an inbox or
	 *  found-on reason. A `mine`-only relay is findability, not delivery. */
	const reachCount = $derived(destinations.filter((d) => destSel[d.url] !== false && d.reasons.some((r) => r === 'inbox' || r === 'found_on')).length);
	const canSend = $derived(foreign && !!preview?.derivative && reachCount > 0 && checkedDestinations.length > 0);

	const termOf = $derived.by(() => {
		const m = new Map<string, string>();
		for (const s of preview?.sections ?? []) for (const h of s.hunks) if (!m.has(h.coordinate)) m.set(h.coordinate, h.term);
		return m;
	});
	const hunkCount = $derived((preview?.sections ?? []).reduce((n, s) => n + s.hunks.length, 0));
	const promotedAfter = $derived(preview?.promoted ?? [...initialPromoted]);
	const glossaryChanged = $derived.by(() => {
		const after = new Set(promotedAfter);
		if (after.size !== initialPromoted.size) return true;
		for (const c of after) if (!initialPromoted.has(c)) return true;
		return false;
	});

	function glossaryEdit(): api.RelinkRequest['glossary'] {
		const add = definitions.filter((c) => glossarySel[c] && !initialPromoted.has(c));
		const remove = definitions.filter((c) => !glossarySel[c] && initialPromoted.has(c));
		if (add.length === 0 && remove.length === 0) return undefined;
		return { ...(add.length ? { add } : {}), ...(remove.length ? { remove } : {}) };
	}
	function body(): api.RelinkRequest {
		const g = glossaryEdit();
		return { root, definitions, ...(g ? { glossary: g } : {}) };
	}

	let previewSeq = 0;
	let previewTimer: ReturnType<typeof setTimeout> | null = null;
	async function runPreview() {
		const seq = ++previewSeq;
		previewing = true;
		previewErr = null;
		try {
			const p = await api.previewRelink(body());
			if (seq === previewSeq) {
				preview = p;
				// New destination rows default to checked; an unchecked one stays unchecked.
				for (const d of p.derivative?.destinations.destinations ?? []) if (!(d.url in destSel)) destSel = { ...destSel, [d.url]: true };
			}
		} catch (e) {
			if (seq === previewSeq) previewErr = api.errorMessage(e, 'preview failed');
		} finally {
			if (seq === previewSeq) previewing = false;
		}
	}
	function schedulePreview() {
		armed = false;
		if (previewTimer) clearTimeout(previewTimer);
		previewTimer = setTimeout(() => void runPreview(), 250);
	}
	function toggleGlossary(coord: string) {
		glossarySel = { ...glossarySel, [coord]: !glossarySel[coord] };
		schedulePreview();
	}
	function toggleDestination(url: string) {
		destSel = { ...destSel, [url]: destSel[url] === false };
		armed = false;
	}
	/** The owner's inbox is not known locally: one Confirm-gated pull of their
	 *  kind 10002 through the indexer composition, merged into the rows. */
	async function fetchInbox() {
		if (fetchingInbox || !root) return;
		fetchingInbox = true;
		try {
			const ds = await api.getMergeRequestDestinations({ target: root, owner: ownerPk, policy: 'fetch_always', mode_confirm: true });
			destExtra = ds;
			for (const d of ds.destinations) if (!(d.url in destSel)) destSel = { ...destSel, [d.url]: true };
			app.pushToast(ds.inbox_known ? `Inbox found — ${ds.destinations.filter((d) => d.reasons.includes('inbox')).length} read relay(s) for the owner` : 'No kind 10002 found for the owner', ds.inbox_known ? 'success' : 'info');
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Inbox lookup failed'), 'error', 5000);
		} finally {
			fetchingInbox = false;
		}
	}
	// Once, on open — from an event handler's timing, not an $effect over the form.
	void runPreview();

	function close() {
		if (previewTimer) clearTimeout(previewTimer);
		app.relink = null;
	}

	async function sign() {
		if (!preview || publishing) return;
		if (!preview.owner && !canSend) return;
		if (!armed) {
			armed = true;
			return;
		}
		publishing = true;
		if (!preview.owner) {
			try {
				const resp = await api.publishRelink({ ...body(), destinations: checkedDestinations, broadcast: true });
				const n = resp.derivative?.sections.length ?? resp.sections.length;
				const m = resp.merge_request?.destinations_used.length ?? checkedDestinations.length;
				app.pushToast(`Derivative signed (index′ + ${n} section${n === 1 ? '' : 's'}) · merge request sent to ${m} relay${m === 1 ? '' : 's'}`, 'success');
				app.markRelinkDone(root);
				app.bumpInsertions();
				close();
			} catch (e) {
				app.pushToast(api.errorMessage(e, 'Fork + merge request failed'), 'error', 5000);
				armed = false;
			} finally {
				publishing = false;
			}
			return;
		}
		try {
			const resp = await api.publishRelink(body());
			const n = resp.sections.length;
			const extras = [resp.glossary ? 'glossary' : null, resp.index ? 'index' : null].filter(Boolean).join(' + ');
			app.pushToast(
				`Relinked ${resp.hunk_count} term${resp.hunk_count === 1 ? '' : 's'} in ${n} section${n === 1 ? '' : 's'}${extras ? ` (+ ${extras})` : ''} — signed, local until broadcast`,
				'success'
			);
			app.markRelinkDone(root);
			app.bumpInsertions();
			close();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Relink failed'), 'error', 5000);
			armed = false;
		} finally {
			publishing = false;
		}
	}

	function short(coord: string): string {
		const [k, pk, ...d] = coord.split(':');
		return `${k}:${pk.slice(0, 8)}…:${d.join(':')}`;
	}
	function dOf(coord: string): string {
		return coord.split(':').slice(2).join(':');
	}
	function label(coord: string): string {
		return termOf.get(coord) ?? dOf(coord);
	}
	/** Split an "after" line around its `{{ref:…}}` runs so the new tokens
	 *  can be tinted; the plain runs stay plain. */
	function runs(line: string): { text: string; ref: boolean }[] {
		const out: { text: string; ref: boolean }[] = [];
		const re = /\{\{ref:[^}]*\}\}/g;
		let last = 0;
		let m: RegExpExecArray | null;
		while ((m = re.exec(line))) {
			if (m.index > last) out.push({ text: line.slice(last, m.index), ref: false });
			out.push({ text: m[0], ref: true });
			last = m.index + m[0].length;
		}
		if (last < line.length) out.push({ text: line.slice(last), ref: false });
		return out;
	}
	function shortTag(t: string[]): string {
		return t.map((v) => (v.length > 44 ? v.slice(0, 20) + '…' + v.slice(-12) : v)).join(' ');
	}
	function tagRows(t: api.UnsignedTemplate | null | undefined): string {
		return (t?.tags ?? []).map((x) => '  [' + shortTag(x) + ']').join('\n');
	}
	function wireText(p: api.RelinkPreview): string {
		const parts: string[] = [];
		if (p.derivative) {
			for (const s of p.derivative.sections) parts.push(`section′ · kind ${s.template.kind} · ${dOf(s.original)} → ${dOf(s.coordinate)}\n${tagRows(s.template)}`);
			if (p.glossary) parts.push(`glossary · kind ${p.glossary.kind}\n${tagRows(p.glossary)}`);
			parts.push(`index′ · kind ${p.derivative.index.kind} · ${dOf(p.derivative.index_coordinate)}\n${tagRows(p.derivative.index)}`);
			parts.push(`merge request · kind ${p.derivative.merge_request.kind}\n  content: ${JSON.stringify(p.derivative.merge_request.content)}\n${tagRows(p.derivative.merge_request)}`);
			return parts.join('\n\n');
		}
		for (const s of p.sections) parts.push(`section · kind ${s.template.kind} · d:${s.d}\n${tagRows(s.template)}`);
		if (p.glossary) parts.push(`glossary · kind ${p.glossary.kind}\n${tagRows(p.glossary)}`);
		if (p.index) parts.push(`index · kind ${p.index.kind}\n${tagRows(p.index)}`);
		return parts.join('\n\n');
	}
	const reasonLabel: Record<api.DestinationReason, string> = { inbox: 'inbox', found_on: 'found on', hint: 'hint', mine: 'yours' };
	const reasonTitle: Record<api.DestinationReason, string> = {
		inbox: 'a read relay from the owner’s kind 10002 — where others write to reach them',
		found_on: 'the relay that served the target (or its index) to you — the owner published there',
		hint: 'a relay hint on the index’s a tag',
		mine: 'one of your write relays — so the merge request is findable from your side'
	};
	function tagOf(t: api.UnsignedTemplate | null | undefined, name: string): string | null {
		return t?.tags.find((x) => x[0] === name)?.[1] ?? null;
	}
	function handleKeydown(e: KeyboardEvent) {
		if (publishing) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="rl-backdrop {yielding ? 'rl-backdrop--yield' : ''}"
	onclick={() => {
		if (!publishing) close();
	}}
	role="presentation"
>
	<div
		class="rl-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Relink definitions into the text"
		tabindex="-1"
	>
		<header class="rl-header">
			<h3 class="rl-title">
				{#if foreign}
					<span class="rl-dot rl-dot--ask" aria-hidden="true">⑂</span>
					fork + merge request
					<span class="rl-title__on">— a derivative under your key + a kind 818 to the owner{#if open?.rootTitle} · <em>{open.rootTitle}</em>{/if}</span>
				{:else}
					<span class="rl-dot" aria-hidden="true">⇄</span>
					relink into your text
					{#if open?.rootTitle}<span class="rl-title__on">— <em>{open.rootTitle}</em></span>{/if}
				{/if}
			</h3>
			<button class="rl-close" onclick={close} aria-label="Close">×</button>
		</header>
		<div class="rl-help">
			first occurrence of each term per section becomes <code>{'{{ref:<definition>|term}}'}</code> — code, headings and existing links skipped · {#if foreign}copies under your key with <code>E</code>/<code>p</code> back-pointers; the owner merges by republishing under their own <code>d</code>{:else}new versions under the same <code>d</code>, local until broadcast{/if}
		</div>

		{#if previewing && !preview}
			<div class="rl-line rl-line--muted">computing the diff…</div>
		{:else if previewErr}
			<div class="rl-line rl-line--bad">{previewErr}</div>
		{:else if preview}
			{#if preview.note}
				<div class="rl-line rl-note">{preview.note}</div>
			{/if}
			{#if preview.sections.length === 0}
				<div class="rl-line rl-line--muted">none of the terms occurs outside code, headings or existing links — nothing to relink</div>
			{/if}
			{#each preview.sections as sec (sec.coordinate)}
				<div class="rl-diff" data-section={sec.coordinate}>
					<div class="rl-diff__head">
						<b>{sec.title || dOf(sec.coordinate)}</b>
						<span>30041 · d:{sec.d} · same d, new version</span>
						<span class="rl-diff__n">{sec.hunks.length} hunk{sec.hunks.length === 1 ? '' : 's'}</span>
					</div>
					{#each sec.hunks as h, i (i)}
						<div class="rl-ln rl-ln--del">- {h.line_before}</div>
						<div class="rl-ln rl-ln--add">+ {#each runs(h.line_after) as r, j (j)}{#if r.ref}<span class="rl-ref">{r.text}</span>{:else}{r.text}{/if}{/each}</div>
					{/each}
					{#each sec.tags_added as t, i (i)}
						<div class="rl-ln rl-ln--tag">  tags  + [{t.map((v) => JSON.stringify(v)).join(', ')}]</div>
					{/each}
				</div>
			{/each}
			{#if preview.skipped?.length}
				<div class="rl-line rl-line--muted">skipped: {preview.skipped.map((s) => `${dOf(s.coordinate)} (${s.why})`).join(' · ')}</div>
			{/if}

			<div class="rl-diff">
				<div class="rl-diff__head"><b>index</b><span>30040 · {preview.index ? 'same d, new version' : 'unchanged'}</span></div>
				{#if promotedAfter.length > 0 && (preview.index || glossaryChanged)}
					<div class="rl-struct"><span class="rl-plus">+ a → Glossary</span><span>30040 bundle · {promotedAfter.length} definition{promotedAfter.length === 1 ? '' : 's'}: {promotedAfter.map(label).join(', ')}</span><span class="rl-diff__n">appendix</span></div>
				{:else if preview.index}
					<div class="rl-struct"><span class="rl-plus">· a → Glossary</span><span>taken out (empty)</span></div>
				{/if}
				{#if preview.unchanged.length > 0}
					<div class="rl-struct"><span class="rl-same">· unchanged</span><span>{preview.unchanged.map(dOf).join(', ')}</span></div>
				{/if}
			</div>

			<div class="rl-label">glossary <span class="rl-label__sub">— optional, per event; the refs resolve without it</span></div>
			<div class="rl-chips">
				{#each definitions as c (c)}
					<label class="rl-check" title={c}>
						<input type="checkbox" id="rl-gl-{dOf(c)}" checked={!!glossarySel[c]} onchange={() => toggleGlossary(c)} disabled={publishing} />
						promote “{label(c)}”{initialPromoted.has(c) ? ' (in glossary)' : ''}
					</label>
				{/each}
			</div>

			{#if foreign && preview.derivative}
				{@const dv = preview.derivative}
				<div class="rl-diff rl-derivative" data-derivative={dv.index_coordinate}>
					<div class="rl-diff__head"><b>derivative</b><span>signed under your key · the owner merges by republishing their own</span><span class="rl-diff__n">{dv.sections.length} section{dv.sections.length === 1 ? '' : 's'} + index′</span></div>
					<div class="rl-struct"><span class="rl-plus">index′</span><span>30040 · d:{dOf(dv.index_coordinate)} · <code>E</code> → their {tagOf(dv.index, 'E') ? (tagOf(dv.index, 'E') ?? '').slice(0, 8) + '…' : 'version'} · <code>p</code> → {ownerPk.slice(0, 8)}…</span></div>
					{#each dv.sections as ds (ds.coordinate)}
						<div class="rl-struct"><span class="rl-plus">section′</span><span>d:{dOf(ds.original)} → d:{dOf(ds.coordinate)} · every tag verbatim · <code>E</code> → the copied version</span></div>
					{/each}
				</div>

				<div class="rl-label">the 818 <span class="rl-label__sub">— NIP-54’s merge request, target widened to a 30040/30041; the owner answers with a + or − reaction</span></div>
				<pre class="rl-wire__pre rl-mr" data-kind="818">{`{ "kind": ${dv.merge_request.kind},
  "content": ${JSON.stringify(dv.merge_request.content)},
  "tags": [
${dv.merge_request.tags.map((t) => '    [' + t.map((v) => JSON.stringify(v)).join(', ') + ']' + (t[0] === 'e' && t[3] === 'source' ? '   ← filled with index′’s id after signing' : t[0] === 'a' ? '   ← the target' : t[0] === 'p' ? '   ← the owner' : t[0] === 'e' ? '   ← the base version' : '')).join(',\n')}
  ] }`}</pre>

				<div class="rl-label">destination set <span class="rl-label__sub">— inbox ∪ found on ∪ hints ∪ yours; the derivative itself goes to your write relays</span></div>
				<div class="rl-dest" data-deliverable={destSet?.deliverable ? 'true' : 'false'}>
					{#each destinations as d (d.url)}
						<label class="rl-dest__row" data-url={d.url}>
							<input type="checkbox" id="rl-dest-{d.url.replace(/[^a-z0-9]/gi, '-')}" checked={destSel[d.url] !== false} onchange={() => toggleDestination(d.url)} disabled={publishing} />
							<span class="rl-dest__url">{d.url}</span>
							{#each d.reasons as r (r)}
								<span class="rl-why rl-why--{r}" title={reasonTitle[r]}>{reasonLabel[r]}</span>
							{/each}
						</label>
					{/each}
					{#if destinations.length === 0}
						<div class="rl-line rl-line--muted">no destinations known</div>
					{/if}
					{#if destSet && !destSet.deliverable}
						<div class="rl-line rl-note rl-dest__note">{destSet.note ?? `no inbox known for ${ownerPk.slice(0, 8)}… and no provenance for this event — the merge request would not reach the owner`}</div>
					{/if}
					{#if destSet && !destSet.inbox_known}
						<div class="rl-dest__fetch">
							<button class="rl-btn rl-btn--mini" onclick={fetchInbox} disabled={fetchingInbox || publishing} title="One Confirm-gated fetch of the owner’s kind 10002 through the indexer relays">{fetchingInbox ? 'fetching…' : 'fetch their kind 10002 ▸'}</button>
							<span class="rl-line rl-line--muted">their read relays are not known locally</span>
						</div>
					{/if}
				</div>
			{/if}

			<details class="rl-wire" bind:open={wireOpen}>
				<summary class="rl-label rl-wire__sum">event preview <span class="rl-label__sub">· what “sign” will sign — {#if preview.derivative}{preview.derivative.sections.length} section′{preview.derivative.sections.length === 1 ? '' : 's'}{preview.glossary ? ' + glossary' : ''} + index′ + the 818{:else}{preview.sections.length} section{preview.sections.length === 1 ? '' : 's'}{preview.glossary ? ' + glossary' : ''}{preview.index ? ' + index' : ''}{/if}</span></summary>
				<pre class="rl-wire__pre">{wireText(preview)}</pre>
			</details>
		{/if}

		<div class="rl-foot">
			<button class="rl-btn" onclick={close} disabled={publishing}>cancel</button>
			<button class="rl-btn" disabled title="later — edit the merged result before signing">open in composer</button>
			<span class="rl-spacer"></span>
				{#if preview && !preview.owner}
				<button
					class="rl-btn rl-btn--send"
					class:rl-btn--armed-ask={armed}
					onclick={sign}
					disabled={!canSign || previewing || publishing || !canSend || preview.sections.length === 0}
					title={!canSign ? 'Sign in to fork' : !preview.derivative ? 'The engine gave no derivative for this root' : !canSend ? 'No checked relay reaches the owner (inbox or found-on) — fetch their kind 10002 or check one' : 'Sign the derivative under your key and send the merge request to the checked relays'}
				>
					{#if publishing}signing…{:else if armed}sure? sign derivative + send 818 to {checkedDestinations.length} relay{checkedDestinations.length === 1 ? '' : 's'}{:else}sign + send merge request ▸{/if}
				</button>
			{:else}
				<button
					class="rl-btn rl-btn--go"
					class:rl-btn--armed={armed}
					onclick={sign}
					disabled={!canSign || !preview || previewing || publishing || (preview.sections.length === 0 && !glossaryChanged)}
					title={canSign ? 'Sign the new versions and keep them local; broadcast is the separate step' : 'Sign in to relink'}
				>
					{#if publishing}signing…{:else if armed}sure? sign {preview?.sections.length ?? 0} section{(preview?.sections.length ?? 0) === 1 ? '' : 's'}{preview?.glossary ? ' + glossary' : ''}{preview?.index ? ' + index' : ''}{:else}sign · keep local ▸{/if}
				</button>
			{/if}
		</div>
	</div>
</div>

<style>
	.rl-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 6dvh;
	}
	.rl-backdrop--yield {
		visibility: hidden;
	}
	.rl-modal {
		width: min(820px, 96vw);
		max-height: 90dvh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.rl-modal > * {
		flex-shrink: 0;
	}
	.rl-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.rl-title {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--fg);
		display: flex;
		align-items: baseline;
		gap: 6px;
		flex-wrap: wrap;
	}
	.rl-dot {
		color: var(--id-local);
	}
	.rl-title__on {
		color: var(--base5);
		font-weight: 400;
	}
	.rl-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
	}
	.rl-help,
	.rl-line {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
	}
	.rl-help code {
		color: var(--fg-muted);
	}
	.rl-line--muted {
		color: var(--base5);
	}
	.rl-line--bad {
		color: var(--danger);
	}
	.rl-note {
		color: var(--warn-fg);
	}
	.rl-label {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-top: 4px;
	}
	.rl-label__sub {
		text-transform: none;
		letter-spacing: 0;
	}
	.rl-diff {
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
		overflow: hidden;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		line-height: 1.5;
	}
	.rl-diff__head {
		background: var(--bg-surface);
		padding: 4px 10px;
		display: flex;
		gap: 10px;
		align-items: baseline;
		flex-wrap: wrap;
		color: var(--base6);
		border-bottom: 1px solid var(--panel-border);
	}
	.rl-diff__head b {
		color: var(--fg);
		font-weight: 600;
	}
	.rl-diff__n {
		margin-left: auto;
		color: var(--base5);
	}
	.rl-ln {
		padding: 1px 10px;
		white-space: pre-wrap;
		word-break: break-word;
		color: var(--base7);
	}
	.rl-ln--del {
		background: rgba(226, 120, 120, 0.12);
		color: var(--red);
	}
	.rl-ln--add {
		background: rgba(180, 190, 130, 0.12);
		color: var(--green);
	}
	.rl-ref {
		background: rgba(180, 190, 130, 0.22);
		border-radius: 2px;
	}
	.rl-ln--tag {
		color: var(--cyan);
		background: rgba(137, 184, 194, 0.08);
	}
	.rl-struct {
		padding: 3px 10px;
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
		border-top: 1px solid var(--panel-border);
		color: var(--base6);
	}
	.rl-plus {
		color: var(--green);
	}
	.rl-same {
		color: var(--base5);
	}
	.rl-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 6px 14px;
	}
	.rl-check {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--fg-muted);
		cursor: pointer;
	}
	.rl-check input {
		margin: 0;
	}
	.rl-wire {
		margin-top: 2px;
	}
	.rl-wire__sum {
		cursor: pointer;
		margin-top: 0;
	}
	.rl-wire__pre {
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
	.rl-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 6px;
		flex-wrap: wrap;
	}
	.rl-spacer {
		flex: 1;
	}
	.rl-btn {
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
	.rl-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.rl-btn--go {
		color: var(--id-local);
	}
	.rl-btn--armed {
		background: var(--id-local);
		color: var(--bg);
		border-color: var(--id-local);
	}
	.rl-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.rl-btn--mini {
		min-height: 24px;
		padding: 1px 8px;
	}
	.rl-dot--ask,
	.rl-btn--send {
		color: var(--orange);
	}
	.rl-btn--armed-ask {
		background: var(--orange);
		color: var(--bg);
		border-color: var(--orange);
	}
	.rl-mr {
		max-height: 200px;
	}
	.rl-dest {
		display: flex;
		flex-direction: column;
		gap: 3px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
	}
	.rl-dest__row {
		display: flex;
		gap: 8px;
		align-items: center;
		flex-wrap: wrap;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.rl-dest__row input {
		margin: 0;
	}
	.rl-dest__url {
		color: var(--fg);
	}
	.rl-why {
		color: var(--base5);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 0 5px;
		font-size: calc(var(--t-xs) - 2px);
	}
	.rl-why--inbox {
		color: var(--orange);
		border-color: rgba(226, 164, 120, 0.5);
	}
	.rl-why--found_on {
		color: var(--cyan);
		border-color: rgba(137, 184, 194, 0.5);
	}
	.rl-why--hint {
		color: var(--yellow);
		border-color: rgba(233, 177, 137, 0.5);
	}
	.rl-why--mine {
		color: var(--blue);
		border-color: rgba(132, 160, 198, 0.5);
	}
	.rl-dest__note {
		margin-top: 2px;
	}
	.rl-dest__fetch {
		display: flex;
		gap: 8px;
		align-items: center;
		margin-top: 2px;
	}
</style>
