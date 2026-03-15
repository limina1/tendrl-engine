<script lang="ts">
	let { data, onclose }: { data: unknown; onclose: () => void } = $props();

	const formatted = $derived(JSON.stringify(data, null, 2));
</script>

<div class="json-overlay">
	<div class="json-header">
		<span>Event JSON</span>
		<button onclick={onclose}>Close</button>
	</div>
	<pre class="json-content">{formatted}</pre>
</div>

<style>
	.json-overlay {
		position: absolute;
		inset: 0;
		z-index: 10;
		display: flex;
		flex-direction: column;
		background: var(--bg);
	}

	.json-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		font-size: 0.85rem;
		font-weight: 600;
	}

	.json-content {
		flex: 1;
		overflow: auto;
		padding: 12px;
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.8rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
