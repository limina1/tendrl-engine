<script lang="ts">
	let {
		currentPrompt,
		onset,
		disabled = false
	}: {
		currentPrompt: string | null;
		onset: (prompt: string) => void;
		disabled?: boolean;
	} = $props();

	let value = $state(currentPrompt ?? '');
</script>

<div class="panel">
	<div class="panel-header">System Prompt</div>
	<textarea
		bind:value
		placeholder="Set a system prompt..."
		rows="3"
		{disabled}
	></textarea>
	<div class="panel-actions">
		<button class="primary" onclick={() => onset(value)} disabled={disabled || !value.trim()}>
			Set
		</button>
	</div>
</div>

<style>
	.panel {
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.panel-header {
		padding: 8px 16px;
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	textarea {
		width: 100%;
		border: none;
		border-radius: 0;
		border-top: 1px solid var(--border);
		border-bottom: 1px solid var(--border);
		font-size: 0.85rem;
	}

	.panel-actions {
		padding: 8px 16px;
		display: flex;
		gap: 8px;
	}
</style>
