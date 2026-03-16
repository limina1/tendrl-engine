<script lang="ts">
	import type { ContextItem, SyncMode } from '$lib/types';

	let {
		item,
		syncMode,
		panel,
		ontogglereadonly
	}: {
		item: ContextItem;
		syncMode: SyncMode;
		panel: 'context' | 'compose' | 'search';
		ontogglereadonly?: (id: string) => void;
	} = $props();

	const otherPanel = $derived(
		panel === 'context' ? 'compose' : panel === 'compose' ? 'context' : null
	);

	const inBoth = $derived(item.in_context && item.in_compose);

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

	const locationColor = $derived(
		syncMode === 'reactive' ? 'synced' : item.modified ? 'modified' : 'synced'
	);

	// Origin badge: clickable to toggle readonly, color reflects lock state
	const originColor = $derived(item.readonly ? 'readonly' : 'synced');
</script>

<span class="badges">
	{#if item.origin === 'chat'}
		<button
			class="badge badge-chat"
			class:badge-readonly={item.readonly}
			onclick={() => ontogglereadonly?.(item.id)}
			title={item.readonly ? 'Unlock' : 'Lock'}
		>chat{#if item.readonly} 🔒{/if}</button>
	{/if}
	{#if item.origin === 'search'}
		<button
			class="badge badge-{originColor}"
			onclick={() => ontogglereadonly?.(item.id)}
			title={item.readonly ? 'Unlock' : 'Lock'}
		>search{#if item.readonly} 🔒{/if}</button>
	{/if}
	{#if item.origin === 'compose'}
		<button
			class="badge badge-{originColor}"
			onclick={() => ontogglereadonly?.(item.id)}
			title={item.readonly ? 'Unlock' : 'Lock'}
		>compose{#if item.readonly} 🔒{/if}</button>
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
		border: none;
		cursor: pointer;
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
