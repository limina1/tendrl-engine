<script lang="ts">
	import type { ContextItem } from '$lib/types';

	// One row of subtle badges that summarise an event's state in the
	// pool: where it lives (context / compose / refs), whether it's
	// diverged from its source (modified), whether it's locked as a
	// transclusion (locked), and whether it came in from chat reasoning
	// rather than an explicit user pick (chat-origin).
	//
	// Used wherever an event renders — search rows, feed rows, profile
	// cards, refs lists, reader outline/sections — so the same vocabulary
	// reads at every glance.
	let { item }: { item: ContextItem | null } = $props();

	// Refs is the auto-hold history view. When the item is already in
	// context or compose, those badges convey "in pool" — showing refs
	// would be noise. Surface refs only when held-only.
	const heldOnly = $derived(
		item != null && item.held && !item.in_context && !item.in_compose
	);
</script>

{#if item}
	{#if item.in_context}
		<span class="psb psb--context" class:psb--modified={item.modified}>context</span>
	{/if}
	{#if item.in_compose}
		<span class="psb psb--compose" class:psb--modified={item.modified}>compose</span>
	{/if}
	{#if heldOnly}
		<span class="psb psb--refs">refs</span>
	{/if}
	{#if item.modified && !item.in_context && !item.in_compose}
		<!-- A held-only item that's been edited — surface the modified
		     bit on its own. When the item is in context/compose, the
		     membership badge already turns yellow via psb--modified. -->
		<span class="psb psb--modified-only">modified</span>
	{/if}
	{#if item.readonly}
		<span class="psb psb--locked" title="Imported as transclusion — locked to original">
			<svg class="psb__lock" viewBox="0 0 16 16" aria-hidden="true">
				<rect x="3" y="7.2" width="10" height="6.8" rx="1.6" />
				<path class="psb__lock-shackle" d="M5.5 7.2 V5 a2.5 2.5 0 0 1 5 0 V7.2" />
			</svg>
			lock
		</span>
	{/if}
	{#if item.origin === 'chat'}
		<span class="psb psb--origin-chat" title="Pulled in by chat reasoning">↩ chat</span>
	{/if}
{/if}

<style>
	.psb {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 3px;
		white-space: nowrap;
		line-height: 1.4;
		font-weight: 600;
	}

	/* Membership: green/synced by default; the .psb--modified class
	   flips to yellow without losing the membership label. */
	.psb--context,
	.psb--compose {
		background: #22c55e33;
		color: #22c55e;
	}
	.psb--context.psb--modified,
	.psb--compose.psb--modified {
		background: #eab30833;
		color: #eab308;
	}

	/* Refs-only state — imported-accent tint reads as "kept aside". */
	.psb--refs {
		background: color-mix(in srgb, var(--id-imported) 25%, transparent);
		color: var(--id-imported);
	}

	/* Standalone modified pill — held-only context. Same warm tint the
	   membership flip uses, so the modified concept is one color. */
	.psb--modified-only {
		background: #eab30833;
		color: #eab308;
	}

	/* Lock — single-color SVG so the affordance reads in dark and light
	   themes without color clash. */
	.psb--locked {
		display: inline-flex;
		align-items: center;
		gap: 3px;
		background: color-mix(in srgb, var(--id-imported) 18%, transparent);
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

	/* Chat origin — purple/violet so the LLM provenance reads as
	   "not directly user-picked". */
	.psb--origin-chat {
		background: color-mix(in srgb, var(--id-yours) 20%, transparent);
		color: var(--id-yours);
	}
</style>
