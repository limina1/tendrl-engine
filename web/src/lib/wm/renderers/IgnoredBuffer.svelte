<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();
</script>

<div class="ignored-view">
	<div class="ignored-header">
		<span>Hidden ({app.ignoredEventIds.length} events, {app.ignoredPubkeys.length} authors, {app.ignoredCoordinates.length} publications)</span>
	</div>
	{#if app.ignoredCoordinates.length > 0}
		<div class="ignored-section-title">Publications</div>
		{#each app.ignoredCoordinates as coord (coord)}
			{@const parts = coord.split(':')}
			<div class="ignored-item">
				<span class="ignored-id">
					{parts.slice(2).join(':') || '(no d-tag)'}
					<span class="ignored-by">by <ProfileName pubkey={parts[1] ?? ''} onviewprofile={app.handleViewProfile} /></span>
				</span>
				<button class="unignore" onclick={() => app.handleUnignore('coordinate', coord)}>Unblock</button>
			</div>
		{/each}
	{/if}
	{#if app.ignoredEventIds.length > 0}
		<div class="ignored-section-title">Events</div>
		{#each app.ignoredEventIds as id (id)}
			<div class="ignored-item">
				<span class="ignored-id">{id.slice(0, 16)}…{id.slice(-8)}</span>
				<button class="unignore" onclick={() => app.handleUnignore('event', id)}>Unblock</button>
			</div>
		{/each}
	{/if}
	{#if app.ignoredPubkeys.length > 0}
		<div class="ignored-section-title">Authors</div>
		{#each app.ignoredPubkeys as pk (pk)}
			<div class="ignored-item">
				<span class="ignored-id"><ProfileName pubkey={pk} onviewprofile={app.handleViewProfile} /></span>
				<button class="unignore" onclick={() => app.handleUnignore('pubkey', pk)}>Unblock</button>
			</div>
		{/each}
	{/if}
	{#if app.ignoredEventIds.length === 0 && app.ignoredPubkeys.length === 0 && app.ignoredCoordinates.length === 0}
		<div class="empty"><p>No hidden publications, events, or authors</p></div>
	{/if}
</div>

<style>
	.ignored-view { flex: 1; overflow-y: auto; min-height: 0; }
	.ignored-header {
		padding: 8px 12px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.ignored-section-title {
		padding: 6px 12px 2px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base5);
		text-transform: uppercase;
	}
	.ignored-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 5px 12px;
		border-bottom: 1px solid var(--panel-border);
	}
	.ignored-id { font-size: var(--t-xs); font-family: var(--font-mono); color: var(--base6); }
	.ignored-by { color: var(--base5); margin-left: 6px; }
	.unignore {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		color: var(--id-yours);
		background: transparent;
		border: 1px solid var(--id-yours);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.unignore:hover { background: var(--id-yours); color: var(--bg); }
	.empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
</style>
