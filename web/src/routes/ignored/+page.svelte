<script lang="ts">
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();
</script>

<svelte:head>
	<title>Hidden - tendrl</title>
</svelte:head>

<div class="document-panel">
	<div class="doc-content">
		<div class="ignored-view">
			<div class="ignored-header">
				<span>Hidden ({app.ignoredEventIds.length} events, {app.ignoredPubkeys.length} authors)</span>
			</div>
			{#if app.ignoredEventIds.length > 0}
				<div class="ignored-section-title">Events</div>
				{#each app.ignoredEventIds as id}
					<div class="ignored-item">
						<span class="ignored-id">{id.slice(0, 16)}...{id.slice(-8)}</span>
						<button class="unignore-btn" onclick={() => app.handleUnignore('event', id)}>Unblock</button>
					</div>
				{/each}
			{/if}
			{#if app.ignoredPubkeys.length > 0}
				<div class="ignored-section-title">Authors</div>
				{#each app.ignoredPubkeys as pk}
					<div class="ignored-item">
						<span class="ignored-id">{pk.slice(0, 16)}...{pk.slice(-8)}</span>
						<button class="unignore-btn" onclick={() => app.handleUnignore('pubkey', pk)}>Unblock</button>
					</div>
				{/each}
			{/if}
			{#if app.ignoredEventIds.length === 0 && app.ignoredPubkeys.length === 0}
				<div class="doc-empty"><p>No hidden events or authors</p></div>
			{/if}
		</div>
	</div>
</div>

<style>
	.document-panel { flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
	.doc-content { flex: 1; position: relative; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
	.doc-empty { flex: 1; display: flex; align-items: center; justify-content: center; color: var(--fg-muted); font-size: 0.85rem; }

	.ignored-view { flex: 1; overflow-y: auto; }
	.ignored-header {
		padding: 10px 16px; font-size: 0.8rem; font-weight: 600;
		color: var(--fg-muted); text-transform: uppercase; letter-spacing: 0.05em;
		border-bottom: 1px solid var(--border);
	}
	.ignored-section-title {
		padding: 8px 16px 4px; font-size: 0.7rem; font-weight: 600;
		color: var(--fg-muted); text-transform: uppercase;
	}
	.ignored-item {
		display: flex; align-items: center; justify-content: space-between;
		padding: 6px 16px; border-bottom: 1px solid var(--border);
	}
	.ignored-id { font-size: 0.75rem; font-family: var(--font-mono); color: var(--fg-muted); }
	.unignore-btn {
		font-size: 0.7rem; padding: 2px 8px; color: var(--accent);
		background: none; border: 1px solid var(--accent); border-radius: var(--radius); cursor: pointer;
	}
	.unignore-btn:hover { background: var(--accent); color: white; }
</style>
