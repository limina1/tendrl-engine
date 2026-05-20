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
		onpilldrop
	}: {
		item: ContextItem | null;
		/** Click handler for the ctx pill. If omitted, the pill is hidden.
		 *  The host decides what "ctx click" means for its surface (add to
		 *  pool, toggle, etc.). */
		onpillctx?: () => void;
		onpillcmp?: () => void;
		/** Drop only renders when an item exists; nothing to drop otherwise. */
		onpilldrop?: () => void;
	} = $props();

	// "In refs but no other membership" — the bookmark-only state. Only
	// surfaces as a passive pill when this is the case (otherwise ctx /
	// cmp pills already convey the in-pool fact).
	const refsOnly = $derived(
		item != null && item.held && !item.in_context && !item.in_compose
	);
</script>

<div class="psb">
	{#if onpillctx}
		<button
			class="psb__pill psb__pill--act"
			class:psb__pill--on={item?.in_context}
			onclick={(e) => {
				e.stopPropagation();
				onpillctx?.();
			}}
			title={item?.in_context ? 'In chat context — click to remove' : 'Send to chat context'}
		>ctx</button>
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
		>cmp</button>
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
	{#if refsOnly}
		<span class="psb__pill psb__pill--passive psb__pill--refs" title="Held in refs">refs</span>
	{/if}
	{#if item?.modified}
		<span class="psb__pill psb__pill--passive psb__pill--mod" title="Diverged from source">mod</span>
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
	.psb {
		display: flex;
		flex-direction: column;
		gap: 4px;
		align-items: stretch;
		flex-shrink: 0;
	}
	.psb__pill {
		font-family: var(--font-mono);
		font-size: 0.62rem;
		line-height: 1.2;
		padding: 2px 10px;
		border-radius: 999px;
		text-align: center;
		min-width: 52px;
		white-space: nowrap;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 3px;
	}

	/* Action pills — actual buttons. Default to a muted outline, green
	   tint when the underlying flag is on. Hover bumps the accent. */
	.psb__pill--act {
		background: transparent;
		border: 1px solid var(--base3);
		color: var(--base6);
		cursor: pointer;
		transition: color 0.1s, border-color 0.1s, background 0.1s;
	}
	.psb__pill--act:hover {
		color: var(--fg);
		border-color: var(--id-yours);
	}
	.psb__pill--on {
		background: color-mix(in srgb, #22c55e 18%, transparent);
		border-color: color-mix(in srgb, #22c55e 55%, transparent);
		color: #22c55e;
	}
	.psb__pill--on:hover {
		background: color-mix(in srgb, #22c55e 30%, transparent);
		border-color: #22c55e;
		color: #22c55e;
	}

	.psb__pill--drop {
		background: transparent;
		border: 1px solid var(--base3);
		color: var(--base6);
		cursor: pointer;
	}
	.psb__pill--drop:hover {
		color: var(--fg);
		border-color: var(--id-imported);
	}

	/* Passive pills — informational only, no border in the muted state
	   so the row reads as info rather than affordance. */
	.psb__pill--passive {
		border: 1px solid transparent;
		font-weight: 600;
	}
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
