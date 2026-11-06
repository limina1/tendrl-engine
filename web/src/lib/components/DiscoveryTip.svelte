<script lang="ts">
	// Floating coachmark for the contextual walkthrough. Points at the element
	// carrying `data-tour="<anchor>"` for the active tip: a light spotlight ring
	// around it (purely visual — pointer-events stay off so the app is still
	// usable underneath) plus a small card with the description, an always-present
	// X, an optional "try it" action, and Next / Skip when more tips are queued.
	// Resolving + positioning is the overlay's job; the store stays DOM-free.
	//
	// The card is non-blocking and *stays* until the user dismisses it — no
	// click-outside dismissal, because the click a card asks for ("open
	// Settings here") must not be the click that kills it. It only shows when
	// its anchor is actionable (mounted, on screen, not under a modal scrim);
	// otherwise it waits, hidden. An anchor that never mounts drops the beat
	// unseen after a short grace so the queue can't wedge.

	import { tick, untrack } from 'svelte';
	import {
		discovery,
		activeTip,
		renderBodyHtml,
		dismissActive,
		skipActive,
		endWalkthrough
	} from '$lib/wm/discovery.svelte';
	import { shell } from '$lib/wm/shell.svelte';

	const GAP = 10; // px between anchor and card
	const MARGIN = 8; // px min gap from any viewport edge

	let rect = $state<DOMRect | null>(null);
	let vw = $state(0);
	let vh = $state(0);

	// The card's own measured size — positioning clamps against these so the
	// card (footer + Next button included) never spills off-screen. Re-measured
	// on every content change via a ResizeObserver, so geometry stays reactive
	// as the tip text and footer change between beats.
	let cardEl = $state<HTMLElement | null>(null);
	let cardW = $state(300);
	let cardH = $state(0);

	// User-drag offset, applied on top of the anchored position. Reset whenever
	// the active tip changes so each beat starts glued to its anchor again.
	let dragOffset = $state<{ x: number; y: number } | null>(null);

	// Per-shell resolution: a tip's `mobile` overrides (anchor / body /
	// placement) apply when the mobile shell is active, so beats anchored to
	// desktop chrome retarget instead of auto-skipping. The spread keeps
	// `key`, so vars/seen/queue bookkeeping is shell-agnostic.
	const rawTip = $derived(activeTip());
	const tip = $derived(
		rawTip && shell.mode === 'mobile' && rawTip.mobile ? { ...rawTip, ...rawTip.mobile } : rawTip
	);
	const body = $derived(tip ? renderBodyHtml(tip) : '');
	const remaining = $derived(discovery.queue.length);
	// A guided segment chains one tip at a time (queue length 1) via `next`, so
	// "more is coming" is either a stacked queue or a declared next link.
	const hasMore = $derived(remaining > 1 || !!tip?.next);

	// A modal (`aria-modal`) is open somewhere: the ring drops its page mask so
	// it never stacks a second dim layer on the modal's own scrim.
	let modalOpen = $state(false);

	const MODAL_SEL = '[aria-modal="true"]';

	/** The anchor is *actionable*: its centre is inside the viewport and nothing
	 *  else — a modal scrim, a dropdown backdrop — sits on top of it (the card
	 *  itself excepted, since a clamped card may overlap its anchor). Pointing
	 *  at a target the user cannot click is worse than waiting. */
	function actionable(el: Element, r: DOMRect): boolean {
		const cx = r.left + r.width / 2;
		const cy = r.top + r.height / 2;
		if (r.width === 0 || r.height === 0) return false;
		if (cx < 0 || cy < 0 || cx > vw || cy > vh) return false;
		const modal = document.querySelector(MODAL_SEL);
		if (modal && !modal.contains(el)) return false;
		const top = document.elementFromPoint(cx, cy);
		if (!top) return false;
		if (cardEl && cardEl.contains(top)) return true;
		return el.contains(top) || top.contains(el);
	}

	/** Resolve the active tip's anchor. Returns whether it is *mounted* (the
	 *  skip clock runs on that); `rect` is set only when it is actionable, so
	 *  the card hides while its target is covered or scrolled away. */
	function locate(): boolean {
		if (typeof window === 'undefined') return false;
		vw = window.innerWidth;
		vh = window.innerHeight;
		modalOpen = !!document.querySelector(MODAL_SEL);
		if (!tip) {
			rect = null;
			return false;
		}
		const el = document.querySelector(`[data-tour="${tip.anchor}"]`);
		if (!el) {
			rect = null;
			return false;
		}
		const r = el.getBoundingClientRect();
		rect = actionable(el, r) ? r : null;
		return true;
	}

	// Re-resolve whenever the active tip changes, and keep the position glued to
	// the anchor through scrolls / resizes plus a light poll (occlusion — a
	// modal opening or closing over the anchor — fires no DOM event we can
	// listen for). Skip clock: an anchor that never mounts drops the beat
	// *unseen* after a short grace; a mounted-but-covered anchor just waits.
	$effect(() => {
		const key = tip?.key;
		if (!key) return;

		let unmountedSince: number | null = null;
		let skipped = false;
		const check = () => {
			if (skipped) return;
			const mounted = untrack(locate);
			if (mounted) {
				unmountedSince = null;
				return;
			}
			const now = Date.now();
			if (unmountedSince === null) unmountedSince = now;
			else if (now - unmountedSince > 600) {
				skipped = true;
				skipActive();
			}
		};
		const update = () => void untrack(locate);
		const poll = setInterval(check, 250);
		tick().then(check);

		window.addEventListener('resize', update);
		window.addEventListener('scroll', update, true);
		return () => {
			clearInterval(poll);
			window.removeEventListener('resize', update);
			window.removeEventListener('scroll', update, true);
		};
	});

	// Each new beat starts re-anchored (drop any drag the user applied to the
	// previous one). Depends only on the tip key, so dragging doesn't re-trigger.
	$effect(() => {
		tip?.key;
		dragOffset = null;
	});

	// Keep the measured card size live as its content (text / footer) changes,
	// so the clamping below reflects the real box rather than a guess.
	$effect(() => {
		if (!cardEl) return;
		const ro = new ResizeObserver(() => {
			cardW = cardEl?.offsetWidth ?? cardW;
			cardH = cardEl?.offsetHeight ?? cardH;
		});
		ro.observe(cardEl);
		return () => ro.disconnect();
	});

	function onHandleDown(e: PointerEvent) {
		// Drag from the empty header space only — leave the dismiss button and the
		// title text alone so the title stays selectable (copy/paste).
		if ((e.target as HTMLElement).closest('.dt-x, .dt-title')) return;
		e.preventDefault();
		const startX = e.clientX;
		const startY = e.clientY;
		const base = dragOffset ?? { x: 0, y: 0 };
		const move = (ev: PointerEvent) => {
			dragOffset = { x: base.x + (ev.clientX - startX), y: base.y + (ev.clientY - startY) };
		};
		const up = () => {
			window.removeEventListener('pointermove', move);
			window.removeEventListener('pointerup', up);
		};
		window.addEventListener('pointermove', move);
		window.addEventListener('pointerup', up);
	}

	function onKey(e: KeyboardEvent) {
		if (e.key !== 'Escape' || !tip) return;
		// A modal owns Esc while it's open — one keypress must never both drop
		// the tip and cancel the fetch the tip is explaining.
		if (document.querySelector(MODAL_SEL)) return;
		e.preventDefault();
		dismissActive();
	}

	function runAction() {
		if (!tip?.action) return;
		tip.action.run();
		dismissActive();
	}

	// Card geometry, derived from the anchor rect + chosen placement, clamped
	// against the *measured* card size so the whole card (Next button included)
	// always stays on-screen, then nudged by any user drag.
	const placement = $derived(tip?.placement ?? 'top');
	const cardStyle = $derived.by(() => {
		if (!rect) return 'visibility:hidden;';
		const cx = rect.left + rect.width / 2;
		let left: number;
		let top: number;
		if (placement === 'bottom') {
			top = rect.bottom + GAP;
			left = cx - cardW / 2;
		} else if (placement === 'top') {
			top = rect.top - GAP - cardH;
			left = cx - cardW / 2;
		} else if (placement === 'left') {
			top = rect.top;
			left = rect.left - cardW - GAP;
		} else {
			top = rect.top;
			left = rect.right + GAP;
		}
		// Clamp both axes to the viewport using the real card box.
		left = Math.max(MARGIN, Math.min(left, vw - cardW - MARGIN));
		top = Math.max(MARGIN, Math.min(top, vh - cardH - MARGIN));
		const drag = dragOffset
			? `transform:translate(${dragOffset.x}px,${dragOffset.y}px);`
			: '';
		return `left:${left}px;top:${top}px;${drag}`;
	});

	const ringStyle = $derived.by(() => {
		if (!rect) return 'display:none;';
		return `left:${rect.left - 4}px;top:${rect.top - 4}px;width:${rect.width + 8}px;height:${rect.height + 8}px;`;
	});
