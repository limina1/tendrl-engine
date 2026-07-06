<script lang="ts">
	// The single renderer for the global in-app text prompt
	// ($lib/wm/text-prompt.svelte.ts). Mounted once in +layout next to
	// ToastStack; renders nothing unless a prompt is active. Callers
	// never touch this component — they await promptText().
	import { textPrompt, resolveTextPrompt } from '$lib/wm/text-prompt.svelte';

	let inputEl: HTMLInputElement | undefined = $state(undefined);

	// Focus the input once the modal is in the DOM.
	$effect(() => {
		if (textPrompt.active) queueMicrotask(() => inputEl?.focus());
	});
</script>

<svelte:window
	onkeydown={(e) => textPrompt.active && e.key === 'Escape' && resolveTextPrompt(false)}
/>

{#if textPrompt.active}
	{@const p = textPrompt.active}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="tp-backdrop" role="presentation" onclick={() => resolveTextPrompt(false)}>
		<div
			class="tp-modal"
			role="dialog"
			aria-modal="true"
			aria-label={p.title}
			tabindex="-1"
			onclick={(e) => e.stopPropagation()}
		>
			<div class="tp-title">{p.title}</div>
			{#if p.hint}
				<p class="tp-hint">{p.hint}</p>
			{/if}
			<input
				class="tp-input"
				type="text"
				bind:this={inputEl}
				bind:value={p.value}
				placeholder={p.placeholder}
				onkeydown={(e) => e.key === 'Enter' && resolveTextPrompt(true)}
			/>
			<div class="tp-actions">
				<button class="tp-btn" onclick={() => resolveTextPrompt(false)}>Cancel</button>
				<button
					class="tp-btn tp-btn--primary"
					onclick={() => resolveTextPrompt(true)}
					disabled={!p.value.trim()}
				>
					{p.confirmLabel}
				</button>
			</div>
		</div>
	</div>
{/if}

<style>
	.tp-backdrop {
		position: fixed;
		inset: 0;
		background: color-mix(in srgb, var(--bg, #000) 60%, transparent);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 320;
	}
	.tp-modal {
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: 16px 18px;
		max-width: 420px;
		width: min(92vw, 420px);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.tp-title {
		font-weight: 600;
	}
	.tp-hint {
		color: var(--fg-muted);
		font-size: var(--t-sm);
		margin: 0;
	}
	.tp-input {
		background: var(--bg, transparent);
		border: 1px solid var(--base3);
		border-radius: var(--r-md);
		color: var(--fg);
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		padding: 6px 8px;
		width: 100%;
	}
	.tp-input:focus {
		outline: none;
		border-color: color-mix(in srgb, var(--state-online) 60%, transparent);
	}
	.tp-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 6px;
	}
	.tp-btn {
		border: 1px solid var(--base3);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
		padding: 4px 12px;
		border-radius: var(--r-md);
	}
	.tp-btn:hover {
		background: var(--bg-hover, color-mix(in srgb, var(--fg) 8%, transparent));
	}
	.tp-btn--primary {
		border-color: color-mix(in srgb, var(--state-online) 60%, transparent);
		color: var(--state-online);
	}
	.tp-btn[disabled] {
		opacity: 0.5;
		cursor: default;
	}
</style>
