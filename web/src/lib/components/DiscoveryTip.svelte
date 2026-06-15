<script lang="ts">
	// Floating coachmark for the contextual walkthrough. Points at the element
	// carrying `data-tour="<anchor>"` for the active tip: a light spotlight ring
	// around it (purely visual — pointer-events stay off so the app is still
	// usable underneath) plus a small card with the description, an always-present
	// X, an optional "try it" action, and Next / Skip when more tips are queued.
	// Resolving + positioning is the overlay's job; the store stays DOM-free.

	import { tick } from 'svelte';
	import {
		discovery,
		activeTip,
		renderBody,
		dismissActive,
		endWalkthrough
	} from '$lib/wm/discovery.svelte';

	const GAP = 10; // px between anchor and card
	const CARD_W = 300;

	let rect = $state<DOMRect | null>(null);
	let vw = $state(0);
	let vh = $state(0);

	const tip = $derived(activeTip());
	const body = $derived(tip ? renderBody(tip) : '');
	const remaining = $derived(discovery.queue.length);
	// A guided segment chains one tip at a time (queue length 1) via `next`, so
	// "more is coming" is either a stacked queue or a declared next link.
	const hasMore = $derived(remaining > 1 || !!tip?.next);

	function locate() {
		if (typeof window === 'undefined') return;
		vw = window.innerWidth;
		vh = window.innerHeight;
		if (!tip) {
			rect = null;
			return;
		}
		const el = document.querySelector(`[data-tour="${tip.anchor}"]`);
		rect = el ? el.getBoundingClientRect() : null;
	}

	// Re-resolve whenever the active tip changes, and keep the position glued to
	// the anchor through scrolls / resizes. If the anchor never mounts, skip the
	// tip after a grace period so it can't wedge the queue.
	$effect(() => {
		const key = tip?.key;
		if (!key) return;

		let skipTimer: ReturnType<typeof setTimeout> | null = null;
		const update = () => locate();

		tick().then(() => {
			locate();
			if (!rect) {
				skipTimer = setTimeout(() => {
					locate();
					if (!rect) dismissActive();
				}, 600);
			}
		});

		window.addEventListener('resize', update);
		window.addEventListener('scroll', update, true);
		return () => {
			if (skipTimer) clearTimeout(skipTimer);
			window.removeEventListener('resize', update);
			window.removeEventListener('scroll', update, true);
		};
	});

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && tip) {
			e.preventDefault();
			dismissActive();
		}
	}

	function runAction() {
		if (!tip?.action) return;
		tip.action.run();
		dismissActive();
	}

	// Card geometry, derived from the anchor rect + chosen placement, clamped so
	// the card never spills off-screen.
	const placement = $derived(tip?.placement ?? 'top');
	const cardStyle = $derived.by(() => {
		if (!rect) return 'visibility:hidden;';
		const cx = rect.left + rect.width / 2;
		let left = cx - CARD_W / 2;
		left = Math.max(8, Math.min(left, vw - CARD_W - 8));
		let vertical: string;
		if (placement === 'bottom') {
			vertical = `top:${Math.min(rect.bottom + GAP, vh - 8)}px;`;
		} else if (placement === 'top') {
			vertical = `bottom:${Math.max(vh - rect.top + GAP, 8)}px;`;
		} else if (placement === 'left') {
			vertical = `top:${Math.max(8, rect.top)}px;`;
			left = Math.max(8, rect.left - CARD_W - GAP);
		} else {
			vertical = `top:${Math.max(8, rect.top)}px;`;
			left = Math.min(vw - CARD_W - 8, rect.right + GAP);
		}
		return `left:${left}px;${vertical}width:${CARD_W}px;`;
	});

	const ringStyle = $derived.by(() => {
		if (!rect) return 'display:none;';
		return `left:${rect.left - 4}px;top:${rect.top - 4}px;width:${rect.width + 8}px;height:${rect.height + 8}px;`;
	});
</script>

<svelte:window on:keydown={onKey} />

