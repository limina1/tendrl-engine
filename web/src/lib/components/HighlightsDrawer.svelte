<script lang="ts">
	import {
		pubkeyToHue,
		pubkeyToSwatch,
		pubkeyToHighlightStroke
	} from '$lib/discussions/colors';
	import { getAuthorDisplayName, hasAuthorName } from '$lib/discussions/authors.svelte';

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
	}

	let {
		highlights,
		open,
		onclose,
		onnavigate,
		onrefresh = null
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
	} = $props();

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
		for (const h of highlights) {
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
		if (e.key === 'Escape' && open) onclose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<aside class="hl-drawer" class:open aria-hidden={!open} aria-label="Highlights drawer">
	<header class="hl-drawer__head">
		<h3 class="hl-drawer__title">Highlights ({highlights.length})</h3>
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
	<ul class="hl-authors">
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
							<li>
								<button
									class="hl-entry"
									onclick={() => onnavigate(entry.id, entry.section_addr)}
									title={entry.content}
									disabled={entry.section_addr === null}
								>
									<span class="hl-stripe" style="background: {group.stripe};"></span>
									<span class="hl-preview">{preview(entry.content)}</span>
								</button>
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
		box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
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
