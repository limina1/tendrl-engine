<script lang="ts">
	import * as api from '$lib/api';
	import type { LazySection } from '$lib/types';
	import { threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import type { Highlight } from '$lib/discussions/highlights';
	import { getAppState } from '$lib/state.svelte';
	import SectionCard from './SectionCard.svelte';
	import InsertionCard from './InsertionCard.svelte';
	import RequestCard from './RequestCard.svelte';
	import { coordMatchesAddr } from '$lib/nostr/nostrdown';
	import type { ResolutionTracker } from '$lib/nostr/resolution-progress.svelte';
	import { readingVars } from '$lib/theme/reading.svelte';
	import CommentThread from './CommentThread.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';

	const app = getAppState();

	let {
		sections,
		currentSection = 0,
		onnavigate,
		onload,
		onrefocus = null,
		onsectionjson,
		highlightsFor = null,
		focusedHighlightId = null,
		threadsFor = null,
		focusedCommentId = null,
		insertionsFor = null,
		rootInsertions = null,
		onopeninsertion = undefined,
		oninsertionpromoted = undefined,
		requestsFor = null,
		rootRequests = null,
		onrequestschanged = undefined,
		publicationAtag = undefined,
		siblings = undefined,
		resolution = undefined,
		hotAddr = null,
		notesMargin = false
	}: {
		sections: LazySection[];
		currentSection?: number;
		onnavigate: (index: number) => void;
		onload?: (index: number) => void;
		/** Refocus the reader on a nested 30040 index. When set, a 30040
		 *  entry renders as a refocus card instead of a (non-readable)
		 *  section body. */
		onrefocus?: ((section: LazySection) => void) | null;
		/** Open the section's underlying event in the structured JSON modal.
		 *  Surfaces on the right margin of the pager. */
		onsectionjson?: (index: number) => void;
		/** Lookup: section addr → all highlights to overlay on its content. */
		highlightsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => Highlight[]) | null;
		/** Id of the highlight to emphasize (from ?highlight= marker). */
		focusedHighlightId?: string | null;
		/** Lookup: given a section addr, return the NIP-22 thread tree to
		 *  render beneath it. Pass null to suppress inline threads. */
		threadsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => ThreadNode[]) | null;
		focusedCommentId?: string | null;
		/** Lookup: section addr → loaded NKBIP-01 insertions anchored to it. */
		insertionsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => api.LoadedInsertion[]) | null;
		/** Insertions anchored to the index itself: shown on the first / last page. */
		rootInsertions?: { before: api.LoadedInsertion[]; after: api.LoadedInsertion[] } | null;
		onopeninsertion?: ((ins: api.LoadedInsertion) => void) | undefined;
		oninsertionpromoted?: ((resp: api.ForkPublishResponse) => void) | undefined;
		/** Lookup: section addr → annotation requests asked on it. */
		requestsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => api.AnnotationRequest[]) | null;
		/** Requests on the publication itself — shown on the first page. */
		rootRequests?: api.AnnotationRequest[] | null;
		onrequestschanged?: (() => void) | undefined;
		/** Containing publication coordinate ("30040:pubkey:dtag") — context for
		 *  resolving nostrdown `{{ref:…}}` sibling references. */
		publicationAtag?: string | undefined;
		/** Unsigned-draft siblings (title + synthetic d-tag) so `{{ref:…}}`
		 *  resolves against the draft's sections in the preview, pre-publish. */
		siblings?: { title?: string; d_tag: string }[] | undefined;
		/** The reader's resolution-progress tracker (threaded, not context). */
		resolution?: ResolutionTracker;
		/** The annotations table's hovered card anchor — the page lights up
		 *  (`.anchor-hot`) with a "N cards →" tip when it is that section. */
		hotAddr?: { root: string; addr: string; count: number } | null;
		/** D9: text | notes beside each other (a 280 px gutter past the
		 *  measure) instead of notes under the page. The reader decides. */
		notesMargin?: boolean;
	} = $props();

	const section = $derived(sections[currentSection]);
	const total = $derived(sections.length);
	const hot = $derived(
		!!hotAddr &&
			!!section &&
			`${section.addr.kind}:${section.addr.pubkey}:${section.addr.d_tag}` === hotAddr.addr
	);
	// Every card this page carries: the publication's own requests and
	// front matter on the first page, the appendix on the last, and the
	// section's own requests + notes in between.
	const pageRequests = $derived.by(() => {
		if (!section) return [] as api.AnnotationRequest[];
		const out: api.AnnotationRequest[] = [];
		if (currentSection === 0 && rootRequests) out.push(...rootRequests);
		if (requestsFor) out.push(...requestsFor(section.addr));
		return out;
	});
	const pageInsertions = $derived.by(() => {
		if (!section) return [] as api.LoadedInsertion[];
		const out: api.LoadedInsertion[] = [];
		if (currentSection === 0 && rootInsertions) out.push(...rootInsertions.before);
		if (insertionsFor) out.push(...insertionsFor(section.addr));
		if (currentSection === sections.length - 1 && rootInsertions) out.push(...rootInsertions.after);
		return out;
	});
	const threads = $derived(threadsFor && section ? threadsFor(section.addr) : []);
	const highlights = $derived(highlightsFor && section ? highlightsFor(section.addr) : []);

	// Per-section thread toggle. Closed by default — same posture as the
	// highlights drawer. Resets each time the user pages, with one
	// exception: when the focused comment (`?focus_comment=<id>`) lives
	// in this section's thread, snap open so the targeted reply is
	// actually visible.
	let threadsOpen = $state(false);
	$effect(() => {
		currentSection;
		threadsOpen = threadContainsId(threads, focusedCommentId);
	});

	let contentEl: HTMLDivElement | undefined = $state();

	// Load current section + prefetch adjacent
	$effect(() => {
		const idx = currentSection;
		onload?.(idx);
		if (idx > 0) onload?.(idx - 1);
		if (idx < total - 1) onload?.(idx + 1);
	});

	// Scroll to top on page change
	$effect(() => {
		currentSection;
		contentEl?.scrollTo(0, 0);
	});

	// Keydown is handled by ReaderBuffer's nav handler (registered via the
	// global buffer-store dispatcher). PaginatedView no longer attaches its
	// own listener — global j/k/arrow already drives onnavigate from there.

	// In-document navigation for nostrdown refs: a ref/wikilink that resolved
	// to a sibling section of this document pages to it (only the current
	// section is in the DOM, so RichContent's scroll-in-place can't reach it).
	function openLocalSection(coord: string): boolean {
		const idx = sections.findIndex((s) => s.addr && coordMatchesAddr(coord, s.addr));
		if (idx < 0) return false;
		if (idx !== currentSection) onnavigate(idx);
		return true;
	}
