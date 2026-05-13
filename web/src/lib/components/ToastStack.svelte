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
</style>
