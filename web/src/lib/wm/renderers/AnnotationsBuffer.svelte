<script lang="ts">
	// The annotations ledger — one buffer kind (`annotations:<root>`): rows
	// grouped by § · author · kind · time, two rows of chrome that filter,
	// each row opening in place (or in a detail pane when wide), a board
	// view, and the workflows hanging off the rows. Beside the reader or
	// filling the window is the WM's split state, not a mode of the
	// ledger. Contract: docs/annotations-ledger-contract.org.
	//
	// Engine-side: which 30041s anchor to the root, their anchors / roles /
	// request pointers / reply pointers (`insertions/load`), the author
	// histogram (`counts`), the per-annotation reply counts (`replies`),
	// the request list. Here: arranging what came back (`ledger-model.ts`)
	// and view state. No Svelte lifecycle hooks (buffer renderers unmount
	// on every switch): props + $effect teardown, view state parked in
	// store.bufferState. Loaders keep their bookkeeping in a plain `book`
	// object and the effects that start them depend only on version
	// counters — never on state the loaders write
	// (project_effect_async_state_loop).
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore, type NavAction } from '../buffer-store.svelte';
	import { shell } from '../shell.svelte';
	import { mobileNav } from '../mobile-nav.svelte';
	import type { Buffer } from '../types';
	import { onStoreEvent, concernsRoot, type StoreEvent } from '$lib/store-events.svelte';
	import { prefetchAuthors, getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import type { TocEntry } from '$lib/types';
	import {
		rootFromAnnotationsId,
		flattenToc,
		buildRows,
		filterRows,
		facetCounts,
		groupRows,
		flatOrder,
		buildSpell,
		parseSpell,
		relTime,
		GROUPS,
		DEFAULT_VIEW,
		type LedgerView,
		type LedgerRow,
		type LedgerKind,
		type GroupBy,
		type Section
	} from '$lib/components/annotations/ledger-model';
	import LedgerHeader from '$lib/components/annotations/LedgerHeader.svelte';
	import LedgerRows from '$lib/components/annotations/LedgerRows.svelte';
	import LedgerBody from '$lib/components/annotations/LedgerBody.svelte';
	import LedgerBoard from '$lib/components/annotations/LedgerBoard.svelte';
	import SpellComposerModal from '$lib/components/SpellComposerModal.svelte';

	let { buffer }: { buffer: Buffer } = $props();
	const store = getActiveStore();
	const app = getAppState();
	const root = $derived(rootFromAnnotationsId(buffer.id));
	const isMobile = $derived(shell.mode === 'mobile');

	// ----- View state: read once, written on teardown -----
	// svelte-ignore state_referenced_locally
	const saved = store.bufferState.get(buffer.id) as Partial<LedgerView> | undefined;
	let view = $state<LedgerView>({
		...DEFAULT_VIEW,
		...(saved ?? {}),
		by: [...(saved?.by ?? [])],
		secs: [...(saved?.secs ?? [])],
		kinds: [...(saved?.kinds ?? [])],
		collapsed: [...(saved?.collapsed ?? [])]
	});
	// The scroll offset, kept off the reactive graph (written on every
	// scroll event; only the teardown reads it).
	const scrollBook = { top: view.scroll };
	$effect(() => {
		return () => {
			store.bufferState.set(buffer.id, { ...$state.snapshot(view), scroll: scrollBook.top } satisfies LedgerView);
			app.setHoverAnchor(null);
		};
	});
	// Mobile Back walks grouping changes (a teleport, not a page turn) and
	// the shell's view switch lists the ledger in the ☰ drawer.
	$effect(() => {
		const id = buffer.id;
		mobileNav.registerViewProvider(id, {
			capture: () => ({ group: view.group }),
			apply: (v) => {
				const g = v.group as GroupBy;
				if (GROUPS.includes(g)) view.group = g;
			}
		});
		return () => mobileNav.unregisterViewProvider(id);
	});

	// ----- Loaders -----
	let sections = $state<Section[]>([]);
	let pubTitle = $state<string | null>(null);
	let pubPubkey = $state<string | null>(null);
	let notes = $state<api.LoadedInsertion[]>([]);
	let counts = $state<api.InsertionCounts | undefined>(undefined);
	let replies = $state<Record<string, number>>({});
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
			pubPubkey = pk;
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
			const resp = await api.loadInsertions({ root: r, policy: 'local_only', counts: true, replies: true, since });
			notes = resp.insertions;
			counts = resp.counts;
			replies = resp.replies ?? {};
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
	// slice after a short debounce, and mark the row `.new` once. A
	// callback, not an effect, so the loaders' writes cannot re-trigger it.
	const liveTimers: Record<string, ReturnType<typeof setTimeout> | undefined> = {};
	let fresh = $state<Set<string>>(new Set());
	function liveDebounce(key: string, fn: () => void) {
		clearTimeout(liveTimers[key]);
		liveTimers[key] = setTimeout(fn, 300);
	}
	function markFresh(id: string) {
		fresh = new Set([...fresh, id]);
		setTimeout(() => {
			const next = new Set(fresh);
			next.delete(id);
			fresh = next;
		}, 2500);
	}
	function onLiveStoreEvent(ev: StoreEvent) {
		const r = root;
		if (!r || !concernsRoot(ev, r)) return;
		if (ev.type === 'request') {
			markFresh(ev.id);
			liveDebounce('requests', () => void loadRequests(r));
			return;
		}
		if (ev.kind === 30041 || ev.kind === 30040 || ev.kind === 818) {
			if (ev.coordinate) markFresh(ev.coordinate);
			liveDebounce('notes', () => void loadNotes(r, sinceTs(view.sinceDays)));
			if (ev.coordinate === r) liveDebounce('toc', () => void loadToc(r));
		}
		if (ev.kind === 1111) liveDebounce('notes', () => void loadNotes(r, sinceTs(view.sinceDays)));
	}
	$effect(() => {
		const off = onStoreEvent(onLiveStoreEvent);
		return () => {
			off();
			for (const t of Object.values(liveTimers)) clearTimeout(t);
		};
	});

	// ----- Model -----
	const allRows = $derived(buildRows(notes, requests, sections, replies));
	const rows = $derived(filterRows(allRows, view));
	const facets = $derived(facetCounts(allRows, view));
	const who = (pk: string) => (app.identityStatus?.pubkey === pk ? 'you' : getAuthorDisplayName(pk));
	// Authors in bar order: the engine's histogram (descending count, kind-0
	// labels), then any author the rows know that it does not (an 818's
	// requester, an unsigned record).
	const authors = $derived.by(() => {
		const seen = new Set<string>();
		const out: { pubkey: string; label: string }[] = [];
		for (const b of [...(counts?.by_author ?? [])].sort((x, y) => y.count - x.count)) {
			seen.add(b.value);
			out.push({ pubkey: b.value, label: who(b.value) === b.value ? (b.label ?? b.value.slice(0, 8) + '…') : who(b.value) });
		}
		const extra = new Map<string, number>();
		for (const r of allRows) if (!seen.has(r.author)) extra.set(r.author, (extra.get(r.author) ?? 0) + 1);
		for (const [pk] of [...extra.entries()].sort((a, b) => b[1] - a[1])) out.push({ pubkey: pk, label: pk ? who(pk) : 'unsigned' });
		return out;
	});
	const groups = $derived(groupRows(rows, sections, view, who));
	const flat = $derived(flatOrder(groups, rows, view));
	const spellText = $derived(root ? buildSpell(root, view, sections) : '');
	const ownsRoot = $derived(!!app.identityStatus?.pubkey && pubPubkey === app.identityStatus.pubkey);
	const titleOf = (id: string) => allRows.find((r) => r.id === id)?.title;
	const countByParent = $derived.by(() => {
		const m = new Map<string, number>();
		for (const r of rows) m.set(r.parent, (m.get(r.parent) ?? 0) + 1);
		return m;
	});
	// Keep the cursor inside the flat list as filters change.
	$effect(() => {
		const n = flat.length;
		if (view.cursor >= n) view.cursor = Math.max(0, n - 1);
	});
	const hotAddr = $derived(app.hoverAnchor && root && app.hoverAnchor.root === root ? app.hoverAnchor.addr : null);
	const hotSec = $derived.by(() => {
		if (localHotSec != null) return localHotSec;
		if (!hotAddr) return null;
		if (hotAddr === root) return -1;
		const i = sections.findIndex((s) => s.coordinate === hotAddr);
		return i >= 0 ? i : null;
	});
	let localHotSec = $state<number | null>(null);
	let width = $state(0);
	const wide = $derived(!isMobile && width >= 900);
	const usePane = $derived(wide && view.detail === 'pane' && view.view === 'rows');
	const expandedRow = $derived(view.expanded ? allRows.find((r) => r.id === view.expanded) ?? null : null);
	const emptyText = $derived(
		error ? error : !loading && allRows.length === 0 ? 'no annotations on this publication yet — notes and requests land here as they are written' : rows.length === 0 && allRows.length > 0 ? 'nothing matches — clear a facet above' : null
	);

	// ----- Actions -----
	function setGroup(g: GroupBy) {
		if (view.group === g) return;
		view.group = g;
		view.cursor = 0;
		mobileNav.pushViewChange(buffer.id);
	}
	function cycleGroup(dir: 1 | -1) {
		const i = GROUPS.indexOf(view.group);
		setGroup(GROUPS[(i + dir + GROUPS.length) % GROUPS.length]);
	}
	function toggleIn<T>(list: T[], v: T): T[] {
		return list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
	}
	function toggleBy(pk: string) {
		view.by = toggleIn(view.by, pk);
		view.cursor = 0;
	}
	function toggleSec(sec: number) {
		view.secs = toggleIn(view.secs, sec);
		view.cursor = 0;
	}
	function toggleKind(k: LedgerKind) {
		view.kinds = toggleIn(view.kinds, k);
		view.cursor = 0;
	}
	function pickDay(day: number) {
		view.day = view.day === day ? null : day;
		view.sinceDays = null;
	}
	function cycleSince() {
		view.sinceDays = view.sinceDays == null ? 7 : view.sinceDays === 7 ? 30 : null;
		view.day = null;
	}
	function applySpell(text: string): boolean {
		const r = root;
		if (!r) return false;
		const parsed = parseSpell(text, r, sections);
		if (!parsed) {
			app.pushToast('That spell names another publication — the ledger stays on this one', 'error', 4000);
			return false;
		}
		view.by = parsed.by;
		view.secs = parsed.secs;
		view.kinds = parsed.kinds;
		view.sinceDays = parsed.sinceDays;
		view.day = parsed.day;
		return true;
	}
	function toggleGroupCollapsed(key: string) {
		view.collapsed = toggleIn(view.collapsed, key);
	}
	let wandOpen = $state(false);
	function select(row: LedgerRow) {
		const i = flat.findIndex((r) => r.id === row.id);
		if (i >= 0) view.cursor = i;
		view.expanded = view.expanded === row.id ? null : row.id;
	}
	function reveal(id: string) {
		const target = allRows.find((r) => r.id === id) ?? allRows.find((r) => r.req?.id === id);
		if (!target) return;
		// Make the row reachable: drop the facets hiding it, expand its group.
		if (!rows.some((r) => r.id === target.id)) {
			view.by = [];
			view.secs = [];
			view.kinds = [];
			view.sinceDays = null;
			view.day = null;
		}
		view.collapsed = [];
		view.expanded = target.id;
		queueMicrotask(() => {
			const i = flat.findIndex((r) => r.id === target.id);
			if (i >= 0) view.cursor = i;
			setTimeout(() => {
				document.querySelector<HTMLElement>(`.abuf[data-root="${CSS.escape(root ?? '')}"] .row[data-id="${CSS.escape(target.id)}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
			}, 50);
		});
	}
	// A reader card's "≡ row" — root + nonce, applied once.
	let appliedRevealNonce = 0;
	$effect(() => {
		const rv = app.revealAnnotation;
		const r = root;
		if (!rv || !r || rv.root !== r || rv.nonce === appliedRevealNonce) return;
		appliedRevealNonce = rv.nonce;
		untrack(() => reveal(rv.id));
	});
	function hover(row: LedgerRow | null) {
		const r = root;
		if (!r) return;
		if (!row) {
			localHotSec = null;
			app.setHoverAnchor(null);
			return;
		}
		// A group header hands a bare `{ sec }` — light the section only.
		const parent = row.parent ?? (row.sec < 0 ? r : (sections[row.sec]?.coordinate ?? r));
		app.setHoverAnchor({ root: r, addr: parent, count: countByParent.get(parent) ?? 1 });
	}
	function jumpTo(row: LedgerRow) {
		jumpSec(row.sec);
	}
	function jumpSec(sec: number) {
		const r = root;
		if (!r) return;
		const addr = sec < 0 ? r : (sections[sec]?.coordinate ?? r);
		app.requestReaderJump(r, addr);
		if (isMobile) {
			store.openBuffer({
				className: 'work',
				buffer: { id: `reader:${r}`, kind: 'reader', label: 'reader', kicker: pubTitle ?? r.split(':').slice(2).join(':') }
			});
		}
	}
	function openCoordinate(coord: string) {
		const [k, pk, ...d] = coord.split(':');
		void app.openAddressableInModal({ kind: Number(k), pubkey: pk, d_tag: d.join(':') });
	}
	function openRow(row: LedgerRow) {
		if (row.kind === 'kasten' && row.note) {
			const [, pk, ...d] = row.id.split(':');
			void app.openAddressableInModal({ kind: 30040, pubkey: pk, d_tag: d.join(':') });
			return;
		}
		if (row.note) {
			const title = row.note.event.tags.find((t) => t[0] === 'title')?.[1] ?? null;
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:event:${row.note.event.id}`,
					kind: 'reader',
					label: 'section',
					kicker: title ?? row.note.event.id.slice(0, 8)
				}
			});
		} else if (row.req) {
			if (row.req.coordinate) openCoordinate(row.req.coordinate);
			else {
				view.expanded = row.id;
				app.pushToast('Unsigned local record — nothing signed to open', 'info');
			}
		}
	}
	function annotate(secHint?: number) {
		const r = root;
		if (!r) return;
		const sec = secHint ?? (view.secs.length === 1 ? view.secs[0] : sections.length ? 0 : -1);
		const s = sec >= 0 ? sections[sec] : undefined;
		const parent = s && s.coordinate.startsWith('30041:') ? s.coordinate : undefined;
		app.annotationRequest = {
			root: r,
			rootTitle: pubTitle,
			parent,
			parentTitle: parent ? (s?.title ?? null) : null
		};
	}
	function boardAction(what: 'answer' | 'review' | 'relink' | 'open' | 'menu', row: LedgerRow) {
		const r = root;
		if (!r) return;
		if (what === 'answer' && row.req) {
			app.annotationRequest = {
				root: r,
				rootTitle: pubTitle,
				parent: row.parent !== r ? row.parent : undefined,
				parentTitle: row.sec >= 0 ? (sections[row.sec]?.title ?? null) : null,
				parentEventId: row.req.parent_event_id,
				output: row.req.output,
				answer: row.req
			};
		} else if (what === 'review') app.openMergeRequestReview(row.note?.merge_request?.id ?? row.id.slice(4));
		else if (what === 'relink') app.relink = { root: r, rootTitle: pubTitle, definitions: [row.id], promoted: row.glossary ? [row.id] : [] };
		else if (what === 'open') openRow(row);
		else if (row.kind === 'mr') void app.getEventForModal(row.note?.merge_request?.id ?? row.id.slice(4));
		else if (row.kind === 'request') {
			if (row.req?.coordinate) openCoordinate(row.req.coordinate);
		} else openCoordinate(row.id);
	}
	function reload() {
		const r = root;
		if (!r) return;
		void loadNotes(r, sinceTs(view.sinceDays));
		void loadRequests(r);
	}

	// ----- Keys (desktop, ledger focused) -----
	function handleNav(action: NavAction): boolean {
		const n = flat.length;
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
				const r = flat[view.cursor];
				if (!r) return false;
				view.expanded = view.expanded === r.id ? null : r.id;
				return true;
			}
			case 'left':
				cycleGroup(-1);
				return true;
			case 'right':
				cycleGroup(1);
				return true;
			case 'menu':
				cycleGroup(1);
				return true;
			case 'insert':
				annotate(flat[view.cursor]?.sec);
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
		if (e.target && /INPUT|TEXTAREA|SELECT/.test((e.target as HTMLElement).tagName)) return;
		if (e.key === 'Escape' && view.expanded) {
			view.expanded = null;
			e.stopPropagation();
		} else if (e.key === 'g') {
			cycleGroup(1);
			e.stopPropagation();
		} else if (e.key === 'n') {
			annotate(flat[view.cursor]?.sec);
			e.stopPropagation();
		}
	}
	const legend = 'j/k moves · Enter opens · n annotates at the cursor · g cycles grouping · hover lights the section · § scrolls the reader';
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="abuf" class:phone={isMobile} data-root={root ?? ''} tabindex="-1" role="region" aria-label="Annotations ledger" {onkeydown} bind:clientWidth={width}>
	{#if !root}
		<div class="empty">not an annotations buffer: <code>{buffer.id}</code></div>
	{:else}
		<LedgerHeader
			spell={spellText}
			{view}
			counts={facets}
			{sections}
			{authors}
			shown={rows.length}
			total={allRows.length}
			phone={isMobile}
			onapply={applySpell}
			onwand={() => (wandOpen = true)}
			onannotate={() => annotate()}
			ongroup={setGroup}
			onview={(v) => (view.view = v)}
			ontoggleby={toggleBy}
			ontogglesec={toggleSec}
			ontogglekind={toggleKind}
			onpickday={pickDay}
			oncyclesince={cycleSince}
			onhoversec={(sec) => (localHotSec = sec)}
		/>
		{#if view.view === 'board' && !isMobile}
			<LedgerBoard
				{rows}
				{view}
				{sections}
				{hotSec}
				cursorId={flat[view.cursor]?.id ?? null}
				{who}
				onselect={select}
				onhover={hover}
				onjump={jumpSec}
				onaction={boardAction}
			/>
		{:else if usePane}
			<div class="split2">
				<LedgerRows
					{groups}
					{rows}
					{flat}
					{view}
					{sections}
					{root}
					rootTitle={pubTitle}
					{ownsRoot}
					{hotSec}
					{fresh}
					phone={isMobile}
					inline={false}
					empty={emptyText}
					{who}
					{titleOf}
					initialScroll={scrollBook.top}
					onscroll={(t) => (scrollBook.top = t)}
					onselect={select}
					onanchor={jumpTo}
					onhover={hover}
					ontogglegroup={toggleGroupCollapsed}
					onopen={openRow}
					onreveal={reveal}
					onchanged={reload}
				/>
				<div class="detail">
					{#if expandedRow}
						<div class="detail__dh" style="--au: {expandedRow.author ? pubkeyToColor(expandedRow.author) : 'var(--base5)'}">
							<b>{expandedRow.title}</b>
							<span class="detail__sp"></span>
							<span class="detail__by"><span class="detail__dot"></span>{expandedRow.author ? who(expandedRow.author) : 'unsigned'} · {relTime(expandedRow.createdAt)}</span>
						</div>
						<LedgerBody
							row={expandedRow}
							answers={rows.filter((r) => r.request === expandedRow.id)}
							{sections}
							{root}
							rootTitle={pubTitle}
							{ownsRoot}
							{who}
							requestTitle={expandedRow.request ? titleOf(expandedRow.request) : undefined}
							onopen={openRow}
							onreveal={reveal}
							onchanged={reload}
						/>
					{:else}
						<div class="detail__empty">select a row — Enter or click<br /><br />the full annotation reads here, with its actions; the rows stay one line each</div>
					{/if}
				</div>
			</div>
		{:else}
			<LedgerRows
				{groups}
				{rows}
				{flat}
				{view}
				{sections}
				{root}
				rootTitle={pubTitle}
				{ownsRoot}
				{hotSec}
				{fresh}
				phone={isMobile}
				inline={true}
				empty={emptyText}
				{who}
				{titleOf}
				initialScroll={scrollBook.top}
				onscroll={(t) => (scrollBook.top = t)}
				onselect={select}
				onanchor={jumpTo}
				onhover={hover}
				ontogglegroup={toggleGroupCollapsed}
				onopen={openRow}
				onreveal={reveal}
				onchanged={reload}
			/>
		{/if}
		{#if !isMobile}
			<div class="legend">
				<span>{legend}</span>
				<span>colour = author, as in highlights</span>
				<span class="legend__sp"></span>
				{#if wide && view.view === 'rows'}
					<span class="seg seg--detail">
						<button class:on={view.detail === 'inline'} onclick={() => (view.detail = 'inline')}>open in place</button>
						<button class:on={view.detail === 'pane'} onclick={() => (view.detail = 'pane')}>detail pane</button>
					</span>
				{/if}
				<span>{rows.length} rows</span>
			</div>
		{/if}
	{/if}
	{#if wandOpen}
		<SpellComposerModal query={spellText} onclose={() => (wandOpen = false)} />
	{/if}
</div>

<style>
	.abuf { display: flex; flex-direction: column; min-height: 0; height: 100%; outline: none; position: relative; }
	.empty { padding: var(--s-2) var(--s-3); font-family: var(--font-mono); font-size: var(--t-xs); color: var(--base5); font-style: italic; border-bottom: 1px solid var(--panel-border); }
	code { font-family: var(--font-mono); color: var(--base6); }
	.split2 { display: flex; flex: 1; min-height: 0; }
	.split2 > :global(.rows) { flex: 1; }
	.detail { flex: 0 0 380px; border-left: 1px solid var(--panel-border); overflow: auto; background: var(--base1); }
	.detail__dh {
		padding: 8px 12px 0;
		display: flex;
		gap: 8px;
		align-items: center;
		font: var(--t-2xs) / 1 var(--font-mono);
		color: var(--base5);
	}
	.detail__dh b { color: var(--fg); font: 600 var(--t-sm) / 1.3 var(--font-sans); min-width: 0; }
	.detail__sp { flex: 1; }
	.detail__by { flex: none; white-space: nowrap; display: inline-flex; gap: 5px; align-items: center; }
	.detail__dot { width: 7px; height: 7px; border-radius: 50%; background: var(--au); display: inline-block; }
	.detail__empty { padding: 30px 14px; color: var(--base5); font: var(--t-xs) / 1.4 var(--font-mono); text-align: center; }
	.legend {
		padding: 3px 10px;
		border-top: 1px solid var(--panel-border);
		font: var(--t-3xs) / 1.4 var(--font-mono);
		color: var(--base5);
		flex-shrink: 0;
		display: flex;
		gap: 10px;
		flex-wrap: wrap;
		align-items: center;
	}
	.legend__sp { flex: 1; }
	.seg { display: inline-flex; border: 1px solid var(--panel-border); border-radius: var(--r-md); overflow: hidden; }
	.seg button { border: 0; border-radius: 0; border-right: 1px solid var(--panel-border); min-height: 18px; padding: 0 7px; font: var(--t-3xs) / 1 var(--font-mono); background: transparent; color: var(--base6); cursor: pointer; }
	.seg button:last-child { border-right: 0; }
	.seg button.on { background: var(--base2); color: var(--fg); }
</style>
