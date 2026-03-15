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
		font-size: 0.8rem;
		font-weight: 600;
		color: #92400e;
		background: #fef3c7;
		border-bottom: 1px solid #fcd34d;
	}

	@media (prefers-color-scheme: dark) {
		.edit-banner {
			color: #fcd34d;
			background: #292524;
			border-bottom-color: #44403c;
		}
	}

	.edit-textarea {
		flex: 1;
		font-family: var(--font-mono);
		font-size: 0.85rem;
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
