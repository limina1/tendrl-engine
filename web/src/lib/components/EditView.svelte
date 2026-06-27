<script lang="ts">
	let {
		editBuffer,
		onapply,
		oncancel
	}: {
		editBuffer: string;
		onapply: (buffer: string) => void;
		oncancel: () => void;
	} = $props();

	let buffer = $state(editBuffer);
	const originalBuffer = editBuffer;
</script>

<div class="edit-view">
	<div class="edit-banner">Editing conversation</div>
	<textarea
		class="edit-textarea"
		bind:value={buffer}
	></textarea>
	<div class="edit-actions">
		<button class="primary" onclick={() => onapply(buffer)}>Apply</button>
		<button onclick={() => oncancel()}>Cancel</button>
	</div>
</div>

<style>
	.edit-view {
		flex: 1;
		display: flex;
		flex-direction: column;
		background: var(--bg-edit);
	}

	.edit-banner {
		padding: 8px 16px;
		font-size: var(--t-2xs);
		font-weight: 600;
		color: var(--warn-fg);
		background: var(--warn-bg);
		border-bottom: 1px solid var(--warn-border);
	}

	.edit-textarea {
		flex: 1;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1.6;
		padding: 16px;
		border: none;
		border-radius: 0;
		background: transparent;
	}

	.edit-actions {
		display: flex;
		gap: 8px;
		padding: 12px 16px;
		border-top: 1px solid var(--border);
	}
</style>
