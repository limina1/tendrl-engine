<script lang="ts">
	import type { Fragment } from '$lib/types';
	import ChatMessage from './ChatMessage.svelte';

	let {
		fragments,
		checkedIds,
		ontogglecheck
	}: {
		fragments: Fragment[];
		checkedIds: Set<number>;
		ontogglecheck: (id: number) => void;
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
		<ChatMessage
			{fragment}
			checked={checkedIds.has(fragment.id)}
			ontoggle={ontogglecheck}
		/>
	{/each}
	{#if fragments.length === 0}
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
		font-size: 0.9rem;
	}
</style>
