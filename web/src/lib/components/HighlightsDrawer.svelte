<script lang="ts">
	import {
		pubkeyToHue,
		pubkeyToSwatch,
		pubkeyToHighlightStroke
	} from '$lib/discussions/colors';
	import { getAuthorDisplayName, hasAuthorName } from '$lib/discussions/authors.svelte';
	import type { Highlighter } from '$lib/discussions/highlights';
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
		open,
		onclose,
		onnavigate,
		onrefresh = null,
		mypubkey = null,
		labelFilter = $bindable(null),
		onrestyle = null,
		onadopt = null
	}: {
		highlights: DrawerHighlight[];
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
	} = $props();

	// The selected entry (actions open under it) + the one flashing.
	let selected = $state<string | null>(null);
	let listEl = $state<HTMLElement | undefined>();
	let flashing = $state<string | null>(null);
	// Recolour form for the selected entry.
	let editLabel = $state('');
	let editColor = $state('#f7d94c');
	let restyling = $state(false);

	function select(h: DrawerHighlight) {
		selected = h.id;
		editLabel = h.highlighter?.label ?? '';
		editColor = h.highlighter?.color ?? '#f7d94c';
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
			if (h.highlighter) {
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
		const l = editLabel.trim();
		restyling = true;
		try {
			await onrestyle(h.id, l ? { label: l, color: editColor } : null);
			selected = null;
		} finally {
			restyling = false;
		}
	}
	const unchanged = (h: DrawerHighlight) =>
		editLabel.trim() === (h.highlighter?.label ?? '') &&
		(!editLabel.trim() || editColor === h.highlighter?.color);

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
		<h3 class="hl-drawer__title">Highlights ({visible.length})</h3>
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
						onclick={() => (labelFilter = labelFilter === '' ? null : '')}>plain</button
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
							<li
								class="hl-entry-wrap"
								class:selected={selected === entry.id}
								class:flash={flashing === entry.id}
								data-hl-id={entry.id}
							>
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
										{#if entry.highlighter}
											<span
												class="hl-label"
												style="background: {entry.highlighter.color}">{entry.highlighter.label}</span
											>
										{/if}
										<span class="hl-preview">{preview(entry.content)}</span>
									</span>
								</button>
								{#if selected === entry.id}
									<div class="hl-actions">
										{#if mine && onrestyle}
											<div class="hl-restyle">
												<input
													class="hl-restyle__label"
													data-entry
													maxlength="64"
													placeholder="label (empty = plain)"
													aria-label="Highlighter label"
													bind:value={editLabel}
													disabled={restyling}
												/>
												<ColorPicker bind:value={editColor} disabled={restyling} label="recolour" />
											</div>
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
		color: #12141c;
		font-size: calc(var(--t-xs) - 2px);
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
		max-height: 60vh;
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
