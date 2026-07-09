<script lang="ts">
	import { untrack } from 'svelte';
	import { isEventSigned, type NostrEvent, type PublicationSummary } from '$lib/types';
	import * as api from '$lib/api';
	import type { Profile } from '$lib/api';
	import { fetchFromRelaysWithPrompt } from '$lib/fetch/relay-fetch.svelte';
	import { promptText } from '$lib/wm/text-prompt.svelte';
	import { getAppState } from '$lib/state.svelte';
	import PoolStateBadges from './PoolStateBadges.svelte';
	import SpellClauseBlock from './SpellClauseBlock.svelte';
	import ProfileName from './ProfileName.svelte';
	import { getActiveStore, type NavAction } from '$lib/wm/buffer-store.svelte';

	const app = getAppState();
	const store = getActiveStore();

	let {
		pubkey,
		bufferId,
		onopenpub,
		onopenaddr,
		oncomment,
		onback
	}: {
		pubkey: string;
		/** Owning buffer id — used to register a nav handler so j/k/Enter/m
		 *  work in this view via the global keymap. Optional so direct
		 *  embeddings (outside a WM buffer) still work without nav. */
		bufferId?: string;
		onopenpub?: (pub_summary: PublicationSummary) => void;
		/** Open any non-30040 addressable (article, wiki, section, etc.) in
		 *  the reader. The buffer-id pattern reader:&lt;kind&gt;:&lt;pk&gt;:&lt;dtag&gt; works
		 *  for these uniformly so the host can route them without caring
		 *  about kind-specific layout. */
		onopenaddr?: (addr: { kind: number; pubkey: string; d_tag: string }, title: string | null) => void;
		/** Open a NIP-22 comment (kind 1111) in its discussion view — the
		 *  comment isn't a standalone reader destination, so the host routes
		 *  it to a DiscussionViewBuffer that resolves the thread context. */
		oncomment?: (event: NostrEvent) => void;
		onback: () => void;
	} = $props();

	type Tab = 'publications' | 'articles' | 'wikis' | 'specs' | 'sections' | 'highlights' | 'comments' | 'spells';
	let activeTab: Tab = $state('publications');
	let profile = $state<Profile | null>(null);
	let publications = $state<PublicationSummary[]>([]);
	// NIP-23 long-form articles (kind 30023) and NKBIP-02 wikis (kind
	// 30818). Both are addressable replaceable events, deduped by d_tag
	// keeping the newest version.
	type AddressableSummary = {
		addr: { kind: number; pubkey: string; d_tag: string };
		title: string | null;
		summary: string | null;
		image: string | null;
		created_at: number;
		/** Provenance — same fields PublicationSummary already carries.
		 *  Threaded through so the draft / remote / relay-label pill lights
		 *  up on articles + wikis the same way it does on publications. */
		signed: boolean;
		relays: string[];
	};
	let articles = $state<AddressableSummary[]>([]);
	let wikis = $state<AddressableSummary[]>([]);
	// NIP specifications (kind 30817) — community-authored protocol specs.
	// Addressable markdown documents, same shape as wikis.
	let specs = $state<AddressableSummary[]>([]);
	let sections = $state<NostrEvent[]>([]);
	// NIP-84 highlights (kind 9802) this author has made on other content.
	let highlights = $state<NostrEvent[]>([]);
	let comments = $state<NostrEvent[]>([]);
	// NIP-A7 spells (kind 777) — the author's spellbook. Parsed engine-side
	// (/api/v1/spell/list); clicking one runs it and shows its result feed
	// in place.
	let spells = $state<api.SpellEntry[]>([]);
	// Spellbooks (kind 30777): the user's curated sets, entries by any
	// author. The tab renders one merged flat list — book entries ∪
	// authored spells, deduped by event id — with per-row provenance
	// (author byline when foreign, book marker when curated).
	let spellBooks = $state<api.SpellBookView[]>([]);
	let spellResults = $state<api.SpellOutcome | null>(null);
	// How the current results were produced — replayed with an `until`
	// cursor by "load older" (the engine pages the spell's source stage).
	let spellRunContext = $state<{ id: string; args: Record<string, string> } | null>(null);
	let spellResultsExhausted = $state(false);
	let spellRunning = $state<string | null>(null);

	type SpellRow = {
		/** Resolved spell — null for a book reference we can't resolve locally. */
		entry: api.SpellEntry | null;
		ref: api.SpellbookEntryRef | null;
		/** Book title/d when the row came from a book. */
		fromBook: string | null;
	};
	const spellRows = $derived.by<SpellRow[]>(() => {
		const rows: SpellRow[] = [];
		const seen = new Set<string>();
		for (const view of spellBooks) {
			const label = view.book.title || view.book.d;
			for (const be of view.entries) {
				const id = be.entry?.event.id ?? be.reference.event_id;
				if (seen.has(id)) continue;
				seen.add(id);
				rows.push({ entry: be.entry, ref: be.reference, fromBook: label });
			}
		}
		for (const entry of spells) {
			if (seen.has(entry.event.id)) continue;
			seen.add(entry.event.id);
			rows.push({ entry, ref: null, fromBook: null });
		}
		return rows.sort(
			(a, b) => (b.entry?.event.created_at ?? 0) - (a.entry?.event.created_at ?? 0)
		);
	});

	// "Add to my book" — bookmark any spell into my default book:
	// engine derives the next book version, we sign it, save local-first.
	async function addToMyBook(entry: api.SpellEntry) {
		try {
			const t = await api.spellBookTemplate({ action: 'add', spell_event_id: entry.event.id });
			const { signed_event } = await api.signTemplate({ template: t.template });
			await api.saveSpellBook({ event: signed_event, broadcast: false });
			app.pushToast(
				t.created ? 'Spellbook created — broadcast it when ready' : 'Added to your spellbook',
				'success'
			);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Add to book failed'), 'error');
		}
	}

	async function broadcastBook(view: api.SpellBookView) {
		try {
			const res = await api.saveSpellBook({ event: view.event, broadcast: true });
			const ok = res.broadcast_results?.filter((r) => r.success).length ?? 0;
			app.pushToast(`Book broadcast — accepted by ${ok} relay${ok === 1 ? '' : 's'}`, ok > 0 ? 'success' : 'error');
			await loadLocal(pubkey);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Broadcast failed'), 'error');
		}
	}
	// PIPE-card `v` chevron: per-spell expanded flag + fetched stage
	// blocks (inspect is local-only; fetched once per card, on demand).
	let spellStagesOpen = $state<Record<string, boolean>>({});
	let spellStageCache = $state<Record<string, api.StageInspection[]>>({});

	function toggleSpellStages(id: string) {
		spellStagesOpen[id] = !spellStagesOpen[id];
		if (spellStagesOpen[id] && !spellStageCache[id]) {
			api.inspectSpell({ id, policy: 'local_only' })
				.then((r) => { spellStageCache[id] = r.stages ?? []; })
				.catch(() => { spellStageCache[id] = []; });
		}
	}
	let loading = $state(true);
	let fetching = $state(false);

	function getTag(event: NostrEvent, name: string): string | null {
		const tag = event.tags.find(t => t[0] === name);
		return tag ? tag[1] : null;
	}

	/** Detect a NIP-54 fork marker on a kind-30040 index event: an `a` or
	 *  `e` tag whose 4th element is the literal "fork". Mirrors the engine
	 *  detection in `Publication::from_event` so client-built summaries
	 *  agree with server-derived ones. */
	function hasForkMarker(event: NostrEvent): boolean {
		return event.tags.some(t => (t[0] === 'a' || t[0] === 'e') && t[3] === 'fork');
	}

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}

	function dedupAddressable(events: NostrEvent[], kind: number): AddressableSummary[] {
		// Replaceable events: same (kind, pubkey, d_tag) → latest version
		// (highest created_at) wins. Same dedup the publication path uses,
		// generalized so 30023 and 30818 reuse it.
		const byDtag = new Map<string, AddressableSummary>();
		for (const e of events) {
			const d_tag = getTag(e, 'd') || '';
			const existing = byDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			byDtag.set(d_tag, {
				addr: { kind, pubkey: e.pubkey, d_tag },
				title: getTag(e, 'title'),
				summary: getTag(e, 'summary'),
				image: getTag(e, 'image'),
				created_at: e.created_at,
				signed: isEventSigned(e.sig),
				relays: e.relays ?? []
			});
		}
		return [...byDtag.values()].sort((a, b) => b.created_at - a.created_at);
	}

	async function loadLocal(pk: string) {
		const [prof, pubResult, artResult, wikiResult, specResult, secResult, hlResult, comResult, spellResult, bookResult] =
			await Promise.all([
				api.getProfile(pk),
				api.queryEvents([{ kinds: [30040], authors: [pk], limit: tabLimits.publications }], 'local_only'),
				api.queryEvents([{ kinds: [30023], authors: [pk], limit: tabLimits.articles }], 'local_only'),
				api.queryEvents([{ kinds: [30818], authors: [pk], limit: tabLimits.wikis }], 'local_only'),
				api.queryEvents([{ kinds: [30817], authors: [pk], limit: tabLimits.specs }], 'local_only'),
				api.queryEvents([{ kinds: [30041], authors: [pk], limit: tabLimits.sections }], 'local_only'),
				api.queryEvents([{ kinds: [9802], authors: [pk], limit: tabLimits.highlights }], 'local_only'),
				api.queryEvents([{ kinds: [1111], authors: [pk], limit: tabLimits.comments }], 'local_only'),
				api.listSpells(pk, tabLimits.spells, 'local_only'),
				api.getSpellBooks(pk, 'local_only')
			]);
		profile = prof.found ? prof : null;
		// 30040 publications: same dedup, but kept as the existing
		// PublicationSummary shape so the openpub callback contract
		// downstream (and the section_count display) is preserved.
		const byDtag = new Map<string, PublicationSummary>();
		for (const e of (pubResult.events as NostrEvent[])) {
			const d_tag = getTag(e, 'd') || '';
			const existing = byDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			byDtag.set(d_tag, {
				addr: { kind: 30040, pubkey: e.pubkey, d_tag },
				title: getTag(e, 'title'),
				summary: getTag(e, 'summary'),
				image: getTag(e, 'image'),
				author_pubkey: e.pubkey,
				version: null,
				created_at: e.created_at,
				// A fork-marker `a` tag is not a content reference — strip
				// it out of section_count so the displayed count matches
				// the engine's (Publication::from_event applies the same
				// filter via the fork-marker branch in its tag loop).
				section_count: e.tags.filter(t => t[0] === 'a' && t[3] !== 'fork').length,
				relays: e.relays ?? [],
				signed: isEventSigned(e.sig),
				forked: hasForkMarker(e)
			} as PublicationSummary);
		}
		publications = [...byDtag.values()].sort((a, b) => b.created_at - a.created_at);
		articles = dedupAddressable(artResult.events as NostrEvent[], 30023);
		wikis = dedupAddressable(wikiResult.events as NostrEvent[], 30818);
		specs = dedupAddressable(specResult.events as NostrEvent[], 30817);
		// Sections are replaceable (kind 30041) — collapse versions by
		// d-tag, newest wins, so a section isn't listed once per edit.
		const secByDtag = new Map<string, NostrEvent>();
		for (const e of (secResult.events as NostrEvent[])) {
			const d_tag = getTag(e, 'd') || '';
			const existing = secByDtag.get(d_tag);
			if (existing && existing.created_at >= e.created_at) continue;
			secByDtag.set(d_tag, e);
		}
		sections = [...secByDtag.values()].sort((a, b) => b.created_at - a.created_at);
		highlights = (hlResult.events as NostrEvent[]).sort((a, b) => b.created_at - a.created_at);
		comments = (comResult.events as NostrEvent[]).sort((a, b) => b.created_at - a.created_at);
		spells = spellResult.entries; // engine returns newest-first
		spellBooks = bookResult.books;
	}

	// Tab → which event kinds to pull. The top-bar Fetch button pulls
	// the union; per-tab refresh buttons scope to a single kind so the
	// user can do targeted refreshes without hammering relays for
	// everything every time.
	const TAB_KINDS: Record<Tab, number[]> = {
		publications: [30040],
		articles: [30023],
		wikis: [30818],
		specs: [30817],
		sections: [30041],
		highlights: [9802],
		comments: [1111],
		spells: [777, 30777]
	};
	const TAB_LABEL: Record<Tab, string> = {
		publications: 'publications',
		articles: 'articles',
		wikis: 'wikis',
		specs: 'specs',
		sections: 'sections',
		highlights: 'highlights',
		comments: 'comments',
		spells: 'spells'
	};

	let tabFetchingKinds = $state<number | null>(null);

	// ----- Backfill ("Load older") -----
	// Same local-first model as the feed: deepen the local query page
	// first; only when that surfaces nothing new, hit relays with an
	// `until` cursor at the oldest event shown. A tab is "exhausted"
	// when a relay round-trip adds nothing — cleared again by any
	// explicit fetch or a pubkey change.
	const BACKFILL_PAGE = 50;
	const TAB_BASE_LIMIT: Record<Tab, number> = {
		publications: 500,
		articles: 200,
		wikis: 200,
		specs: 200,
		sections: 200,
		highlights: 200,
		comments: 200,
		spells: 200
	};
	function freshTabFlags(): Record<Tab, boolean> {
		return {
			publications: false,
			articles: false,
			wikis: false,
			specs: false,
			sections: false,
			highlights: false,
			comments: false,
			spells: false
		};
	}
	// Deliberately NOT $state: no template reads it — it's imperative
	// paging bookkeeping consumed by loadLocal. Making it reactive loops
	// the load $effect (loadLocal's read re-arms the effect that resets
	// it, even under untrack — the ERR_INSUFFICIENT_RESOURCES flood).
	let tabLimits: Record<Tab, number> = { ...TAB_BASE_LIMIT };
	let exhausted = $state<Record<Tab, boolean>>(freshTabFlags());
	let loadingOlder = $state(false);

	function listFor(tab: Tab): Array<{ created_at: number }> {
		if (tab === 'publications') return publications;
		if (tab === 'articles') return articles;
		if (tab === 'wikis') return wikis;
		if (tab === 'specs') return specs;
		if (tab === 'sections') return sections;
		if (tab === 'highlights') return highlights;
		if (tab === 'spells') {
			// While a spell's result feed is shown, the cursor walks the
			// results; otherwise it walks the merged book+authored rows
			// (unresolved book references count as rows with no timestamp).
			return spellResults
				? spellResults.events
				: spellRows.map((r) => ({ created_at: r.entry?.event.created_at ?? 0 }));
		}
		return comments;
	}

	// Page a spell's result feed: re-run the spell with `until` set to just
	// before the source stage's oldest event, append what's new.
	async function loadOlderSpellResults() {
		const results = spellResults;
		if (!results || !spellRunContext || results.cmd === 'COUNT' || loadingOlder) return;
		if (results.oldest_source == null) {
			spellResultsExhausted = true;
			return;
		}
		loadingOlder = true;
		try {
			const older = await api.executeSpell({
				id: spellRunContext.id,
				args: spellRunContext.args,
				policy: 'local_first',
				mode_confirm: true,
				until: results.oldest_source - 1
			});
			const seen = new Set(results.events.map((e) => e.id));
			const fresh = older.events.filter((e) => !seen.has(e.id));
			const auxSeen = new Set(results.auxiliary.map((e) => e.id));
			const freshAux = older.auxiliary.filter((e) => !auxSeen.has(e.id));
			if (fresh.length === 0) spellResultsExhausted = true;
			spellResults = {
				...results,
				events: [...results.events, ...fresh],
				auxiliary: [...results.auxiliary, ...freshAux],
				count: results.events.length + fresh.length,
				provenance: { ...results.provenance, ...older.provenance },
				oldest_source: older.oldest_source
			};
			if (older.oldest_source == null) spellResultsExhausted = true;
		} catch (e) {
			console.error('Spell load older failed:', e);
			app.pushToast(api.errorMessage(e, 'Load older failed'), 'error');
		} finally {
			loadingOlder = false;
		}
	}

	async function handleLoadOlder() {
		const tab = activeTab;
		if (tab === 'spells' && spellResults) return loadOlderSpellResults();
		const list = listFor(tab);
		if (loadingOlder || list.length === 0) return;
		loadingOlder = true;
		try {
			const before = list.length;
			const oldest = Math.min(...list.map((e) => e.created_at));
			// Deepen the local page — older events may already be in the
			// db beyond the current query cap.
			tabLimits[tab] += BACKFILL_PAGE;
			await loadLocal(pubkey);
			if (listFor(tab).length > before) return;
			// Nothing new locally — page relays strictly older than what
			// is shown. Goes through the same confirm-gated fetch path as
			// the refresh buttons.
			await fetchFromRelaysWithPrompt(
				{
					title: `Load older ${TAB_LABEL[tab]}`,
					kinds: TAB_KINDS[tab],
					authors: [pubkey],
					limit: BACKFILL_PAGE,
					until: oldest - 1
				},
				{ isOnline }
			);
			await new Promise((r) => setTimeout(r, 400));
			await loadLocal(pubkey);
			if (listFor(tab).length <= before) exhausted[tab] = true;
		} catch (e) {
			console.error('Load older failed:', e);
		} finally {
			loadingOlder = false;
		}
	}

	const isOnline = $derived(app.networkStatus?.mode === 'auto');

	async function runFetch(opts: { title: string; kinds: number[] }) {
		console.debug('[ProfileView] fetch start', {
			title: opts.title,
			kinds: opts.kinds,
			pubkey,
			isOnline
		});
		const result = await fetchFromRelaysWithPrompt(
			{ title: opts.title, kinds: opts.kinds, authors: [pubkey], limit: 500 },
			{ isOnline }
		);
		console.debug('[ProfileView] fetch result', result);
		if (!result) return null;
		// nostrdb ingest is async on the engine side — give it a beat
		// before re-reading locally so the new events show up.
		await new Promise((r) => setTimeout(r, 400));
		await loadLocal(pubkey);
		return result;
	}

	async function handleFetch() {
		fetching = true;
		exhausted = freshTabFlags();
		try {
			await runFetch({
				title: `Fetch all events for ${profile?.display_name || profile?.name || pubkey.slice(0, 12) + '…'}`,
				kinds: [0, 30040, 30023, 30818, 30817, 30041, 9802, 1111, 777, 30777]
			});
			// Profile prefetch hits general relays unconditionally — names
			// don't go through the prompted flow because they're a side
			// effect of any fetch, not the primary target.
			await api.prefetchProfiles([pubkey]);
		} catch (e) {
			console.error('Fetch failed:', e);
		} finally {
			fetching = false;
		}
	}

	async function handleTabFetch(tab: Tab) {
		const kinds = TAB_KINDS[tab];
		tabFetchingKinds = kinds[0];
		exhausted[tab] = false;
		try {
			await runFetch({
				title: `Fetch ${TAB_LABEL[tab]} for ${profile?.display_name || profile?.name || pubkey.slice(0, 12) + '…'}`,
				kinds
			});
		} catch (e) {
			console.error('Tab fetch failed:', e);
		} finally {
			tabFetchingKinds = null;
		}
	}

	// ----- Running a spell -----
	// Clicking a spellbook entry executes it: prompt for each declared
	// argument (promptText — never window.prompt), run through the engine
	// with mode_confirm so Confirm mode raises the fetch-intent modal, and
	// show the result feed in place of the spellbook list.
	async function runSpell(entry: api.SpellEntry) {
		if (!entry.spell || entry.error) {
			// Unparseable spell — the raw event is all there is to show.
			app.eventModalData = entry.event;
			return;
		}
		if (entry.partial) {
			app.pushToast('Partial spell ($in.*) — only runs inside a pipeline', 'info');
			return;
		}
		if (spellRunning) return;
		const args: Record<string, string> = {};
		for (const name of entry.required_args) {
			const declared = entry.spell.params.find((p) => p.name === name);
			const value = await promptText({
				title: `Spell argument: ${name}`,
				placeholder: declared?.prompt ?? name,
				confirmLabel: 'Bind'
			});
			if (value === null) return; // cancelled
			args[name] = value;
		}
		spellRunning = entry.event.id;
		try {
			const outcome = await api.executeSpell({
				id: entry.event.id,
				args,
				policy: 'local_first',
				mode_confirm: true
			});
			spellResults = outcome;
			spellRunContext = { id: entry.event.id, args };
			spellResultsExhausted = false;
			cursor = 0;
			const label = outcome.name || entry.spell.name || 'spell';
			app.pushToast(
				outcome.cmd === 'COUNT'
					? `${label}: count ${outcome.count}`
					: `${label}: ${outcome.count} event${outcome.count === 1 ? '' : 's'}`,
				'success'
			);
		} catch (e) {
			console.error('Spell execution failed:', e);
			app.pushToast(api.errorMessage(e, 'Spell failed'), 'error');
		} finally {
			spellRunning = null;
		}
	}

	// Ignore is engine-persistent: list/book/execute all honor the ignore
	// list, so a mis-specced spell (or junk result) stays gone across
	// reloads and re-runs. Reversible from the ignore list.
	async function ignoreSpell(entry: api.SpellEntry) {
		try {
			await api.ignoreEvents([entry.event.id]);
			spells = spells.filter((e) => e.event.id !== entry.event.id);
			spellBooks = spellBooks.map((v) => ({
				...v,
				entries: v.entries.filter((en) => en.reference.event_id !== entry.event.id)
			}));
			app.pushToast('Spell ignored', 'info');
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Ignore failed'), 'error');
		}
	}

	async function ignoreResultEvent(ev: NostrEvent) {
		try {
			await api.ignoreEvents([ev.id]);
			if (spellResults) {
				const events = spellResults.events.filter((x) => x.id !== ev.id);
				spellResults = { ...spellResults, events, count: events.length };
			}
			app.pushToast('Event ignored', 'info');
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Ignore failed'), 'error');
		}
	}

	$effect(() => {
		const pk = pubkey; // sole dependency — reload only on profile switch
		loading = true;
		profile = null;
		publications = [];
		articles = [];
		wikis = [];
		specs = [];
		sections = [];
		highlights = [];
		comments = [];
		spells = [];
		spellBooks = [];
		spellResults = null;
		spellRunContext = null;
		spellResultsExhausted = false;
		spellStagesOpen = {};
		spellStageCache = {};
		tabLimits = { ...TAB_BASE_LIMIT };
		exhausted = freshTabFlags();
		loadLocal(pk).catch(() => {}).finally(() => { loading = false; });
	});

	// ----- Cursor + nav handler -----
	// One cursor index keyed by the active tab. j/k walk the active tab's
	// list, Enter / l opens the cursored item, m opens it in the event
	// menu. Resets when the tab changes so the cursor doesn't point past
	// the new list's end.

	let cursor = $state(0);
	let listEl: HTMLDivElement | undefined = $state();
	let tabsEl: HTMLDivElement | undefined = $state();

	// Keep the active tab visible as the rail scrolls — matters once
	// there are more kind-feeds than fit the pane width.
	$effect(() => {
		void activeTab;
		tabsEl?.querySelector('.tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
	});

	$effect(() => {
		// Reset cursor when the active tab swaps.
		void activeTab;
		untrack(() => { cursor = 0; });
	});

	function activeList(): Array<unknown> {
		return listFor(activeTab);
	}

	function scrollCursorIntoView() {
		if (!listEl) return;
		const row = listEl.querySelector<HTMLDivElement>(`[data-cursor="${cursor}"]`);
		if (!row) return;
		const listRect = listEl.getBoundingClientRect();
		const rowRect = row.getBoundingClientRect();
		if (rowRect.top < listRect.top) {
			listEl.scrollTop -= listRect.top - rowRect.top;
		} else if (rowRect.bottom > listRect.bottom) {
			listEl.scrollTop += rowRect.bottom - listRect.bottom;
		}
	}

	function openCursorItem() {
		const list = activeList();
		const item = list[cursor];
		if (!item) return;
		if (activeTab === 'publications') {
			onopenpub?.(item as PublicationSummary);
		} else if (activeTab === 'articles' || activeTab === 'wikis' || activeTab === 'specs') {
			const x = item as { addr: { kind: number; pubkey: string; d_tag: string }; title: string | null };
			onopenaddr?.(x.addr, x.title);
		} else if (activeTab === 'sections') {
			const sec = item as NostrEvent;
			const dTag = getTag(sec, 'd') || '';
			const title = getTag(sec, 'title') || dTag || '[Untitled]';
			onopenaddr?.({ kind: 30041, pubkey: sec.pubkey, d_tag: dTag }, title);
		} else if (activeTab === 'spells') {
			if (spellResults) {
				app.eventModalData = item as NostrEvent;
			} else {
				const row = spellRows[cursor];
				if (row?.entry) runSpell(row.entry);
			}
		} else {
			// Comments and highlights both route to the discussion view — it
			// resolves the thread / highlighted target the event points at.
			oncomment?.(item as NostrEvent);
		}
	}

	function openCursorMenu() {
		const list = activeList();
		const item = list[cursor];
		if (!item) return;
		if (activeTab === 'spells') {
			// The 777 event, or the cursored result while a feed is showing.
			const event = spellResults
				? (item as NostrEvent)
				: spellRows[cursor]?.entry?.event;
			if (event) app.eventModalData = event;
		} else if (activeTab === 'comments' || activeTab === 'highlights') {
			// Comments and highlights aren't addressable — feed the modal the
			// raw event.
			app.eventModalData = item as NostrEvent;
		} else if (activeTab === 'sections') {
			const sec = item as NostrEvent;
			const dTag = getTag(sec, 'd') || '';
			app.openAddressableInModal({ kind: 30041, pubkey: sec.pubkey, d_tag: dTag });
		} else {
			const addr = (item as { addr: { kind: number; pubkey: string; d_tag: string } }).addr;
			app.openAddressableInModal(addr);
		}
	}

	function handleNav(action: NavAction): boolean {
		const total = activeList().length;
		if (total === 0) return false;
		if (action === 'down') {
			cursor = Math.min(total - 1, cursor + 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'up') {
			cursor = Math.max(0, cursor - 1);
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'top') {
			cursor = 0;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'bottom') {
			cursor = total - 1;
			queueMicrotask(scrollCursorIntoView);
			return true;
		}
		if (action === 'select' || action === 'right') {
			openCursorItem();
			return true;
		}
		if (action === 'menu') {
			openCursorMenu();
			return true;
		}
		return false;
	}

	$effect(() => {
		if (!bufferId) return;
		const id = bufferId;
		const handler = handleNav;
		untrack(() => store.registerNavHandler(id, handler));
		return () => untrack(() => store.unregisterNavHandler(id));
	});

	// ----- Profile-bar hamburger menu -----
	// Copy npub / copy nprofile / ignore author. NIP-19 strings are
	// pre-encoded engine-side when the pubkey changes so copy() stays
	// synchronous — awaiting inside the click handler would lose the
	// clipboard user-gesture (same pattern as EventViewModal).

	let barMenuOpen = $state(false);
	let npub = $state('');
	let nprofile = $state('');

	$effect(() => {
		const pk = pubkey;
		npub = '';
		nprofile = '';
		Promise.all([
			api.encode({ kind: 'npub', pubkey: pk }),
			api.encode({ kind: 'nprofile', pubkey: pk })
		])
			.then(([n, np]) => {
				if (pk !== pubkey) return; // stale — pubkey swapped mid-flight
				npub = n;
				nprofile = np;
			})
			.catch(() => {});
	});

	function copyText(s: string, label: string) {
		if (!s) return;
		navigator.clipboard?.writeText(s);
		app.pushToast(`${label} copied`, 'success');
	}
</script>

<svelte:window
	onclick={() => { if (barMenuOpen) barMenuOpen = false; }}
	onkeydown={(e) => { if (barMenuOpen && e.key === 'Escape') barMenuOpen = false; }}
/>

<div class="profile-view">
	<div class="profile-bar">
		<button class="back-btn" onclick={onback}>&larr;</button>
		{#if profile?.picture}
			<img class="avatar" src={profile.picture} alt="" />
		{:else}
			<div class="avatar placeholder">?</div>
		{/if}
		<div class="identity">
			<span class="name">{profile?.display_name || profile?.name || pubkey.slice(0, 12) + '...'}</span>
			{#if profile?.about}
				<span class="about">{profile.about}</span>
			{/if}
		</div>
		<span class="bar-spacer"></span>
		<button
			class="fetch-btn"
			onclick={() => app.handleAddProfileToContext(pubkey, profile)}
			title="Add this profile to the chat context"
		>+ Context</button>
		<button class="fetch-btn" onclick={handleFetch} disabled={fetching} title="Fetch this author's events from relays">
			{fetching ? 'Fetching...' : '↻ Fetch'}
		</button>
		<div class="bar-menu">
			<button
				class="fetch-btn"
				onclick={(e) => { e.stopPropagation(); barMenuOpen = !barMenuOpen; }}
				aria-haspopup="menu"
				aria-expanded={barMenuOpen}
				title="Profile actions"
			>☰</button>
			{#if barMenuOpen}
				<div class="bar-menu__list" role="menu">
					<button
						class="bar-menu__item"
						role="menuitem"
						disabled={!npub}
						onclick={() => { barMenuOpen = false; copyText(npub, 'npub'); }}
					>Copy npub</button>
					<button
						class="bar-menu__item"
						role="menuitem"
						disabled={!nprofile}
						onclick={() => { barMenuOpen = false; copyText(nprofile, 'nprofile'); }}
					>Copy nprofile</button>
					<button
						class="bar-menu__item bar-menu__item--danger"
						role="menuitem"
						onclick={() => {
							barMenuOpen = false;
							app.ignoreAuthor(pubkey, profile?.display_name || profile?.name || undefined);
						}}
						title="Hide this author — ignore every event from this pubkey (undo in the ignored buffer)"
					>Ignore author</button>
				</div>
			{/if}
		</div>
	</div>

	{#snippet tabCell(t: Tab, label: string, count: number)}
		<div class="tab" class:active={activeTab === t}>
			<button class="tab-label" onclick={() => (activeTab = t)}>
				{label} ({count})
			</button>
			<button
				class="tab-refresh"
				onclick={() => handleTabFetch(t)}
				disabled={tabFetchingKinds === TAB_KINDS[t][0]}
				title={isOnline
					? `Fetch ${label.toLowerCase()} from configured relays`
					: `Choose relays and fetch ${label.toLowerCase()}`}
			>
				{tabFetchingKinds === TAB_KINDS[t][0] ? '…' : '↻'}
			</button>
		</div>
	{/snippet}

	{#snippet menuBtn(open: () => void)}
		<button
			class="item-menu"
			onclick={(e) => { e.stopPropagation(); open(); }}
			onkeydown={(e) => e.stopPropagation()}
			title="Open menu (m)"
			aria-label="Open event menu"
		>menu</button>
	{/snippet}

	<div
		class="tabs"
		bind:this={tabsEl}
		onwheel={(e) => {
			// Vertical wheel drives the horizontal rail — the bar has no
			// vertical scroll of its own, so this steals nothing.
			if (!e.deltaX && tabsEl && tabsEl.scrollWidth > tabsEl.clientWidth) {
				tabsEl.scrollLeft += e.deltaY;
			}
		}}
	>
		{@render tabCell('publications', 'Publications', publications.length)}
		{@render tabCell('articles', 'Articles', articles.length)}
		{@render tabCell('wikis', 'Wikis', wikis.length)}
		{@render tabCell('specs', 'Specs', specs.length)}
		{@render tabCell('sections', 'Sections', sections.length)}
		{@render tabCell('highlights', 'Highlights', highlights.length)}
		{@render tabCell('comments', 'Comments', comments.length)}
		{@render tabCell('spells', 'Spells', spellRows.length)}
	</div>

	<div class="tab-content" bind:this={listEl}>
		{#if loading}
			<div class="empty">Loading...</div>
		{:else if activeTab === 'publications'}
			{#if publications.length === 0}
				<div class="empty">No publications</div>
			{:else}
				{#each publications as pub_item, i (`${pub_item.addr.pubkey}:${pub_item.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenpub?.(pub_item); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenpub?.(pub_item); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-title">{pub_item.title ?? '[Untitled]'}</span>
							{#if pub_item.summary}
								<p class="item-preview">{pub_item.summary}</p>
							{/if}
							<span class="item-time">{formatTime(pub_item.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByAddr(pub_item.addr)}
								onpillctx={() => app.pillActionByAddr(pub_item.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(pub_item.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(pub_item.addr, 'drop')}
								signed={pub_item.signed}
								relays={pub_item.relays}
								forked={pub_item.forked}
							/>
							<span class="item-meta">{pub_item.section_count} sections</span>
							{@render menuBtn(() => app.openAddressableInModal(pub_item.addr))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'articles'}
			{#if articles.length === 0}
				<div class="empty">No articles</div>
			{:else}
				{#each articles as art, i (`${art.addr.pubkey}:${art.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(art.addr, art.title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(art.addr, art.title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-title">{art.title ?? '[Untitled]'}</span>
							{#if art.summary}
								<p class="item-preview">{art.summary}</p>
							{/if}
							<span class="item-time">{formatTime(art.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByAddr(art.addr)}
								onpillctx={() => app.pillActionByAddr(art.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(art.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(art.addr, 'drop')}
								signed={art.signed}
								relays={art.relays}
							/>
							<span class="item-meta">long-form</span>
							{@render menuBtn(() => app.openAddressableInModal(art.addr))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'wikis'}
			{#if wikis.length === 0}
				<div class="empty">No wikis</div>
			{:else}
				{#each wikis as wiki, i (`${wiki.addr.pubkey}:${wiki.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(wiki.addr, wiki.title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(wiki.addr, wiki.title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-title">{wiki.title ?? wiki.addr.d_tag ?? '[Untitled]'}</span>
							{#if wiki.summary}
								<p class="item-preview">{wiki.summary}</p>
							{/if}
							<span class="item-time">{formatTime(wiki.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByAddr(wiki.addr)}
								onpillctx={() => app.pillActionByAddr(wiki.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(wiki.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(wiki.addr, 'drop')}
								signed={wiki.signed}
								relays={wiki.relays}
							/>
							<span class="item-meta">wiki</span>
							{@render menuBtn(() => app.openAddressableInModal(wiki.addr))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'specs'}
			{#if specs.length === 0}
				<div class="empty">No specs</div>
			{:else}
				{#each specs as spec, i (`${spec.addr.pubkey}:${spec.addr.d_tag}`)}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(spec.addr, spec.title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(spec.addr, spec.title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-title">{spec.title ?? spec.addr.d_tag ?? '[Untitled]'}</span>
							{#if spec.summary}
								<p class="item-preview">{spec.summary}</p>
							{/if}
							<span class="item-time">{formatTime(spec.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByAddr(spec.addr)}
								onpillctx={() => app.pillActionByAddr(spec.addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(spec.addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(spec.addr, 'drop')}
								signed={spec.signed}
								relays={spec.relays}
							/>
							<span class="item-meta">spec</span>
							{@render menuBtn(() => app.openAddressableInModal(spec.addr))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'sections'}
			{#if sections.length === 0}
				<div class="empty">No sections</div>
			{:else}
				{#each sections as sec, i (sec.id)}
					{@const dTag = getTag(sec, 'd') || ''}
					{@const title = getTag(sec, 'title') || dTag || '[Untitled]'}
					{@const parentAddr = getTag(sec, 'a')}
					{@const addr = { kind: 30041, pubkey: sec.pubkey, d_tag: dTag }}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; onopenaddr?.(addr, title); }}
						onkeydown={(e) => { if (e.key === 'Enter') onopenaddr?.(addr, title); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-title">{title}</span>
							{#if sec.content}
								<p class="item-preview">{sec.content.slice(0, 200)}</p>
							{/if}
							<div class="item-footer">
								{#if parentAddr}
									<span class="item-ref">{parentAddr.split(':').pop()}</span>
								{/if}
								<span class="item-time">{formatTime(sec.created_at)}</span>
							</div>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByAddr(addr)}
								onpillctx={() => app.pillActionByAddr(addr, 'context')}
								onpillcmp={() => app.pillActionByAddr(addr, 'compose')}
								onpilldrop={() => app.pillActionByAddr(addr, 'drop')}
								signed={isEventSigned(sec.sig)}
								relays={sec.relays ?? []}
							/>
							{@render menuBtn(() => app.openAddressableInModal(addr))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'highlights'}
			{#if highlights.length === 0}
				<div class="empty">No highlights</div>
			{:else}
				{#each highlights as hl, i (hl.id)}
					{@const sourceAddr = getTag(hl, 'a') || getTag(hl, 'e')}
					{@const annotation = getTag(hl, 'comment')}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; oncomment?.(hl); }}
						onkeydown={(e) => { if (e.key === 'Enter') oncomment?.(hl); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							{#if sourceAddr}
								<span class="item-ref">on {sourceAddr.split(':').pop()}</span>
							{/if}
							<p class="item-content item-content--highlight">{hl.content}</p>
							{#if annotation}
								<p class="item-preview">{annotation}</p>
							{/if}
							<span class="item-time">{formatTime(hl.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByEventId(hl.id)}
								onpillctx={() => app.pillActionByEventId(hl.id, 'context')}
								onpillcmp={() => app.pillActionByEventId(hl.id, 'compose')}
								onpilldrop={() => app.pillActionByEventId(hl.id, 'drop')}
								signed={isEventSigned(hl.sig)}
								relays={hl.relays ?? []}
							/>
							{@render menuBtn(() => (app.eventModalData = hl))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'comments'}
			{#if comments.length === 0}
				<div class="empty">No comments</div>
			{:else}
				{#each comments as comment, i (comment.id)}
					{@const rootAddr = getTag(comment, 'A') || getTag(comment, 'E') || getTag(comment, 'I')}
					{@const rootKind = getTag(comment, 'K')}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; oncomment?.(comment); }}
						onkeydown={(e) => { if (e.key === 'Enter') oncomment?.(comment); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							{#if rootAddr}
								<span class="item-ref">on {rootKind ? `k:${rootKind}` : ''} {rootAddr.split(':').pop()}</span>
							{/if}
							<p class="item-content">{comment.content}</p>
							<span class="item-time">{formatTime(comment.created_at)}</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByEventId(comment.id)}
								onpillctx={() => app.pillActionByEventId(comment.id, 'context')}
								onpillcmp={() => app.pillActionByEventId(comment.id, 'compose')}
								onpilldrop={() => app.pillActionByEventId(comment.id, 'drop')}
								signed={isEventSigned(comment.sig)}
								relays={comment.relays ?? []}
							/>
							{@render menuBtn(() => (app.eventModalData = comment))}
						</div>
					</div>
				{/each}
			{/if}
		{:else if activeTab === 'spells'}
			{#if spellResults}
				<div class="spell-results-head">
					<button
						class="spell-back"
						onclick={() => {
							spellResults = null;
							spellRunContext = null;
							spellResultsExhausted = false;
							cursor = 0;
						}}
					>
						← spellbook
					</button>
					<span class="spell-results-label">
						{spellResults.name || 'spell'} — {spellResults.count}
						{spellResults.cmd === 'COUNT' ? 'matched' : `event${spellResults.count === 1 ? '' : 's'}`}
						{spellResults.auxiliary.length ? ` (+${spellResults.auxiliary.length} auxiliary)` : ''}
					</span>
				</div>
				{#if spellResults.events.length === 0}
					<div class="empty">
						{spellResults.cmd === 'COUNT' ? `Count: ${spellResults.count}` : 'No events matched'}
					</div>
				{:else}
					{#each spellResults.events as ev, i (ev.id)}
						{@const filedBy = spellResults.provenance[ev.id]?.length ?? 0}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="item pub-item"
							class:item--cursor={i === cursor}
							data-cursor={i}
							onclick={() => { cursor = i; app.eventModalData = ev; }}
							onkeydown={(e) => { if (e.key === 'Enter') (app.eventModalData = ev); }}
							onfocus={() => (cursor = i)}
							role="button"
							tabindex="0"
						>
							<div class="item-main">
								<span class="item-ref">
									k:{ev.kind}{filedBy ? ` · filed by ${filedBy} label${filedBy === 1 ? '' : 's'}` : ''}
								</span>
								<p class="item-content">{ev.content.slice(0, 240)}</p>
								<span class="item-time">{formatTime(ev.created_at)}</span>
							</div>
							<div class="item-rail">
								<PoolStateBadges
									item={app.findPoolItemByEventId(ev.id)}
									onpillctx={() => app.pillActionByEventId(ev.id, 'context')}
									onpillcmp={() => app.pillActionByEventId(ev.id, 'compose')}
									onpilldrop={() => app.pillActionByEventId(ev.id, 'drop')}
									signed={isEventSigned(ev.sig)}
									relays={ev.relays ?? []}
								/>
								<button
									class="spell-bookmark"
									onclick={(e) => { e.stopPropagation(); ignoreResultEvent(ev); }}
									onkeydown={(e) => e.stopPropagation()}
									title="Ignore this event — hides it from feeds, search, and future runs"
								>ignore</button>
								{@render menuBtn(() => (app.eventModalData = ev))}
							</div>
						</div>
					{/each}
				{/if}
			{:else if spellRows.length === 0}
				<div class="empty">No spells</div>
			{:else}
				{#each spellBooks.filter((v) => v.local) as view (view.book.d)}
					<div class="spell-results-head">
						<span class="spell-results-label">
							book “{view.book.title || view.book.d}” is local-only
						</span>
						<button class="spell-back" onclick={() => broadcastBook(view)}>
							Broadcast book
						</button>
					</div>
				{/each}
				{#each spellRows as row, i (row.entry?.event.id ?? row.ref?.event_id ?? i)}
					{#if !row.entry}
						<div class="item pub-item" data-cursor={i} class:item--cursor={i === cursor}>
							<div class="item-main">
								<span class="item-ref">in “{row.fromBook}” · not fetched</span>
								<span class="item-title">{(row.ref?.event_id ?? '').slice(0, 16)}…</span>
								{#if row.ref?.author_hint}
									<p class="item-preview">by <ProfileName pubkey={row.ref.author_hint} /></p>
								{/if}
							</div>
						</div>
					{:else}
					{@const entry = row.entry}
					{@const s = entry.spell}
					<!-- svelte-ignore a11y_no_static_element_interactions -->
					<div
						class="item pub-item"
						class:item--cursor={i === cursor}
						data-cursor={i}
						onclick={() => { cursor = i; runSpell(entry); }}
						onkeydown={(e) => { if (e.key === 'Enter') runSpell(entry); }}
						onfocus={() => (cursor = i)}
						role="button"
						tabindex="0"
					>
						<div class="item-main">
							<span class="item-ref">
								{s ? s.cmd : 'invalid'}{s && s.stages.length ? ` · ${s.stages.length}-stage pipeline` : ''}{s?.input ? ' · chained' : ''}{entry.partial ? ' · partial' : ''}{entry.needs_identity ? ' · uses your identity' : ''}{row.fromBook ? ` · in “${row.fromBook}”` : ''}
							</span>
							{#if entry.event.pubkey !== pubkey}
								<span class="spell-byline">by <ProfileName pubkey={entry.event.pubkey} /></span>
							{/if}
							<span class="item-title">{s?.name ?? '[Unnamed spell]'}</span>
							{#if s?.description}
								<p class="item-preview">{s.description}</p>
							{/if}
							{#if entry.error}
								<p class="item-preview">{entry.error}</p>
							{/if}
							<SpellClauseBlock clauses={entry.clauses} />
							{#if s?.topics.length}
								<p
									class="spell-topics"
									title="Topics label the spell itself (NIP-A7 top-level t) — they are not filter conditions"
								>
									topics: {s.topics.map((t) => `#${t}`).join(' ')}
								</p>
							{/if}
							{#if s && ((s.cmd === 'PIPE' && s.stages.length) || s.input)}
								<button
									class="stage-toggle"
									onclick={(e) => { e.stopPropagation(); toggleSpellStages(entry.event.id); }}
									onkeydown={(e) => e.stopPropagation()}
									title={s.input ? 'Unpack the input chain' : 'Unpack pipeline stages'}
								>
									{spellStagesOpen[entry.event.id] ? '⌃' : '⌄'}
									{#if s.input}
										input chain
									{:else}
										{s.stages.length} stage{s.stages.length === 1 ? '' : 's'}
										({s.stages.map((st) => st.combinator ?? 'source').join(' → ')})
									{/if}
								</button>
								{#if spellStagesOpen[entry.event.id]}
									{#each spellStageCache[entry.event.id] ?? [] as st, si (st.spell_id + si)}
										<div class="stage-block">
											<span class="stage-head">
												stage {si + 1}{st.combinator ? ` · ${st.combinator}` : ''}:
												{st.name ?? st.spell_id.slice(0, 12) + '…'}
											</span>
											{#if st.error}
												<span class="stage-error">{st.error}</span>
											{:else}
												<SpellClauseBlock clauses={st.clauses} />
											{/if}
										</div>
									{/each}
								{/if}
							{/if}
							{#if entry.required_args.length}
								<p class="item-preview">args: {entry.required_args.join(', ')}</p>
							{/if}
							<span class="item-time">
								{spellRunning === entry.event.id ? 'running…' : formatTime(entry.event.created_at)}
							</span>
						</div>
						<div class="item-rail">
							<PoolStateBadges
								item={app.findPoolItemByEventId(entry.event.id)}
								onpillctx={() => app.pillActionByEventId(entry.event.id, 'context')}
								onpillcmp={() => app.pillActionByEventId(entry.event.id, 'compose')}
								onpilldrop={() => app.pillActionByEventId(entry.event.id, 'drop')}
								signed={isEventSigned(entry.event.sig)}
								relays={entry.event.relays ?? []}
							/>
							<button
								class="spell-bookmark"
								onclick={(e) => { e.stopPropagation(); addToMyBook(entry); }}
								onkeydown={(e) => e.stopPropagation()}
								title="Add this spell to your spellbook (kind 30777)"
							>+book</button>
							<button
								class="spell-bookmark"
								onclick={(e) => { e.stopPropagation(); ignoreSpell(entry); }}
								onkeydown={(e) => e.stopPropagation()}
								title="Ignore this spell event — drops it from lists, books, and results"
							>ignore</button>
							{@render menuBtn(() => (app.eventModalData = entry.event))}
						</div>
					</div>
					{/if}
				{/each}
			{/if}
		{/if}
		{#if !loading && activeList().length > 0 && !(activeTab === 'spells' && spellResults?.cmd === 'COUNT')}
			{@const olderExhausted =
				activeTab === 'spells' && spellResults ? spellResultsExhausted : exhausted[activeTab]}
			<div class="load-older">
				<button onclick={handleLoadOlder} disabled={loadingOlder || olderExhausted}>
					{loadingOlder
						? 'Loading…'
						: olderExhausted
							? 'No older events found'
							: 'Load older'}
				</button>
			</div>
		{/if}
	</div>
</div>

<style>
	.profile-view {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.profile-bar {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
	}

	.back-btn {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
	}

	.back-btn:hover {
		color: var(--fg);
	}

	.avatar {
		width: 36px;
		height: 36px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.avatar.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: var(--t-base);
	}

	.identity {
		min-width: 0;
		display: flex;
		flex-direction: column;
	}

	.name {
		font-weight: 600;
		font-size: var(--t-sm);
	}

	.about {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.bar-spacer {
		flex: 1;
	}

	.fetch-btn {
		font-size: var(--t-3xs);
		padding: 4px 10px;
		background: none;
		border: 1px solid var(--accent);
		color: var(--accent);
		border-radius: var(--radius);
		cursor: pointer;
		white-space: nowrap;
	}

	.fetch-btn:hover:not(:disabled) {
		background: var(--accent);
		color: white;
	}

	.fetch-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}

	/* Profile-bar hamburger menu — copy npub / copy nprofile / ignore
	   author. Anchored dropdown (position: relative wrapper), closed by
	   outside click or Escape via svelte:window. */
	.bar-menu {
		position: relative;
	}
	.bar-menu__list {
		position: absolute;
		top: calc(100% + 4px);
		right: 0;
		z-index: 20;
		display: flex;
		flex-direction: column;
		min-width: 140px;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		box-shadow: 0 4px 12px rgba(0, 0, 0, 0.25);
		overflow: hidden;
	}
	.bar-menu__item {
		background: none;
		border: none;
		text-align: left;
		font-size: 0.75rem;
		padding: 7px 12px;
		color: var(--fg);
		cursor: pointer;
		white-space: nowrap;
	}
	.bar-menu__item:hover:not(:disabled) {
		background: color-mix(in srgb, var(--accent) 15%, transparent);
	}
	.bar-menu__item:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.bar-menu__item--danger {
		color: var(--state-error);
		border-top: 1px solid var(--border);
	}
	.bar-menu__item--danger:hover:not(:disabled) {
		background: color-mix(in srgb, var(--state-error) 15%, transparent);
	}

	.tabs {
		display: flex;
		border-bottom: 1px solid var(--border);
		/* Horizontal rail: tabs fill the width while they fit; past that
		   the bar scrolls instead of crushing labels, so new kind-feeds
		   can keep being added. */
		overflow-x: auto;
		scrollbar-width: thin;
		scrollbar-color: var(--base3) transparent;
	}

	.tab {
		/* Grow to share spare width, but never shrink below the label —
		   overflow goes to the rail's scroll instead. */
		flex: 1 0 auto;
		display: flex;
		align-items: stretch;
		justify-content: center;
		border-bottom: 2px solid transparent;
	}
	.tab.active {
		border-bottom-color: var(--accent);
	}
	.tab-label {
		flex: 1;
		padding: 8px 4px 8px 12px;
		font-size: var(--t-2xs);
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		text-align: center;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tab-label:hover {
		color: var(--fg);
	}
	.tab.active .tab-label {
		color: var(--fg);
	}

	.tab-refresh {
		padding: 0 6px;
		background: none;
		border: none;
		color: var(--base5);
		cursor: pointer;
		font-size: var(--t-xs);
		line-height: 1;
		opacity: 0.6;
		transition: opacity 100ms;
	}
	.tab-refresh:hover:not(:disabled) {
		color: var(--state-online);
		opacity: 1;
	}
	.tab-refresh:disabled {
		opacity: 0.3;
		cursor: not-allowed;
	}

	.tab-content {
		flex: 1;
		overflow-y: auto;
	}

	.empty {
		padding: 24px;
		text-align: center;
		color: var(--fg-muted);
		font-size: var(--t-xs);
	}

	.item {
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
		/* Two columns: text content (truncating) | controls rail. The rail
		   is fixed-width so previews/content can never run under the
		   pills/menu, whatever the pane width. */
		display: flex;
		align-items: flex-start;
		gap: 8px;
	}

	.item-main {
		flex: 1;
		min-width: 0;
	}

	.item-rail {
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 3px;
		max-width: 22ch;
	}

	.pub-item {
		cursor: pointer;
		border-left: 3px solid var(--selection);
	}

	.pub-item:hover {
		background: var(--bg-surface);
	}

	/* Cursor highlight: same ranger-style bar as FeedBuffer rows so the
	   j/k cursor is unmistakable. The accent comes from --id-yours;
	   click and tab-focus both snap the cursor onto the row. */
	.item--cursor {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		border-left-color: var(--id-yours);
		border-left-width: 5px;
		padding-left: 14px;
	}
	.item--cursor .item-title { color: var(--fg); font-weight: 700; }

	.item-title {
		display: block;
		font-size: var(--t-sm);
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		margin-bottom: 2px;
	}

	.item-meta {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.item-preview {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		/* Unbroken runs (URLs, naddrs) wrap instead of clipping wide. */
		overflow-wrap: anywhere;
	}

	.item-content {
		font-size: var(--t-xs);
		line-height: 1.5;
		margin: 4px 0;
		white-space: pre-wrap;
		word-break: break-word;
	}

	/* The highlighted excerpt itself — quoted, with the same tint the
	   discussion-count "hl" pills use so highlights read consistently. */
	.item-content--highlight {
		border-left: 3px solid var(--state-online);
		background: color-mix(in srgb, var(--state-online) 10%, transparent);
		padding: 4px 8px;
		font-style: italic;
	}

	.item-footer {
		display: flex;
		gap: 8px;
		align-items: center;
	}

	.item-ref {
		font-size: var(--t-3xs);
		color: var(--accent);
		font-family: var(--font-mono);
	}

	.item-time {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
	}

	/* Per-item "menu" affordance — opens the unified event menu modal on
	   the raw event. Also reachable via `m` on the focused card.
	   stopPropagation keeps clicks off the card, which otherwise routes
	   to the reader / discussion view. */
	.item-menu {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		padding: 1px 6px;
		cursor: pointer;
		line-height: 1.5;
	}
	.item-menu:hover {
		color: var(--accent);
		border-color: var(--accent);
	}

	/* Spell result feed header — back affordance + run summary. */
	.spell-results-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--border);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.spell-back {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--accent);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		cursor: pointer;
	}
	.spell-back:hover {
		border-color: var(--accent);
	}
	.spell-results-label {
		color: var(--fg-muted);
	}

	/* Pipeline unpack: the `v` chevron + per-stage clause blocks. */
	.stage-toggle {
		align-self: flex-start;
		background: none;
		border: none;
		color: var(--accent);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		padding: 0;
		cursor: pointer;
	}
	.stage-toggle:hover {
		text-decoration: underline;
	}
	.stage-block {
		display: flex;
		flex-direction: column;
		border-left: 2px solid var(--border);
		padding-left: 8px;
		margin: 2px 0;
	}
	.stage-head {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
	}
	.stage-error {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		font-style: italic;
	}
	.spell-byline {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
	}
	.spell-topics {
		margin: 2px 0 0;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		opacity: 0.8;
	}
	.spell-bookmark {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		padding: 1px 6px;
		cursor: pointer;
		line-height: 1.5;
	}
	.spell-bookmark:hover {
		color: var(--accent);
		border-color: var(--accent);
	}

	/* Backfill footer — same affordance as the feed's "Load more". */
	.load-older {
		padding: 12px;
		text-align: center;
	}
	.load-older button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 16px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--fg);
		cursor: pointer;
	}
	.load-older button:disabled {
		color: var(--fg-muted);
		cursor: default;
	}
</style>
