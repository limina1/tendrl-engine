<script lang="ts">
	import {
		pubkeyToHue,
		pubkeyToSwatch,
		pubkeyToHighlightStroke
	} from '$lib/discussions/colors';
	import { getAuthorDisplayName, hasAuthorName } from '$lib/discussions/authors.svelte';
	import { marksOf, type Highlighter, type HighlightMark } from '$lib/discussions/highlights';
	import { markSampleStyle } from '$lib/discussions/colors';
	import MarkToggles from './MarkToggles.svelte';
	import type { HighlightAttached } from '$lib/api';
	import { flattenThread } from '$lib/discussions/thread';
	import {
		highlightDisplay,
		ALPHA_MIN,
		ALPHA_MAX
	} from '$lib/discussions/highlight-display.svelte';
	import ColorPicker from './ColorPicker.svelte';

	export interface DrawerHighlight {
		/** kind-9802 event id */
		id: string;
		/** Author pubkey */
		pubkey: string;
		/** Highlighted text (event content) */
		content: string;
		created_at: number;
		/** Section addr (kind:pk:dtag) the highlight actually renders in,
		 *  resolved by the reader's content matcher. May be null when no
		 *  section content contains the text — that highlight won't be
		 *  scrollable, only listed. */
		section_addr: string | null;
		/** The highlight's style (`["highlighter", label, color]`), if any. */
		highlighter?: Highlighter;
	}

	let {
		highlights,
		scope = null,
		open,
		onclose,
		onnavigate,
		onrefresh = null,
		mypubkey = null,
		labelFilter = $bindable(null),
		onrestyle = null,
		onadopt = null,
		ondelete = null,
		attached = {},
		canSign = false,
		oncomment = null,
		onannotate = null
	}: {
		highlights: DrawerHighlight[];
		/** What the list is narrowed to (e.g. "this page"); `null` = everything. */
		scope?: string | null;
		open: boolean;
		onclose: () => void;
		/** Called when the user clicks a row. The drawer doesn't perform
		 *  the scroll itself — the reader knows which section index
		 *  corresponds to an addr and can flip the pager before
		 *  scrolling into view. */
		onnavigate: (highlightId: string, sectionAddr: string | null) => void;
		/** Optional: refetch kind 0 from relays for every author in the
		 *  drawer. Used to pick up renames without leaving the panel. */
		onrefresh?: (() => Promise<void>) | null;
		/** The signer's pubkey — own highlights can be recoloured. */
		mypubkey?: string | null;
		/** Label filter (`null` = all, `''` = plain only) — the reader paints
		 *  only the matching highlights too. */
		labelFilter?: string | null;
		/** Republish one of yours with a new style (`null` = plain). */
		onrestyle?: ((id: string, highlighter: Highlighter | null) => Promise<void>) | null;
		/** Someone else's style → the next capture starts with it. */
		onadopt?: ((h: Highlighter) => void) | null;
		/** Publish a NIP-09 deletion for one of yours (and the versions it
		 *  superseded); the reader drops it from the list on success. */
		ondelete?: ((id: string) => Promise<void>) | null;
		/** Per highlight id: its author's annotation + the comments on it. */
		attached?: Record<string, HighlightAttached>;
		canSign?: boolean;
		/** Comment on a highlight — social (kind 1111), anyone. */
		oncomment?: ((id: string, text: string) => Promise<void>) | null;
		/** Annotate your own highlight — why it was made (kind 30041). */
		onannotate?: ((id: string, text: string) => Promise<void>) | null;
	} = $props();

	// One panel open at a time, under its entry: the comment thread + reply
	// box, or the annotation editor. Text is view state; cleared on send.
	let panel = $state<{ id: string; kind: 'comment' | 'annotate' } | null>(null);
	let draft = $state('');
	let sending = $state(false);
	function openPanel(id: string, kind: 'comment' | 'annotate') {
		if (panel?.id === id && panel.kind === kind) {
			panel = null;
			return;
		}
		panel = { id, kind };
		draft = kind === 'annotate' ? (attached[id]?.annotation?.content ?? '') : '';
	}
	async function send() {
		if (!panel || sending || !draft.trim()) return;
		const fn = panel.kind === 'comment' ? oncomment : onannotate;
		if (!fn) return;
		sending = true;
		try {
			await fn(panel.id, draft.trim());
			draft = '';
			if (panel.kind === 'annotate') panel = null;
		} finally {
			sending = false;
		}
	}

	// Two-step armed delete (feat-ui-patterns): first × arms for 4 s, the
	// second publishes the deletion.
	let armed = $state<string | null>(null);
	let deleting = $state<string | null>(null);
	let armTimer: ReturnType<typeof setTimeout> | undefined;
	async function onDeleteClick(id: string) {
		if (!ondelete || deleting) return;
		if (armed !== id) {
			armed = id;
			clearTimeout(armTimer);
			armTimer = setTimeout(() => (armed = null), 4000);
			return;
		}
		clearTimeout(armTimer);
		armed = null;
		deleting = id;
		try {
			await ondelete(id);
			if (selected === id) selected = null;
		} finally {
			deleting = null;
		}
	}

	// The selected entry (actions open under it) + the one flashing.
	let selected = $state<string | null>(null);
	let listEl = $state<HTMLElement | undefined>();
	let flashing = $state<string | null>(null);
	// Recolour form for the selected entry.
	let editLabel = $state('');
	let editColor = $state('#f7d94c');
	let editMarks = $state<HighlightMark[]>(['fill']);
	// A colour picked by hand (or the entry's own) — part of what makes a
	// style; the label is an optional class on top.
	let editColorTouched = $state(false);
	const editStyle = $derived(
		editLabel.trim() || editColorTouched || editMarks.join() !== 'fill'
			? { label: editLabel.trim(), color: editColor, marks: editMarks }
			: null
	);
	let restyling = $state(false);

	function select(h: DrawerHighlight) {
		selected = h.id;
		editLabel = h.highlighter?.label ?? '';
		editColor = h.highlighter?.color ?? '#f7d94c';
		editMarks = marksOf(h.highlighter);
		editColorTouched = !!h.highlighter;
	}
	function editPlain() {
		editLabel = '';
		editMarks = ['fill'];
		editColorTouched = false;
	}

	let flashTimer: ReturnType<typeof setTimeout> | undefined;
	/** A highlight clicked in the text: expand its author, select it, flash
	 *  it, scroll the list to it. Called imperatively by the reader (from its
	 *  click handler) — as an effect on a prop, the writes looped the
	 *  reader's flush (effect_update_depth_exceeded). */
	export function focusHighlight(id: string) {
		const h = highlights.find((x) => x.id === id);
		if (!h) return;
		if (labelFilter !== null && (h.highlighter?.label ?? '') !== labelFilter) labelFilter = null;
		if (!expanded.has(h.pubkey)) expanded = new Set([...expanded, h.pubkey]);
		select(h);
		flashing = h.id;
		clearTimeout(flashTimer);
		flashTimer = setTimeout(() => (flashing = null), 1300);
		// Scroll the drawer's own list only — `scrollIntoView` would also
		// scroll the reader behind the drawer.
		requestAnimationFrame(() => {
			const list = listEl;
			const row = list?.querySelector<HTMLElement>(`[data-hl-id="${h.id}"]`);
			if (!list || !row) return;
			const top = row.offsetTop - list.offsetTop;
			if (top < list.scrollTop || top + row.offsetHeight > list.scrollTop + list.clientHeight)
				list.scrollTop = top - 8;
		});
	}

	// Label filter chips: every label present, plus `plain` when some
	// highlights carry none.
	const labels = $derived.by(() => {
		const m = new Map<string, string>();
		let plain = false;
		for (const h of highlights) {
			// Filter chips are classes; unlabelled styles sit with `no label`.
			if (h.highlighter?.label) {
				if (!m.has(h.highlighter.label)) m.set(h.highlighter.label, h.highlighter.color);
			} else plain = true;
		}
		const out = [...m.entries()].map(([label, color]) => ({ label, color }));
		out.sort((a, b) => a.label.localeCompare(b.label));
		return { list: out, plain };
	});
	const visible = $derived(
		labelFilter === null
			? highlights
			: highlights.filter((h) => (h.highlighter?.label ?? '') === labelFilter)
	);

	async function republish(h: DrawerHighlight) {
		if (!onrestyle || restyling) return;
		restyling = true;
		try {
			await onrestyle(h.id, editStyle);
			selected = null;
		} finally {
			restyling = false;
		}
	}
	const unchanged = (h: DrawerHighlight) => {
		const a = editStyle;
		const b = h.highlighter;
		if (!a || !b) return !a && !b;
		return a.label === b.label && a.color === b.color && a.marks.join() === marksOf(b).join();
	};

	let refreshing = $state(false);
	async function handleRefresh() {
		if (!onrefresh || refreshing) return;
		refreshing = true;
		try {
			await onrefresh();
		} finally {
			refreshing = false;
		}
	}

	type AuthorGroup = {
		pubkey: string;
		hue: number;
		swatch: string;
		stripe: string;
		highlights: DrawerHighlight[];
	};

	const groups = $derived.by<AuthorGroup[]>(() => {
		const byPubkey = new Map<string, DrawerHighlight[]>();
		for (const h of visible) {
			const bucket = byPubkey.get(h.pubkey) ?? [];
			bucket.push(h);
			byPubkey.set(h.pubkey, bucket);
		}
		return Array.from(byPubkey.entries())
			.map(([pubkey, hs]) => ({
				pubkey,
				hue: pubkeyToHue(pubkey),
				swatch: pubkeyToSwatch(pubkey),
				stripe: pubkeyToHighlightStroke(pubkey),
				highlights: hs.sort((a, b) => b.created_at - a.created_at)
			}))
			.sort((a, b) => b.highlights.length - a.highlights.length);
	});

	// Multi-author expansion. Keeping it a Set lets the user keep two
	// authors open at once to compare highlights side by side.
	let expanded = $state<Set<string>>(new Set());
	function toggle(pubkey: string) {
		if (expanded.has(pubkey)) expanded.delete(pubkey);
		else expanded.add(pubkey);
		expanded = new Set(expanded);
	}

	function short(s: string, n: number): string {
		return s.length > n ? `${s.slice(0, n)}…` : s;
	}
	function preview(content: string): string {
		const oneLine = content.replace(/\s+/g, ' ').trim();
		return short(oneLine, 80);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key !== 'Escape' || !open) return;
		if (selected) selected = null;
		else onclose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<aside class="hl-drawer" class:open aria-hidden={!open} aria-label="Highlights drawer">
	<header class="hl-drawer__head">
		<h3 class="hl-drawer__title">
			Highlights{#if scope}<span class="hl-drawer__scope"> · {scope}</span>{/if} ({visible.length})
		</h3>
		<div class="hl-drawer__head-actions">
			{#if onrefresh}
				<button
					class="hl-drawer__refresh"
					onclick={handleRefresh}
					disabled={refreshing}
					title="Re-fetch kind 0 profiles from relays so renamed authors show up"
				>
					{refreshing ? '…' : '↻ names'}
				</button>
			{/if}
			<button class="hl-drawer__close" onclick={onclose} aria-label="Close drawer">×</button>
		</div>
	</header>
	<div class="hl-tools">
		<label class="hl-alpha">
			<span>alpha</span>
			<input
				type="range"
				min={ALPHA_MIN}
				max={ALPHA_MAX}
				value={highlightDisplay.alpha}
				oninput={(e) => highlightDisplay.setAlpha(Number((e.currentTarget as HTMLInputElement).value))}
				aria-label="Highlight alpha"
			/>
			<span class="hl-alpha__v">{highlightDisplay.alpha}%</span>
		</label>
		{#if labels.list.length}
			<div class="hl-filters" role="group" aria-label="Filter by highlighter">
				<button class="hl-filter" class:on={labelFilter === null} onclick={() => (labelFilter = null)}
					>all</button
				>
				{#each labels.list as l (l.label)}
					<button
						class="hl-filter"
						class:on={labelFilter === l.label}
						aria-pressed={labelFilter === l.label}
						onclick={() => (labelFilter = labelFilter === l.label ? null : l.label)}
					>
						<span class="hl-filter__dot" style="background: {l.color}"></span>{l.label}
					</button>
				{/each}
				{#if labels.plain}
					<button
						class="hl-filter"
						class:on={labelFilter === ''}
						aria-pressed={labelFilter === ''}
						onclick={() => (labelFilter = labelFilter === '' ? null : '')}>no label</button
					>
				{/if}
			</div>
		{/if}
	</div>
	<ul class="hl-authors" bind:this={listEl}>
		{#each groups as group (group.pubkey)}
			{@const isOpen = expanded.has(group.pubkey)}
			<li class="hl-author">
				<button
					class="hl-author-row"
					onclick={() => toggle(group.pubkey)}
					aria-expanded={isOpen}
				>
					<span class="hl-swatch" style="background: {group.swatch};"></span>
					{#if hasAuthorName(group.pubkey)}
						<span class="hl-author-name" title={group.pubkey}>
							{getAuthorDisplayName(group.pubkey)}
						</span>
					{:else}
						<code class="hl-author-pk">{short(group.pubkey, 16)}</code>
					{/if}
					<span class="hl-count">{group.highlights.length}</span>
					<span class="hl-chevron" class:open={isOpen}>▸</span>
				</button>
				{#if isOpen}
					<ul class="hl-entries">
						{#each group.highlights as entry (entry.id)}
							{@const mine = !!mypubkey && entry.pubkey === mypubkey}
							{@const att = attached[entry.id]}
							<li
								class="hl-entry-wrap"
								class:selected={selected === entry.id}
								class:flash={flashing === entry.id}
								data-hl-id={entry.id}
							>
								<div class="hl-entry-row">
								<button
									class="hl-entry"
									onclick={() => {
										if (selected === entry.id) selected = null;
										else select(entry);
										if (entry.section_addr !== null) onnavigate(entry.id, entry.section_addr);
									}}
									title={entry.content}
									aria-expanded={selected === entry.id}
								>
									<span
										class="hl-stripe"
										style="background: {entry.highlighter?.color ?? group.stripe};"
									></span>
									<span class="hl-body">
										{#if entry.highlighter?.label}
											<span class="hl-label" style={markSampleStyle(entry.highlighter)}
												>{entry.highlighter.label}</span
											>
										{/if}
										<span class="hl-preview">{preview(entry.content)}</span>
									</span>
								</button>
								<button
									class="hl-icon"
									class:on={panel?.id === entry.id && panel.kind === 'comment'}
									onclick={() => openPanel(entry.id, 'comment')}
									aria-label="Comments ({att?.comment_count ?? 0})"
									title="Comment — a social reply anyone can make"
								>
									<svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
										><path d="M2.5 3.5h11v7h-6l-3 2.5v-2.5h-2z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" /></svg
									>{#if att?.comment_count}<span class="hl-icon__n">{att.comment_count}</span>{/if}
								</button>
								{#if mine && onannotate}
									<button
										class="hl-icon"
										class:on={panel?.id === entry.id && panel.kind === 'annotate'}
										class:has={!!att?.annotation}
										onclick={() => openPanel(entry.id, 'annotate')}
										aria-label={att?.annotation ? 'Edit your annotation' : 'Annotate — why you highlighted this'}
										title="Annotate — why you highlighted this (kept with the highlight for knowledge-base readers)"
									>
										<svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
											><path d="M3 13l1-3.5 6.5-6.5 2.5 2.5-6.5 6.5zM9.5 4l2.5 2.5" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" /></svg
										>
									</button>
								{/if}
								{#if mine && ondelete}
									<button
										class="hl-del"
										class:armed={armed === entry.id}
										disabled={deleting === entry.id}
										onclick={() => onDeleteClick(entry.id)}
										aria-label={armed === entry.id ? 'Confirm: publish a deletion request' : 'Delete this highlight'}
										title={armed === entry.id
											? 'Click again to publish a deletion request (NIP-09)'
											: 'Delete — publishes a deletion request for this highlight'}
										>{deleting === entry.id ? '…' : armed === entry.id ? 'delete?' : '×'}</button
									>
								{/if}
								</div>
								{#if att?.annotation && !(panel?.id === entry.id && panel.kind === 'annotate')}
									<div class="hl-why" title="The highlighter's annotation">{att.annotation.content}</div>
								{/if}
								{#if panel?.id === entry.id}
									<div class="hl-panel">
										{#if panel.kind === 'comment'}
											{#each flattenThread(att?.comments ?? []) as node (node.event.id)}
												<div class="hl-cmt" style="margin-left: {Math.min(node.depth, 4) * 10}px">
													<span class="hl-cmt__by">{getAuthorDisplayName(node.event.pubkey)}</span>
													<span class="hl-cmt__t">{node.event.content}</span>
												</div>
											{:else}
												<div class="hl-actions__hint">no comments yet</div>
											{/each}
										{/if}
										{#if canSign}
											<textarea
												class="hl-draft"
												data-entry
												rows="2"
												placeholder={panel.kind === 'comment'
													? 'Comment — goes to social clients'
													: 'Why did you highlight this?'}
												bind:value={draft}
												disabled={sending}
												onkeydown={(e) => {
													if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
														e.preventDefault();
														void send();
													}
												}}
											></textarea>
											<div class="hl-actions__row">
												<span class="hl-actions__hint"
													>{panel.kind === 'comment'
														? 'kind 1111 · broadcast'
														: 'kind 30041 annotation · kept with the highlight'}</span
												>
												<button class="hl-act hl-act--primary" disabled={sending || !draft.trim()} onclick={send}>
													{sending ? '…' : panel.kind === 'comment' ? 'Comment' : 'Save'}
												</button>
											</div>
										{:else}
											<div class="hl-actions__hint">sign in to {panel.kind === 'comment' ? 'comment' : 'annotate'}</div>
										{/if}
									</div>
								{/if}
								{#if selected === entry.id}
									<div class="hl-actions">
										{#if mine && onrestyle}
											<div class="hl-restyle">
												<input
													class="hl-restyle__label"
													data-entry
													maxlength="64"
													placeholder="label (optional)"
													aria-label="Highlighter label"
													bind:value={editLabel}
													disabled={restyling}
												/>
												<ColorPicker
													bind:value={editColor}
													onchange={() => (editColorTouched = true)}
													disabled={restyling}
													label="recolour"
												/>
												<button
													class="hl-act"
													class:on={!editStyle}
													disabled={restyling || !editStyle}
													onclick={editPlain}
													title="No style — the author's hue">plain</button
												>
											</div>
											<MarkToggles bind:value={editMarks} color={editColor} disabled={restyling} />
											<div class="hl-actions__row">
												<span class="hl-actions__hint">publishes a new highlight; this one is dropped</span>
												<button
													class="hl-act hl-act--primary"
													disabled={restyling || unchanged(entry)}
													onclick={() => republish(entry)}
												>
													{restyling ? 'Publishing…' : 'Republish'}
												</button>
											</div>
										{:else if !mine && entry.highlighter && onadopt}
											<div class="hl-actions__row">
												<span class="hl-actions__hint">not yours to recolour</span>
												<button class="hl-act" onclick={() => onadopt?.(entry.highlighter!)}
													>Highlight with this style</button
												>
											</div>
										{/if}
									</div>
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
			</li>
		{/each}
		{#if groups.length === 0}
			<li class="hl-empty">No highlights yet.</li>
		{/if}
	</ul>
</aside>

<style>
	.hl-tools {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 6px 10px;
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
	}
	.hl-alpha {
		display: flex;
		align-items: center;
		gap: 6px;
		color: var(--base5);
	}
	.hl-alpha input {
		flex: 1;
		accent-color: var(--id-yours);
	}
	.hl-alpha__v {
		width: 3ch;
		text-align: right;
		color: var(--fg-muted);
	}
	.hl-filters {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		/* Many highlighters would otherwise push the list out of the cap. */
		max-height: 4.5rem;
		overflow-y: auto;
	}
	.hl-filter {
		font: inherit;
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 1px 6px;
		color: var(--base5);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hl-filter.on {
		color: var(--fg);
		background: var(--base2);
		border-color: var(--base5);
	}
	.hl-filter__dot {
		width: 8px;
		height: 8px;
		border-radius: 2px;
	}
	.hl-entry-wrap {
		border-radius: var(--r-sm);
	}
	.hl-entry-row {
		display: flex;
		align-items: flex-start;
	}
	.hl-entry-row .hl-entry {
		flex: 1;
		min-width: 0;
	}
	.hl-icon {
		flex-shrink: 0;
		display: inline-flex;
		align-items: center;
		gap: 2px;
		margin: 4px 0 0;
		padding: 1px 4px;
		font: inherit;
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hl-icon:hover,
	.hl-icon.on {
		color: var(--fg);
		border-color: var(--panel-border);
	}
	.hl-icon.has {
		color: var(--id-yours);
	}
	.hl-why {
		margin: 0 8px 4px 22px;
		padding-left: 6px;
		border-left: 2px solid var(--id-yours);
		font-family: var(--font-serif);
		font-style: italic;
		font-size: var(--t-xs);
		color: var(--fg-muted);
		white-space: pre-wrap;
	}
	.hl-panel {
		display: flex;
		flex-direction: column;
		gap: 5px;
		padding: 2px 8px 8px 22px;
	}
	.hl-cmt {
		display: flex;
		flex-direction: column;
		gap: 1px;
		padding: 3px 6px;
		border-left: 2px solid var(--panel-border);
	}
	.hl-cmt__by {
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
	}
	.hl-cmt__t {
		font-family: var(--font-sans);
		color: var(--fg);
		white-space: pre-wrap;
	}
	.hl-draft {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 6px;
		resize: vertical;
	}
	.hl-draft:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.hl-del {
		flex-shrink: 0;
		margin: 4px 4px 0 0;
		padding: 0 6px;
		font: inherit;
		line-height: 18px;
		color: var(--base5);
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hl-del:hover:not(:disabled) {
		color: var(--danger);
		border-color: var(--panel-border);
	}
	.hl-del.armed {
		color: var(--bg);
		background: var(--danger);
		border-color: var(--danger);
	}
	.hl-entry-wrap.selected {
		background: var(--base1, var(--bg-surface));
	}
	@keyframes hl-entry-flash {
		0%,
		60% {
			box-shadow: 0 0 0 2px var(--fg);
		}
		100% {
			box-shadow: 0 0 0 0 transparent;
		}
	}
	.hl-entry-wrap.flash {
		animation: hl-entry-flash 1.3s ease-out;
	}
	.hl-body {
		display: flex;
		flex-direction: column;
		gap: 3px;
		min-width: 0;
	}
	.hl-label {
		align-self: flex-start;
		padding: 0 5px;
		border-radius: 2px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.hl-actions {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 4px 8px 8px 16px;
	}
	.hl-restyle {
		display: flex;
		align-items: flex-start;
		gap: 6px;
		flex-wrap: wrap;
	}
	.hl-restyle__label {
		flex: 1;
		min-width: 90px;
		font: inherit;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 6px;
	}
	.hl-actions__row {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.hl-actions__hint {
		flex: 1;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.hl-act {
		font: inherit;
		padding: 3px 8px;
		color: var(--fg-muted);
		background: var(--base2);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hl-act--primary {
		color: var(--bg);
		background: var(--id-yours);
		border-color: var(--id-yours);
	}
	.hl-act:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.hl-drawer {
		position: fixed;
		right: 16px;
		bottom: 16px;
		width: 320px;
		/* Hard cap: the list scrolls inside, the drawer never grows past
		   this however many highlights / highlighters there are. */
		max-height: min(50vh, 480px);
		overflow: hidden;
		background: var(--bg-panel, var(--bg-surface));
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		display: flex;
		flex-direction: column;
		box-shadow: var(--shadow-md);
		transform: translateX(calc(100% + 32px));
		transition: transform 180ms ease;
		z-index: 200;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.hl-drawer.open {
		transform: none;
	}

	.hl-drawer__head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 10px;
		border-bottom: 1px solid var(--panel-border);
	}
	.hl-drawer__scope {
		color: var(--base5);
		font-weight: normal;
	}
	.hl-drawer__title {
		margin: 0;
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--state-online);
	}
	.hl-drawer__head-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.hl-drawer__refresh {
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base5);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		padding: 2px 6px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hl-drawer__refresh:hover:not(:disabled) {
		color: var(--state-online);
		border-color: var(--state-online);
	}
	.hl-drawer__refresh:disabled {
		opacity: 0.6;
		cursor: not-allowed;
	}
	.hl-drawer__close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		line-height: 1;
		cursor: pointer;
		padding: 2px 6px;
	}
	.hl-drawer__close:hover {
		color: var(--fg);
	}

	.hl-authors {
		list-style: none;
		margin: 0;
		padding: 4px 0;
		overflow-y: auto;
		flex: 1;
		min-height: 0;
	}
	.hl-author {
		margin: 0;
	}
	.hl-author-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 6px 10px;
		width: 100%;
		background: transparent;
		border: none;
		color: var(--fg);
		text-align: left;
		cursor: pointer;
		font: inherit;
	}
	.hl-author-row:hover {
		background: var(--bg-surface);
	}
	.hl-swatch {
		width: 10px;
		height: 10px;
		border-radius: 2px;
		flex-shrink: 0;
	}
	.hl-author-pk {
		background: transparent;
		color: var(--base6);
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.hl-author-name {
		flex: 1;
		min-width: 0;
		color: var(--base7);
		font-family: var(--font-sans, inherit);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.hl-count {
		color: var(--state-online);
	}
	.hl-chevron {
		color: var(--base5);
		transition: transform 120ms ease;
		min-width: 1ch;
	}
	.hl-chevron.open {
		transform: rotate(90deg);
	}

	.hl-entries {
		list-style: none;
		margin: 0 0 4px;
		padding: 0 10px 0 22px;
	}
	.hl-entries li {
		margin: 2px 0;
	}
	.hl-entry {
		display: flex;
		align-items: stretch;
		gap: 8px;
		padding: 4px 6px;
		width: 100%;
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--fg);
		text-align: left;
		cursor: pointer;
		font: inherit;
		min-height: 22px;
	}
	.hl-entry:hover:not(:disabled) {
		border-color: var(--panel-border);
		background: var(--bg-surface);
	}
	.hl-entry:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.hl-stripe {
		width: 3px;
		flex-shrink: 0;
		border-radius: 1px;
	}
	.hl-preview {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--base6);
	}
	.hl-empty {
		padding: 16px 12px;
		color: var(--base5);
		font-style: italic;
	}
</style>
