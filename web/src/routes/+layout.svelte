<script lang="ts">
	import '$lib/styles/tokens.css';
	import '../app.css';
	import { createAppState } from '$lib/state.svelte';

	let { children } = $props();

	const app = createAppState();

	let initialized = $state(false);
	$effect(() => {
		if (initialized) return;
		initialized = true;
		app.initialize();
		const cleanup = app.startNetworkPoll();
		return cleanup;
	});
</script>

{@render children()}

{#if app.jsonModalData}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="json-modal-backdrop" onclick={() => (app.jsonModalData = null)} role="presentation">
		<div class="json-modal" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
			<div class="json-modal-header">
				<span>Event JSON</span>
				<button onclick={() => (app.jsonModalData = null)}>Close</button>
			</div>
			<pre class="json-modal-body">{JSON.stringify(app.jsonModalData, null, 2)}</pre>
		</div>
	</div>
{/if}

<style>
	.json-modal-backdrop {
		position: fixed;
		inset: 0;
		z-index: 100;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.json-modal {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: 90vw;
		max-width: 720px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
	}

	.json-modal-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
		font-weight: 600;
		font-size: 0.85rem;
	}

	.json-modal-body {
		flex: 1;
		overflow: auto;
		padding: 14px;
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.8rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
