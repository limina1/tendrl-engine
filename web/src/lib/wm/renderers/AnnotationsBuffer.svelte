<script lang="ts">
	// The annotations table — one buffer kind (`annotations:<root>`) with its
	// own views: by section · by author · timeline · free · collective. Beside
	// the reader or filling the window is the WM's split state, not a mode
	// of the table. Contract: docs/annotations-table-contract.org.
	//
	// Engine-side: which 30041s anchor to the root, their anchors/roles/
	// request pointers (`insertions/load`), the author/parent histograms
	// (`counts`), the request list. Here: arranging what came back
	// (`table-model.ts`) and view state. No Svelte lifecycle hooks (buffer
	// renderers unmount on every switch): props + $effect teardown, view
	// state parked in store.bufferState. Loaders keep their bookkeeping in a
	// plain `book` object and the effects that start them depend only on
	// version counters — never on state the loaders write (the query-flood
	// lesson, project_effect_async_state_loop).
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import { shell } from '../shell.svelte';
	import { mobileNav } from '../mobile-nav.svelte';
	import type { Buffer } from '../types';
	import { onStoreEvent, concernsRoot, type StoreEvent } from '$lib/store-events.svelte';
	import { prefetchAuthors, getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import type { TocEntry } from '$lib/types';
	import {
		rootFromAnnotationsId,
		flattenToc,
		buildCards,
		layoutCards,
		buildSpell,
		parseSpell,
		authorBars,
		DEFAULT_VIEW,
		type AnnotationsViewState,
		type LayoutMode,
		type Section,
		type TableCard
	} from '$lib/components/annotations/table-model';
	import AnnotationsSpell from '$lib/components/annotations/AnnotationsSpell.svelte';
	import AnnotationsTable from '$lib/components/annotations/AnnotationsTable.svelte';
	import AnnotationsList from '$lib/components/annotations/AnnotationsList.svelte';
	import SpellComposerModal from '$lib/components/SpellComposerModal.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();
	const app = getAppState();

	const root = $derived(rootFromAnnotationsId(buffer.id));
	const isMobile = $derived(shell.mode === 'mobile');
	const MODES: LayoutMode[] = ['section', 'author', 'timeline', 'free', 'collective'];
	const modes = $derived(isMobile ? MODES.filter((m) => m !== 'free') : MODES);

	// ----- View state: read once, written on teardown -----
	// svelte-ignore state_referenced_locally
	const saved = store.bufferState.get(buffer.id) as Partial<AnnotationsViewState> | undefined;
	let view = $state<AnnotationsViewState>({
		...DEFAULT_VIEW,
		...(saved ?? {}),
		pins: { ...(saved?.pins ?? {}) },
		scroll: { ...(saved?.scroll ?? DEFAULT_VIEW.scroll) }
	});
	// The table's scroll offsets, kept off the reactive graph (written on
	// every scroll event; only the teardown reads them).
	const scrollBook = { x: view.scroll.x, y: view.scroll.y };
	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, {
				...$state.snapshot(view),
				scroll: { x: scrollBook.x, y: scrollBook.y }
			} satisfies AnnotationsViewState);
			app.setHoverAnchor(null);
		};
	});

	// Mobile Back walks layout changes (a teleport, not a page turn) and the
	// shell's view switch lists the table in the ☰ drawer.
	$effect(() => {
		const id = buffer.id;
		mobileNav.registerViewProvider(id, {
			capture: () => ({ mode: view.mode }),
			apply: (v) => {
				const m = v.mode as LayoutMode;
				if (MODES.includes(m)) view.mode = m;
			}
		});
		return () => mobileNav.unregisterViewProvider(id);
	});

	// ----- Loaders -----
	let sections = $state<Section[]>([]);
	let pubTitle = $state<string | null>(null);
	let notes = $state<api.LoadedInsertion[]>([]);
	let counts = $state<api.InsertionCounts | undefined>(undefined);
	let requests = $state<api.AnnotationRequest[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	const book = { toc: false, notes: false, requests: false, notesAgain: false };

	function walkToc(entries: TocEntry[], acc: { addr: TocEntry['addr']; title: string | null; depth: number }[]) {
		for (const e of entries) {
			acc.push({ addr: e.addr, title: e.title, depth: e.depth });
			if (e.children?.length) walkToc(e.children, acc);
		}
		return acc;
	}
	async function loadToc(r: string) {
		if (book.toc) return;
		book.toc = true;
		const [, pk, ...d] = r.split(':');
		try {
			const res = await api.getPublication(pk, d.join(':'), 'local_only', 2);
			sections = flattenToc(walkToc(res.toc, []));
			pubTitle = res.publication?.title ?? null;
			error = null;
		} catch (e) {
			error = api.errorMessage(e, 'Failed to load the publication');
		} finally {
			book.toc = false;
			loading = false;
		}
	}
	function sinceTs(days: number | null): number | undefined {
		return days == null ? undefined : Math.floor(Date.now() / 1000) - days * 86400;
	}
	async function loadNotes(r: string, since: number | undefined) {
		if (book.notes) {
			book.notesAgain = true;
			return;
		}
		book.notes = true;
		try {
			const resp = await api.loadInsertions({ root: r, policy: 'local_only', counts: true, since });
			notes = resp.insertions;
			counts = resp.counts;
			const authors = new Set(resp.insertions.map((i) => i.event.pubkey));
			if (authors.size) prefetchAuthors([...authors]);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Loading annotations failed'), 'error', 5000);
		} finally {
			book.notes = false;
			if (book.notesAgain) {
				book.notesAgain = false;
				void loadNotes(r, since);
			}
		}
	}
	async function loadRequests(r: string) {
		if (book.requests) return;
		book.requests = true;
		try {
			const resp = await api.listRequests({ root: r, status: 'all' });
			requests = resp.requests;
		} catch {
			requests = [];
		} finally {
			book.requests = false;
		}
	}

	$effect(() => {
		const r = root;
		if (!r) return;
		untrack(() => void loadToc(r));
	});
	// Notes re-read on the version counter (a note published from the
	// composer / over MCP) and when the time window changes — `sinceDays`
	// is view state the loader never writes.
	$effect(() => {
		const r = root;
		void app.insertionsVersion;
		const since = sinceTs(view.sinceDays);
		if (!r) return;
		untrack(() => void loadNotes(r, since));
	});
	$effect(() => {
		const r = root;
		void app.requestsVersion;
		if (!r) return;
		untrack(() => void loadRequests(r));
	});

	// Live: the store stream names what just landed; re-read the affected
	// slice after a short debounce. A callback, not an effect, so the
	// loaders' writes cannot re-trigger it.
	const liveTimers: Record<string, ReturnType<typeof setTimeout> | undefined> = {};
	function liveDebounce(key: string, fn: () => void) {
		clearTimeout(liveTimers[key]);
		liveTimers[key] = setTimeout(fn, 300);
	}
	function onLiveStoreEvent(ev: StoreEvent) {
		const r = root;
		if (!r || !concernsRoot(ev, r)) return;
		if (ev.type === 'request') {
			liveDebounce('requests', () => void loadRequests(r));
			return;
		}
		if (ev.kind === 30041 || ev.kind === 30040) {
			liveDebounce('notes', () => void loadNotes(r, sinceTs(view.sinceDays)));
			if (ev.coordinate === r) liveDebounce('toc', () => void loadToc(r));
		}
	}
	$effect(() => {
		const off = onStoreEvent(onLiveStoreEvent);
		return () => {
			off();
			for (const t of Object.values(liveTimers)) clearTimeout(t);
		};
	});

	// ----- Model -----
	const allCards = $derived(buildCards(notes, requests));
	const cards = $derived.by(() => {
		const by = view.by;
		const since = sinceTs(view.sinceDays);
		return allCards.filter(
			(c) => (by.length === 0 || by.includes(c.author)) && (since == null || c.createdAt >= since)
		);
	});
	const unsignedRequests = $derived(requests.filter((r) => !r.coordinate).length);
	const bars = $derived(authorBars(counts, unsignedRequests));
	const spellText = $derived(root ? buildSpell(root, view) : '');
	const secWith = $derived(new Set(cards.map((c) => c.parent)).size);
	const countByParent = $derived.by(() => {
		const m = new Map<string, number>();
		for (const c of cards) m.set(c.parent, (m.get(c.parent) ?? 0) + 1);
		return m;
	});
	const authorLabel = (pk: string) => getAuthorDisplayName(pk);

	let width = $state(0);
	const layout = $derived.by(() =>
		layoutCards(
			cards,
			sections,
			view.mode,
			{ expanded: view.expanded, pins: view.pins },
			Math.max(420, width),
			authorLabel
		)
	);
	// Keep the cursor inside the placed list as filters change.
	$effect(() => {
		const n = layout.placed.length;
		if (view.cursor >= n) view.cursor = Math.max(0, n - 1);
	});

	const hotAddr = $derived(
		app.hoverAnchor && root && app.hoverAnchor.root === root ? app.hoverAnchor.addr : null
	);

	// ----- Actions -----
	function setMode(m: LayoutMode) {
		if (!modes.includes(m) || view.mode === m) return;
		view.mode = m;
		mobileNav.pushViewChange(buffer.id);
	}
	function cycleMode(dir: 1 | -1) {
		const i = modes.indexOf(view.mode);
		setMode(modes[(i + dir + modes.length) % modes.length]);
	}
	function toggleBy(pk: string) {
		view.by = view.by.includes(pk) ? view.by.filter((x) => x !== pk) : [...view.by, pk];
	}
	function cycleSince() {
		view.sinceDays = view.sinceDays == null ? 7 : view.sinceDays === 7 ? 30 : null;
	}
	function applySpell(text: string): boolean {
		const r = root;
		if (!r) return false;
		const parsed = parseSpell(text, r);
		if (!parsed) {
			app.pushToast('That spell names another publication — the table stays on this one', 'error', 4000);
			return false;
		}
		view.by = parsed.by;
		view.sinceDays = parsed.sinceDays;
		return true;
	}
	let wandOpen = $state(false);
	function toggleExpand(card: TableCard) {
		view.expanded = view.expanded === card.id ? null : card.id;
	}
	function hover(card: TableCard | null) {
		const r = root;
		if (!r) return;
		app.setHoverAnchor(card ? { root: r, addr: card.parent, count: countByParent.get(card.parent) ?? 1 } : null);
	}
	function jumpTo(card: TableCard) {
		const r = root;
		if (!r) return;
		app.requestReaderJump(r, card.parent);
	}
	function jumpToAndSwitch(card: TableCard) {
		const r = root;
		if (!r) return;
		jumpTo(card);
		store.openBuffer({
			className: 'work',
			buffer: { id: `reader:${r}`, kind: 'reader', label: 'reader', kicker: pubTitle ?? r.split(':').slice(2).join(':') }
		});
	}
	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		void app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}
	function openCard(card: TableCard) {
		if (card.note) {
			const title = card.note.event.tags.find((t) => t[0] === 'title')?.[1] ?? null;
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:event:${card.note.event.id}`,
					kind: 'reader',
					label: 'section',
					kicker: title ?? card.note.event.id.slice(0, 8)
				}
			});
		} else if (card.req) {
			// A request opens the way RequestCard's menu does: its signed
			// 30041 in the event menu; an unsigned record has no event.
			if (card.req.coordinate) openCoordinate(card.req.coordinate);
			else {
				view.expanded = card.id;
				app.pushToast('Unsigned local record — nothing signed to open', 'info');
			}
		}
	}
	function menuCard(card: TableCard) {
		if (card.note) {
			openCoordinate(card.note.coordinate);
		} else if (card.req?.coordinate) {
			openCoordinate(card.req.coordinate);
		}
	}
	function openRequestOf(card: TableCard) {
		if (card.request) openCoordinate(card.request);
	}
	function pin(id: string, x: number, y: number) {
		view.pins = { ...view.pins, [id]: { x, y } };
		app.pushToast('Card pinned — positions are yours, never published', 'info');
	}
	function unpin(id: string) {
		if (!view.pins[id]) return;
		const next = { ...view.pins };
		delete next[id];
		view.pins = next;
		app.pushToast('Card unpinned', 'info');
	}

	// ----- Keys (desktop, table focused) -----
	function handleNav(action: NavAction): boolean {
		const n = layout.placed.length;
		switch (action) {
			case 'down':
				if (!n) return false;
				view.cursor = Math.min(n - 1, view.cursor + 1);
				return true;
			case 'up':
				if (!n) return false;
				view.cursor = Math.max(0, view.cursor - 1);
				return true;
			case 'top':
				view.cursor = 0;
				return true;
			case 'bottom':
				view.cursor = Math.max(0, n - 1);
				return true;
			case 'select': {
				const p = layout.placed[view.cursor];
				if (!p) return false;
				toggleExpand(p.card);
				return true;
			}
			case 'left':
				cycleMode(-1);
				return true;
			case 'right':
				cycleMode(1);
				return true;
			case 'menu':
				cycleMode(1);
				return true;
			case 'back':
				if (view.expanded) {
					view.expanded = null;
					return true;
				}
				return false;
		}
		return false;
	}
	$effect(() => {
		const id = buffer.id;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});
	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && view.expanded) {
			view.expanded = null;
			e.stopPropagation();
		}
	}

	const legend = $derived(
		view.mode === 'free'
			? 'drag pins a card · double-click unpins · positions are yours, never published'
			: 'j/k moves the cursor · Enter expands · ←/→ or m switch layout · hover a card lights its section · → §n scrolls the reader'
	);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="abuf" data-root={root ?? ''} tabindex="-1" role="region" aria-label="Annotations table" {onkeydown}>
	{#if !root}
		<div class="empty">not an annotations buffer: <code>{buffer.id}</code></div>
	{:else if isMobile}
		<AnnotationsList
			{cards}
			{sections}
			{bars}
			{view}
			spell={spellText}
			onMode={setMode}
			onToggleBy={toggleBy}
			onWand={() => (wandOpen = true)}
			onAnchor={jumpToAndSwitch}
			onOpen={openCard}
		/>
	{:else}
		<AnnotationsSpell
			spell={spellText}
			mode={view.mode}
			{modes}
			by={view.by}
			{bars}
			sinceDays={view.sinceDays}
			{secWith}
			secTotal={sections.length}
			selectedCount={cards.length}
			totalCount={allCards.length}
			onapply={applySpell}
			onwand={() => (wandOpen = true)}
			ontoggleby={toggleBy}
			oncyclesince={cycleSince}
			onmode={setMode}
		/>
		{#if error}
			<div class="empty">{error}</div>
		{:else if !loading && allCards.length === 0}
			<div class="empty">no annotations on this publication yet — notes and requests land here as they are written</div>
		{/if}
		<AnnotationsTable
			{layout}
			mode={view.mode}
			{sections}
			cursor={view.cursor}
			expanded={view.expanded}
			{hotAddr}
			bind:width
			initialScroll={{ x: scrollBook.x, y: scrollBook.y }}
			onscroll={(x, y) => {
				scrollBook.x = x;
				scrollBook.y = y;
			}}
			onselect={(i) => (view.cursor = i)}
			onanchor={jumpTo}
			onexpand={toggleExpand}
			onopen={openCard}
			onmenu={menuCard}
			onrequest={openRequestOf}
			onhover={hover}
			onpin={pin}
			onunpin={unpin}
		/>
		<div class="legend">{legend} · colour = author, as in highlights · {cards.length} cards</div>
	{/if}
	{#if wandOpen}
		<SpellComposerModal query={spellText} onclose={() => (wandOpen = false)} />
	{/if}
</div>

<style>
	.abuf { display: flex; flex-direction: column; min-height: 0; height: 100%; outline: none; }
	.empty { padding: var(--s-2) var(--s-3); font-family: var(--font-mono); font-size: var(--t-xs); color: var(--base5); font-style: italic; border-bottom: 1px solid var(--panel-border); }
	code { font-family: var(--font-mono); color: var(--base6); }
	.legend { padding: 4px 10px; border-top: 1px solid var(--panel-border); font: var(--t-3xs) / 1.4 var(--font-mono); color: var(--base5); }
</style>
