<script lang="ts">
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import FocusGraph, { type GraphNode } from '$lib/components/FocusGraph.svelte';
	import SectionCard from '$lib/components/SectionCard.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import PoolStateBadges from '$lib/components/PoolStateBadges.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import { ResolutionTracker } from '$lib/nostr/resolution-progress.svelte';
	import type {
		LazySection,
		NAddr,
		PubLoadEvent,
		PublicationDetail,
		TagEntry,
		ViewMode,
		ContextItem
	} from '$lib/types';
	import type { Buffer } from '../types';
	import { sectionState, segmentSections } from '$lib/compose/state';
	import { threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import type { Highlight } from '$lib/discussions/highlights';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import HighlightList from '$lib/components/HighlightList.svelte';
	import HighlightsDrawer, {
		type DrawerHighlight
	} from '$lib/components/HighlightsDrawer.svelte';
	import { prefetchAuthors, refreshAuthors } from '$lib/discussions/authors.svelte';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	// Aggregate nostrdown reference-resolution progress across this reader's
	// sections (each RichContent reports in via the `resolution` prop). Drives the
	// "N/M refs" indicator so a reference-heavy document shows it's still resolving
	// — the not-local embeds fetch from relays and can take a while.
	const resolution = new ResolutionTracker();
	const store = getActiveStore();

	let publication = $state<PublicationDetail | null>(null);
	let pristineSections = $state<LazySection[]>([]);

	// Walkthrough: the reader tour (reader-open → menu → edit) is opt-in only —
	// it's declared as this buffer kind's `tour` in the WM registry and runs
	// from the mode-line W, never auto-thrown on every publication load.

	// Cross-publication navigation has two axes.
	//
	// `focusStack` is the *breadcrumb*: an append-only list of the distinct
	// 30040 indexes ever focused, in first-visit order. `focusMarker` points
	// at the current focus — not necessarily the tail, since cycling back to
	// an already-visited index just moves the marker. Crumbs past the marker
	// are the loop tail, reachable ahead of the current focus.
	type FocusCrumb = { pubkey: string; d_tag: string; title: string };
	let focusStack = $state<FocusCrumb[]>([]);
	let focusMarker = $state(0);
	// `visitLog` is the *temporal* axis: every focus in entry order, revisits
	// included. `Prev Level` steps `visitIndex` back through it. There is no
	// Next — inward movement is the refocus action, not a forward step.
	let visitLog = $state<FocusCrumb[]>([]);
	let visitIndex = $state(0);
	// The accumulated 30040 reference graph behind the graph panel. Every
	// index event seen — across all focuses and depths — merges in here and
	// is never dropped, so the panel shows the structure discovered so far
	// and grows as depth rises or sub-publications are refocused into.
	// Keyed by addrKey (`kind:pubkey:d_tag`).
	let focusGraph = $state<Record<string, GraphNode>>({});
	let graphOpen = $state(false);
	// Target eager-expansion depth: how many levels of the tree are *fully*
	// loaded. `treeDepth` N loads levels 0..N-1 and leaves level N as d-tag
	// preview stubs — so the engine is asked for `treeDepth - 1` (its
	// `max_depth` counts the deepest fully-loaded level). Minimum 1: level 0
	// (the focused index itself) is always loaded.
	const MAX_DEPTH = 6;
	let treeDepth = $state(2);
	// Depth the current focus's tree has actually been streamed to. Lowering
	// `treeDepth` at or below this is a no-op — the deeper tree stays loaded
	// and ready, so no re-stream and no counter reset.
	let loadedDepth = $state(-1);
	// True while an SSE stream is actively loading the tree.
	let loaderRunning = $state(false);
	// Bumped on every fresh load (buffer change / refocus / breadcrumb /
	// depth change) so events from a superseded stream are discarded.
	let loadGeneration = 0;
	// The in-flight publication SSE stream — closed to abort the engine loader.
	let streamSource: EventSource | null = null;
	// Per-event load counter for the modeline: `resolvedCount` (i) is events
	// resolved so far; `knownCount` (N) is the in-horizon total, climbing as
	// indexes reveal their `a`-tag children. `loadingPath` is the truncated
	// root->node name path of the most recently resolved node.
	let resolvedCount = $state(0);
	let knownCount = $state(0);
	// `displayedCount` is `resolvedCount` ramped — it steps toward the real
	// count a chunk per frame, so a fast DB burst (~40 events resolved in one
	// frame) reads as a smooth climb rather than a jump. On a slow relay load
	// it just tracks the real count 1:1.
	let displayedCount = $state(0);
	let loadingPath = $state('');
	// Per-focus result cache, keyed by publication address. Refocusing into a
	// sub-publication and breadcrumbing back restores instantly from here
	// instead of re-streaming — stepping back out must not re-fetch. Only
	// completed loads are cached.
	type FocusCacheEntry = {
		publication: PublicationDetail;
		sections: LazySection[];
		/** Depth this focus's tree was streamed to. */
		loadedDepth: number;
	};
	const focusCache = new Map<string, FocusCacheEntry>();
	const focusKey = (pubkey: string, d_tag: string) => `${pubkey}:${d_tag}`;
	// A node in the streamed tree, keyed by addr; assembled as events arrive.
	type StreamNode = {
		addr: NAddr;
		title: string | null;
		isIndex: boolean;
		content: string | null;
		/** addr-keys of children, in tree order (from the index event). */
		childKeys: string[];
		/** addr-key of the parent index — set when the parent is processed.
		 *  Lets the modeline show the root->node name path of a loading node. */
		parentKey?: string;
		status: 'pending' | 'loaded' | 'error';
		error?: string;
		/** Provenance pulled from the matching PubLoadEvent. Stays undefined
		 *  for stub children (the index event named them but their own
		 *  Leaf/Index hasn't streamed yet) — PoolStateBadges suppresses the
		 *  pill on undefined so half-loaded rows don't lie. */
		relays?: string[];
		signed?: boolean;
		/** NIP-54 fork marker on this node's index event (Leaf nodes never
		 *  set it — the marker only lives on 30040 index events). */
		forked?: boolean;
	};
	// Default to outline. If the buffer carries `?highlight=<id>` the
	// effect below switches to paginated so the highlight overlay is
	// visible right away.
	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);
	let loading = $state(true);
	// Secondary line under "Loading…" while a standalone event/addressable
	// falls through the local cache to a relay round-trip.
	let loadingStatus = $state<string | null>(null);
	let error = $state<string | null>(null);

	// NIP-22 comments + NIP-84 highlights referencing each event. Keyed by
	// the `a` tag value (kind:pubkey:d-tag). Populated in two phases: a
	// fast local_only query for instant indicators, then if online a
	// fetch_always query to pick up new events from connected relays.
	let discussionCounts = $state<Record<string, api.DiscussionCount>>({});
	let discussionSource = $state<{ local_count: number; relay_count: number } | null>(null);
	let discussionLoading = $state(false);
	let discussionRefreshedAt = $state<number | null>(null);

	// Full discussion-event payloads keyed by section addr (kind:pk:dtag).
	// Comments and highlights both land here; downstream renderers split
	// by event.kind. Populated alongside discussionCounts so the reader's
	// inline threads stay in sync with the badges.
	let discussionEvents = $state<Record<string, api.DiscussionEvent[]>>({});

	const loadingPromises = new Map<number, Promise<void>>();

	function addrKey(addr: { kind: number; pubkey: string; d_tag: string }): string {
		return `${addr.kind}:${addr.pubkey}:${addr.d_tag}`;
	}

	function publicationAddresses(): string[] {
		if (!publication) return [];
		const keys = new Set<string>();
		keys.add(addrKey(publication.addr));
		for (const s of pristineSections) keys.add(addrKey(s.addr));
		return Array.from(keys);
	}

	async function loadDiscussionCounts(
		policy: 'local_only' | 'local_first' | 'fetch_always',
		options: { bypassOffline?: boolean } = {}
	) {
		const addrs = publicationAddresses();
		if (addrs.length === 0) return;
		try {
			// Pull the full event payloads so we can render inline threads,
			// plus the engine's per-address badge tallies (resp.counts) in the
			// same round trip. The badge counts are NOT re-derived here — the
			// engine reduces the same event set it returns (api.rs
			// tally_discussion_counts), so there's one source of truth.
			const resp = await api.getDiscussionList({
				addresses: addrs,
				kinds: [1111, 9802],
				policy,
				bypassOffline: options.bypassOffline,
				limit: 500,
				threaded: true
			});
			// Group the returned events by referenced address for inline thread
			// rendering (view concern). Tag scan is grouping-only; the counts
			// come pre-computed from the engine.
			const byAddr: Record<string, api.DiscussionEvent[]> = {};
			for (const a of addrs) byAddr[a] = [];
			const addrSet = new Set(addrs);
			for (const ev of resp.events) {
				const matched = new Set<string>();
				for (const tag of ev.tags) {
					if (!tag || tag.length < 2) continue;
					if (tag[0] !== 'a' && tag[0] !== 'A') continue;
					const value = tag[1];
					if (addrSet.has(value)) matched.add(value);
				}
				for (const m of matched) byAddr[m].push(ev);
			}
			discussionEvents = byAddr;
			// Per-section thread forests, built engine-side (threaded: true).
			threadsBySection = resp.threads_by_address ?? {};
			discussionCounts = resp.counts;
			discussionSource = resp.source;
			discussionRefreshedAt = Date.now();

			// Kick a debounced prefetch for every distinct discussion
			// author so names land in the profile cache without us
			// blocking on it. The drawer and inline threads pick them up
			// reactively via getAuthorDisplayName.
			const authors = new Set<string>();
			for (const ev of resp.events) authors.add(ev.pubkey);
			if (authors.size > 0) prefetchAuthors([...authors]);

			console.debug('[ReaderBuffer] discussions loaded', {
				policy,
				requested: addrs.length,
				events: resp.events.length,
				authors: authors.size,
				source: resp.source
			});
		} catch (e) {
			console.warn('[ReaderBuffer] discussion load failed', e);
		}
	}

	async function refreshDiscussions() {
		if (discussionLoading) return;
		discussionLoading = true;
		try {
			// Explicit user action: bypass the engine's offline-mode
			// downgrade so the button actually reaches out to relays even
			// when the global network mode is offline. The user's workflow
			// for the offline case is "click the button to pull manually".
			await loadDiscussionCounts('fetch_always', { bypassOffline: true });
			// Same click also force-refetches kind 0 for every distinct
			// author we now know about, so renamed/avatar-updated profiles
			// surface in the drawer and inline threads. We do this after
			// the discussions land so the author set is fully populated.
			const authors = new Set<string>();
			for (const events of Object.values(discussionEvents)) {
				for (const ev of events) authors.add(ev.pubkey);
			}
			if (authors.size > 0) {
				try {
					await refreshAuthors([...authors]);
				} catch (e) {
					console.warn('[ReaderBuffer] author refresh failed', e);
				}
			}
		} finally {
			discussionLoading = false;
		}
	}

	function discussionFor(addr: { kind: number; pubkey: string; d_tag: string }): api.DiscussionCount {
		return discussionCounts[addrKey(addr)] ?? { comments: 0, highlights: 0 };
	}

	const publicationDiscussion = $derived(
		publication ? discussionFor(publication.addr) : { comments: 0, highlights: 0 }
	);
	const totalDiscussion = $derived.by(() => {
		let comments = 0;
		let highlights = 0;
		for (const v of Object.values(discussionCounts)) {
			comments += v.comments;
			highlights += v.highlights;
		}
		return { comments, highlights };
	});

	// Buffer ids may carry a `?highlight=<eventId>` or
	// `?focus_comment=<eventId>` suffix when the search router sends the
	// user here from a discussion hit. We strip those for address
	// parsing and surface them as separate fields.
	function splitBufferId(id: string): {
		core: string;
		highlightId: string | null;
		focusCommentId: string | null;
	} {
		// Take everything before the first `?`, then parse the query.
		const q = id.indexOf('?');
		if (q < 0) return { core: id, highlightId: null, focusCommentId: null };
		const core = id.slice(0, q);
		const params = new URLSearchParams(id.slice(q + 1));
		const hl = params.get('highlight');
		const fc = params.get('focus_comment');
		const isHex = (s: string | null) => (s && /^[0-9a-fA-F]{64}$/.test(s) ? s.toLowerCase() : null);
		return {
			core,
			highlightId: isHex(hl),
			focusCommentId: isHex(fc)
		};
	}

	function parseBufferId(id: string): { kind: number; pubkey: string; dTag: string } | null {
		const { core } = splitBufferId(id);
		const match = core.match(/^reader:(\d+):([0-9a-fA-F]{64}):(.+)$/);
		if (!match) return null;
		const kind = parseInt(match[1], 10);
		if (!Number.isFinite(kind)) return null;
		return { kind, pubkey: match[2].toLowerCase(), dTag: match[3] };
	}

	function parseEventId(id: string): string | null {
		const { core } = splitBufferId(id);
		const match = core.match(/^reader:event:([0-9a-fA-F]{64})$/);
		return match ? match[1].toLowerCase() : null;
	}

	const parsedAddr = $derived(parseBufferId(buffer.id));
	const parsedEventId = $derived(parseEventId(buffer.id));
	const parsedHighlightId = $derived(splitBufferId(buffer.id).highlightId);
	const parsedFocusCommentId = $derived(splitBufferId(buffer.id).focusCommentId);

	// Per-section NIP-22 thread forests, keyed by section addr. Built
	// engine-side (`POST /discussions/list` with `threaded: true`) and stored
	// from the response in loadDiscussionCounts — the reader renders these
	// directly. Highlights (kind 9802) are excluded engine-side; they surface
	// as section badges + overlays, not inline comments. The client-side
	// `discussionEvents` grouping is kept only for those highlights + author
	// prefetch, not for threading.
	let threadsBySection = $state<Record<string, ThreadNode[]>>({});

	function threadsForSection(addr: { kind: number; pubkey: string; d_tag: string }): ThreadNode[] {
		return threadsBySection[addrKey(addr)] ?? [];
	}

	// Comments scoped to the publication index (kind 30040) itself rather
	// than a specific section. Surfaced near the title so that
	// article-level discussion isn't invisible when the user is paging
	// through individual 30041 sections.
	const publicationThreads = $derived<ThreadNode[]>(
		publication ? threadsBySection[addrKey(publication.addr)] ?? [] : []
	);
	// Threads are collapsed by default — same posture as the highlights
	// drawer. The user expands the block they care about; routine reading
	// isn't interrupted by walls of comments.
	let publicationThreadsOpen = $state(false);
	// Auto-open when the user arrives via ?focus_comment=<id> and the
	// focused comment lives in the publication-level thread.
	$effect(() => {
		if (threadContainsId(publicationThreads, parsedFocusCommentId)) {
			untrack(() => {
				publicationThreadsOpen = true;
			});
		}
	});

	// All NIP-84 highlights to overlay on a given section. Two sources:
	//   - Events tagging this section's addr directly.
	//   - Events tagging the publication root (kind 30040); we cascade
	//     these down to whichever section's content their substring
	//     actually matches. This was a documented open question in the
	//     plan and the user's content shows highlights almost always
	//     scope to the publication, so substring-matching them onto
	//     individual sections is the only way they show at all.
	function highlightsForSection(addr: {
		kind: number;
		pubkey: string;
		d_tag: string;
	}): Highlight[] {
		const out: Highlight[] = [];
		const seen = new Set<string>();
		const push = (ev: api.DiscussionEvent) => {
			if (seen.has(ev.id) || ev.kind !== 9802) return;
			seen.add(ev.id);
			out.push({ id: ev.id, content: ev.content ?? '', pubkey: ev.pubkey });
		};
		for (const ev of discussionEvents[addrKey(addr)] ?? []) push(ev);
		if (publication) {
			for (const ev of discussionEvents[addrKey(publication.addr)] ?? []) push(ev);
		}
		return out;
	}

	// Outline-specific helpers: the inline list is the de-duped set of
	// highlights actually attributable to this section — direct refs
	// plus publication-cascaded ones whose content substring-matches the
	// section's text. Count + list are computed off the same set so they
	// always agree.
	type SectionHighlightEntry = Highlight & { created_at: number };

	function effectiveHighlightsForSection(addr: {
		kind: number;
		pubkey: string;
		d_tag: string;
	}): SectionHighlightEntry[] {
		const section = pristineSections.find((s) => addrKey(s.addr) === addrKey(addr));
		const content = (section?.content ?? '').toLowerCase();
		const out: SectionHighlightEntry[] = [];
		const seen = new Set<string>();
		const push = (ev: api.DiscussionEvent) => {
			if (seen.has(ev.id) || ev.kind !== 9802) return;
			seen.add(ev.id);
			out.push({
				id: ev.id,
				content: ev.content ?? '',
				pubkey: ev.pubkey,
				created_at: ev.created_at
			});
		};
		for (const ev of discussionEvents[addrKey(addr)] ?? []) push(ev);
		if (publication && content) {
			for (const ev of discussionEvents[addrKey(publication.addr)] ?? []) {
				if (ev.kind !== 9802) continue;
				const needle = (ev.content ?? '').trim().toLowerCase();
				if (needle && content.includes(needle)) push(ev);
			}
		}
		return out;
	}

	function effectiveHighlightCount(addr: {
		kind: number;
		pubkey: string;
		d_tag: string;
	}): number {
		return effectiveHighlightsForSection(addr).length;
	}

	// Outline expansion state: which sections have their inline comments
	// thread or flat highlights list open. Keyed by index since the
	// outline iterates pristineSections positionally.
	let outlineCommentsOpen = $state<Record<number, boolean>>({});
	let outlineHighlightsOpen = $state<Record<number, boolean>>({});

	function toggleOutlineComments(i: number) {
		outlineCommentsOpen[i] = !(outlineCommentsOpen[i] ?? false);
	}
	function toggleOutlineHighlights(i: number) {
		outlineHighlightsOpen[i] = !(outlineHighlightsOpen[i] ?? false);
	}

	// Outline tree collapse: nested 30040 indexes are collapsible folders,
	// keyed by addr. Collapsed by default — the reader expands deeper
	// levels deliberately, by index or all at once.
	let outlineExpanded = $state<Record<string, boolean>>({});
	const nestedAddrKey = (a: { kind: number; pubkey: string; d_tag: string }) =>
		`${a.kind}:${a.pubkey}:${a.d_tag}`;
	const isNestedIndex = (s: LazySection) => s.addr?.kind === 30040;

	function toggleOutlineIndex(addr: { kind: number; pubkey: string; d_tag: string }) {
		const k = nestedAddrKey(addr);
		outlineExpanded = { ...outlineExpanded, [k]: !(outlineExpanded[k] ?? false) };
	}
	function expandAllOutline() {
		const next: Record<string, boolean> = {};
		for (const s of pristineSections) if (isNestedIndex(s)) next[nestedAddrKey(s.addr)] = true;
		outlineExpanded = next;
	}
	function collapseAllOutline() {
		outlineExpanded = {};
	}

	// Direct-child + descendant counts per nested-index position in the
	// flattened TOC. An index with zero descendants sits beyond the depth
	// horizon — expandable only via refocus, not in place.
	const outlineChildInfo = $derived.by(() => {
		const info = new Map<number, { direct: number; descendants: number }>();
		for (let i = 0; i < pristineSections.length; i++) {
			if (!isNestedIndex(pristineSections[i])) continue;
			const d = pristineSections[i].depth ?? 0;
			let direct = 0;
			let descendants = 0;
			for (let j = i + 1; j < pristineSections.length; j++) {
				const dj = pristineSections[j].depth ?? 0;
				if (dj <= d) break;
				descendants++;
				if (dj === d + 1) direct++;
			}
			info.set(i, { direct, descendants });
		}
		return info;
	});

	// The outline rows actually on screen — entries under a collapsed
	// nested index are hidden until it's expanded. `hideDeeperThan` holds
	// the depth of the nearest collapsed ancestor.
	const outlineVisible = $derived.by(() => {
		const rows: { section: LazySection; index: number }[] = [];
		let hideDeeperThan: number | null = null;
		for (let i = 0; i < pristineSections.length; i++) {
			const s = pristineSections[i];
			const d = s.depth ?? 0;
			if (hideDeeperThan !== null) {
				if (d > hideDeeperThan) continue;
				hideDeeperThan = null;
			}
			rows.push({ section: s, index: i });
			if (
				isNestedIndex(s) &&
				!(outlineExpanded[nestedAddrKey(s.addr)] ?? false) &&
				(outlineChildInfo.get(i)?.descendants ?? 0) > 0
			) {
				hideDeeperThan = d;
			}
		}
		return rows;
	});

	const outlineIndexCount = $derived(pristineSections.filter(isNestedIndex).length);

	// ── Modeline loading indicator ──────────────────────────────────────
	// A small live progress line shown ONLY in the WM modeline (not the
	// buffer switcher or pane headers): a spinner, the truncated root->node
	// name path of the node that just resolved, and the stream's i/N. The
	// path scrubs through the tree as events arrive — finer-grained feedback
	// than a bare counter, which jumps when many events land in one frame.
	// Empty string when not loading.
	const modelineStatus = $derived.by(() => {
		// Stay visible after the stream ends until the ramped count catches
		// up, so the user always sees i reach N.
		if (!loaderRunning && displayedCount >= resolvedCount) return '';
		return `⟳ ${loadingPath || '…'}: ${displayedCount}/${knownCount}`;
	});
	$effect(() => {
		const s = modelineStatus;
		untrack(() => {
			if (s) store.setModelineStatus(buffer.id, s);
			else store.clearModelineStatus(buffer.id);
		});
	});
	// Cleanup on unmount — abort the stream and clear the modeline status.
	// Done via an `$effect` teardown, not `onDestroy`: lifecycle hooks are
	// unreliable under BufferRenderer's `{#key}{#if}` dispatch (see
	// FeedBuffer), whereas an effect's teardown is tied to the effect tree.
	// No reactive reads → the effect runs once; its teardown fires on destroy.
	$effect(() => {
		return () => {
			streamSource?.close();
			store.clearModelineStatus(buffer.id);
		};
	});

	// Drawer state. `drawerOpen` is toggled from the discussion summary
	// chip. `drawerHighlights` walks every distinct kind-9802 event in
	// discussionEvents (a single event can appear under multiple addr
	// keys when it cascades, so we dedupe by id) and resolves each one
	// to the section addr whose content actually contains its text —
	// that's what makes click-to-scroll work for publication-level
	// highlights that don't carry a section addr themselves.
	let drawerOpen = $state(false);

	const drawerHighlights = $derived.by<DrawerHighlight[]>(() => {
		const byId = new Map<string, api.DiscussionEvent>();
		for (const events of Object.values(discussionEvents)) {
			for (const ev of events) {
				if (ev.kind === 9802 && !byId.has(ev.id)) byId.set(ev.id, ev);
			}
		}
		const out: DrawerHighlight[] = [];
		for (const ev of byId.values()) {
			const needle = (ev.content ?? '').trim().toLowerCase();
			let sectionAddr: string | null = null;
			if (needle) {
				for (const s of pristineSections) {
					if ((s.content ?? '').toLowerCase().includes(needle)) {
						sectionAddr = addrKey(s.addr);
						break;
					}
				}
			}
			out.push({
				id: ev.id,
				pubkey: ev.pubkey,
				content: ev.content ?? '',
				created_at: ev.created_at,
				section_addr: sectionAddr
			});
		}
		return out.sort((a, b) => b.created_at - a.created_at);
	});

	function sectionIndexForAddr(addr: string): number {
		return pristineSections.findIndex((s) => addrKey(s.addr) === addr);
	}

	function scrollToHighlight(highlightId: string, sectionAddr: string | null) {
		// Step 1: if the highlight lives in a section the pager isn't
		// currently showing, switch to it and into paginated view first.
		if (sectionAddr) {
			const idx = sectionIndexForAddr(sectionAddr);
			if (idx >= 0 && currentSection !== idx) {
				currentSection = idx;
				viewMode = 'paginated';
			}
		}
		// Step 2: wait one frame so the DOM has settled, then locate the
		// mark and scroll it into view. We use the `~=` attribute
		// selector — `data-hl-ids` is a space-separated id list for
		// composite-highlight forward compatibility, even though today's
		// segmenter only ever puts one id per mark.
		requestAnimationFrame(() => {
			const safeId = highlightId.replace(/"/g, '\\"');
			const mark = document.querySelector<HTMLElement>(
				`mark.hl-overlay[data-hl-ids~="${safeId}"]`
			);
			if (!mark) return;
			mark.scrollIntoView({ behavior: 'smooth', block: 'center' });
			mark.classList.add('hl-flash');
			setTimeout(() => mark.classList.remove('hl-flash'), 1200);
		});
	}

	// When parsedHighlightId is present, fetch the highlight event and
	// extract its content. Sections render with an overlay span around
	// any substring matching this text.
	let highlightText = $state<string | null>(null);
	let highlightMeta = $state<{ author: string; created_at: number } | null>(null);

	$effect(() => {
		const id = parsedHighlightId;
		if (!id) {
			highlightText = null;
			highlightMeta = null;
			return;
		}
		// Default into paginated view so the overlay is visible up front;
		// outline mode only shows titles and would hide the match.
		viewMode = 'paginated';
		untrack(async () => {
			try {
				const resp = await api.getEvent(id);
				const ev = resp.event as
					| { content?: string; pubkey?: string; created_at?: number; kind?: number }
					| null;
				if (ev && ev.kind === 9802) {
					highlightText = ev.content ?? '';
					highlightMeta = {
						author: ev.pubkey ?? '',
						created_at: ev.created_at ?? 0
					};
				}
			} catch (e) {
				console.warn('[ReaderBuffer] failed to load highlight', e);
			}
		});
	});

	// When the highlight text becomes available, try to jump to the
	// section containing it so the overlay is in view without scrolling.
	$effect(() => {
		const text = highlightText;
		if (!text) return;
		untrack(() => {
			const needle = text.trim().toLowerCase();
			if (!needle) return;
			const idx = pristineSections.findIndex(
				(s) => (s.content ?? '').toLowerCase().includes(needle)
			);
			if (idx >= 0) currentSection = idx;
		});
	});

	// When ?focus_comment=<id> is set and threads have loaded, jump to
	// the section whose thread contains that comment, switch to
	// paginated view, and let CommentThread scroll the matching node
	// into view from there.
	$effect(() => {
		const id = parsedFocusCommentId;
		if (!id) return;
		const idLc = id.toLowerCase();
		untrack(() => {
			for (let i = 0; i < pristineSections.length; i++) {
				const addr = pristineSections[i].addr;
				const events = discussionEvents[addrKey(addr)] ?? [];
				if (events.some((e) => e.id.toLowerCase() === idLc)) {
					currentSection = i;
					viewMode = 'paginated';
					return;
				}
			}
		});
	});

	// ReaderBuffer always shows the *pristine* published view fetched from
	// the engine. Draft state lives in a separate `draft-reader` buffer
	// (kind: 'draft-reader') so editing a publication can't bleed back into
	// the original article shown in the feed. To preview a draft, use
	// ComposeView's "Read" affordance which spawns the draft buffer.
	const isDraftMode = false;
	const sections = $derived<LazySection[]>(pristineSections);
	const segments = $derived<ReturnType<typeof segmentSections>>([]);

	async function load() {
		if (parsedEventId) {
			await loadEvent(parsedEventId);
			return;
		}
		if (!parsedAddr) {
			error = 'Buffer id does not encode a publication address';
			loading = false;
			return;
		}
		// Non-NKBIP-01 addressables (long-form articles 30023, wikis 30818,
		// etc.) don't have a separate index/section structure — they're a
		// single event with a content body. Fetch the addressable directly
		// and wrap it as a single-section view so the rest of the reader
		// (paginated/continuous, highlights, comments) works uniformly.
		if (parsedAddr.kind !== 30040) {
			await loadAddressable(parsedAddr.kind, parsedAddr.pubkey, parsedAddr.dTag);
			return;
		}
		// Seed the focus stack with the buffer's own publication on first
		// load. Refocus / breadcrumb navigation mutates the stack and calls
		// load() again without the buffer id changing.
		if (focusStack.length === 0) {
			const seed = { pubkey: parsedAddr.pubkey, d_tag: parsedAddr.dTag, title: '' };
			focusStack = [seed];
			focusMarker = 0;
			visitLog = [seed];
			visitIndex = 0;
		}
		// Supersede any stream in flight.
		streamSource?.close();
		streamSource = null;
		loaderRunning = false;
		error = null;
		const focus = focusStack[focusMarker];
		const cached = focusCache.get(focusKey(focus.pubkey, focus.d_tag));
		if (cached) {
			// Seen before — restore instantly: no blank screen, no re-stream.
			loadGeneration++; // discard any late events from a prior stream
			publication = cached.publication;
			pristineSections = cached.sections;
			loadedDepth = cached.loadedDepth;
			loading = false;
			resolvedCount = 0;
			knownCount = 0;
			displayedCount = 0;
			if (cached.publication.title && !focus.title) {
				focusStack[focusMarker] = {
					...focusStack[focusMarker],
					title: cached.publication.title
				};
			}
			// If the target is deeper than what was cached, stream the rest.
			if (treeDepth > loadedDepth) runLoader();
		} else {
			// First visit — stream the tree from scratch.
			loading = true;
			publication = null;
			pristineSections = [];
			runLoader();
		}
	}

	/** Stream-load the current focus. Opens an SSE stream of per-node events
	 *  and assembles the tree from an addr-keyed map as they arrive (in
	 *  resolution order, not tree order). `resolvedCount` / `knownCount` drive
	 *  the modeline's per-event i/N. A bumped `loadGeneration` plus closing the
	 *  EventSource discard / abort a superseded stream. */
	function runLoader() {
		const focus = focusStack[focusMarker];
		if (!focus) return;
		const myGen = ++loadGeneration;
		streamSource?.close();
		loaderRunning = true;
		loadedDepth = -1;
		resolvedCount = 0;
		knownCount = 0;
		displayedCount = 0;
		loadingPath = '';
		error = null;

		// Ramp `displayedCount` toward `resolvedCount` a chunk per frame —
		// small gaps step ~10, large gaps close in ~6 frames so a huge load
		// never lags absurdly. Self-perpetuates each frame until it has caught
		// up and the loader has settled.
		const rampFrame = () => {
			if (myGen !== loadGeneration) return;
			const gap = resolvedCount - displayedCount;
			if (gap > 0) {
				displayedCount += Math.min(gap, Math.max(10, Math.ceil(gap / 6)));
			}
			if (loaderRunning || displayedCount < resolvedCount) {
				requestAnimationFrame(rampFrame);
			}
		};
		requestAnimationFrame(rampFrame);

		// The streamed tree under construction. Keyed by addr; `childKeys`
		// (captured from index events) gives tree order, so the engine's
		// concurrent, out-of-order delivery is a non-issue.
		const nodes = new Map<string, StreamNode>();
		let rootKey: string | null = null;
		const inHorizon = new Set<string>(); // addr-keys that count toward N

		const upsert = (key: string, patch: Partial<StreamNode> & { addr: NAddr }) => {
			const existing = nodes.get(key);
			if (existing) {
				Object.assign(existing, patch);
				return;
			}
			nodes.set(key, {
				addr: patch.addr,
				title: patch.title ?? null,
				isIndex: patch.isIndex ?? false,
				content: patch.content ?? null,
				childKeys: patch.childKeys ?? [],
				parentKey: patch.parentKey,
				status: patch.status ?? 'pending',
				error: patch.error
			});
		};

		// Truncated root->node name path (e.g. "Douay:NewT:Matth") of a node,
		// for the modeline. Walks parentKey up; caps to the last 5 levels.
		const pathOf = (key: string): string => {
			const parts: string[] = [];
			let cur: string | undefined = key;
			let guard = 0;
			while (cur && guard++ < 32) {
				const n = nodes.get(cur);
				if (!n) break;
				const name = (n.title ?? n.addr.d_tag ?? '').trim();
				parts.push(name.slice(0, 6) || '·');
				cur = n.parentKey;
			}
			parts.reverse();
			return (parts.length > 5 ? ['…', ...parts.slice(-5)] : parts).join(':');
		};

		// Rebuild publication + pristineSections from the map. Throttled to a
		// frame so a burst of events paints once, not once per event.
		let commitScheduled = false;
		const commit = () => {
			commitScheduled = false;
			if (myGen !== loadGeneration || !rootKey) return;
			const root = nodes.get(rootKey);
			if (!root) return;
			publication = {
				addr: root.addr,
				title: root.title,
				summary: null,
				image: null,
				author_pubkey: root.addr.pubkey,
				version: null,
				created_at: 0,
				index: null,
				relays: root.relays,
				signed: root.signed,
				forked: root.forked
			};
			// pristineSections is the root's subtree — its direct children at
			// depth 0, matching the old flattenToc. The root itself is
			// `publication`, not a section row.
			const acc: LazySection[] = [];
			// Depth-first flatten with a path-scoped cycle guard. `nodes` is a
			// graph, not a tree: a 30040 may reference one of its ancestors
			// (the loader's cycle data, e.g. E → A → C → E), and once the tree
			// is streamed deep enough to close that loop an unguarded walk
			// recurses forever. The cyclic node is still pushed — it shows as
			// a refocus target — but its subtree is not re-walked. `ancestors`
			// is the current root→node path, not a global visited set, so a
			// diamond (a node reached by two distinct paths) still expands
			// under each parent.
			const walk = (key: string, depth: number, ancestors: Set<string>) => {
				const n = nodes.get(key);
				if (!n) return;
				acc.push({
					addr: n.addr,
					title: n.title,
					content: n.content,
					position: acc.length,
					depth,
					status: n.status,
					error: n.error,
					relays: n.relays,
					signed: n.signed,
					forked: n.forked
				});
				if (ancestors.has(key)) return; // cycle — node shown, subtree cut
				const next = new Set(ancestors).add(key);
				for (const ck of n.childKeys) walk(ck, depth + 1, next);
			};
			for (const ck of root.childKeys) walk(ck, 0, new Set([rootKey]));
			pristineSections = acc;
			loading = false;
			if (publication?.title && !focus.title) {
				focusStack[focusMarker] = {
					...focusStack[focusMarker],
					title: publication.title
				};
			}
		};
		const scheduleCommit = () => {
			if (commitScheduled) return;
			commitScheduled = true;
			requestAnimationFrame(commit);
		};
		const finish = (total: number) => {
			if (myGen !== loadGeneration) return;
			streamSource?.close();
			streamSource = null;
			loaderRunning = false;
			loading = false;
			knownCount = total;
			if (!rootKey && !error) {
				error = 'Publication index could not be loaded';
			}
			commit();
			// Cache the completed tree + the depth it reached, so a refocus
			// back restores it and a depth change at or below `loadedDepth`
			// needs no re-stream.
			if (rootKey && publication) {
				loadedDepth = treeDepth;
				focusCache.set(focusKey(focus.pubkey, focus.d_tag), {
					publication,
					sections: pristineSections,
					loadedDepth: treeDepth
				});
			}
		};

		// `treeDepth` N fully loads levels 0..N-1; the engine's `max_depth` is
		// the deepest fully-loaded level, so it gets `treeDepth - 1`. Level N
		// comes back as d-tag preview stubs.
		const es = api.streamPublication(
			focus.pubkey,
			focus.d_tag,
			'local_first',
			Math.max(0, treeDepth - 1)
		);
		streamSource = es;

		es.onmessage = (msg) => {
			if (myGen !== loadGeneration) {
				es.close();
				return;
			}
			let ev: PubLoadEvent;
			try {
				ev = JSON.parse(msg.data) as PubLoadEvent;
			} catch {
				return;
			}
			if (ev.type === 'index') {
				const k = addrKey(ev.addr);
				upsert(k, {
					addr: ev.addr,
					title: ev.title,
					isIndex: true,
					status: 'loaded',
					childKeys: ev.children.map((c) => addrKey(c.addr)),
					relays: ev.relays ?? [],
					signed: ev.signed ?? true,
					forked: ev.forked ?? false
				});
				if (ev.is_root) rootKey = k;
				for (const c of ev.children) {
					const ck = addrKey(c.addr);
					upsert(ck, { addr: c.addr, isIndex: c.is_index, parentKey: k });
					if (c.in_horizon) inHorizon.add(ck);
				}
				inHorizon.add(k);
				// Merge this index into the persistent reference graph: the
				// node itself plus stub entries for any nested 30040 it
				// references, so a child shows as a node before its own
				// index event arrives.
				{
					const g = { ...focusGraph };
					g[k] = {
						addr: ev.addr,
						title: ev.title,
						childKeys: ev.children.filter((c) => c.is_index).map((c) => addrKey(c.addr))
					};
					for (const c of ev.children) {
						if (!c.is_index) continue;
						const ck = addrKey(c.addr);
						if (!g[ck]) g[ck] = { addr: c.addr, title: null, childKeys: [] };
					}
					focusGraph = g;
				}
				resolvedCount++;
				knownCount = inHorizon.size;
				loadingPath = pathOf(k);
				scheduleCommit();
			} else if (ev.type === 'leaf') {
				const k = addrKey(ev.addr);
				upsert(k, {
					addr: ev.addr,
					title: ev.title,
					isIndex: false,
					content: ev.content,
					status: 'loaded',
					relays: ev.relays ?? [],
					signed: ev.signed ?? true
				});
				inHorizon.add(k);
				resolvedCount++;
				knownCount = inHorizon.size;
				loadingPath = pathOf(k);
				scheduleCommit();
			} else if (ev.type === 'error') {
				const k = addrKey(ev.addr);
				upsert(k, { addr: ev.addr, status: 'error', error: ev.message });
				inHorizon.add(k);
				resolvedCount++;
				knownCount = inHorizon.size;
				loadingPath = pathOf(k);
				scheduleCommit();
			} else if (ev.type === 'done') {
				finish(ev.total);
			}
		};
		es.onerror = () => {
			// A publication load is finite — EventSource would auto-reconnect
			// on a transient drop, so close it and finish with what landed.
			if (myGen !== loadGeneration) return;
			finish(knownCount);
		};
	}

	/** Refocus the reader on a nested 30040 index: push the current focus
	 *  onto the breadcrumb stack and load the nested publication at the same
	 *  depth, fetching whatever events haven't been loaded for this level. */
	/** Refocus the reader on the 30040 at `pubkey`/`d_tag`. Onto an already-
	 *  visited index it moves the breadcrumb marker (a cycle-back); onto a
	 *  new one it appends a crumb. Either way the temporal visit axis grows. */
	function refocusTo(pubkey: string, d_tag: string, title: string) {
		const crumb: FocusCrumb = { pubkey, d_tag, title };
		const key = focusKey(pubkey, d_tag);
		const existing = focusStack.findIndex((c) => focusKey(c.pubkey, c.d_tag) === key);
		if (existing >= 0) {
			// Cycle-back: the target is already a breadcrumb (a 30040 that
			// references an ancestor). Move the marker, don't duplicate it.
			focusMarker = existing;
		} else {
			// A publication not yet visited — append a crumb, advance marker.
			focusStack = [...focusStack, crumb];
			focusMarker = focusStack.length - 1;
		}
		// The temporal axis records every entry, revisits included.
		visitLog = [...visitLog, crumb];
		visitIndex = visitLog.length - 1;
		currentSection = 0;
		outlineCursor = 0;
		load();
	}

	function refocus(section: LazySection) {
		if (section.addr.kind !== 30040) return;
		refocusTo(section.addr.pubkey, section.addr.d_tag, section.title ?? 'Nested publication');
	}

	/** Graph-panel node click: refocus onto the node and drop into outline. */
	function navigateGraph(addr: NAddr) {
		graphOpen = false;
		viewMode = 'outline';
		refocusTo(
			addr.pubkey,
			addr.d_tag,
			focusGraph[addrKey(addr)]?.title ?? 'Nested publication'
		);
	}

	// Upward containment: the kind-30040 indexes that reference the *current
	// focus* as a child. The downward structure (NESTED/graph) is the reader's
	// existing axis; this is the reverse — "what is this part of". Recomputed
	// per focus (a sub-publication can be contained somewhere its parent isn't).
	let containedIn = $state<{ id: string; pubkey: string; d_tag: string; title: string }[]>([]);
	$effect(() => {
		const focus = focusStack[focusMarker];
		if (!focus) {
			containedIn = [];
			return;
		}
		const coord = { kind: 30040, pubkey: focus.pubkey, d_tag: focus.d_tag };
		untrack(() => {
			app.containingByCoord(coord).then((idx) => {
				// The focus may have moved while the query was in flight.
				const cur = focusStack[focusMarker];
				if (cur && cur.pubkey === coord.pubkey && cur.d_tag === coord.d_tag) {
					containedIn = idx;
				}
			});
		});
	});

	/** "part of N" pill → find the publications that contain the current focus.
	 *  Mirrors the feed badge: reveal the search buffer and run the reverse
	 *  a-tag query so all containers (cross-author) list out. */
	function findContainers() {
		const focus = focusStack[focusMarker];
		if (!focus) return;
		store.openBuffer({
			className: 'research',
			buffer: { id: 'search', kind: 'search', label: 'search', kicker: 'containing' }
		});
		app.searchFor(`k:30040 a:30040:${focus.pubkey}:${focus.d_tag}`);
	}

	// Graph-panel inputs — addr-keyed so they line up with `focusGraph`.
	const crumbGraphKey = (c: FocusCrumb) =>
		addrKey({ kind: 30040, pubkey: c.pubkey, d_tag: c.d_tag });
	const graphRootKey = $derived(focusStack[0] ? crumbGraphKey(focusStack[0]) : '');
	const graphCurrentKey = $derived(
		focusStack[focusMarker] ? crumbGraphKey(focusStack[focusMarker]) : ''
	);
	const graphPathKeys = $derived(
		focusStack.slice(0, focusMarker + 1).map(crumbGraphKey)
	);
	// The graph is worth showing once it has more than the root node.
	// `hasGraph` doubles as the flat/nested signal: the root index always seeds
	// one node, and a *nested* publication seeds stub nodes for each child 30040
	// it references — so >1 node ⟺ nested. A flat publication (index → 30041
	// sections only) never adds a second node.
	const hasGraph = $derived(Object.keys(focusGraph).length > 1);
	// True once the root index has streamed in (parent-before-child ordering
	// guarantees it's the first graph node). Until then the flat/nested verdict
	// is unknown, so the toolbar shows neither hint — avoiding a SINGLE LEVEL →
	// NESTED flip as the root resolves.
	const rootLoaded = $derived(Object.keys(focusGraph).length >= 1);

	/** Move the focus marker to breadcrumb crumb `index` and reload it. */
	function breadcrumbTo(index: number) {
		if (index < 0 || index >= focusStack.length || index === focusMarker) return;
		focusMarker = index;
		visitLog = [...visitLog, focusStack[index]];
		visitIndex = visitLog.length - 1;
		currentSection = 0;
		outlineCursor = 0;
		load();
	}

	/** `Prev Level` — step back one focus along the temporal visit axis.
	 *  Walks `visitIndex` back through `visitLog` and re-points the
	 *  breadcrumb marker at that publication. No Next: inward movement is
	 *  the refocus action, not a deterministic forward step. */
	function prevLevel() {
		if (visitIndex <= 0) return;
		visitIndex--;
		const v = visitLog[visitIndex];
		const key = focusKey(v.pubkey, v.d_tag);
		const i = focusStack.findIndex((c) => focusKey(c.pubkey, c.d_tag) === key);
		if (i >= 0) focusMarker = i;
		currentSection = 0;
		outlineCursor = 0;
		load();
	}
	const canPrevLevel = $derived(visitIndex > 0);

	/** Adjust the depth target. Always available — never disabled mid-load.
	 *  Changing it re-streams the current focus at the new depth: `runLoader`
	 *  closes the prior stream, bumps the generation, and opens a fresh one. */
	function setDepth(d: number) {
		const clamped = Math.max(1, Math.min(MAX_DEPTH, d));
		if (clamped === treeDepth) return;
		treeDepth = clamped;
		if (!parsedAddr || parsedAddr.kind !== 30040) return;
		// Lowering the depth, or any target within what's already loaded, is
		// a no-op: the deeper tree stays in place, ready if depth is raised
		// again. Only a target beyond `loadedDepth` needs a fresh stream.
		if (treeDepth > loadedDepth) runLoader();
	}

	// Standalone-event reader: a `reader:event:<id>` buffer renders one
	// section, no TOC walk, and defaults to paginated view so the user
	// reads exactly the event they searched for.
	//
	// Two-phase load (same shape as DiscussionViewBuffer.pullThread):
	// phase 1 reads the local cache only; a miss falls through to a
	// relay round-trip with `bypassOffline` so the engine runs its
	// Confirm-mode intent flow (modal approval) instead of silently
	// failing. A declined modal degrades to the local miss.
	async function loadEvent(eventId: string) {
		loading = true;
		loadingStatus = null;
		try {
			let resp = await api.getEvent(eventId, { policy: 'local_only' });
			if (!resp.event) {
				loadingStatus = 'Not in local cache — fetching from relays…';
				resp = await api.getEvent(eventId, { policy: 'fetch_always', bypassOffline: true });
			}
			const ev = resp.event as
				| { kind?: number; pubkey?: string; tags?: string[][]; content?: string; created_at?: number; relays?: string[] }
				| null;
			if (!ev) {
				error = 'Event not found locally or on your relays.';
				return;
			}
			const tags = ev.tags ?? [];
			const dTag = tags.find((t) => t[0] === 'd')?.[1] ?? '';
			const titleTag = tags.find((t) => t[0] === 'title')?.[1] ?? null;
			const addr = {
				kind: ev.kind ?? 0,
				pubkey: ev.pubkey ?? '',
				d_tag: dTag
			};
			publication = {
				addr,
				title: titleTag,
				summary: null,
				image: null,
				author_pubkey: ev.pubkey ?? '',
				version: null,
				created_at: ev.created_at ?? 0,
				index: ev,
				relays: ev.relays ?? [],
				signed: true
			};
			pristineSections = [
				{
					addr,
					title: titleTag,
					content: ev.content ?? '',
					position: 0,
					status: 'loaded' as const,
					relays: ev.relays ?? []
				}
			];
			viewMode = 'paginated';
		} catch (e) {
			error = api.errorMessage(e);
		} finally {
			loading = false;
			loadingStatus = null;
		}
	}

	// Addressable reader: fetch a 30023 (long-form article) / 30818 (wiki)
	// / other addressable event by its kind+pubkey+d-tag triple, then
	// render it as a single-section view. Reuses the same publication
	// shape as loadEvent so the downstream rendering paths (paginated,
	// continuous, discussion overlay) don't have to special-case kinds.
	// Same two-phase local→relay load as loadEvent.
	async function loadAddressable(kind: number, pubkey: string, dTag: string) {
		loading = true;
		loadingStatus = null;
		try {
			let resp = await api.getAddressable(kind, pubkey, dTag, 'local_only');
			if (!resp.event) {
				loadingStatus = 'Not in local cache — fetching from relays…';
				resp = await api.getAddressable(kind, pubkey, dTag, 'fetch_always', {
					bypassOffline: true
				});
			}
			const ev = resp.event as
				| { id?: string; kind?: number; pubkey?: string; tags?: string[][]; content?: string; created_at?: number; relays?: string[] }
				| null;
			if (!ev) {
				error = `${kind === 30023 ? 'Article' : kind === 30818 ? 'Wiki' : 'Addressable event'} not found locally or on your relays.`;
				return;
			}
			const tags = ev.tags ?? [];
			const titleTag = tags.find((t) => t[0] === 'title')?.[1] ?? null;
			const summaryTag = tags.find((t) => t[0] === 'summary')?.[1] ?? null;
			const imageTag = tags.find((t) => t[0] === 'image')?.[1] ?? null;
			const addr = { kind, pubkey, d_tag: dTag };
			publication = {
				addr,
				title: titleTag ?? (kind === 30818 ? dTag : null),
				summary: summaryTag,
				image: imageTag,
				author_pubkey: ev.pubkey ?? pubkey,
				version: null,
				created_at: ev.created_at ?? 0,
				index: ev,
				relays: ev.relays ?? [],
				signed: true
			};
			pristineSections = [
				{
					addr,
					title: titleTag ?? (kind === 30818 ? dTag : null),
					content: ev.content ?? '',
					position: 0,
					status: 'loaded' as const,
					relays: ev.relays ?? []
				}
			];
			viewMode = 'paginated';
		} catch (e) {
			error = api.errorMessage(e);
		} finally {
			loading = false;
			loadingStatus = null;
		}
	}

	// Plain (non-reactive) guard: load() reads and writes `focusStack`, and
	// because load() is async its reactive reads can't be fully suppressed by
	// untrack — without this guard the effect would re-trigger itself in a
	// loop. Comparing against a non-$state variable means a spurious re-run
	// early-returns instead of reloading.
	let lastLoadedBufferId: string | null = null;
	$effect(() => {
		const id = buffer.id;
		if (id === lastLoadedBufferId) return;
		lastLoadedBufferId = id;
		// Switching the buffer to a different publication drops the refocus
		// stack; load() re-seeds it from the new buffer address.
		untrack(() => {
			focusStack = [];
			focusMarker = 0;
			visitLog = [];
			visitIndex = 0;
			focusGraph = {};
			graphOpen = false;
			load();
		});
	});

	// Hydrate discussion counts each time the progressive loader settles at
	// a new depth — so badges fill in as the tree deepens, not just once on
	// first load. `loadDiscussionCounts` POSTs the address set, so a deep
	// level carrying hundreds of section addresses is fine. Phase A is
	// local_only for instant rendering; phase B is fetch_always when online.
	let discussionLoadedKey = $state<string | null>(null);
	$effect(() => {
		const settled =
			!loaderRunning && !loading && !!publication && pristineSections.length > 0;
		if (!settled) return;
		// Key on the current focus address (not buffer.id — refocus keeps the
		// same buffer) and the loaded depth, so each focus + actual load
		// hydrates discussions once (a no-op depth-down doesn't re-fetch).
		const key = `${addrKey(publication!.addr)}@${loadedDepth}`;
		if (discussionLoadedKey === key) return;
		discussionLoadedKey = key;
		untrack(async () => {
			await loadDiscussionCounts('local_only');
			if (app.networkStatus?.mode === 'auto') {
				refreshDiscussions();
			}
		});
	});

	// Reset discussion state when the buffer changes so a different
	// publication triggers a fresh fetch.
	$effect(() => {
		buffer.id;
		untrack(() => {
			discussionLoadedKey = null;
			discussionCounts = {};
			discussionRefreshedAt = null;
		});
	});

	// Lazy fallback for a section that came back without content (a load
	// failure inside the depth horizon). Loads by the section's own address
	// rather than by index — the flattened list spans nested publications,
	// so a root-relative section index is meaningless here. Nested 30040
	// indexes are refocus targets, not readable sections, and are skipped.
	function handleLoadSection(index: number) {
		if (isDraftMode) return; // draft sections are already loaded
		if (index < 0 || index >= pristineSections.length) return;
		const cur = pristineSections[index];
		if (cur.addr.kind === 30040) return; // nested index — refocus, not load
		if (cur.status === 'loaded' || cur.status === 'loading') return;
		if (loadingPromises.has(index)) return;
		pristineSections[index] = { ...cur, status: 'loading' };
		const promise = (async () => {
			try {
				const resp = await api.getAddressable(
					cur.addr.kind,
					cur.addr.pubkey,
					cur.addr.d_tag
				);
				const ev = resp.event as
					| { content?: string; tags?: string[][] }
					| null;
				if (!ev) throw new Error('Section event not found');
				const titleTag = ev.tags?.find((t) => t[0] === 'title')?.[1];
				pristineSections[index] = {
					...pristineSections[index],
					title: titleTag ?? pristineSections[index].title,
					content: ev.content ?? '',
					status: 'loaded'
				};
			} catch (e) {
				pristineSections[index] = {
					...pristineSections[index],
					status: 'error',
					error: String(e)
				};
			} finally {
				loadingPromises.delete(index);
			}
		})();
		loadingPromises.set(index, promise);
	}

	function handleNavigate(index: number) {
		currentSection = index;
		outlineCursor = index;
	}

	// Per-section / per-nested-index "fetch from relays" affordance.
	// Forces `fetch_always` so the engine runs through the relay path
	// (and confirm mode pops a single-event modal). After the event
	// lands, we patch the section in place for a leaf, or re-stream
	// the tree for a nested index so its children become discoverable.
	let backfillingAll = $state(false);
	const refetchingSection = $state<Record<number, boolean>>({});

	async function refetchSection(index: number) {
		const cur = pristineSections[index];
		if (!cur) return;
		if (refetchingSection[index]) return;
		refetchingSection[index] = true;
		const wasIndex = cur.addr.kind === 30040;
		try {
			const resp = await api.getAddressable(
				cur.addr.kind,
				cur.addr.pubkey,
				cur.addr.d_tag,
				'fetch_always'
			);
			const ev = resp.event as
				| { content?: string; tags?: string[][] }
				| null;
			if (wasIndex) {
				// Nested 30040 stub got pulled — re-run the loader so its
				// children stream in under this focus.
				if (publication?.addr.pubkey === parsedAddr?.pubkey) runLoader();
			} else if (ev) {
				const titleTag = ev.tags?.find((t) => t[0] === 'title')?.[1];
				pristineSections[index] = {
					...pristineSections[index],
					title: titleTag ?? pristineSections[index].title,
					content: ev.content ?? '',
					status: 'loaded'
				};
			}
		} catch (e) {
			pristineSections[index] = {
				...pristineSections[index],
				status: 'error',
				error: String(e)
			};
		} finally {
			refetchingSection[index] = false;
		}
	}

	async function backfillAll() {
		if (!parsedAddr || parsedAddr.kind !== 30040) return;
		if (backfillingAll) return;
		backfillingAll = true;
		try {
			const resp = await api.backfillPublication(parsedAddr.pubkey, parsedAddr.dTag);
			if (resp.fetched > 0) runLoader();
		} catch (e) {
			console.warn('[ReaderBuffer] backfill failed', e);
		} finally {
			backfillingAll = false;
		}
	}

	// JSON-viewer affordances. The publication-level button opens the
	// kind-30040 index event; the per-section kebab + pager's "§ json"
	// link opens the corresponding section event. All three resolve via
	// the addressable coordinate, so they handle replaceable updates the
	// same way (newest event for that (kind, pubkey, d) wins).
	function openPublicationJson() {
		if (!publication) return;
		app.openAddressableInModal(publication.addr);
	}

	function openSectionJsonByIndex(index: number) {
		const s = pristineSections[index];
		if (!s) return;
		app.openAddressableInModal(s.addr);
	}

	function openSectionJsonBySection(s: { addr: { kind: number; pubkey: string; d_tag: string } }) {
		app.openAddressableInModal(s.addr);
	}

	function extractPublicationTags(pub: PublicationDetail | null): TagEntry[] {
		if (!pub) return [];
		const skip = new Set(['d', 'a', 'alt', 'e', 'p']);
		const rawTags =
			(pub.index as { data?: { tags?: string[][] } } | null)?.data?.tags ?? [];
		return rawTags
			.filter((t) => !skip.has(t[0]))
			.map((t) => ({ name: t[0], value: t.slice(1).join(', ') }));
	}

	async function ensureAllSectionsLoaded() {
		for (let i = 0; i < pristineSections.length; i++) {
			if (pristineSections[i].status === 'pending') handleLoadSection(i);
		}
		const inflight = Array.from(loadingPromises.values());
		if (inflight.length) await Promise.all(inflight);
	}

	function publicationEventId(pub: PublicationDetail | null): string | null {
		if (!pub) return null;
		const ev = pub.index as { id?: unknown } | null;
		return typeof ev?.id === 'string' ? ev.id : null;
	}

	// Seed compose state from the loaded publication so subsequent lock/
	// unlock/reorder actions write into a real draft. Idempotent — calling
	// it when isDraftMode is already true is a no-op. If there's an
	// existing draft for a different publication, prompt before clobbering
	// it (only one in-progress draft at a time for now).
	async function seedDraftFromPublication(): Promise<boolean> {
		if (isDraftMode) return true;
		const existingSrc = app.compose.source_publication_addr;
		// A draft for THIS publication is already in progress: continue it as-is.
		// Re-seeding here would clearComposePool + re-import pristine sections,
		// silently wiping the user's edits — "Continue editing this draft" must
		// actually continue.
		const sameDraft =
			!!existingSrc &&
			!!parsedAddr &&
			existingSrc.pubkey.toLowerCase() === parsedAddr.pubkey.toLowerCase() &&
			existingSrc.d_tag === parsedAddr.dTag;
		if (sameDraft) return true;
		// Clobber guard: a draft seeded from a *different* publication, or a
		// from-scratch draft with actual content, both die in clearComposePool
		// below. (A pristine empty composer — one blank section — doesn't count.)
		const hasOtherDraft =
			(!!existingSrc && !!parsedAddr) ||
			(!existingSrc &&
				(app.compose.title.trim().length > 0 ||
					app.compose.sections.some(
						(s) => s.title.trim().length > 0 || s.content.trim().length > 0
					)));
		if (hasOtherDraft) {
			const label = existingSrc ? `for "${existingSrc.d_tag}"` : '(unsaved)';
			const ok = confirm(
				`A draft is already in progress ${label}. Discard it and start a new draft for this publication?`
			);
			if (!ok) return false;
		}
		await ensureAllSectionsLoaded();
		app.clearComposePool();
		app.seedDraftMetadata(
			publication?.title ?? null,
			extractPublicationTags(publication),
			{
				pub_addr: publication?.addr ?? null,
				pub_event_id: publicationEventId(publication),
				section_order: pristineSections.map((s) => s.addr)
			}
		);
		for (const s of pristineSections) {
			if (s.status !== 'loaded' || s.content == null) continue;
			app.importSectionToCompose(s.addr, s.title, s.content);
		}
		return true;
	}

	async function editInComposer() {
		const ok = await seedDraftFromPublication();
		if (!ok) return;
		app.navigateToCompose();
	}

	async function editFocusedSection() {
		const s = pristineSections[currentSection];
		if (!s) return;
		if (s.status !== 'loaded' || s.content == null) {
			handleLoadSection(currentSection);
			const inflight = Array.from(loadingPromises.values());
			if (inflight.length) await Promise.all(inflight);
		}
		const reloaded = pristineSections[currentSection];
		if (
			!reloaded ||
			reloaded.status !== 'loaded' ||
			reloaded.content == null
		)
			return;
		app.clearComposePool();
		app.seedDraftMetadata(null, []);
		app.importSectionToCompose(reloaded.addr, reloaded.title, reloaded.content);
		app.navigateToCompose();
	}

	function itemAt(index: number): ContextItem | null {
		return app.compose.sections[index] ?? null;
	}

	function stateAt(index: number) {
		const item = itemAt(index);
		return item ? sectionState(item) : 'original';
	}

	async function ensureDraftThenToggle(index: number) {
		// Click on a lock from pristine view: implicitly enter draft mode
		// (seed compose from the publication), then toggle the clicked
		// section. After this returns, isDraftMode is true and subsequent
		// lock clicks operate directly on compose state.
		if (!isDraftMode) {
			const ok = await seedDraftFromPublication();
			if (!ok) return;
		}
		const item = app.compose.sections[index];
		if (!item) return;
		app.handleToggleReadonly(item.id);
	}

	function toggleLockDraft(index: number) {
		const item = itemAt(index);
		if (item) app.handleToggleReadonly(item.id);
	}

	function moveSection(index: number, direction: 'up' | 'down') {
		const item = itemAt(index);
		if (!item) return;
		app.reorderComposeSection(item.id, direction);
	}

	function removeAt(index: number) {
		const item = itemAt(index);
		if (!item) return;
		app.handleDeleteFromCompose([item]);
	}

	const anyUnlocked = $derived(
		isDraftMode &&
			app.compose.sections.some((s) => s.source_addr && !s.readonly)
	);
	const anyLockable = $derived(
		isDraftMode && app.compose.sections.some((s) => s.source_addr && s.readonly)
	);

	function unlockAllImported() {
		for (const s of app.compose.sections) {
			if (s.source_addr && s.readonly) app.handleToggleReadonly(s.id);
		}
	}

	function lockAllUnlocked() {
		for (const s of app.compose.sections) {
			if (s.source_addr && !s.readonly && s.content === s.original_content) {
				app.handleToggleReadonly(s.id);
			}
		}
	}

	// Outline cursor — separate from paginated currentSection so the two
	// don't fight: cursor is the selection in outline view, currentSection
	// is the page in paginated view. Pressing Enter (or l/right) on a
	// cursored outline entry switches to paginated mode at that index.
	let outlineCursor = $state(0);
	let outlineEl: HTMLDivElement | undefined = $state();
	let contentWrap: HTMLDivElement | undefined = $state();

	function clampCursor() {
		const total = sections.length;
		if (total === 0) outlineCursor = 0;
		else if (outlineCursor >= total) outlineCursor = total - 1;
		else if (outlineCursor < 0) outlineCursor = 0;
	}

	function scrollOutlineCursorIntoView() {
		// The outline rows live inside `.outline-overlay`, which itself
		// lives inside the scrollable `.content` (contentWrap). Manipulate
		// scrollTop on the actual scroll ancestor.
		if (!contentWrap || !outlineEl) return;
		const row = outlineEl.querySelector<HTMLElement>(`[data-cursor="${outlineCursor}"]`);
		if (!row) return;
		const wrapRect = contentWrap.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < wrapRect.top) {
			contentWrap.scrollTop -= wrapRect.top - rowRect.top;
		} else if (rowRect.bottom > wrapRect.bottom) {
			contentWrap.scrollTop += rowRect.bottom - wrapRect.bottom;
		}
	}

	function openCursorInPaginated() {
		if (sections.length === 0) return;
		const cur = sections[outlineCursor];
		// A nested 30040 index: expand it in place when its subtree is
		// loaded, so drilling the outline matches the chevron. Refocus
		// only when the index hasn't been pulled into the tree yet.
		if (cur && cur.addr.kind === 30040) {
			if ((outlineChildInfo.get(outlineCursor)?.descendants ?? 0) > 0) {
				toggleOutlineIndex(cur.addr);
			} else {
				refocus(cur);
			}
			return;
		}
		if (!isDraftMode) handleLoadSection(outlineCursor);
		viewMode = 'paginated';
		handleNavigate(outlineCursor);
	}

	// View-mode order — left/right (h/l) cycles through these. Outline's
	// l/→ is special: it drills into paginated and loads the cursored
	// section. Otherwise l advances through the cycle, h reverses.
	const VIEW_ORDER: ViewMode[] = ['outline', 'paginated', 'continuous'];

	function cycleView(dir: 1 | -1) {
		const i = VIEW_ORDER.indexOf(viewMode);
		const n = VIEW_ORDER.length;
		viewMode = VIEW_ORDER[(i + dir + n) % n];
	}

	function handleNav(action: NavAction): boolean {
		if (sections.length === 0) return false;
		if (viewMode === 'outline') {
			// j/k step over *visible* rows — entries hidden under a
			// collapsed nested index are skipped.
			if (action === 'down' || action === 'up') {
				const vis = outlineVisible;
				if (vis.length > 0) {
					let pos = vis.findIndex((r) => r.index === outlineCursor);
					if (pos < 0) pos = 0;
					pos = Math.min(
						vis.length - 1,
						Math.max(0, pos + (action === 'down' ? 1 : -1))
					);
					outlineCursor = vis[pos].index;
				}
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'top') {
				outlineCursor = outlineVisible[0]?.index ?? 0;
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'bottom') {
				outlineCursor = outlineVisible[outlineVisible.length - 1]?.index ?? 0;
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'select' || action === 'right') {
				// Outline → paginated drills with the selected section.
				openCursorInPaginated();
				return true;
			}
			if (action === 'left') {
				// Cycle backward: outline ← continuous.
				cycleView(-1);
				return true;
			}
			if (action === 'menu') {
				openSectionJsonByIndex(outlineCursor);
				return true;
			}
			return false;
		}
		if (viewMode === 'paginated') {
			if (action === 'down') {
				if (currentSection < sections.length - 1) handleNavigate(currentSection + 1);
				return true;
			}
			if (action === 'up') {
				if (currentSection > 0) handleNavigate(currentSection - 1);
				return true;
			}
			if (action === 'top') {
				handleNavigate(0);
				return true;
			}
			if (action === 'bottom') {
				handleNavigate(sections.length - 1);
				return true;
			}
			if (action === 'left' || action === 'right') {
				cycleView(action === 'right' ? 1 : -1);
				return true;
			}
			if (action === 'select') return true;
			if (action === 'menu') {
				openSectionJsonByIndex(currentSection);
				return true;
			}
			return false;
		}
		// continuous: j/k page-scroll by viewport; h/l cycles modes;
		// gg / G snap to top / bottom of the document.
		if (viewMode === 'continuous') {
			if (action === 'left' || action === 'right') {
				cycleView(action === 'right' ? 1 : -1);
				return true;
			}
			if (action === 'menu') {
				// In continuous view there's no per-row cursor — the menu
				// opens on whichever section the pager last advanced to.
				openSectionJsonByIndex(currentSection);
				return true;
			}
			if (contentWrap) {
				if (action === 'top') {
					contentWrap.scrollTop = 0;
					return true;
				}
				if (action === 'bottom') {
					contentWrap.scrollTop = contentWrap.scrollHeight;
					return true;
				}
				const step = Math.max(80, contentWrap.clientHeight - 60);
				if (action === 'down') {
					contentWrap.scrollTop += step;
					return true;
				}
				if (action === 'up') {
					contentWrap.scrollTop -= step;
					return true;
				}
			}
		}
		return false;
	}

	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	$effect(() => {
		sections.length;
		untrack(clampCursor);
	});

	// Collapsing a nested index can hide the cursored row. Snap the cursor
	// back to the nearest visible ancestor (the collapsed index itself).
	$effect(() => {
		const vis = outlineVisible;
		if (vis.length === 0) return;
		untrack(() => {
			if (vis.some((r) => r.index === outlineCursor)) return;
			let snap = vis[0].index;
			for (const r of vis) {
				if (r.index <= outlineCursor) snap = r.index;
				else break;
			}
			outlineCursor = snap;
		});
	});
