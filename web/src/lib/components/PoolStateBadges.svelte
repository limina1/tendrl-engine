<script lang="ts">
	import type { ContextItem } from '$lib/types';

	// One vertical pill stack that summarises an event's pool state and
	// — when the host wires the callbacks — doubles as the action surface.
	// Membership pills (ctx, cmp, drop) turn green when the underlying flag
	// is set and clicking toggles. State pills (lock, modified, chat-origin)
	// are passive — they appear only when active and have no click.
	//
	// Surfaces: search rows, refs rows, feed rows, profile cards, reader
	// outline / paginated header. The same component everywhere so users
	// don't relearn affordances when flipping between views.
	let {
		item,
		onpillctx,
		onpillcmp,
		onpilldrop,
		signed,
		relays,
		forked,
		orientation = 'vertical'
	}: {
		item: ContextItem | null;
		/** Click handler for the ctx pill. If omitted, the pill is hidden.
		 *  The host decides what "ctx click" means for its surface (add to
		 *  pool, toggle, etc.). */
		onpillctx?: () => void;
		onpillcmp?: () => void;
		/** Drop only renders when an item exists; nothing to drop otherwise. */
		onpilldrop?: () => void;
		/** Event provenance — passed in by surfaces that know it
		 *  (FeedBuffer, reader publication header). undefined = unknown,
		 *  pill suppressed. Search rows leave these unset. */
		signed?: boolean;
		relays?: string[];
		/** True when the index event carries a NIP-54 e-tag with the
		 *  `fork` marker — i.e. the publication is forked from another. */
		forked?: boolean;
		/** Vertical (default) stacks pills column-wise — works on row-style
		 *  surfaces (feed/profile/search rows). Horizontal lays them inline
		 *  for header surfaces where a column would push content down
		 *  (paginated section title, reader publication header). */
		orientation?: 'vertical' | 'horizontal';
	} = $props();

	/** Compact relay-label: "first-host +N" when multiple, just the host
	 *  when one. Mirrors the legacy FeedBuffer relayLabel(). */
	function relayLabel(rs: string[]): string {
		const host = rs[0].replace(/^wss?:\/\//, '').replace(/\/+$/, '');
		return rs.length > 1 ? `${host} +${rs.length - 1}` : host;
	}

	// "In refs but no other membership" — the bookmark-only state. Only
	// surfaces as a passive pill when this is the case (otherwise ctx /
	// cmp pills already convey the in-pool fact).
	const refsOnly = $derived(
		item != null && item.held && !item.in_context && !item.in_compose
	);
</script>

<div class="psb" class:psb--horizontal={orientation === 'horizontal'}>
	<!-- Provenance first — where the event lives in the network.
	     draft (unsigned), relay-label or remote (signed), fork
	     (NIP-54 e-tag with fork marker). Renders only when the host
	     supplies signed/relays/forked; suppressed otherwise. -->
	{#if signed === false}
		<span class="psb__pill psb__pill--passive psb__pill--draft" title="Unsigned draft — not yet signed">draft</span>
	{:else if signed === true && relays && relays.length > 0}
		<span class="psb__pill psb__pill--passive psb__pill--remote" title={`On ${relays.length} relay(s):\n${relays.join('\n')}`}>{relayLabel(relays)}</span>
	{:else if signed === true}
		<span class="psb__pill psb__pill--passive psb__pill--remote" title="From relays — origin relay not recorded">remote</span>
	{/if}
	{#if forked}
		<span class="psb__pill psb__pill--passive psb__pill--fork" title="Forked from another publication (NIP-54 e-tag with fork marker)">fork</span>
	{/if}
	<!-- Pool routing actions — clickable toggles. -->
	{#if onpillctx}
		<button
			class="psb__pill psb__pill--act"
			class:psb__pill--on={item?.in_context}
			onclick={(e) => {
				e.stopPropagation();
				onpillctx?.();
			}}
			title={item?.in_context ? 'In chat context — click to remove' : 'Send to chat context'}
		>context</button>
	{/if}
	{#if onpillcmp}
		<button
			class="psb__pill psb__pill--act"
			class:psb__pill--on={item?.in_compose}
			onclick={(e) => {
				e.stopPropagation();
				onpillcmp?.();
			}}
			title={item?.in_compose ? 'In compose — click to remove' : 'Send to compose'}
		>compose</button>
	{/if}
	{#if onpilldrop && item}
		<button
			class="psb__pill psb__pill--drop"
			onclick={(e) => {
				e.stopPropagation();
				onpilldrop?.();
			}}
			title="Drop from pool — clears context/compose/refs"
		>drop</button>
	{/if}
	<!-- Pool-derived state — passive informational pills. -->
	{#if refsOnly}
		<span class="psb__pill psb__pill--passive psb__pill--refs" title="Held in refs">refs</span>
	{/if}
	{#if item?.modified}
		<span class="psb__pill psb__pill--passive psb__pill--mod" title="Diverged from source">modified</span>
	{/if}
	{#if item?.readonly}
		<span class="psb__pill psb__pill--passive psb__pill--lock" title="Locked — transclusion">
			<svg class="psb__lock" viewBox="0 0 16 16" aria-hidden="true">
				<rect x="3" y="7.2" width="10" height="6.8" rx="1.6" />
				<path class="psb__lock-shackle" d="M5.5 7.2 V5 a2.5 2.5 0 0 1 5 0 V7.2" />
			</svg>
			lock
		</span>
	{/if}
	{#if item?.origin === 'chat'}
		<span class="psb__pill psb__pill--passive psb__pill--chat" title="Pulled in by chat reasoning">chat</span>
	{/if}
</div>

<style>
	/* Vertical stack of subtle state pills. Same look as the original
	   loc-badge / kind-badge family (small tinted chips) so the row
	   reads as state at a glance, not a CTA strip. Action pills are
	   still clickable — same shape, just transparent in the
	   off-state and tinted green when the underlying flag is on. */
	.psb {
		display: flex;
		flex-direction: column;
		gap: 3px;
		align-items: flex-start;
		flex-shrink: 0;
	}
	/* Horizontal layout for header surfaces — paginated section title,
	   reader publication header, continuous-view section titles. Pills
	   wrap onto a second line gracefully if the title runs long. */
	.psb--horizontal {
		flex-direction: row;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px;
	}
	.psb__pill {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		line-height: 1.4;
		padding: 0 6px;
		border-radius: 3px;
		white-space: nowrap;
		display: inline-flex;
		align-items: center;
		gap: 3px;
		font-weight: 600;
		border: none;
		background: transparent;
	}

	/* Action pills — muted by default (just the label visible),
	   tinted green when the underlying flag is set. Click toggles. */
	.psb__pill--act {
		color: var(--base5);
		cursor: pointer;
		transition: background 0.1s, color 0.1s;
	}
	.psb__pill--act:hover {
		color: var(--fg);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.psb__pill--on {
		background: #22c55e33;
		color: #22c55e;
	}
	.psb__pill--on:hover {
		background: #22c55e55;
		color: #22c55e;
	}

	.psb__pill--drop {
		color: var(--base5);
		cursor: pointer;
	}
	.psb__pill--drop:hover {
		background: color-mix(in srgb, var(--id-imported) 18%, transparent);
		color: var(--id-imported);
	}

	/* Passive pills — informational only. */
	.psb__pill--refs {
		background: color-mix(in srgb, var(--id-imported) 22%, transparent);
		color: var(--id-imported);
	}
	.psb__pill--mod {
		background: #eab30833;
		color: #eab308;
	}
	.psb__pill--lock {
		background: color-mix(in srgb, var(--id-imported) 18%, transparent);
		color: var(--id-imported);
	}
	.psb__pill--chat {
		background: color-mix(in srgb, var(--id-yours) 20%, transparent);
		color: var(--id-yours);
	}
	/* Provenance pills — sit at the bottom of the stack and signal
	   where the event lives in the network. Tints come from the same
	   palette the legacy .pill--remote / .pill--draft used so the
	   migration is visually invisible. fork uses imported-accent
	   since it points back to another event. */
	.psb__pill--draft {
		background: rgba(226, 120, 120, 0.12);
		color: var(--id-draft);
	}
	.psb__pill--remote {
		background: rgba(137, 184, 194, 0.12);
		color: var(--id-remote);
	}
	.psb__pill--fork {
		background: color-mix(in srgb, var(--id-imported) 22%, transparent);
		color: var(--id-imported);
	}

	.psb__lock {
		width: 9px;
		height: 9px;
		flex-shrink: 0;
	}
	.psb__lock rect { fill: currentColor; }
	.psb__lock-shackle {
		fill: none;
		stroke: currentColor;
		stroke-width: 1.8;
	}
</style>
