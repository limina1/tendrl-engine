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
		local,
		forked,
		containedIn,
		onpartof,
		orientation = 'vertical',
		anchor
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
		/** True = a signed snapshot the host created that hasn't been broadcast
		 *  to any relay yet. Drives the "local" pill (takes precedence over the
		 *  relay label). undefined on surfaces that don't track it. */
		local?: boolean;
		/** True when the index event carries a NIP-54 e-tag with the
		 *  `fork` marker — i.e. the publication is forked from another. */
		forked?: boolean;
		/** How many publications reference this event as a child (reverse
		 *  a-tag). Drives the clickable "part of N" pill — the upward
		 *  counterpart to provenance. Suppressed when 0/undefined or when
		 *  `onpartof` isn't wired. */
		containedIn?: number;
		/** Click handler for the "part of N" pill — typically a search for
		 *  the containing publications. */
		onpartof?: () => void;
		/** Vertical (default) stacks pills column-wise — works on row-style
		 *  surfaces (feed/profile/search rows). Horizontal lays them inline
		 *  for header surfaces where a column would push content down
		 *  (paginated section title, reader publication header). */
		orientation?: 'vertical' | 'horizontal';
		/** Optional `data-tour` value for the walkthrough to anchor a coachmark
		 *  at this pill stack (e.g. the feed's first row). Omitted = no anchor. */
		anchor?: string;
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

<div class="psb" class:psb--horizontal={orientation === 'horizontal'} data-tour={anchor}>
	<!-- Provenance first — where the event lives in the network.
	     local (signed but not broadcast), relay-label or remote (on relays),
	     fork (NIP-54 e-tag with fork marker). Renders only when the host
	     supplies signed/relays/local/forked; suppressed otherwise. The
	     unsigned "draft" pill is legacy — the signed-snapshot model never
	     writes unsigned events to the db, but search rows may still pass it. -->
	{#if signed === false}
		<span class="psb__pill psb__pill--passive psb__pill--draft" title="Unsigned event (placeholder signature)">unsigned</span>
	{:else if local}
		<span class="psb__pill psb__pill--passive psb__pill--local" title="Signed local snapshot — not broadcast to any relay yet">local</span>
	{:else if signed === true && relays && relays.length > 0}
		<span class="psb__pill psb__pill--passive psb__pill--remote" title={`On ${relays.length} relay(s):\n${relays.join('\n')}`}>{relayLabel(relays)}</span>
	{:else if signed === true}
		<span class="psb__pill psb__pill--passive psb__pill--remote" title="From relays — origin relay not recorded">remote</span>
	{/if}
	{#if forked}
		<span class="psb__pill psb__pill--passive psb__pill--fork" title="Forked from another publication (NIP-54 e-tag with fork marker)">fork</span>
	{/if}
	<!-- Containment — the upward "what is this part of" relationship. Clickable:
	     opens a search for the containing publications. -->
	{#if onpartof && containedIn && containedIn > 0}
		<button
			class="psb__pill psb__pill--partof"
			onclick={(e) => {
				e.stopPropagation();
				onpartof?.();
			}}
			title={`In ${containedIn} ${containedIn === 1 ? 'index' : 'indices'} — click to find the containing publications`}
		>⊂ in {containedIn} {containedIn === 1 ? 'index' : 'indices'}</button>
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
		font-size: var(--t-3xs);
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
		background: color-mix(in srgb, var(--success) 20%, transparent);
		color: var(--success);
	}
	.psb__pill--on:hover {
		background: color-mix(in srgb, var(--success) 33%, transparent);
		color: var(--success);
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
		background: color-mix(in srgb, var(--warning) 20%, transparent);
		color: var(--warning);
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
		background: color-mix(in srgb, var(--red) 12%, transparent);
		color: var(--id-draft);
	}
	.psb__pill--remote {
		background: color-mix(in srgb, var(--cyan) 12%, transparent);
		color: var(--id-remote);
	}
	/* Signed but not broadcast — the user's local-only snapshot. Distinct
	   token from draft/remote so "I haven't pushed this yet" reads at a glance. */
	.psb__pill--local {
		background: color-mix(in srgb, var(--id-local, var(--id-imported, var(--id-yours))) 16%, transparent);
		color: var(--id-local, var(--id-imported, var(--id-yours)));
	}
	.psb__pill--fork {
		background: color-mix(in srgb, var(--id-imported) 22%, transparent);
		color: var(--id-imported);
	}
	/* Containment — clickable, green (same token as the active ctx/cmp pills)
	   since it points at the publications this event belongs to. */
	.psb__pill--partof {
		background: color-mix(in srgb, var(--success) 20%, transparent);
		color: var(--success);
		cursor: pointer;
	}
	.psb__pill--partof:hover {
		background: color-mix(in srgb, var(--success) 33%, transparent);
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