</script>

<div class="reader-wrap">
	<div class="toolbar" data-tour="reader-toolbar">
		<!-- Order matches the h/l drill axis: outline → paginated → continuous.
		     l/→ cycles right, h/← cycles left. Outline's l/→ is special —
		     it drills into paginated with the cursored section loaded. -->
		<button
			class:active={viewMode === 'outline'}
			onclick={() => (viewMode = 'outline')}>Outline</button
		>
		<button
			class:active={viewMode === 'paginated'}
			onclick={() => (viewMode = 'paginated')}>Paginated</button
		>
		<button
			class:active={viewMode === 'continuous'}
			onclick={() => (viewMode = 'continuous')}>Continuous</button
		>
		{#if resolution.resolving}
			<span
				class="nd-resolving"
				title="Resolving nostrdown references — {resolution.resolved} of {resolution.total} done (not-local embeds fetch from relays)"
				style="--nd-frac: {resolution.fraction}"
			>
				<span class="nd-resolving__track"><span class="nd-resolving__fill"></span></span>
				<span class="nd-resolving__label">{resolution.resolved}/{resolution.total} refs</span>
			</span>
		{/if}
		<button
			class="json-btn"
			data-tour="reader-menu"
			onclick={openPublicationJson}
			disabled={!publication}
			title="Open this publication's event menu (m)"
		>menu</button>
		{#if parsedAddr?.kind === 30040 && rootLoaded}
			{#if hasGraph}
				<!-- Nested: this index references child 30040 sub-publications.
				     Show the NESTED tag + the depth knob (how many levels the
				     loader walks); the graph button lives later, in the nav. -->
				<span
					class="level-hint level-hint--nested"
					title="Nested publication — its index references child 30040 sub-publications. Set how many levels deep the loader walks below; the ⊞ graph button (in the nav) maps the structure."
				>NESTED</span>
				<span
					class="depth-knob"
					title="Levels of nested 30040 indexes the loader walks toward. The buttons stay live during loading; past depth {MAX_DEPTH} you refocus into a sub-publication."
				>
					<span class="depth-knob__label">depth</span>
					<button
						class="depth-knob__step"
						onclick={() => setDepth(treeDepth - 1)}
						disabled={treeDepth <= 1}
						aria-label="Decrease depth"
					>−</button>
					<span class="depth-knob__val">{treeDepth}</span>
					<button
						class="depth-knob__step"
						onclick={() => setDepth(treeDepth + 1)}
						disabled={treeDepth >= MAX_DEPTH}
						aria-label="Increase depth"
					>+</button>
				</span>
			{:else}
				<!-- Flat: the index references only 30041 sections. It loads in
				     one shot — there's nothing for a depth control to walk, so
				     drop it and just mark the level. -->
				<span
					class="level-hint level-hint--flat"
					title="Single-level publication — its index references only 30041 sections, no nested sub-publications. The whole thing loads at once; there's no depth to set."
				>SINGLE LEVEL</span>
			{/if}
		{/if}
		<span class="sp"></span>
		{#if isDraftMode}
			<span class="draft-pill" title="A draft of this publication is in progress">DRAFT</span>
			<button
				class="bulk"
				onclick={unlockAllImported}
				disabled={!anyLockable}
				title="Unlock all imported sections (yellow — claimed for reorder/edit)"
			>Unlock all</button>
			<button
				class="bulk"
				onclick={lockAllUnlocked}
				disabled={!anyUnlocked}
				title="Re-lock unlocked sections that haven't been modified"
			>Lock all</button>
		{/if}
		{#if viewMode === 'paginated'}
			<button
				class="edit"
				onclick={editFocusedSection}
				disabled={!publication}
				title="Send focused section to composer">Edit §</button
			>
		{/if}
		<button
			class="edit"
			data-tour="reader-edit"
			onclick={editInComposer}
			disabled={!publication}
			title={isDraftMode ? 'Continue editing this draft' : 'Open this publication in the composer'}
		>Edit</button>
		<button
			class="discussions-refresh"
			onclick={refreshDiscussions}
			disabled={discussionLoading || !publication}
			title={app.networkStatus?.mode === 'auto'
				? 'Pull new comments and highlights from relays'
				: 'Offline — pull comments and highlights from relays anyway (manual override)'}
		>
			{discussionLoading ? '…' : 'Refresh discussions'}
		</button>
	</div>

	{#if loading}
		<div class="empty">
			<p>Loading…</p>
			{#if loadingStatus}
				<p class="loading-status">{loadingStatus}</p>
			{/if}
		</div>
	{:else if error}
		<div class="empty"><p>Error: {error}</p></div>
	{:else if !publication}
		<div class="empty"><p>No publication loaded</p></div>
	{:else}
		{#if focusStack.length > 1 || hasGraph}
			<!-- Cross-publication navigation. The breadcrumb is the trail of
			     focused 30040 indexes; the marker (crumb--current) is where
			     the reader is. Crumbs past the marker are the loop tail —
			     reachable ahead of the current focus, e.g. after cycling
			     back into an ancestor. `Prev Level` steps back along the
			     temporal visit axis; the graph button opens the full
			     reference graph. -->
			<nav class="crumbs" aria-label="Publication navigation">
				{#if focusStack.length > 1}
					<button
						class="crumb-prev"
						onclick={prevLevel}
						disabled={!canPrevLevel}
						title="Prev Level — step back to the previously focused publication"
					>‹ Prev Level</button>
					<span class="crumb-sep" aria-hidden="true">·</span>
					{#each focusStack as crumb, ci (ci + ':' + crumb.pubkey + ':' + crumb.d_tag)}
						{#if ci > 0}<span class="crumb-sep" aria-hidden="true">›</span>{/if}
						{#if ci === focusMarker}
							<span class="crumb crumb--current" aria-current="page"
								>{crumb.title || publication.title || 'Publication'}</span>
						{:else}
							<button
								class="crumb"
								class:crumb--ahead={ci > focusMarker}
								onclick={() => breadcrumbTo(ci)}
								title={ci > focusMarker
									? `Loop ahead — jump forward to ${crumb.title || 'this publication'}`
									: `Back up to ${crumb.title || 'this publication'}`}
							>{ci > focusMarker ? '↻ ' : ''}{crumb.title || 'Publication'}</button>
						{/if}
					{/each}
				{/if}
				{#if hasGraph}
					<button
						class="crumb-graph"
						class:active={graphOpen}
						onclick={() => (graphOpen = !graphOpen)}
						title="Toggle the publication reference graph"
					>⊞ graph</button>
				{/if}
			</nav>
		{/if}
		{#if graphOpen && hasGraph}
			<FocusGraph
				nodes={focusGraph}
				rootKey={graphRootKey}
				currentKey={graphCurrentKey}
				pathKeys={graphPathKeys}
				onnavigate={navigateGraph}
				onclose={() => (graphOpen = false)}
			/>
		{/if}
		<!-- Header row renders even for untitled events: the pill cluster
		     (provenance / context / compose) is chrome every view carries,
		     title or not — see feat-ui-patterns "row action-cluster order". -->
		{#if publication}
			{@const pubAddr = publication.addr}
			<div class="title">
				{#if publication.title}
					<span class="title__text">{publication.title}</span>
				{/if}
				{#if pubAddr.kind === 30040}
					<button
						class="title-fetch"
						class:spin={backfillingAll}
						onclick={backfillAll}
						disabled={backfillingAll}
						title="Fetch this publication's index + backfill all missing sections from relays"
						aria-label="Fetch and backfill"
					>⟳</button>
				{/if}
				<PoolStateBadges
					item={app.findPoolItemByAddr(pubAddr)}
					onpillctx={() => app.pillActionByAddr(pubAddr, 'context')}
					onpillcmp={() => app.pillActionByAddr(pubAddr, 'compose')}
					onpilldrop={() => app.pillActionByAddr(pubAddr, 'drop')}
					signed={publication.signed}
					relays={publication.relays}
					forked={publication.forked}
					containedIn={containedIn.length}
					onpartof={findContainers}
					orientation="horizontal"
				/>
			</div>
		{/if}
		{#if highlightText !== null}
			<div class="hl-banner">
				<span class="hl-banner__label">Viewing highlight</span>
				<span class="hl-banner__sample" title={highlightText}>
					“{highlightText.length > 120 ? highlightText.slice(0, 120) + '…' : highlightText}”
				</span>
				{#if highlightMeta}
					<span class="hl-banner__meta">
						by <ProfileName pubkey={highlightMeta.author} onviewprofile={app.handleViewProfile} />
					</span>
				{/if}
			</div>
		{/if}
		{#if discussionRefreshedAt !== null || discussionLoading}
			<div class="discussion-summary" title="Across the publication index and all sections">
				{#if discussionLoading}
					<span class="ds-badge">Fetching discussions…</span>
				{:else if totalDiscussion.comments === 0 && totalDiscussion.highlights === 0}
					<span class="ds-badge ds-badge--empty">No comments or highlights found</span>
					{#if discussionSource}
						<span class="ds-sep">·</span>
						<span class="ds-source" title="Engine returned this many events from local DB and relays for the underlying query">
							scanned {discussionSource.local_count} local / {discussionSource.relay_count} relay events
						</span>
					{/if}
				{:else}
					<span
						class="ds-badge ds-badge--comments"
						title="{totalDiscussion.comments} comment{totalDiscussion.comments === 1 ? '' : 's'} across the publication and its sections"
					>
						cmt {totalDiscussion.comments}
					</span>
					<button
						class="ds-badge ds-badge--highlights ds-badge--button"
						onclick={() => (drawerOpen = !drawerOpen)}
						title="{totalDiscussion.highlights} highlight{totalDiscussion.highlights === 1 ? '' : 's'} — {drawerOpen ? 'hide highlights drawer' : 'open highlights drawer (grouped by author, click to scroll)'}"
					>
						hl {totalDiscussion.highlights}
					</button>
					{#if publicationDiscussion.comments > 0 || publicationDiscussion.highlights > 0}
						<span class="ds-sep">·</span>
						<span class="ds-on-index" title="Comments/highlights on the publication index itself (kind 30040)">
							index: cmt {publicationDiscussion.comments} · hl {publicationDiscussion.highlights}
						</span>
					{/if}
					{#if discussionSource}
						<span class="ds-sep">·</span>
						<span class="ds-source" title="local DB matches + relay-fetched events for this query">
							{discussionSource.local_count}L / {discussionSource.relay_count}R
						</span>
					{/if}
				{/if}
			</div>
		{/if}
		{#if publicationThreads.length > 0}
			<div class="pub-threads">
				<button
					class="pub-threads-head"
					onclick={() => (publicationThreadsOpen = !publicationThreadsOpen)}
					aria-expanded={publicationThreadsOpen}
				>
					<span class="ptr">{publicationThreadsOpen ? '▾' : '▸'}</span>
					Comments on this article ({publicationThreads.length})
				</button>
				{#if publicationThreadsOpen}
					<CommentThread nodes={publicationThreads} focusedEventId={parsedFocusCommentId} />
				{/if}
			</div>
		{/if}
		<div class="content" bind:this={contentWrap}>
			{#if viewMode === 'outline'}
				{#if isDraftMode}
					<!-- Draft outline: lock/unlock per section, up/down reorder,
					     remove on non-imported. Border colors derive from
					     sectionState (green=imported, yellow=claimed,
					     violet=forked, none=original). -->
					<div class="outline-overlay" bind:this={outlineEl}>
						{#each segments as seg, segIdx (segIdx + ':' + seg.indices.join(','))}
							<div
								class="segment"
								class:segment--imported={seg.state === 'imported'}
								class:segment--claimed={seg.state === 'claimed'}
								class:segment--forked={seg.state === 'forked'}
								class:segment--original={seg.state === 'original'}
								class:segment--group={seg.indices.length > 1}
							>
								{#each seg.indices as i (i)}
									{@const item = app.compose.sections[i]}
									{@const st = stateAt(i)}
									{@const isLast = seg.indices[seg.indices.length - 1] === i}
									{@const isFirstInSeg = seg.indices[0] === i}
									<div
										class="entry"
										class:entry--imported={st === 'imported'}
										class:entry--claimed={st === 'claimed'}
										class:entry--forked={st === 'forked'}
										class:entry--original={st === 'original'}
										class:entry--cursor={i === outlineCursor}
										data-cursor={i}
									>
										<div class="rail" aria-hidden="true">
											{#if seg.indices.length > 1}
												<span class="rail-glyph"
													>{isLast
														? '└'
														: isFirstInSeg
															? '┌'
															: '│'}</span
												>
											{/if}
										</div>
										{#if item && item.source_addr}
											<button
												class="lock"
												class:lock--unlocked={st === 'claimed' ||
													st === 'forked'}
												onclick={() => toggleLockDraft(i)}
												title={st === 'imported'
													? 'Unlock — claim for reorder / fork'
													: st === 'forked'
														? 'Forked — re-lock blocked'
														: 'Lock — restore as transcluded'}
												disabled={st === 'forked'}
											>{st === 'imported' ? '🔒' : '🔓'}</button>
										{:else}
											<span
												class="lock lock--placeholder"
												title="Original — no source to lock against">·</span
											>
										{/if}
										<div class="entry-body">
											<SectionCard
												section={sections[i]}
												preview
												index={i + 1}
												onclick={() => {
													viewMode = 'paginated';
													handleNavigate(i);
												}}
												onviewjson={openSectionJsonBySection}
											/>
										</div>
										<div class="row-actions">
											{#if st !== 'imported'}
												<button
													class="row-btn"
													onclick={() => moveSection(i, 'up')}
													disabled={i === 0}
													title="Move up"
												>▲</button>
												<button
													class="row-btn"
													onclick={() => moveSection(i, 'down')}
													disabled={i === sections.length - 1}
													title="Move down"
												>▼</button>
												<button
													class="row-btn remove"
													onclick={() => removeAt(i)}
													title="Remove from draft"
												>✕</button>
											{:else if isFirstInSeg && seg.indices.length > 1}
												<!-- Group reorder: imported runs move as a single
												     unit. Anchor the up/down on the first row of
												     each group. -->
												<button
													class="row-btn"
													onclick={() => {
														for (const idx of seg.indices) {
															moveSection(idx, 'up');
														}
													}}
													disabled={i === 0}
													title="Move group up"
												>▲▲</button>
												<button
													class="row-btn"
													onclick={() => {
														for (const idx of [...seg.indices].reverse()) {
															moveSection(idx, 'down');
														}
													}}
													disabled={
														seg.indices[seg.indices.length - 1] ===
														sections.length - 1
													}
													title="Move group down"
												>▼▼</button>
											{/if}
										</div>
									</div>
								{/each}
							</div>
						{/each}
						<p class="hint">
							🔒 click to unlock. Unlocked sections (yellow) reorder atomically;
							locked imports (green) move together. Forked (violet) sections
							carry diverged content — go to compose to keep editing.
						</p>
					</div>
				{:else}
					<!-- Pristine outline: the depth-N tree as a collapsible
					     hierarchy. 30041 sections render as section cards;
					     nested 30040 indexes are collapsible folders — the
					     caret expands children inline, `refocus` re-roots. -->
					{#if outlineIndexCount > 0}
						<div class="outline-treebar">
							<span class="outline-treebar__label"
								>{outlineIndexCount} nested {outlineIndexCount === 1
									? 'index'
									: 'indexes'}</span
							>
							<span class="outline-treebar__spacer"></span>
							<button class="outline-treebar__btn" onclick={expandAllOutline}
								>Expand all</button
							>
							<button class="outline-treebar__btn" onclick={collapseAllOutline}
								>Collapse all</button
							>
						</div>
					{/if}
					<div class="outline-overlay" bind:this={outlineEl}>
						{#each outlineVisible as row (`${row.index}:${row.section.addr.pubkey}:${row.section.addr.d_tag}`)}
							{@const section = row.section}
							{@const i = row.index}
							{#if section.addr.kind === 30040}
								{@const info = outlineChildInfo.get(i)}
								{@const direct = info?.direct ?? 0}
								{@const loadable = (info?.descendants ?? 0) > 0}
								{@const pending = loaderRunning && !loadable}
								{@const open = outlineExpanded[nestedAddrKey(section.addr)] ?? false}
								<!-- Nested publication index — a collapsible folder. -->
								<div
									class="entry entry--nested"
									class:entry--cursor={i === outlineCursor}
									data-cursor={i}
									style="--depth:{section.depth ?? 0}"
								>
									<button
										class="nested-row"
										onclick={() =>
											loadable ? toggleOutlineIndex(section.addr) : refocus(section)}
										aria-expanded={loadable ? open : undefined}
										title={loadable
											? open
												? 'Collapse this nested publication'
												: 'Expand this nested publication'
											: pending
												? 'Loading this level…'
												: 'Refocus the reader on this nested publication'}
									>
										<span
											class="nested-caret"
											class:spin={pending}
											aria-hidden="true"
											>{loadable ? (open ? '▾' : '▸') : pending ? '⟳' : '·'}</span
										>
										<span class="nested-icon" aria-hidden="true">⊞</span>
										<span class="nested-title"
											>{section.title || 'Nested publication'}</span
										>
										<PoolStateBadges
											item={app.findPoolItemByAddr(section.addr)}
											onpillctx={() => app.pillActionByAddr(section.addr, 'context')}
											onpillcmp={() => app.pillActionByAddr(section.addr, 'compose')}
											onpilldrop={() => app.pillActionByAddr(section.addr, 'drop')}
											signed={section.signed}
											relays={section.relays}
											forked={section.forked}
										/>
										{#if loadable}
											<span class="nested-count"
												>{direct} {direct === 1 ? 'item' : 'items'}</span
											>
										{:else if pending}
											<span class="nested-count nested-count--pending">loading…</span>
										{:else}
											<span class="nested-count nested-count--empty">not loaded</span>
										{/if}
									</button>
									{#if !loadable}
										<button
											class="nested-fetch-btn"
											class:spin={refetchingSection[i]}
											onclick={() => refetchSection(i)}
											disabled={refetchingSection[i]}
											title="Fetch this nested index event from relays so its children become visible in place"
											aria-label="Fetch nested index"
										>⟳</button>
									{/if}
									<button
										class="nested-refocus-btn"
										onclick={() => refocus(section)}
										title="Refocus the reader on this nested publication"
										>refocus ⟳</button
									>
								</div>
							{:else}
								{@const disc = discussionFor(section.addr)}
								{@const sectionHighlights = effectiveHighlightsForSection(section.addr)}
								{@const highlightN = sectionHighlights.length}
								{@const commentN = disc.comments}
								{@const sectionThreads = threadsForSection(section.addr)}
								{@const commentsOpen = outlineCommentsOpen[i] ?? false}
								{@const highlightsOpen = outlineHighlightsOpen[i] ?? false}
								<div
									class="entry entry--pristine"
									class:entry--cursor={i === outlineCursor}
									data-cursor={i}
									style="--depth:{section.depth ?? 0}"
								>
									{#if section.status !== 'loaded'}
										<button
											class="section-fetch"
											class:spin={refetchingSection[i] || section.status === 'loading'}
											onclick={(e) => {
												e.stopPropagation();
												refetchSection(i);
											}}
											disabled={refetchingSection[i] || section.status === 'loading'}
											title="Fetch this section from relays"
											aria-label="Fetch section"
										>⟳</button>
									{/if}
									<button
										class="lock"
										onclick={() => ensureDraftThenToggle(i)}
										title="Unlock to start a draft for reorder/fork">🔒</button
									>
									<div class="entry-body">
										<SectionCard
											{section}
											preview
											index={i + 1}
											onclick={() => {
												handleLoadSection(i);
												viewMode = 'paginated';
												handleNavigate(i);
											}}
										/>
									</div>
									<div class="section-actions">
									<PoolStateBadges
										item={app.findPoolItemByAddr(section.addr)}
										onpillctx={() => app.pillActionByAddr(section.addr, 'context')}
										onpillcmp={() => app.pillActionByAddr(section.addr, 'compose')}
										onpilldrop={() => app.pillActionByAddr(section.addr, 'drop')}
										signed={section.signed}
										relays={section.relays}
									/>
									{#if highlightN > 0}
										<button
											class="section-action section-action--highlights"
											class:open={highlightsOpen}
											onclick={(e) => {
												e.stopPropagation();
												toggleOutlineHighlights(i);
											}}
											title="{highlightsOpen ? 'Hide' : 'Show'} the {highlightN} highlight{highlightN === 1 ? '' : 's'} on this section"
										>hl {highlightN}</button>
									{/if}
									{#if commentN > 0}
										<button
											class="section-action section-action--comments"
											class:open={commentsOpen}
											onclick={(e) => {
												e.stopPropagation();
												toggleOutlineComments(i);
											}}
											title="{commentsOpen ? 'Hide' : 'Show'} the {commentN} threaded comment{commentN === 1 ? '' : 's'} on this section"
										>cmt {commentN}</button>
									{/if}
									<button
										class="pill pill--menu"
										onclick={(e) => {
											e.stopPropagation();
											openSectionJsonBySection(section);
										}}
										title="Open this section's event menu (m)"
									>menu</button>
								</div>
							</div>
							{#if highlightsOpen && highlightN > 0}
								<div class="outline-detail outline-detail--highlights">
									<HighlightList highlights={sectionHighlights} />
								</div>
							{/if}
								{#if commentsOpen && sectionThreads.length > 0}
									<div class="outline-detail outline-detail--comments">
										<CommentThread nodes={sectionThreads} focusedEventId={parsedFocusCommentId} />
									</div>
								{/if}
							{/if}
						{/each}
					</div>
				{/if}
			{:else if viewMode === 'continuous'}
				<ContinuousView
					{sections}
					onrefocus={refocus}
					publication={{
						title: publication.title,
						summary: publication.summary
					}}
					publicationAtag={`${publication.addr.kind}:${publication.addr.pubkey}:${publication.addr.d_tag}`}
					onload={isDraftMode ? undefined : handleLoadSection}
					onviewjson={openSectionJsonBySection}
					highlightsFor={highlightsForSection}
					focusedHighlightId={parsedHighlightId}
					threadsFor={threadsForSection}
					focusedCommentId={parsedFocusCommentId}
					{resolution}
				/>
			{:else}
				<PaginatedView
					{sections}
					{currentSection}
					onnavigate={handleNavigate}
					onrefocus={refocus}
					onload={isDraftMode ? undefined : handleLoadSection}
					onsectionjson={openSectionJsonByIndex}
					highlightsFor={highlightsForSection}
					focusedHighlightId={parsedHighlightId}
					threadsFor={threadsForSection}
					focusedCommentId={parsedFocusCommentId}
					publicationAtag={`${publication.addr.kind}:${publication.addr.pubkey}:${publication.addr.d_tag}`}
					{resolution}
				/>
			{/if}
		</div>
	{/if}
	<HighlightsDrawer
		highlights={drawerHighlights}
		open={drawerOpen}
		onclose={() => (drawerOpen = false)}
		onnavigate={scrollToHighlight}
		onrefresh={async () => {
			const authors = new Set<string>(drawerHighlights.map((h) => h.pubkey));
			if (authors.size > 0) await refreshAuthors([...authors]);
		}}
	/>
</div>

<style>
	.reader-wrap {
		display: flex;
		flex-direction: column;
		height: 100%;
		min-height: 0;
	}
	.toolbar {
		display: flex;
		gap: 4px;
		padding: 6px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		flex-shrink: 0;
		align-items: center;
	}
	.toolbar button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.toolbar button.active {
		background: var(--id-yours);
		color: var(--bg);
		border-color: var(--id-yours);
	}
	.toolbar .sp { flex: 1; }
	.toolbar .draft-pill {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		font-weight: 700;
		letter-spacing: 0.08em;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--yellow) 22%, transparent);
		color: var(--yellow);
	}
	.toolbar .bulk:disabled { opacity: 0.4; cursor: not-allowed; }
	.toolbar .edit {
		color: var(--id-draft);
		border-color: var(--id-draft);
	}
	.toolbar .edit:hover:not(:disabled) {
		background: var(--id-draft);
		color: var(--bg);
	}
	.toolbar .edit:disabled { opacity: 0.5; cursor: not-allowed; }
	/* JSON action — distinct from view-mode toggles so it doesn't read
	   as a fourth view mode. Tinted with --id-yours like other modal /
	   nav affordances. */
	.toolbar .json-btn {
		color: var(--id-yours);
		border-color: color-mix(in srgb, var(--id-yours) 40%, transparent);
		margin-left: 4px;
	}
	.toolbar .json-btn:hover:not(:disabled) {
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
		border-color: var(--id-yours);
	}
	.toolbar .json-btn:disabled { opacity: 0.5; cursor: not-allowed; }

	.toolbar .discussions-refresh {
		margin-left: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.toolbar .discussions-refresh:disabled { opacity: 0.5; cursor: not-allowed; }

	/* Reference-resolution progress: a slim bar + count, shown only while
	   references are still resolving (fetching not-local embeds, etc.). */
	.nd-resolving {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		margin-left: 6px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.nd-resolving__track {
		display: inline-block;
		width: 48px;
		height: 4px;
		border-radius: 2px;
		background: var(--border, rgba(127, 127, 127, 0.25));
		overflow: hidden;
	}
	.nd-resolving__fill {
		display: block;
		height: 100%;
		width: calc(var(--nd-frac, 0) * 100%);
		background: var(--accent, var(--id-yours));
		transition: width 0.2s ease;
	}
	.nd-resolving__label { white-space: nowrap; }
	@media (prefers-reduced-motion: reduce) {
		.nd-resolving__fill { transition: none; }
	}

	.discussion-summary {
		display: flex;
		align-items: center;
		gap: 6px;
		padding: 4px var(--s-3);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
	}
	/* Publication-level discussion counts — same quiet pill family as the
	   outline's per-section `hl n` / `cmt n` chips (and PoolStateBadges),
	   so counts read the same at every level. */
	.ds-badge {
		color: var(--base6);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		font-weight: 600;
		padding: 0 6px;
		border-radius: 3px;
		line-height: 1.6;
		white-space: nowrap;
	}
	.ds-badge--comments {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours, var(--base7));
	}
	.ds-badge--highlights {
		background: color-mix(in srgb, var(--state-online) 12%, transparent);
		color: var(--state-online, var(--base6));
	}
	.ds-badge--empty { color: var(--base5); font-style: italic; padding: 0; }
	.ds-sep { color: var(--base4); }
	.ds-on-index { color: var(--base5); }
	.ds-source { color: var(--base4); font-size: calc(var(--t-xs) - 1px); }

	/* The highlights badge is interactive — toggles the drawer. */
	.ds-badge--button {
		border: none;
		cursor: pointer;
		font: inherit;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		font-weight: 600;
	}
	.ds-badge--button:hover {
		background: color-mix(in srgb, var(--state-online) 28%, transparent);
	}

	.pub-threads {
		padding: 10px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
		max-height: 40%;
		overflow-y: auto;
	}
	.pub-threads-head {
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
	.pub-threads-head:hover { color: var(--fg); }
	.pub-threads-head .ptr { min-width: 1ch; }

	.hl-banner {
		display: flex;
		gap: 8px;
		align-items: baseline;
		padding: 6px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		background: color-mix(in srgb, var(--state-online) 8%, transparent);
		font-size: var(--t-xs);
		flex-shrink: 0;
		flex-wrap: wrap;
	}
	.hl-banner__label {
		font-family: var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--state-online);
	}
	.hl-banner__sample {
		color: var(--fg);
		font-style: italic;
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.hl-banner__meta {
		font-family: var(--font-mono);
		color: var(--base5);
	}

	.section-actions {
		display: flex;
		gap: 4px;
		align-items: center;
		padding: 0 6px;
		flex-shrink: 0;
		align-self: center;
	}
	/* Discussion-count toggles — sized and tinted like the PoolStateBadges
	   pills next to them so the cluster reads as one family, not a CTA
	   strip. `hl n` / `cmt n`; the tooltip carries the full wording. */
	.section-action {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		font-weight: 600;
		padding: 0 6px;
		border-radius: 3px;
		border: none;
		line-height: 1.4;
		white-space: nowrap;
		cursor: pointer;
	}
	.section-action:hover {
		filter: brightness(1.2);
	}
	.section-action--highlights {
		background: color-mix(in srgb, var(--state-online) 12%, transparent);
		color: var(--state-online);
	}
	.section-action--highlights.open {
		background: color-mix(in srgb, var(--state-online) 28%, transparent);
	}
	.section-action--comments {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		color: var(--id-yours);
	}
	.section-action--comments.open {
		background: color-mix(in srgb, var(--id-yours) 28%, transparent);
	}

	.outline-detail {
		margin-left: 22px;
		margin-right: 6px;
		margin-bottom: 6px;
		padding: 6px 8px;
		border-left: 2px solid var(--panel-border);
	}
	.outline-detail--highlights {
		border-left-color: var(--state-online);
	}
	.outline-detail--comments {
		border-left-color: var(--id-yours);
	}

	.title {
		padding: 8px var(--s-3);
		font-size: var(--t-md);
		font-weight: 700;
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
	}
	/* Long publication titles wrap (this is the content surface — don't
	   ellipsize) instead of forcing the badges out of the pane. */
	.title__text {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.content { flex: 1; overflow: auto; min-height: 0; }
	.empty {
		flex: 1;
		display: flex;
		flex-direction: column;
		gap: 6px;
		align-items: center;
		justify-content: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.loading-status {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
	}

	/* Outline-overlay layout (used by both draft and pristine modes). */
	.outline-overlay {
		padding: 8px;
	}
	.segment { margin-bottom: 6px; }
	.segment--group.segment--imported {
		border-left: 2px solid var(--green);
		padding-left: 4px;
	}
	.entry {
		display: grid;
		grid-template-columns: 14px auto 1fr auto;
		gap: 6px;
		align-items: flex-start;
		padding: 4px 6px;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		margin-bottom: 2px;
	}
	.entry--pristine {
		grid-template-columns: auto 1fr auto;
	}
	.entry--imported {
		border-color: var(--green);
		background: color-mix(in srgb, var(--green) 6%, transparent);
	}
	.entry--claimed {
		border-color: var(--yellow);
		background: color-mix(in srgb, var(--yellow) 7%, transparent);
	}
	.entry--forked {
		border-color: var(--id-forked);
		background: color-mix(in srgb, var(--id-forked) 8%, transparent);
	}
	.entry--original { /* no border on purpose */ }

	/* Ranger-style outline cursor: bright bar + tinted background. Wins
	   over the provenance-derived border so the cursor stays legible
	   regardless of section state. */
	.entry--cursor {
		box-shadow: inset 4px 0 0 var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}

	/* Depth-N indentation: each nesting level shifts the row right. The
	   `--depth` custom property is set inline from the TOC entry. */
	.entry--pristine,
	.entry--nested {
		margin-left: calc(var(--depth, 0) * 18px);
	}

	/* Nested 30040 index — a collapsible folder. The caret expands the
	   subtree inline; `refocus` re-roots the reader on the sub-publication
	   and pushes a breadcrumb. */
	.entry--nested {
		display: flex;
		align-items: stretch;
		gap: 6px;
		padding: 2px 6px;
	}
	.nested-row {
		display: flex;
		align-items: center;
		gap: 8px;
		flex: 1;
		min-width: 0;
		padding: 6px 10px;
		border: 1px dashed var(--base3);
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
		color: var(--base6);
		cursor: pointer;
		text-align: left;
	}
	.nested-row:hover {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 13%, transparent);
		color: var(--fg);
	}
	.nested-row[aria-expanded='true'] {
		border-style: solid;
		background: color-mix(in srgb, var(--id-yours) 11%, transparent);
	}
	.nested-caret {
		min-width: 1ch;
		color: var(--id-yours);
		font-size: var(--t-3xs);
	}
	.nested-icon { color: var(--id-yours); font-size: var(--t-base); line-height: 1; }
	.nested-title {
		font-weight: 600;
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nested-count {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		white-space: nowrap;
	}
	.nested-count--empty { font-style: italic; }
	.nested-count--pending { color: var(--id-yours); font-style: italic; }
	.nested-refocus-btn {
		background: none;
		border: 1px dashed var(--base3);
		border-radius: var(--r-sm);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.07em;
		padding: 0 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.nested-refocus-btn:hover {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 13%, transparent);
	}

	/* Per-section / per-nested-index fetch (radial arrow). Pairs with
	 * .spin for the loading state — borrows the existing keyframe so a
	 * pending fetch reads visually the same as a streaming tree node. */
	.nested-fetch-btn,
	.section-fetch,
	.title-fetch {
		background: none;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 0 6px;
		min-width: 22px;
		height: 22px;
		line-height: 20px;
		cursor: pointer;
		text-align: center;
	}
	.title-fetch {
		margin-left: 6px;
		font-size: var(--t-sm);
	}
	.nested-fetch-btn:hover:not(:disabled),
	.section-fetch:hover:not(:disabled),
	.title-fetch:hover:not(:disabled) {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 13%, transparent);
	}
	.nested-fetch-btn:disabled,
	.section-fetch:disabled,
	.title-fetch:disabled {
		opacity: 0.6;
		cursor: progress;
	}

	/* Outline tree controls — expand/collapse the whole hierarchy at once. */
	.outline-treebar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 8px 0;
	}
	.outline-treebar__label {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.07em;
		color: var(--fg-muted);
	}
	.outline-treebar__spacer { flex: 1; }
	.outline-treebar__btn {
		background: none;
		border: 1px solid var(--panel-border);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.outline-treebar__btn:hover {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}

	/* Cross-publication breadcrumb trail. */
	.crumbs {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 4px;
		padding: 6px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		flex-shrink: 0;
	}
	.crumb {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 6px;
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--id-yours);
		cursor: pointer;
		max-width: 36ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.crumb:hover { border-color: var(--id-yours); }
	.crumb--current {
		color: var(--base6);
		cursor: default;
		font-weight: 700;
	}
	/* Loop tail — crumbs ahead of the marker, reachable by cycling back. */
	.crumb--ahead {
		color: var(--base5);
		font-style: italic;
	}
	.crumb--ahead:hover { border-color: var(--base5); }
	.crumb-sep { color: var(--base5); font-size: var(--t-xs); }
	.crumb-prev {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 8px;
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--id-yours);
		cursor: pointer;
	}
	.crumb-prev:hover:not(:disabled) { border-color: var(--id-yours); }
	.crumb-prev:disabled { color: var(--base5); cursor: default; opacity: 0.5; }
	.crumb-graph {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 8px;
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--id-yours);
		cursor: pointer;
	}
	.crumb-graph:hover { border-color: var(--id-yours); }
	.crumb-graph.active {
		background: color-mix(in srgb, var(--id-yours) 16%, transparent);
		border-color: var(--id-yours);
	}

	/* Depth stepper in the toolbar. */
	/* Flat/nested level tag sitting where the depth control used to live. */
	.level-hint {
		display: inline-flex;
		align-items: center;
		margin-left: 4px;
		padding: 1px 6px;
		border-radius: var(--r-md);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		letter-spacing: 0.06em;
		white-space: nowrap;
	}
	/* Flat = green, reads "self-contained, loaded whole". */
	.level-hint--flat {
		background: color-mix(in srgb, var(--state-online) 14%, transparent);
		color: var(--state-online);
	}
	/* Nested = the structural purple, paired with the depth knob beside it. */
	.level-hint--nested {
		background: color-mix(in srgb, var(--id-imported) 14%, transparent);
		color: var(--id-imported);
	}
	.depth-knob {
		display: inline-flex;
		align-items: center;
		gap: 3px;
		margin-left: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.depth-knob__label { text-transform: uppercase; letter-spacing: 0.06em; }
	.depth-knob__val { min-width: 1ch; text-align: center; color: var(--base6); }
	.depth-knob__step {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1;
		padding: 1px 5px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.depth-knob__step:disabled { opacity: 0.4; cursor: not-allowed; }
	.depth-knob__step:hover:not(:disabled) { border-color: var(--id-yours); }

	/* Spinner for pending tree nodes while the loader is streaming. */
	.spin {
		display: inline-block;
		animation: reader-spin 1s linear infinite;
	}
	@keyframes reader-spin {
		to {
			transform: rotate(360deg);
		}
	}

	.rail {
		font-family: var(--font-mono);
		color: var(--green);
		font-size: var(--t-md);
		line-height: 1;
		padding-top: 6px;
	}
	.lock {
		flex-shrink: 0;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		font-size: var(--t-sm);
		padding: 0 6px;
		cursor: pointer;
		color: var(--base6);
		align-self: flex-start;
	}
	.lock--unlocked {
		border-color: var(--yellow);
		color: var(--yellow);
	}
	.lock--placeholder { opacity: 0.3; cursor: default; }
	.lock:hover:not(:disabled):not(.lock--placeholder) {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.lock:disabled { opacity: 0.6; cursor: not-allowed; }

	.entry-body { min-width: 0; }

	.row-actions {
		display: flex;
		flex-direction: column;
		gap: 2px;
		align-self: flex-start;
	}
	.row-btn {
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		font-size: var(--t-2xs);
		padding: 0 4px;
		min-width: 22px;
		cursor: pointer;
		color: var(--base6);
		font-family: var(--font-mono);
	}
	.row-btn:hover:not(:disabled) {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.row-btn:disabled { opacity: 0.35; cursor: not-allowed; }
	.row-btn.remove:hover:not(:disabled) {
		border-color: var(--red);
		color: var(--red);
	}

	.hint {
		padding: 12px;
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
		text-align: center;
		margin: 0;
	}
</style>