{#if tip}
	<!-- Spotlight ring: visual only, never intercepts clicks. -->
	<div class="dt-ring" style={ringStyle}></div>

	<div class="dt-card" style={cardStyle} role="dialog" aria-label={tip.title}>
		<header class="dt-head">
			<span class="dt-badge">W</span>
			<h4 class="dt-title">{tip.title}</h4>
			<button class="dt-x" onclick={dismissActive} title="Dismiss" aria-label="Dismiss">×</button>
		</header>
		<p class="dt-body">{body}</p>
		<footer class="dt-foot">
			{#if remaining > 1}
				<div class="dt-dots" aria-hidden="true">
					{#each discovery.queue as k, i (k)}
						<span class="dt-dot {i === 0 ? 'dt-dot--on' : ''}"></span>
					{/each}
				</div>
			{/if}
			{#if hasMore}
				<button class="dt-skip" onclick={endWalkthrough}>Skip the rest</button>
			{/if}
			<span class="dt-foot-spacer"></span>
			{#if tip.action}
				<button class="dt-try" onclick={runAction}>{tip.action.label}</button>
			{/if}
			<button class="dt-next" onclick={dismissActive}>
				{hasMore ? 'Next' : 'Got it'}
			</button>
		</footer>
	</div>
{/if}

<style>
	/* Light highlight ring around the anchored element. pointer-events:none so
	   the user can keep working the live UI underneath — the walkthrough floats,
	   it doesn't trap. */
	.dt-ring {
		position: fixed;
		z-index: 290; /* below the card, above app chrome; under the mode modal (300) */
		border: 1.5px solid var(--affordance-walkthrough);
		border-radius: var(--r-md);
		box-shadow:
			0 0 0 2px color-mix(in srgb, var(--affordance-walkthrough) 30%, transparent),
			0 0 0 9999px rgba(0, 0, 0, 0.32);
		pointer-events: none;
		transition: all 140ms ease;
	}
	.dt-card {
		position: fixed;
		z-index: 291;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-md);
		font-family: var(--font-mono);
		box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
		display: flex;
		flex-direction: column;
	}
	.dt-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 9px 10px 6px 11px;
	}
	.dt-badge {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 17px;
		height: 17px;
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--affordance-walkthrough) 22%, transparent);
		color: var(--affordance-walkthrough);
		font-weight: 700;
		font-size: var(--t-xs);
		line-height: 1;
	}
	.dt-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-sm);
		color: var(--base7);
	}
	.dt-x {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		line-height: 1;
		cursor: pointer;
		padding: 0 2px;
	}
	.dt-x:hover {
		color: var(--fg);
	}
	.dt-body {
		margin: 0;
		padding: 0 11px 8px;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.55;
	}
	.dt-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 10px 10px;
		border-top: 1px solid var(--panel-border);
	}
	.dt-foot-spacer {
		flex: 1;
	}
	.dt-dots {
		display: flex;
		gap: 4px;
		align-items: center;
	}
	.dt-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--base4);
	}
	.dt-dot--on {
		background: var(--affordance-walkthrough);
	}
	.dt-skip {
		background: transparent;
		border: none;
		color: var(--base5);
		font: inherit;
		font-size: calc(var(--t-xs) - 1px);
		cursor: pointer;
		padding: 0;
	}
	.dt-skip:hover {
		color: var(--base6);
	}
	.dt-try,
	.dt-next {
		font: inherit;
		font-size: var(--t-xs);
		padding: 4px 11px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.dt-try {
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base6);
	}
	.dt-try:hover {
		border-color: var(--affordance-walkthrough);
		color: var(--affordance-walkthrough);
	}
	.dt-next {
		background: color-mix(in srgb, var(--affordance-walkthrough) 18%, transparent);
		border: 1px solid color-mix(in srgb, var(--affordance-walkthrough) 50%, transparent);
		color: var(--affordance-walkthrough);
	}
	.dt-next:hover {
		background: color-mix(in srgb, var(--affordance-walkthrough) 28%, transparent);
	}
</style>
