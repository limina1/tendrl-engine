<script lang="ts">
	import type { ContextItem, SyncMode } from '$lib/types';

	let {
		item,
		syncMode,
		panel
	}: {
		item: ContextItem;
		syncMode: SyncMode;
		panel: 'context' | 'compose' | 'search';
	} = $props();

	const otherPanel = $derived(
		panel === 'context' ? 'compose' : panel === 'compose' ? 'context' : null
	);

	const inBoth = $derived(item.in_context && item.in_compose);

	// Location badge: where else does this item live?
	const locationLabel = $derived(
		panel === 'search'
			? item.in_context && item.in_compose
				? 'ctx+doc'
				: item.in_context
					? 'context'
					: item.in_compose
						? 'compose'
						: null
			: inBoth
				? otherPanel
				: null
	);

	// Location color: green if synced, yellow if modified
	const locationColor = $derived(
		syncMode === 'reactive' ? 'synced' : item.modified ? 'modified' : 'synced'
	);
</script>

<span class="badges">
	{#if item.readonly}
		<span class="badge badge-readonly">readonly</span>
	{/if}
	{#if item.origin === 'chat'}
		<span class="badge badge-chat">chat</span>
	{/if}
	{#if item.origin === 'search'}
		<span class="badge badge-synced">search</span>
	{/if}
	{#if locationLabel}
		<span class="badge badge-{locationColor}">{locationLabel}</span>
	{/if}
</span>

<style>
	.badges {
		display: inline-flex;
		gap: 3px;
		flex-shrink: 0;
	}

	.badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 4px;
		white-space: nowrap;
		font-weight: 600;
		line-height: 1.6;
	}

	.badge-synced {
		background: color-mix(in srgb, var(--badge-synced) 20%, transparent);
		color: var(--badge-synced);
	}

	.badge-modified {
		background: color-mix(in srgb, var(--badge-modified) 20%, transparent);
		color: var(--badge-modified);
	}

	.badge-chat {
		background: color-mix(in srgb, var(--badge-chat) 20%, transparent);
		color: var(--badge-chat);
	}

	.badge-readonly {
		background: color-mix(in srgb, var(--badge-readonly) 20%, transparent);
		color: var(--badge-readonly);
	}
</style>
