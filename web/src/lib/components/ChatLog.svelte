<script lang="ts">
	import type { Fragment, ContextItem } from '$lib/types';
	import ChatMessage from './ChatMessage.svelte';

	let {
		fragments,
		checkedIds,
		ontogglecheck,
		chatFragmentItems,
		loading = false
	}: {
		fragments: Fragment[];
		checkedIds: Set<number>;
		ontogglecheck: (id: number) => void;
		chatFragmentItems: Map<number, ContextItem>;
		loading?: boolean;
	} = $props();

	let container: HTMLDivElement | undefined = $state();

	$effect(() => {
		fragments.length;
		if (container) {
			container.scrollTop = container.scrollHeight;
		}
	});
</script>

<div class="log" bind:this={container}>
	{#each fragments as fragment (fragment.id)}
		{@const composeItem = chatFragmentItems.get(fragment.id)}
		<ChatMessage
			{fragment}
			checked={checkedIds.has(fragment.id)}
			ontoggle={ontogglecheck}
			inCompose={!!composeItem}
			composeModified={!!composeItem && composeItem.content !== composeItem.original_content}
		/>
	{/each}
	{#if loading}
		<div class="loading">
			<span class="dot"></span><span class="dot"></span><span class="dot"></span>
		</div>
	{/if}
	{#if fragments.length === 0 && !loading}
		<p class="empty">No messages yet. Start a conversation.</p>
	{/if}
</div>

<style>
	.log {
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 16px;
	}

	.empty {
		color: var(--fg-muted);
		text-align: center;
		margin-top: 40px;
		font-size: var(--t-sm);
	}

	.loading {
		display: flex;
		gap: 4px;
		padding: 10px 14px;
		align-self: flex-start;
	}

	.dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--fg-muted);
		animation: bounce 1.2s infinite ease-in-out;
	}

	.dot:nth-child(2) { animation-delay: 0.2s; }
	.dot:nth-child(3) { animation-delay: 0.4s; }

	@keyframes bounce {
		0%, 60%, 100% { opacity: 0.3; transform: translateY(0); }
		30% { opacity: 1; transform: translateY(-4px); }
	}
</style>
