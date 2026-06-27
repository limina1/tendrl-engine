<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import { fly } from 'svelte/transition';

	const app = getAppState();

	// One global RAF loop drives every visible toast's countdown angle.
	// Recomputed each frame from (now - startedAt) / ttlMs. Frozen for
	// pinned toasts so the slice stops eating into the circle.
	let nowTick = $state(Date.now());
	$effect(() => {
		let raf: number;
		const loop = () => {
			nowTick = Date.now();
			raf = requestAnimationFrame(loop);
		};
		raf = requestAnimationFrame(loop);
		return () => cancelAnimationFrame(raf);
	});

	function angleFor(toast: { pinned: boolean; ttlMs: number; startedAt: number }): number {
		if (toast.pinned) return 0;
		const elapsed = Math.min(nowTick - toast.startedAt, toast.ttlMs);
		// 360deg when nothing has elapsed → 0deg when fully elapsed.
		return Math.max(0, 360 * (1 - elapsed / toast.ttlMs));
	}

	function onToastClick(toast: { id: number; pinned: boolean }) {
		if (toast.pinned) return; // pinned toasts only dismiss via the × button
		app.pinToast(toast.id);
	}
</script>

<div class="toast-stack" aria-live="polite" aria-atomic="false">
	{#each app.toasts as t (t.id)}
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<!-- svelte-ignore a11y_no_static_element_interactions -->
		<div
			class="toast toast--{t.kind}"
			class:toast--pinned={t.pinned}
			role="status"
			onclick={() => onToastClick(t)}
			in:fly={{ y: 12, duration: 140 }}
			out:fly={{ y: 12, duration: 140 }}
			title={t.pinned ? '' : 'Click to pin'}
		>
			<!-- Clock-style radial countdown. Conic-gradient covers the
				 remaining time; the rest of the circle is transparent. -->
			{#if t.pinned}
				<span class="toast__dot" aria-hidden="true"></span>
			{:else}
				<span
					class="toast__countdown"
					aria-hidden="true"
					style="background: conic-gradient(currentColor {angleFor(t)}deg, transparent 0);"
				></span>
			{/if}

			<span class="toast__msg">{t.message}</span>

			{#if t.pinned}
				{#if t.activity}
					<button
						class="toast__btn"
						onclick={(e) => {
							e.stopPropagation();
							app.expandActivityToast(t.id);
						}}
						title="Expand — full request detail"
						aria-label="Expand activity detail"
					>
						⤢
					</button>
				{/if}
				<button
					class="toast__btn toast__btn--close"
					onclick={(e) => {
						e.stopPropagation();
						app.dismissToast(t.id);
					}}
					title="Close"
					aria-label="Close toast"
				>
					×
				</button>
			{/if}
		</div>
	{/each}
</div>

<style>
	.toast-stack {
		position: fixed;
		right: 16px;
		/* Stay above the modeline so the stack never overlaps the pill
		   cluster — same constraint that modal backdrops obey via
		   --modeline-h. */
		bottom: calc(var(--modeline-h, 23px) + 12px);
		display: flex;
		flex-direction: column;
		gap: 6px;
		align-items: flex-end;
		/* Above every modal layer (modals are 100–110, popover 120). */
		z-index: 200;
		pointer-events: none;
	}
	.toast {
		pointer-events: auto;
		display: inline-flex;
		align-items: center;
		gap: 8px;
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: 6px 10px 6px 12px;
		font-family: var(--font-sans);
		font-size: var(--t-2xs);
		color: var(--fg);
		box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3);
		cursor: pointer;
		max-width: 360px;
	}
	.toast--pinned {
		cursor: default;
		/* A subtle indicator that this toast won't auto-dismiss. */
		box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3), 0 0 0 1px var(--panel-border-strong, var(--panel-border));
	}
	.toast__dot {
		display: inline-block;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--id-yours);
		flex-shrink: 0;
	}
	/* Clock-style countdown. `currentColor` picks up the per-kind color
	   from .toast--success / .toast--info / .toast--error / .toast--pending. */
	.toast__countdown {
		display: inline-block;
		width: 11px;
		height: 11px;
		border-radius: 50%;
		flex-shrink: 0;
		/* Slightly inset so the slice doesn't look like a rectangle when
		   nearly full. */
		mask: radial-gradient(circle, transparent 30%, black 32%);
		-webkit-mask: radial-gradient(circle, transparent 30%, black 32%);
	}
	.toast__msg {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		flex: 1;
	}
	.toast__btn {
		appearance: none;
		background: none;
		border: none;
		color: var(--muted);
		font-size: var(--t-sm);
		line-height: 1;
		padding: 2px 4px;
		cursor: pointer;
		border-radius: 3px;
		flex-shrink: 0;
	}
	.toast__btn:hover {
		background: color-mix(in srgb, var(--fg) 10%, transparent);
		color: var(--fg);
	}
	.toast--success {
		border-color: color-mix(in srgb, var(--state-online, var(--green)) 40%, var(--panel-border));
		color: var(--state-online, var(--green));
	}
	.toast--success .toast__dot,
	.toast--success .toast__countdown {
		background-color: var(--state-online, var(--green));
		color: var(--state-online, var(--green));
	}
	.toast--success .toast__msg {
		color: var(--fg);
	}
	.toast--info {
		border-color: color-mix(in srgb, var(--id-yours) 40%, var(--panel-border));
		color: var(--id-yours);
	}
	.toast--info .toast__dot,
	.toast--info .toast__countdown {
		background-color: var(--id-yours);
		color: var(--id-yours);
	}
	.toast--info .toast__msg {
		color: var(--fg);
	}
	.toast--error {
		border-color: color-mix(in srgb, var(--state-error, var(--red)) 40%, var(--panel-border));
		color: var(--state-error, var(--red));
	}
	.toast--error .toast__dot,
	.toast--error .toast__countdown {
		background-color: var(--state-error, var(--red));
		color: var(--state-error, var(--red));
	}
	.toast--error .toast__msg {
		color: var(--fg);
	}
	/* In-progress state — violet with a soft pulse so the user can tell
	   the operation hasn't completed yet. `updateToast` later flips the
	   kind to success (green) or error (red), which keeps the same dom
	   node so the cross-fade between border/dot colors reads as a
	   single status changing rather than a new toast appearing. */
	.toast--pending {
		border-color: color-mix(in srgb, var(--id-forked) 40%, var(--panel-border));
		color: var(--id-forked);
		transition: border-color 220ms ease, color 220ms ease;
	}
	.toast--pending .toast__dot {
		background: var(--id-forked);
		animation: toast-pulse 1.1s ease-in-out infinite;
	}
	.toast--pending .toast__countdown {
		background-color: var(--id-forked);
	}
	.toast--pending .toast__msg {
		color: var(--id-forked);
	}
	.toast .toast__dot,
	.toast .toast__countdown {
		transition: background-color 220ms ease;
	}
	.toast .toast__msg {
		transition: color 220ms ease;
	}
	@keyframes toast-pulse {
		0%,
		100% {
			opacity: 1;
			box-shadow: 0 0 0 0 var(--id-forked);
		}
		50% {
			opacity: 0.55;
			box-shadow: 0 0 0 4px color-mix(in srgb, var(--id-forked) 25%, transparent);
		}
	}
</style>
