<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import { fly } from 'svelte/transition';

	const app = getAppState();
</script>

<div class="toast-stack" aria-live="polite" aria-atomic="false">
	{#each app.toasts as t (t.id)}
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<div
			class="toast toast--{t.kind}"
			role="status"
			onclick={() => app.dismissToast(t.id)}
			in:fly={{ y: 12, duration: 140 }}
			out:fly={{ y: 12, duration: 140 }}
			title="Click to dismiss"
		>
			<span class="toast__dot" aria-hidden="true"></span>
			<span class="toast__msg">{t.message}</span>
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
		padding: 6px 12px;
		font-family: var(--font-sans);
		font-size: 0.78rem;
		color: var(--fg);
		box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3);
		cursor: pointer;
		max-width: 360px;
	}
	.toast__dot {
		display: inline-block;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--id-yours);
		flex-shrink: 0;
	}
	.toast__msg {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.toast--success {
		border-color: color-mix(in srgb, var(--state-online, var(--green)) 40%, var(--panel-border));
	}
	.toast--success .toast__dot {
		background: var(--state-online, var(--green));
	}
	.toast--info {
		border-color: color-mix(in srgb, var(--id-yours) 40%, var(--panel-border));
	}
	.toast--info .toast__dot {
		background: var(--id-yours);
	}
	.toast--error {
		border-color: color-mix(in srgb, var(--state-error, var(--red)) 40%, var(--panel-border));
	}
	.toast--error .toast__dot {
		background: var(--state-error, var(--red));
	}
	/* In-progress state — violet with a soft pulse so the user can tell
	   the operation hasn't completed yet. `updateToast` later flips the
	   kind to success (green) or error (red), which keeps the same dom
	   node so the cross-fade between border/dot colors reads as a
	   single status changing rather than a new toast appearing. */
	.toast--pending {
		border-color: color-mix(in srgb, var(--id-forked) 40%, var(--panel-border));
		transition: border-color 220ms ease;
	}
	.toast--pending .toast__dot {
		background: var(--id-forked);
		animation: toast-pulse 1.1s ease-in-out infinite;
	}
	.toast--pending .toast__msg {
		color: var(--id-forked);
	}
	.toast .toast__dot { transition: background 220ms ease; }
	.toast .toast__msg { transition: color 220ms ease; }
	@keyframes toast-pulse {
		0%, 100% { opacity: 1; box-shadow: 0 0 0 0 var(--id-forked); }
		50%      { opacity: 0.55; box-shadow: 0 0 0 4px color-mix(in srgb, var(--id-forked) 25%, transparent); }
	}
</style>
