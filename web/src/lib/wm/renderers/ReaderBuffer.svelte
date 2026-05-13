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

	let { buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
	const store = getActiveStore();

	let publication = $state<PublicationDetail | null>(null);
	let pristineSections = $state<LazySection[]>([]);
	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const loadingPromises = new Map<number, Promise<void>>();

	function parseBufferId(id: string): { pubkey: string; dTag: string } | null {
		const match = id.match(/^reader:\d+:([0-9a-fA-F]{64}):(.+)$/);
		if (!match) return null;
		return { pubkey: match[1].toLowerCase(), dTag: match[2] };
	}

	function parseEventId(id: string): string | null {
		const match = id.match(/^reader:event:([0-9a-fA-F]{64})$/);
		return match ? match[1].toLowerCase() : null;
	}

	const parsedAddr = $derived(parseBufferId(buffer.id));
	const parsedEventId = $derived(parseEventId(buffer.id));

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
							</div>
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
				/>
			{:else}
				<PaginatedView
					{sections}
					{currentSection}
					onnavigate={handleNavigate}
					onload={isDraftMode ? undefined : handleLoadSection}
					onsectionjson={openSectionJsonByIndex}
				/>
			{/if}
		</div>
	{/if}
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
		grid-template-columns: auto 1fr;
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
