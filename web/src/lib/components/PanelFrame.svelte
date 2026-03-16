<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		collapsed = false,
		ontoggle,
		children
	}: {
		title: string;
		collapsed: boolean;
		ontoggle: () => void;
		children: Snippet;
	} = $props();
</script>

{#if collapsed}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="panel-bar" onclick={ontoggle} role="button" tabindex="0" title="Expand {title}">
		<span class="toggle">›</span>
		<span class="label">{title}</span>
	</div>
{:else}
	<div class="panel">
		<!-- svelte-ignore a11y_click_events_have_key_events -->
		<div class="panel-head" onclick={ontoggle} role="button" tabindex="0" title="Collapse {title}">
			<span class="name">{title}</span>
			<span class="toggle">‹</span>
		</div>
		<div class="panel-body">
			{@render children()}
		</div>
	</div>
{/if}

<style>
	.panel-bar {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
		padding: 6px 0;
		background: var(--bg-surface);
		width: 32px;
		min-width: 32px;
		cursor: pointer;
	}

	.panel-bar:hover {
		background: var(--border);
	}

	.label {
		writing-mode: vertical-rl;
		font-size: 0.7rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.toggle {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 24px;
		height: 24px;
		border: none;
		border-radius: 4px;
		background: transparent;
		color: var(--fg-muted);
		font-size: 1rem;
		cursor: pointer;
		padding: 0;
	}

	.toggle:hover {
		background: var(--border);
		color: var(--fg);
	}

	.panel {
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.panel-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 8px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		min-height: 28px;
		cursor: pointer;
	}

	.panel-head:hover {
		background: var(--border);
	}

	.name {
		font-size: 0.7rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.panel-body {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}
</style>
