<script lang="ts">
	// One loaded insertion, rendered beside the section it anchors to — a
	// post-it stuck to the page, visibly not a page. Structure is Direction B
	// from the insertions canvas (a flat margin card with a left --id-local
	// bar); the `sticky` skin is a dress on the same structure (Appearance →
	// Notes). Below the reader's gutter width it folds to a 44 px row under
	// the anchor and expands in place (Direction C). Promotion runs here: a
	// preview from `/publication/fork/preview` explains what the copy is
	// (new version vs derivative), then one armed button publishes it.
	// A loaded 30041 whose `role` is `request` (a signed annotation request)
	// renders nothing here — RequestCard draws it from the merged request
	// list. A note that answers a request (`request` set) wears an
	// "↳ answers" chip that opens the request.
	import * as api from '$lib/api';
	import RichContent from './RichContent.svelte';
	import EventLinks from './EventLinks.svelte';
	import { getAppState } from '$lib/state.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { reading } from '$lib/theme/reading.svelte';
	import type { ResolvedRef, ParsedToken } from '$lib/nostr/nostrdown';
	import { styleTitle } from '$lib/styles.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';

	let {
		insertion,
		open = undefined,
		onopen = undefined,
		onpromoted = undefined
	}: {
		insertion: api.LoadedInsertion;
		/** Fold state on narrow screens — bindable so the reader can park it. */
		open?: boolean | undefined;
		/** Open the section in its own reader buffer. */
		onopen?: ((ins: api.LoadedInsertion) => void) | undefined;
		/** A promotion landed — the reader refreshes the publication. */
		onpromoted?: ((resp: api.ForkPublishResponse) => void) | undefined;
	} = $props();

	const app = getAppState();
	const ev = $derived(insertion.event);
	const anchor = $derived(insertion.anchor);
	const title = $derived(ev.tags.find((t) => t[0] === 'title')?.[1] ?? '');
	const contextTag = $derived(ev.tags.find((t) => t[0] === 'context')?.[1] ?? null);
	const styleCoord = $derived(ev.tags.find((t) => t[0] === 'style')?.[1] ?? null);
	const mine = $derived(!!app.identityStatus?.pubkey && ev.pubkey === app.identityStatus.pubkey);
	const who = $derived(mine ? 'you' : getAuthorDisplayName(ev.pubkey));
	const placementLabel = $derived(
		anchor.placement === 'before'
			? anchor.parent === anchor.root
				? '▴ front matter'
				: '▴ inserted before'
			: anchor.placement === 'after'
				? anchor.parent === anchor.root
					? '▾ appendix'
					: '▾ inserted after'
				: '◂ attached'
	);
	const ago = $derived(relTime(ev.created_at));
	const canSign = $derived(identityCanSign(app.identityStatus));
	const ownsRoot = $derived(
		!!app.identityStatus?.pubkey && anchor.root_pubkey === app.identityStatus.pubkey
	);

	// Nostrdown in the note — `{{ref:slug}}` to the publication's sections —
	// resolves against the anchor's root, the same way a section resolves
	// against its containing index. Parsed and resolved engine-side; the
	// card only renders the spans.
	let ndRefs = $state<ResolvedRef[]>([]);
	let ndTokens = $state<ParsedToken[]>([]);
	$effect(() => {
		const text = ev.content;
		const root = anchor.root;
		const author = ev.pubkey;
		const key = insertion.coordinate;
		if (!text) {
			ndRefs = [];
			ndTokens = [];
			return;
		}
		let cancelled = false;
		api.parseNostrdown([{ key, content: text }])
			.then((m) => {
				if (!cancelled) ndTokens = m[key] ?? [];
			})
			.catch(() => {});
		api.resolveNostrdownProgressive(
			[{ key, content: text, publication: root, author }],
			(m) => {
				if (!cancelled) ndRefs = m[key] ?? [];
			},
			{ fetch: false }
		).catch(() => {});
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

	// The request this note answers: its title from the local store, so the
	// chip reads "answers: <ask>" rather than a d-tag. Engine derived the
	// pointer (`insertion.request`); the card only looks the title up.
	let requestTitle = $state<string | null>(null);
	$effect(() => {
		const coord = insertion.request;
		requestTitle = null;
		if (!coord) return;
		const [k, pk, ...d] = coord.split(':');
		let cancelled = false;
		api.queryEvents([{ kinds: [Number(k)], authors: [pk], '#d': [d.join(':')] }], 'local_only')
			.then((resp) => {
				if (cancelled) return;
				const evs = (resp.events ?? []) as { created_at: number; tags: string[][] }[];
				evs.sort((a, b) => b.created_at - a.created_at);
				requestTitle = evs[0]?.tags.find((x) => x[0] === 'title')?.[1] ?? null;
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	});
	function openRequest() {
		const coord = insertion.request;
		if (!coord) return;
		const [k, pk, ...d] = coord.split(':');
		app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}

	// Fold (narrow layout only — CSS decides when the row shows). Local
	// state unless the reader binds it.
	let expanded = $state(false);
	const isOpen = $derived(open ?? expanded);
	function toggle() {
		if (open === undefined) expanded = !expanded;
		else open = !open;
	}

	// --- Promotion --------------------------------------------------------
	let promoteOpen = $state(false);
	let preview = $state<api.ForkPreview | null>(null);
	let previewErr = $state<string | null>(null);
	let previewing = $state(false);
	let publishing = $state(false);

	async function openPromote() {
		promoteOpen = !promoteOpen;
		if (!promoteOpen || preview) return;
		previewing = true;
		previewErr = null;
		try {
			preview = await api.previewFork({
				index: anchor.root,
				insertion: insertion.coordinate
			});
		} catch (e) {
			previewErr = api.errorMessage(e, 'preview failed');
		} finally {
			previewing = false;
		}
	}

	async function publishPromotion() {
		if (!preview || publishing) return;
		publishing = true;
		try {
			const resp = await api.forkPublication({
				index: anchor.root,
				insertion: insertion.coordinate,
				slot: preview.slot
			});
			const results = resp.broadcast_results ?? [];
			const ok = results.filter((r) => r.success).length;
			app.pushToast(
				resp.derivative
					? `Derivative index published (${ok}/${results.length} relays)`
					: `Index republished with the note in (${ok}/${results.length} relays)`,
				ok === 0 && results.length > 0 ? 'error' : 'success'
			);
			promoteOpen = false;
			onpromoted?.(resp);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Promotion failed'), 'error', 5000);
		} finally {
			publishing = false;
		}
	}

	function slotLabel(p: api.ForkPreview): string {
		if (p.after) return `after ${p.after.split(':').slice(2).join(':')}`;
		return 'first';
	}
</script>

{#if insertion.role !== 'request'}
<div
	class="icard icard--{reading.notes}"
	class:icard--open={isOpen}
	class:icard--insert={anchor.placement !== 'annotation'}
	data-insertion={insertion.coordinate}
	style="--au: {pubkeyToColor(ev.pubkey)}"
>
	<!-- Narrow-layout fold row: one 44 px line under the anchor. -->
	<button class="icard__fold" onclick={toggle} aria-expanded={isOpen}>
		<span class="icard__fold-cap">{placementLabel}</span>
		<span class="icard__fold-title">{title || 'untitled'}</span>
		<span class="icard__fold-by"><span class="icard__dot" aria-hidden="true"></span>{who} · {ago}</span>
		<span class="icard__fold-ptr" aria-hidden="true">{isOpen ? '▾' : '▸'}</span>
	</button>

	<div class="icard__body">
		{#if reading.notes === 'sticky'}
			<span class="icard__tape" aria-hidden="true"></span>
		{/if}
		<div class="icard__who">
			<span class="icard__cap">{placementLabel}</span>
			<span class="icard__by"><span class="icard__dot" aria-hidden="true"></span>{who} · {ago} · pending</span>
		</div>
		{#if title}
			<h4 class="icard__title">{title}</h4>
		{/if}
		{#if contextTag}
			<blockquote class="icard__context">{contextTag}</blockquote>
		{/if}
		{#if insertion.role === 'bundle'}
			<div class="icard__text icard__bundle">
				A kasten of {insertion.children?.length ?? 0} entries, attached to the whole publication.
				<button
					class="icard__act"
					onclick={() => app.openAddressableInModal({ kind: 30040, pubkey: ev.pubkey, d_tag: insertion.coordinate.split(':').slice(2).join(':') })}
					title="Open the bundle as a publication"
				>open kasten</button>
			</div>
		{:else}
		<div class="icard__text">
			<RichContent content={ev.content} refs={ndRefs} tokens={ndTokens} />
					<EventLinks tags={ev.tags} compact />
		</div>
		{/if}
		<div class="icard__actions">
			<span class="icard__pill icard__pill--att">
				{anchor.placement === 'annotation' ? 'attached' : 'inserted · pending'}
			</span>
			{#if insertion.request}
				<button class="icard__act icard__act--answers" onclick={openRequest} title="Written for this request — {insertion.request}">↳ answers {requestTitle ?? 'a request'}</button>
			{/if}
			{#if styleCoord}
				<button class="icard__act" onclick={() => app.handleSearch(`style:${styleCoord}`, { scopeToMe: false })} title="Written in this style — click for everything in it ({styleCoord})">✎ {styleTitle(styleCoord) ?? 'style'}</button>
			{/if}
			{#if onopen}
				<button class="icard__act" onclick={() => onopen?.(insertion)} title="Open this section in its own reader buffer">open</button>
			{/if}
			<button
				class="icard__act"
				onclick={() => app.openAddressableInModal({ kind: insertion.role === 'bundle' ? 30040 : 30041, pubkey: ev.pubkey, d_tag: insertion.coordinate.split(':').slice(2).join(':') })}
				title="Event menu — JSON, comment, broadcast, pool"
			>menu</button>
			<span class="icard__spacer"></span>
			<button
				class="icard__act icard__act--promote"
				class:on={promoteOpen}
				onclick={openPromote}
				disabled={!canSign}
				title={canSign
					? ownsRoot
						? 'Fold this note into the publication: republish your index with it in'
						: 'Fold this note in by publishing your own derivative of this publication'
					: 'Sign in to promote'}
			>promote</button>
		</div>

		{#if promoteOpen}
			<div class="icard__promote">
				{#if previewing}
					<div class="icard__promote-line">computing the copy…</div>
				{:else if previewErr}
					<div class="icard__promote-line icard__promote-line--bad">{previewErr}</div>
				{:else if preview}
					<div class="icard__promote-head">
						{preview.derivative ? 'Copy this publication with the note in' : 'Republish your index with the note in'}
					</div>
					<div class="icard__promote-line">
						{#if preview.derivative}
							You are not the author. This publishes a <strong>derivative index</strong> under your key
							(<code>p</code> + <code>E</code> to the original). The note itself is unchanged — its author's
							signature stands.
						{:else}
							A <strong>new version</strong> of your index (same <code>d</code>). Readers following
							<code>auto-update</code> see it; the note itself is unchanged.
						{/if}
					</div>
					<div class="icard__promote-line">
						<span class="icard__k">added</span>
						<code>a</code> at slot {preview.slot} — {slotLabel(preview)}
					</div>
					<div class="icard__promote-line">
						<span class="icard__k">copied verbatim</span> {preview.copied_tags} tags
					</div>
					<div class="icard__promote-actions">
						<button class="icard__act" onclick={() => (promoteOpen = false)} disabled={publishing}>cancel</button>
						<button
							class="icard__act icard__act--go"
							onclick={publishPromotion}
							disabled={publishing}
						>{publishing ? 'publishing…' : preview.derivative ? 'publish derivative' : 'publish new version'}</button>
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>
{/if}

<style>
	/* Direction B — flat margin card. The left rail is the author's colour
	   (`--au`, the highlight recipe — the same hue the annotations table
	   gives this author); the "local / not synced" hue stays on the
	   pending pill and the promote action. */
	.icard {
		--ic-accent: var(--id-local);
		position: relative;
		margin: 8px 0 6px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--ic-accent));
		border-radius: var(--r-sm, 3px);
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.45;
		color: var(--fg);
	}
	.icard--insert {
		/* An inserted page sits in the reading column like a slip between
		   pages; an annotation hugs the margin. */
		margin-inline: 0;
	}
	.icard__fold {
		display: none;
	}
	.icard__body {
		padding: 8px 12px 10px;
	}
	.icard__who {
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
		margin-bottom: 4px;
	}
	.icard__cap {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ic-accent);
	}
	.icard__by {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	/* Author dot — colour = author, as in highlights and on the table. */
	.icard__dot {
		display: inline-block;
		flex: none;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au, var(--ic-accent));
	}
	.icard__title {
		margin: 0 0 6px;
		font-size: var(--t-sm);
		font-weight: 600;
	}
	.icard__context {
		margin: 0 0 6px;
		padding: 2px 8px;
		border-left: 2px solid var(--panel-border);
		color: var(--fg-muted);
		font-style: italic;
	}
	.icard__text {
		font-family: var(--rd-font, var(--font-sans));
		font-size: var(--rd-size, var(--t-base));
		line-height: var(--rd-leading, 1.55);
	}
	.icard__actions {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-top: 8px;
		flex-wrap: wrap;
	}
	.icard__pill {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		line-height: 1.4;
		padding: 0 6px;
		border-radius: 3px;
		font-weight: 600;
		color: var(--base5);
		border: 1px solid var(--panel-border);
	}
	.icard__pill--att {
		color: var(--ic-accent);
		border-color: color-mix(in srgb, var(--ic-accent) 45%, transparent);
	}
	.icard__spacer {
		flex: 1;
	}
	.icard__act {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base6);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 2px 8px;
		min-height: 26px;
		cursor: pointer;
	}
	.icard__act--answers {
		color: var(--id-remote);
		border-style: dashed;
		max-width: 22ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.icard__act:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.icard__act:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.icard__act--promote {
		color: var(--ic-accent);
	}
	.icard__act--promote.on {
		background: var(--ic-accent);
		color: var(--bg);
	}
	.icard__act--go {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.icard__promote {
		margin-top: 8px;
		padding: 8px 10px;
		border: 1px dashed var(--panel-border);
		border-radius: var(--r-sm);
		font-size: calc(var(--t-xs) - 1px);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.icard__promote-head {
		font-weight: 600;
	}
	.icard__promote-line {
		color: var(--fg-muted);
	}
	.icard__promote-line--bad {
		color: var(--error, #e06c75);
	}
	.icard__promote-line code {
		font-family: var(--font-mono);
		color: var(--fg);
	}
	.icard__k {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin-right: 6px;
	}
	.icard__promote-actions {
		display: flex;
		gap: 6px;
		justify-content: flex-end;
		margin-top: 4px;
	}

	/* --- sticky skin: same structure, dressed as a post-it --------------- */
	.icard--sticky {
		background: color-mix(in srgb, var(--au, var(--id-local)) 14%, transparent);
		border-color: transparent;
		border-left: 3px solid var(--au, var(--id-local));
		border-radius: 2px;
		box-shadow: 1px 2px 6px rgba(0, 0, 0, 0.18);
		transform: rotate(-0.4deg);
	}
	.icard--sticky.icard--insert {
		transform: none;
	}
	.icard--sticky .icard__tape {
		position: absolute;
		top: -6px;
		left: 50%;
		width: 44px;
		height: 12px;
		margin-left: -22px;
		background: color-mix(in srgb, var(--fg) 12%, transparent);
		transform: rotate(-2deg);
		pointer-events: none;
	}
	.icard--sticky .icard__text {
		font-family: var(--font-sans);
	}

	/* --- Direction C: below the gutter width, fold into a 44 px row ------ */
	@media (max-width: 768px) {
		.icard__fold {
			display: flex;
			align-items: center;
			gap: 8px;
			width: 100%;
			min-height: 44px;
			padding: 0 10px;
			background: transparent;
			border: none;
			color: var(--fg);
			font: inherit;
			text-align: left;
			cursor: pointer;
		}
		.icard__fold-cap {
			font-family: var(--font-mono);
			font-size: calc(var(--t-xs) - 3px);
			letter-spacing: 0.08em;
			text-transform: uppercase;
			color: var(--ic-accent);
			white-space: nowrap;
		}
		.icard__fold-title {
			flex: 1;
			overflow: hidden;
			text-overflow: ellipsis;
			white-space: nowrap;
			font-weight: 600;
		}
		.icard__fold-by {
			display: inline-flex;
			align-items: center;
			gap: 5px;
			font-family: var(--font-mono);
			font-size: calc(var(--t-xs) - 3px);
			color: var(--base5);
			white-space: nowrap;
		}
		.icard__fold-ptr {
			color: var(--base5);
		}
		.icard__body {
			display: none;
		}
		.icard--open .icard__body {
			display: block;
			border-top: 1px solid var(--panel-border);
		}
		.icard--open .icard__who {
			display: none;
		}
		.icard--sticky {
			transform: none;
		}
	}
</style>
