<script lang="ts">
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import SectionCard from '$lib/components/SectionCard.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import type {
		LazySection,
		PublicationDetail,
		TagEntry,
		ViewMode,
		ContextItem
	} from '$lib/types';
	import type { Buffer } from '../types';
	import { sectionState, segmentSections } from '$lib/compose/state';
	import { buildThread, threadContainsId, type ThreadNode } from '$lib/discussions/thread';
	import type { Highlight } from '$lib/discussions/highlights';
	import CommentThread from '$lib/components/CommentThread.svelte';
	import HighlightList from '$lib/components/HighlightList.svelte';
	import HighlightsDrawer, {
		type DrawerHighlight
	} from '$lib/components/HighlightsDrawer.svelte';
	import { prefetchAuthors, refreshAuthors } from '$lib/discussions/authors.svelte';

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	let publication = $state<PublicationDetail | null>(null);
	let pristineSections = $state<LazySection[]>([]);
	// Default to outline. If the buffer carries `?highlight=<id>` the
	// effect below switches to paginated so the highlight overlay is
	// visible right away.
	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);
	let loading = $state(true);
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
			// Pull the full event payloads so we can render inline threads
			// in addition to the badge counts. Counts are derived from the
			// same set client-side — one round trip serves both views.
			const resp = await api.getDiscussionList({
				addresses: addrs,
				kinds: [1111, 9802],
				policy,
				bypassOffline: options.bypassOffline,
				limit: 500
			});
			const byAddr: Record<string, api.DiscussionEvent[]> = {};
			const counts: Record<string, api.DiscussionCount> = {};
			for (const a of addrs) {
				byAddr[a] = [];
				counts[a] = { comments: 0, highlights: 0 };
			}
			const addrSet = new Set(addrs);
			for (const ev of resp.events) {
				const matched = new Set<string>();
				for (const tag of ev.tags) {
					if (!tag || tag.length < 2) continue;
					if (tag[0] !== 'a' && tag[0] !== 'A') continue;
					const value = tag[1];
					if (addrSet.has(value)) matched.add(value);
				}
				for (const m of matched) {
					byAddr[m].push(ev);
					if (ev.kind === 1111) counts[m].comments += 1;
					else if (ev.kind === 9802) counts[m].highlights += 1;
				}
			}
			discussionEvents = byAddr;
			discussionCounts = counts;
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

	function parseBufferId(id: string): { pubkey: string; dTag: string } | null {
		const { core } = splitBufferId(id);
		const match = core.match(/^reader:\d+:([0-9a-fA-F]{64}):(.+)$/);
		if (!match) return null;
		return { pubkey: match[1].toLowerCase(), dTag: match[2] };
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

	// Build a thread tree per section addr from the loaded discussion
	// events. Highlights (kind 9802) are excluded from the tree — they
	// surface as section badges + overlays, not inline comments.
	const threadsBySection = $derived.by(() => {
		const out: Record<string, ThreadNode[]> = {};
		for (const [addr, events] of Object.entries(discussionEvents)) {
			const comments = events.filter((e) => e.kind === 1111);
			out[addr] = buildThread(comments);
		}
		return out;
	});

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
		loading = true;
		try {
			const resp = await api.getPublication(
				parsedAddr.pubkey,
				parsedAddr.dTag,
				'local_first'
			);
			publication = resp.publication;
			pristineSections = resp.toc.map((entry, i) => ({
				addr: entry.addr,
				title: entry.title,
				content: null,
				position: i,
				status: 'pending' as const
			}));
			// Eager-load every section in the background. Outline mode only
			// shows titles and never triggers loads, and continuous's
			// IntersectionObserver root is nested inside another scroll
			// container so visibility events are unreliable. handleLoadSection
			// is idempotent (early-returns on loading/loaded), so view-mode
			// hooks just no-op once a load is already in flight.
			for (let i = 0; i < pristineSections.length; i++) {
				handleLoadSection(i);
			}
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	// Standalone-event reader: a `reader:event:<id>` buffer renders one
	// section, no TOC walk, and defaults to paginated view so the user
	// reads exactly the event they searched for.
	async function loadEvent(eventId: string) {
		loading = true;
		try {
			const resp = await api.getEvent(eventId);
			const ev = resp.event as
				| { kind?: number; pubkey?: string; tags?: string[][]; content?: string; created_at?: number }
				| null;
			if (!ev) {
				error = 'Event not found';
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
				index: ev
			};
			pristineSections = [
				{
					addr,
					title: titleTag,
					content: ev.content ?? '',
					position: 0,
					status: 'loaded' as const
				}
			];
			viewMode = 'paginated';
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		buffer.id;
		load();
	});

	// After a publication finishes loading, hydrate discussion counts.
	// Phase A is local_only for instant rendering; phase B is fetch_always
	// when online so newly-published comments/highlights show up without
	// requiring a manual refresh. Guard with `loadedFor` so per-section
	// loads (which mutate pristineSections as their status flips) don't
	// re-trigger this effect — the counts only depend on the addresses,
	// which are known as soon as the TOC arrives.
	let loadedFor = $state<string | null>(null);
	$effect(() => {
		const id = buffer.id;
		const done = !loading && !!publication && pristineSections.length > 0;
		if (!done) return;
		if (loadedFor === id) return;
		loadedFor = id;
		untrack(async () => {
			await loadDiscussionCounts('local_only');
			if (app.networkStatus?.mode === 'online') {
				refreshDiscussions();
			}
		});
	});

	// Reset the guard when the buffer changes so a different publication
	// triggers a fresh fetch.
	$effect(() => {
		buffer.id;
		untrack(() => {
			loadedFor = null;
			discussionCounts = {};
			discussionRefreshedAt = null;
		});
	});

	function handleLoadSection(index: number) {
		if (isDraftMode) return; // draft sections are already loaded
		if (index < 0 || index >= pristineSections.length) return;
		const cur = pristineSections[index];
		if (cur.status === 'loaded' || cur.status === 'loading') return;
		if (loadingPromises.has(index)) return;
		pristineSections[index] = { ...cur, status: 'loading' };
		if (!parsedAddr) return;
		const promise = (async () => {
			try {
				const resp = await api.getSection(
					parsedAddr.pubkey,
					parsedAddr.dTag,
					index
				);
				pristineSections[index] = {
					...pristineSections[index],
					title: resp.section.title ?? pristineSections[index].title,
					content: resp.section.content,
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
		const hasOtherDraft =
			!!existingSrc &&
			parsedAddr &&
			(existingSrc.pubkey.toLowerCase() !== parsedAddr.pubkey ||
				existingSrc.d_tag !== parsedAddr.dTag);
		if (hasOtherDraft) {
			const ok = confirm(
				`A draft is already in progress for "${existingSrc!.d_tag}". Discard it and start a new draft for this publication?`
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
			if (action === 'down') {
				outlineCursor = Math.min(sections.length - 1, outlineCursor + 1);
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'up') {
				outlineCursor = Math.max(0, outlineCursor - 1);
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'top') {
				outlineCursor = 0;
				queueMicrotask(scrollOutlineCursorIntoView);
				return true;
			}
			if (action === 'bottom') {
				outlineCursor = sections.length - 1;
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
			return false;
		}
		// continuous: j/k page-scroll by viewport; h/l cycles modes;
		// gg / G snap to top / bottom of the document.
		if (viewMode === 'continuous') {
			if (action === 'left' || action === 'right') {
				cycleView(action === 'right' ? 1 : -1);
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
</script>

<div class="reader-wrap">
	<div class="toolbar">
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
		<button
			class="json-btn"
			onclick={openPublicationJson}
			disabled={!publication}
			title="Open the publication index (kind 30040) in the JSON viewer"
		>JSON</button>
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
			onclick={editInComposer}
			disabled={!publication}
			title={isDraftMode ? 'Continue editing this draft' : 'Open this publication in the composer'}
		>Edit</button>
		<button
			class="discussions-refresh"
			onclick={refreshDiscussions}
			disabled={discussionLoading || !publication}
			title={app.networkStatus?.mode === 'online'
				? 'Pull new comments and highlights from relays'
				: 'Offline — pull comments and highlights from relays anyway (manual override)'}
		>
			{discussionLoading ? '…' : 'Refresh discussions'}
		</button>
	</div>

	{#if loading}
		<div class="empty"><p>Loading…</p></div>
	{:else if error}
		<div class="empty"><p>Error: {error}</p></div>
	{:else if !publication}
		<div class="empty"><p>No publication loaded</p></div>
	{:else}
		{#if publication.title}
			<div class="title">{publication.title}</div>
		{/if}
		{#if highlightText !== null}
			<div class="hl-banner">
				<span class="hl-banner__label">Viewing highlight</span>
				<span class="hl-banner__sample" title={highlightText}>
					“{highlightText.length > 120 ? highlightText.slice(0, 120) + '…' : highlightText}”
				</span>
				{#if highlightMeta}
					<span class="hl-banner__meta">
						by <code>{highlightMeta.author.slice(0, 12)}…</code>
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
					<span class="ds-badge ds-badge--comments">
						{totalDiscussion.comments} comment{totalDiscussion.comments === 1 ? '' : 's'}
					</span>
					<span class="ds-sep">·</span>
					<button
						class="ds-badge ds-badge--highlights ds-badge--button"
						onclick={() => (drawerOpen = !drawerOpen)}
						title={drawerOpen ? 'Hide highlights drawer' : 'Open highlights drawer (grouped by author, click to scroll)'}
					>
						{totalDiscussion.highlights} highlight{totalDiscussion.highlights === 1 ? '' : 's'}
					</button>
					{#if publicationDiscussion.comments > 0 || publicationDiscussion.highlights > 0}
						<span class="ds-sep">·</span>
						<span class="ds-on-index" title="Comments/highlights on the publication index itself (kind 30040)">
							index: c {publicationDiscussion.comments} h {publicationDiscussion.highlights}
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
					<!-- Pristine outline: same SectionCard as before, plus a per-
					     section lock toggle. The first lock click seeds compose
					     state from this publication and switches into draft mode. -->
					<div class="outline-overlay" bind:this={outlineEl}>
						{#each pristineSections as section, i (`${i}:${section.addr.pubkey}:${section.addr.d_tag}`)}
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
							>
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
									{#if highlightN > 0}
										<button
											class="section-action section-action--highlights"
											class:open={highlightsOpen}
											onclick={(e) => {
												e.stopPropagation();
												toggleOutlineHighlights(i);
											}}
											title="{highlightsOpen ? 'Hide' : 'Show'} the {highlightN} highlight{highlightN === 1 ? '' : 's'} on this section"
										>highlights {highlightN}</button>
									{/if}
									{#if commentN > 0}
										<button
											class="section-action section-action--comments"
											class:open={commentsOpen}
											onclick={(e) => {
												e.stopPropagation();
												toggleOutlineComments(i);
											}}
											title="{commentsOpen ? 'Hide' : 'Show'} the threaded comments on this section"
										>comments {commentN}</button>
									{/if}
									<button
										class="section-action section-action--more"
										onclick={(e) => {
											e.stopPropagation();
											openSectionJsonBySection(section);
										}}
										title="View this section's raw event in the JSON viewer"
									>⋮</button>
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
						{/each}
					</div>
				{/if}
			{:else if viewMode === 'continuous'}
				<ContinuousView
					{sections}
					publication={{
						title: publication.title,
						summary: publication.summary
					}}
					onload={isDraftMode ? undefined : handleLoadSection}
					onviewjson={openSectionJsonBySection}
					highlightsFor={highlightsForSection}
					focusedHighlightId={parsedHighlightId}
					threadsFor={threadsForSection}
					focusedCommentId={parsedFocusCommentId}
				/>
			{:else}
				<PaginatedView
					{sections}
					{currentSection}
					onnavigate={handleNavigate}
					onload={isDraftMode ? undefined : handleLoadSection}
					onsectionjson={openSectionJsonByIndex}
					highlightsFor={highlightsForSection}
					focusedHighlightId={parsedHighlightId}
					threadsFor={threadsForSection}
					focusedCommentId={parsedFocusCommentId}
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
		font-size: 9px;
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
	.ds-badge { color: var(--base6); }
	.ds-badge--comments { color: var(--id-yours, var(--base7)); }
	.ds-badge--highlights { color: var(--state-online, var(--base6)); }
	.ds-badge--empty { color: var(--base5); font-style: italic; }
	.ds-sep { color: var(--base4); }
	.ds-on-index { color: var(--base5); }
	.ds-source { color: var(--base4); font-size: calc(var(--t-xs) - 1px); }

	/* The highlights badge is interactive — toggles the drawer. */
	.ds-badge--button {
		background: transparent;
		border: 1px solid transparent;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		cursor: pointer;
		font: inherit;
	}
	.ds-badge--button:hover {
		border-color: var(--state-online);
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
	.hl-banner__meta code {
		background: var(--bg-surface);
		padding: 0 4px;
		border-radius: var(--r-sm);
	}

	.section-actions {
		display: flex;
		gap: 4px;
		align-items: center;
		padding: 0 6px;
		flex-shrink: 0;
		align-self: center;
	}
	.section-action {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 6px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: var(--bg-surface);
		color: var(--base6);
		line-height: 1.4;
		cursor: pointer;
	}
	.section-action:hover {
		filter: brightness(1.2);
	}
	.section-action--highlights {
		border-color: var(--state-online);
		color: var(--state-online);
	}
	.section-action--highlights.open {
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
	}
	.section-action--comments {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.section-action--comments.open {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.section-action--more {
		padding: 1px 7px;
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
	}
	.content { flex: 1; overflow: auto; min-height: 0; }
	.empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--base5);
		font-size: var(--t-sm);
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

	.rail {
		font-family: var(--font-mono);
		color: var(--green);
		font-size: 14px;
		line-height: 1;
		padding-top: 6px;
	}
	.lock {
		flex-shrink: 0;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		font-size: 12px;
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
		font-size: 10px;
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
