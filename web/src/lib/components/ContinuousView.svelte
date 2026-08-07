<script lang="ts">
	import { tick } from 'svelte';
	import type { LazySection } from '$lib/types';
	import { getAppState } from '$lib/state.svelte';
	import * as api from '$lib/api';

	import CommentThread from './CommentThread.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';
	import RichContent from './RichContent.svelte';
	import { threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import { type Highlight, type HighlightSpan } from '$lib/discussions/highlights';
	import { coordMatchesAddr, type ResolvedRef, type ParsedToken } from '$lib/nostr/nostrdown';
	import type { ResolutionTracker } from '$lib/nostr/resolution-progress.svelte';

	const app = getAppState();

	let {
		sections,
		publication = null,
		onload,
		onrefocus = null,
		onviewjson,
		highlightsFor = null,
		focusedHighlightId = null,
		threadsFor = null,
		focusedCommentId = null,
		publicationAtag = undefined,
		siblings = undefined,
		resolution = undefined
	}: {
		sections: LazySection[];
		publication?: { title: string | null; summary: string | null } | null;
		onload?: (index: number) => void;
		/** Refocus the reader on a nested 30040 index encountered inline. */
		onrefocus?: ((section: LazySection) => void) | null;
		/** Kebab affordance per section — opens the section's underlying
		 *  event in the structured JSON modal. */
		onviewjson?: (section: LazySection) => void;
		/** Lookup: section addr → highlights to overlay. */
		highlightsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => Highlight[]) | null;
		focusedHighlightId?: string | null;
		/** Lookup: section addr → thread tree. Pass null to suppress. */
		threadsFor?: ((addr: { kind: number; pubkey: string; d_tag: string }) => ThreadNode[]) | null;
		focusedCommentId?: string | null;
		/** Containing publication coordinate ("30040:pubkey:dtag") — context for
		 *  resolving nostrdown `{{ref:…}}` sibling references. */
		publicationAtag?: string | undefined;
		/** Unsigned-draft siblings (title + synthetic d-tag) so `{{ref:…}}`
		 *  resolves against the draft's own sections in the preview, before
		 *  anything is published. Mutually exclusive with `publicationAtag`. */
		siblings?: { title?: string; d_tag: string }[] | undefined;
		/** The reader's resolution-progress tracker (threaded, not context). */
		resolution?: ResolutionTracker;
	} = $props();

	function addrKey(addr: { kind: number; pubkey: string; d_tag: string }): string {
		return `${addr.kind}:${addr.pubkey}:${addr.d_tag}`;
	}

	// Highlight spans per section, resolved engine-side in one batched round
	// trip (POST /highlights/resolve) for every loaded section that has
	// highlights. Stored async into a map keyed by section addr; the template
	// slices content by these via `segmentsFromSpans`. Re-runs when the section
	// set or any section's highlights change.
	let spansBySection = $state<Record<string, HighlightSpan[]>>({});
	$effect(() => {
		const items: { key: string; content: string; highlights: Highlight[] }[] = [];
		for (const s of sections) {
			if (s.status !== 'loaded' || !s.content || !s.addr) continue;
			const hls = highlightsFor ? highlightsFor(s.addr) : [];
			if (hls.length === 0) continue;
			items.push({ key: addrKey(s.addr), content: s.content, highlights: hls });
		}
		if (items.length === 0) {
			spansBySection = {};
			return;
		}
		let cancelled = false;
		api.resolveHighlights(items)
			.then((m) => {
				if (!cancelled) spansBySection = m;
			})
			.catch(() => {
				if (!cancelled) spansBySection = {};
			});
		return () => {
			cancelled = true;
		};
	});

	// Nostrdown `{{ }}` references per section, resolved engine-side in one
	// batched round trip (POST /nostrdown/resolve) for every loaded section whose
	// content carries a `{{` token. Keyed by section addr; `RichContent` merges
	// these with the highlight spans. The publication coordinate scopes `ref:`
	// sibling lookups; each section's own pubkey scopes `wiki:`.
	let refsBySection = $state<Record<string, ResolvedRef[]>>({});
	// Pre-resolution "resolving" chip spans, parsed engine-side in parallel with
	// resolve (parse is pure + fast, so chips land first; resolve supersedes).
	let tokensBySection = $state<Record<string, ParsedToken[]>>({});
	$effect(() => {
		const items: {
			key: string;
			content: string;
			publication?: string;
			author?: string;
			siblings?: { title?: string; d_tag: string }[];
		}[] = [];
		for (const s of sections) {
			if (s.status !== 'loaded' || !s.content || !s.addr) continue;
			if (!(s.content.includes('{{') || s.content.includes('[['))) continue;
			items.push({
				key: addrKey(s.addr),
				content: s.content,
				publication: publicationAtag,
				author: s.addr.pubkey,
				siblings
			});
		}
		if (items.length === 0) {
			refsBySection = {};
			tokensBySection = {};
			return;
		}
		let cancelled = false;
		api.parseNostrdown(items.map((i) => ({ key: i.key, content: i.content })))
			.then((m) => {
				if (!cancelled) tokensBySection = m;
			})
			.catch(() => {
				if (!cancelled) tokensBySection = {};
			});
		api.resolveNostrdown(items)
			.then((m) => {
				if (!cancelled) refsBySection = m;
			})
			.catch(() => {
				if (!cancelled) refsBySection = {};
			});
		return () => {
			cancelled = true;
		};
	});

	// ── Tree collapse model ─────────────────────────────────────────────
	// `sections` is the depth-N TOC flattened depth-first: a 30040 index
	// entry is immediately followed by its descendants (greater `depth`)
	// until the next entry at its own depth or shallower. The continuous
	// view honours that shape — a nested 30040 is a collapsible folder, not
	// a flat dump of every leaf. Indices start collapsed, so the reader
	// sees one level (the indices + their 30041 leaves) and expands deeper
	// levels deliberately — by index, or all at once.
	let expandedByAddr = $state<Record<string, boolean>>({});
	function isExpanded(addr: { kind: number; pubkey: string; d_tag: string }): boolean {
		return expandedByAddr[addrKey(addr)] ?? false;
	}
	function toggleIndex(addr: { kind: number; pubkey: string; d_tag: string }) {
		const k = addrKey(addr);
		expandedByAddr = { ...expandedByAddr, [k]: !(expandedByAddr[k] ?? false) };
	}

	const isIndex = (s: LazySection) => s.addr?.kind === 30040;

	// Companion (preamble) sections: a 30041 sharing pubkey + d-tag with a
	// 30040 index is that index's own body text — the tendrl/kasten authoring
	// convention. Its title duplicates the index header rendered immediately
	// above it, so the title row is suppressed and only content shows (often
	// nothing: an index with no text before its first subheading publishes an
	// empty companion). The root index isn't in `sections`, so its companion
	// is matched against publicationAtag.
	const companionOwnerKeys = $derived.by(() => {
		const keys = new Set<string>();
		for (const s of sections) {
			if (s.addr && isIndex(s)) keys.add(`${s.addr.pubkey}:${s.addr.d_tag}`);
		}
		if (publicationAtag) {
			const [, pubkey, ...d] = publicationAtag.split(':');
			if (pubkey && d.length) keys.add(`${pubkey}:${d.join(':')}`);
		}
		return keys;
	});
	const isCompanion = (s: LazySection) =>
		!!s.addr && !isIndex(s) && companionOwnerKeys.has(`${s.addr.pubkey}:${s.addr.d_tag}`);

	// Per-index child bookkeeping, keyed by the entry's position in
	// `sections`: how many *direct* children (one level down) it carries
	// and whether any descendants were loaded at all. An index with zero
	// descendants sits beyond the depth horizon — it can only be reached
	// by refocusing, not expanded in place.
	const childInfo = $derived.by(() => {
		const info = new Map<number, { direct: number; descendants: number }>();
		for (let i = 0; i < sections.length; i++) {
			if (!isIndex(sections[i])) continue;
			const d = sections[i].depth ?? 0;
			let direct = 0;
			let descendants = 0;
			for (let j = i + 1; j < sections.length; j++) {
				const dj = sections[j].depth ?? 0;
				if (dj <= d) break;
				descendants++;
				if (dj === d + 1) direct++;
			}
			info.set(i, { direct, descendants });
		}
		return info;
	});

	const indexCount = $derived(sections.filter(isIndex).length);

	// Flatten the tree to the rows actually on screen: skip every entry
	// that sits under a collapsed index. `hideDeeperThan` holds the depth
	// of the nearest collapsed ancestor; any entry deeper than it is
	// hidden until an entry at that depth or shallower closes the run.
	const visibleRows = $derived.by(() => {
		const rows: { section: LazySection; index: number }[] = [];
		let hideDeeperThan: number | null = null;
		for (let i = 0; i < sections.length; i++) {
			const s = sections[i];
			const d = s.depth ?? 0;
			if (hideDeeperThan !== null) {
				if (d > hideDeeperThan) continue;
				hideDeeperThan = null;
			}
			rows.push({ section: s, index: i });
			if (
				isIndex(s) &&
				s.addr &&
				!isExpanded(s.addr) &&
				(childInfo.get(i)?.descendants ?? 0) > 0
			) {
				hideDeeperThan = d;
			}
		}
		return rows;
	});

	export function expandAll() {
		const next: Record<string, boolean> = {};
		for (const s of sections) {
			if (isIndex(s) && s.addr) next[addrKey(s.addr)] = true;
		}
		expandedByAddr = next;
	}
	export function collapseAll() {
		expandedByAddr = {};
	}

	// Per-section thread toggles, keyed by addr string. Each section's
	// thread block can be collapsed independently in the continuous
	// view since they're all on screen at the same time. Closed by
	// default — auto-open below if a section contains the focused
	// comment from a `?focus_comment=<id>` marker.
	let threadOpenByAddr = $state<Record<string, boolean>>({});
	function isThreadOpen(addr: { kind: number; pubkey: string; d_tag: string }): boolean {
		return threadOpenByAddr[addrKey(addr)] ?? false;
	}
	function toggleThread(addr: { kind: number; pubkey: string; d_tag: string }) {
		const k = addrKey(addr);
		threadOpenByAddr[k] = !(threadOpenByAddr[k] ?? false);
	}

	$effect(() => {
		if (!focusedCommentId || !threadsFor) return;
		for (const s of sections) {
			if (!s.addr) continue;
			const t = threadsFor(s.addr);
			if (threadContainsId(t, focusedCommentId)) {
				threadOpenByAddr[addrKey(s.addr)] = true;
			}
		}
	});

	let containerEl: HTMLDivElement | undefined = $state();

	// In-document navigation for nostrdown refs: a ref/wikilink that resolved
	// to a sibling section hidden under a collapsed index (RichContent's own
	// DOM scroll already handles visible rows) expands its ancestors and
	// scrolls to it, instead of popping the target out into a new buffer.
	function openLocalSection(coord: string): boolean {
		const idx = sections.findIndex((s) => s.addr && coordMatchesAddr(coord, s.addr));
		// Index rows aren't readable in place — let those refocus/pop out.
		if (idx < 0 || isIndex(sections[idx])) return false;
		scrollToSection(idx);
		return true;
	}

	/** Expand the target's collapsed ancestors and scroll it into view.
	 *  Exported for the reader's mobile TOC drawer (bind:this). */
	export function scrollToSection(idx: number) {
		if (idx < 0 || idx >= sections.length) return;
		const next = { ...expandedByAddr };
		let d = sections[idx].depth ?? 0;
		for (let j = idx - 1; j >= 0 && d > 0; j--) {
			const s = sections[j];
			const dj = s.depth ?? 0;
			if (dj < d && isIndex(s) && s.addr) {
				next[addrKey(s.addr)] = true;
				d = dj;
			}
		}
		expandedByAddr = next;
		tick().then(() => {
			containerEl
				?.querySelector(`[data-section-index="${idx}"]`)
				?.scrollIntoView({ behavior: 'smooth', block: 'start' });
		});
	}

	// Visibility-driven lazy load. Re-runs whenever `visibleRows` changes
	// so sections revealed by an expand get observed too — a one-shot
	// onMount observer would miss every row mounted after a toggle.
	$effect(() => {
		if (!containerEl || !onload) return;
		// Depend on the visible set so the observer is rebuilt on expand.
		visibleRows;

		const observer = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (!entry.isIntersecting) continue;
					const idx = Number((entry.target as HTMLElement).dataset.sectionIndex);
					if (isNaN(idx)) continue;
					const section = sections[idx];
					if (section && section.status === 'pending') {
						onload!(idx);
						// Read-ahead: prefetch next 2
						if (idx + 1 < sections.length) onload!(idx + 1);
						if (idx + 2 < sections.length) onload!(idx + 2);
					}
				}
			},
			{
				root: containerEl,
				rootMargin: '200px 0px 400px 0px'
			}
		);

		containerEl
			.querySelectorAll('[data-section-index]')
			.forEach((el) => observer.observe(el));

		return () => observer.disconnect();
	});