</script>

<div class="paginated-view">
	<div class="paginated-nav">
		<button onclick={() => onnavigate(currentSection - 1)} disabled={currentSection <= 0}>
			Prev
		</button>
		<span class="page-counter">
			{currentSection + 1} / {total}
			<span class="section-label">Section {currentSection + 1} of {total}</span>
		</span>
		<button
			onclick={() => onnavigate(currentSection + 1)}
			disabled={currentSection >= total - 1}
		>
			Next
		</button>
		<span class="pager-spacer"></span>
		{#if onsectionjson && section}
			<button
				class="pager-json"
				onclick={() => onsectionjson?.(currentSection)}
				title="Open this section's event menu (m)"
			>§ menu</button>
		{/if}
	</div>
	{#if section?.title}
		<div class="paginated-title">
			<span class="paginated-title__text">{section.title}</span>
			<PoolStateBadges
				item={app.findPoolItemByAddr(section.addr)}
				onpillctx={() => app.pillActionByAddr(section.addr, 'context')}
				onpillcmp={() => app.pillActionByAddr(section.addr, 'compose')}
				onpilldrop={() => app.pillActionByAddr(section.addr, 'drop')}
				signed={section.signed}
				relays={section.relays}
				orientation="horizontal"
			/>
		</div>
	{/if}
	<!-- Reading typography (face / size / measure / leading) rides on the
	     scroller; SectionCard centres itself in the measure. -->
	<div class="paginated-content" style={readingVars()} bind:this={contentEl}>
		{#if section && section.addr.kind === 30040}
			<button
				class="nested-page"
				onclick={() => onrefocus?.(section)}
				disabled={!onrefocus}
				title="Refocus the reader on this nested publication"
			>
				<span class="nested-page__icon" aria-hidden="true">⊞</span>
				<span class="nested-page__title"
					>{section.title || 'Nested publication'}</span>
				<span class="nested-page__hint">Nested publication — refocus ⟳</span>
			</button>
		{:else if section}
			<div
				class="section pv-section"
				class:anchor-hot={hot}
				class:notes-margin={notesMargin}
				data-section-addr={`${section.addr.kind}:${section.addr.pubkey}:${section.addr.d_tag}`}
			>
				{#if hot}
					<span class="anchor-tip">{hotAddr?.count ?? 0} {hotAddr?.count === 1 ? 'card' : 'cards'} →</span>
				{/if}
				<div class="section__main">
					<SectionCard
						{section}
						{highlights}
						{focusedHighlightId}
						{publicationAtag}
						{siblings}
						{resolution}
						onopenlocal={openLocalSection}
					/>
				</div>
				{#if pageRequests.length > 0 || pageInsertions.length > 0}
					<div class="section__side">
						{#each pageRequests as rq (rq.id)}
							<RequestCard request={rq} onchanged={onrequestschanged} />
						{/each}
						{#each pageInsertions as ins (ins.coordinate)}
							<InsertionCard insertion={ins} onopen={onopeninsertion} onpromoted={oninsertionpromoted} />
						{/each}
					</div>
				{/if}
			</div>
			{#if threads.length > 0}
				<div class="paginated-threads">
					<button
						class="paginated-threads-head"
						onclick={() => (threadsOpen = !threadsOpen)}
						aria-expanded={threadsOpen}
					>
						<span class="ptr">{threadsOpen ? '▾' : '▸'}</span>
						Comments on this section ({threads.length})
					</button>
					{#if threadsOpen}
						<CommentThread nodes={threads} focusedEventId={focusedCommentId} />
					{/if}
				</div>
			{/if}
		{/if}
	</div>
</div>

<style>
	.paginated-view {
		display: flex;
		flex-direction: column;
		flex: 1;
		min-height: 0;
	}

	.paginated-title {
		padding: 10px 16px;
		font-size: var(--t-sm);
		font-weight: 700;
		border-bottom: 1px solid var(--border);
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.paginated-content {
		flex: 1;
		overflow-y: auto;
	}
	/* The page: SectionCard centres itself in the measure; the cards under
	   it (inline) share that column. Lit from the annotations table the same
	   way a scroll-view section is. */
	.pv-section {
		position: relative;
		max-width: var(--rd-measure, none);
		margin-inline: auto;
	}
	.pv-section > .section__side {
		padding: 0 16px 12px;
	}
	.section.anchor-hot {
		box-shadow: -3px 0 0 0 var(--id-local);
	}
	.section .anchor-tip {
		position: absolute;
		right: 16px;
		top: 12px;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		font-weight: 600;
		line-height: 1.2;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--id-local);
		pointer-events: none;
		z-index: 1;
	}
	/* D9: text | notes. The page widens by the gutter; the card keeps the
	   measure in column 1, the notes stack in column 2 with a dashed rail. */
	.pv-section.notes-margin {
		max-width: calc(var(--rd-measure, 0px) + 308px);
		display: grid;
		grid-template-columns: minmax(0, var(--rd-measure, 1fr)) 280px;
		column-gap: 28px;
		align-items: start;
	}
	.pv-section.notes-margin > .section__main {
		grid-column: 1;
		min-width: 0;
	}
	.pv-section.notes-margin > .section__side {
		grid-column: 2;
		position: relative;
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 14px 0 0;
	}
	.pv-section.notes-margin > .section__side::before {
		content: '';
		position: absolute;
		left: -14px;
		top: 18px;
		bottom: 6px;
		border-left: 1px dashed var(--border);
	}
	.pv-section.notes-margin.anchor-hot > .section__side::before {
		border-left-color: var(--id-local);
		border-left-style: solid;
	}
	.pv-section.notes-margin .section__side :global(.icard),
	.pv-section.notes-margin .section__side :global(.rcard) {
		margin: 0;
	}
	.pv-section.notes-margin .section__side :global(.icard__text) {
		font-size: var(--t-xs);
		line-height: 1.45;
	}

	.paginated-nav {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		flex-shrink: 0;
	}
	.pager-spacer { flex: 1; }
	.pager-json {
		background: none;
		border: 1px solid var(--border);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		padding: 2px 8px;
		border-radius: var(--radius);
		cursor: pointer;
	}
	.pager-json:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-color: var(--id-yours);
	}

	.page-counter {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		min-width: 60px;
		text-align: center;
	}

	.section-label {
		margin-left: 8px;
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		opacity: 0.7;
	}

	.nested-page {
		display: flex;
		flex-direction: column;
		gap: 6px;
		width: calc(100% - 32px);
		margin: 16px;
		padding: 20px;
		border: 1px dashed var(--border);
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
		color: var(--fg);
		cursor: pointer;
		text-align: left;
	}
	.nested-page:hover:not(:disabled) {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.nested-page:disabled { cursor: default; opacity: 0.7; }
	.nested-page__icon { font-size: var(--t-xl); color: var(--id-yours); }
	.nested-page__title { font-size: var(--t-base); font-weight: 700; }
	.nested-page__hint {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
	}

	.paginated-threads {
		padding: 12px 16px;
		border-top: 1px solid var(--border);
		background: var(--bg);
	}
	.paginated-threads-head {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin-bottom: 6px;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: transparent;
		border: none;
		padding: 0;
		cursor: pointer;
	}
	.paginated-threads-head:hover { color: var(--fg); }
	.paginated-threads-head .ptr {
		min-width: 1ch;
		display: inline-block;
	}
</style>