</script>

<svelte:window on:keydown={onKey} />

{#if tip}
	<!-- Spotlight ring: visual only, never intercepts clicks. Mask-less while
	     a modal is open so its scrim isn't dimmed twice. -->
	<div class="dt-ring" class:dt-ring--nomask={modalOpen} style={ringStyle}></div>

	<div class="dt-card" bind:this={cardEl} style={cardStyle} role="dialog" aria-label={tip.title}>
		<!-- Header doubles as the drag handle so a wonky auto-placement can be
		     nudged out of the way. -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<header class="dt-head" onpointerdown={onHandleDown} title="Drag to reposition">
			<span class="dt-badge">W</span>
			<h4 class="dt-title">{tip.title}</h4>
			<button class="dt-x" onclick={dismissActive} title="Dismiss" aria-label="Dismiss">×</button>
		</header>
		<p class="dt-body">{@html body}</p>
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
			0 0 0 9999px var(--spotlight-mask);
		pointer-events: none;
		transition: all 140ms ease;
	}
	.dt-ring--nomask {
		box-shadow: 0 0 0 2px color-mix(in srgb, var(--affordance-walkthrough) 30%, transparent);
	}
	.dt-card {
		position: fixed;
		z-index: 291;
		/* Content-sized within bounds so the box adapts reactively to the tip
		   text rather than forcing a fixed width that buttons can overflow. */
		width: max-content;
		min-width: 240px;
		max-width: min(340px, calc(100vw - 16px));
		background: var(--bg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-md);
		font-family: var(--font-mono);
		box-shadow: var(--shadow-md);
		display: flex;
		flex-direction: column;
	}
	.dt-head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 9px 10px 6px 11px;
		cursor: grab;
		touch-action: none;
		user-select: none;
	}
	.dt-head:active {
		cursor: grabbing;
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
		/* Selectable so it can be copied; not part of the drag handle. */
		user-select: text;
		cursor: text;
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
	/* Inline keyword / action chips — same accent as the search help panel's
	   tokens, so syntax and action names pop without shouting. */
	.dt-body :global(.dt-kw) {
		font-family: var(--font-mono);
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		padding: 0 3px;
		border-radius: var(--r-sm);
		font-size: 0.95em;
		white-space: nowrap;
	}
	.dt-body :global(.dt-em) {
		font-style: normal;
		color: var(--base7);
		font-weight: 600;
	}
	.dt-foot {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		row-gap: 6px;
		padding: 8px 10px 10px;
		border-top: 1px solid var(--panel-border);
	}
	.dt-foot-spacer {
		flex: 1 1 0;
		min-width: 0;
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
		max-width: 100%;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.dt-next {
		flex: 0 0 auto;
	}
	.dt-try {
		flex: 0 1 auto;
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