</script>

<div class="continuous-view" bind:this={containerEl}>
	<!-- No doc title or summary here — the reader's title row owns both
	     (title rendered three times stacked on a phone; the summary now
	     lives in the reader's collapsible summary drawer, every view).
	     Expand/collapse-all moved to the reader's depth toolbar row. -->

	{#each visibleRows as row, ri (`${row.index}:${row.section.addr?.pubkey ?? ''}:${row.section.addr?.d_tag ?? ''}`)}
		{@const section = row.section}
		{@const i = row.index}
		{#if isIndex(section)}
			{@const info = childInfo.get(i)}
			{@const direct = info?.direct ?? 0}
			{@const loadable = (info?.descendants ?? 0) > 0}
			{@const open = section.addr ? isExpanded(section.addr) : false}
			<div class="cv-index" class:cv-index--open={open} style="--depth:{section.depth ?? 0}">
				<button
					class="cv-index__main"
					onclick={() => section.addr && toggleIndex(section.addr)}
					disabled={!loadable}
					aria-expanded={open}
					title={loadable
						? open
							? 'Collapse this nested publication'
							: 'Expand this nested publication'
						: 'Nested publication — refocus to load its contents'}
				>
					<span class="cv-index__caret" aria-hidden="true"
						>{loadable ? (open ? '▾' : '▸') : '·'}</span
					>
					<span class="cv-index__icon" aria-hidden="true">⊞</span>
					<span class="cv-index__title">{section.title || 'Nested publication'}</span>
					{#if loadable}
						<span class="cv-index__count">{direct} {direct === 1 ? 'item' : 'items'}</span>
					{:else}
						<span class="cv-index__count cv-index__count--empty">not loaded</span>
					{/if}
				</button>
				{#if onrefocus}
					<button
						class="cv-index__refocus"
						onclick={() => onrefocus?.(section)}
						title="Refocus the reader on this nested publication"
					>refocus ⟳</button>
				{/if}
			</div>
		{:else}
			<div
				class="continuous-section"
				style="--depth:{section.depth ?? 0}"
				data-section-index={i}
				data-section-addr={section.addr ? `${section.addr.kind}:${section.addr.pubkey}:${section.addr.d_tag}` : undefined}
			>
				{#if !isCompanion(section) && (section.title || onviewjson)}
					<h3 class="section-title">
						<span class="section-title__text">{section.title ?? ''}</span>
						<PoolStateBadges
							item={app.findPoolItemByAddr(section.addr)}
							onpillctx={() => app.pillActionByAddr(section.addr, 'context')}
							onpillcmp={() => app.pillActionByAddr(section.addr, 'compose')}
							onpilldrop={() => app.pillActionByAddr(section.addr, 'drop')}
							signed={section.signed}
							relays={section.relays}
							orientation="horizontal"
						/>
						{#if onviewjson}
							<button
								class="section-menu"
								onclick={() => onviewjson?.(section)}
								title="Open this section's event menu (m)"
							>menu</button>
						{/if}
					</h3>
				{/if}
				{#if section.status === 'loaded'}
					<!-- Loaded-and-empty renders nothing. It used to fall through
					     to the pending skeleton below and pulse forever — empty
					     companion sections looked permanently "waiting to load". -->
					{#if section.content}
						{@const k = section.addr ? addrKey(section.addr) : ''}
						<RichContent
							content={section.content}
							spans={k ? spansBySection[k] ?? [] : []}
							refs={k ? refsBySection[k] ?? [] : []}
							tokens={k ? tokensBySection[k] ?? [] : []}
							{resolution}
							{focusedHighlightId}
							onopenlocal={openLocalSection}
						/>
					{/if}
				{:else if section.status === 'loading'}
					<div class="skeleton"></div>
				{:else if section.status === 'error'}
					<p class="section-error">{section.error ?? 'Failed to load'}</p>
				{:else}
					<div class="skeleton pending"></div>
				{/if}
				{#if threadsFor && section.addr}
					{@const t = threadsFor(section.addr)}
					{#if t.length > 0}
						<div class="cv-threads">
							<button
								class="cv-threads-head"
								onclick={() => toggleThread(section.addr)}
								aria-expanded={isThreadOpen(section.addr)}
							>
								<span class="ptr">{isThreadOpen(section.addr) ? '▾' : '▸'}</span>
								Comments ({t.length})
							</button>
							{#if isThreadOpen(section.addr)}
								<CommentThread nodes={t} focusedEventId={focusedCommentId} />
							{/if}
						</div>
					{/if}
				{/if}
			</div>
		{/if}
		{#if ri < visibleRows.length - 1 && !isIndex(section) && !(isCompanion(section) && !section.content)}
			<!-- An empty companion renders nothing at all — no divider either,
			     or every index would be followed by a stray rule. -->
			<hr class="section-divider" />
		{/if}
	{/each}
	{#if sections.length === 0}
		<p class="empty">No sections loaded</p>
	{/if}
</div>

<style>
	.continuous-view {
		flex: 1;
		overflow-y: auto;
		padding: 16px;
	}
	/* Phone reading: roomier gutters, larger headings, clear of the bottom
	   bar / home-indicator region. */
	@media (max-width: 768px) {
		.continuous-view {
			padding: 14px 16px calc(24px + env(safe-area-inset-bottom));
		}
		.section-title {
			font-size: var(--t-base);
		}
	}

	.continuous-section {
		padding: 8px 0;
		padding-left: calc(var(--depth, 0) * 18px);
	}

	/* Inline nested-publication node — a collapsible folder. The caret
	   expands its children inline; `refocus` re-roots the reader on it. */
	.cv-index {
		display: flex;
		align-items: stretch;
		gap: 6px;
		margin: 6px 0;
		margin-left: calc(var(--depth, 0) * 18px);
	}
	.cv-index__main {
		display: flex;
		align-items: center;
		gap: 8px;
		flex: 1;
		min-width: 0;
		padding: 8px 12px;
		border: 1px dashed var(--border);
		border-radius: var(--radius);
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
		color: var(--fg);
		cursor: pointer;
		text-align: left;
	}
	.cv-index__main:hover:not(:disabled) {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.cv-index__main:disabled { cursor: default; opacity: 0.65; }
	.cv-index--open .cv-index__main {
		border-style: solid;
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
	}
	.cv-index__caret {
		min-width: 1ch;
		color: var(--id-yours);
		font-size: var(--t-2xs);
	}
	.cv-index__icon { color: var(--id-yours); font-size: var(--t-md); }
	.cv-index__title {
		font-weight: 600;
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.cv-index__count {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		white-space: nowrap;
	}
	.cv-index__count--empty { font-style: italic; }
	.cv-index__refocus {
		background: none;
		border: 1px dashed var(--border);
		border-radius: var(--radius);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 0 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.cv-index__refocus:hover {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}

	.section-title {
		font-size: var(--t-sm);
		font-weight: 600;
		margin-bottom: 6px;
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	.section-title__text { flex: 1; min-width: 0; }
	/* Pill-shaped "menu" chip — matches the feed and outline rows so the
	   affordance reads the same across the reader. */
	.section-menu {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1.4;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base6);
		cursor: pointer;
	}
	.section-menu:hover {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}

	/* Section body now renders via RichContent (owns `.section-content` +
	   highlight/nostrdown overlay styles). */

	.skeleton {
		height: 60px;
		background: var(--border);
		border-radius: 4px;
		animation: pulse 1.5s ease-in-out infinite;
	}

	.skeleton.pending {
		opacity: 0.4;
	}

	@keyframes pulse {
		0%, 100% { opacity: 0.3; }
		50% { opacity: 0.6; }
	}

	.section-error {
		color: var(--danger);
		font-size: var(--t-2xs);
	}

	.section-divider {
		border: none;
		border-top: 1px solid var(--border);
		margin: 4px 0;
		opacity: 0.5;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: var(--t-xs);
	}
	.cv-threads {
		margin-top: 10px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
	}
	.cv-threads-head {
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
	.cv-threads-head:hover { color: var(--fg); }
	.cv-threads-head .ptr { min-width: 1ch; }
</style>
