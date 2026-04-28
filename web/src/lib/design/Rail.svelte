<script lang="ts">
	import type { Snippet } from 'svelte';

	export type RailItem = {
		id: string;
		label: string;
		key: string;
		active?: boolean;
		icon: Snippet;
	};

	type Props = {
		side?: 'left' | 'right';
		items: RailItem[];
		onOpen?: (id: string) => void;
		footer?: Snippet;
	};

	let { side = 'left', items, onOpen, footer }: Props = $props();
</script>

<div class="rail {side === 'right' ? 'rail--right' : ''}">
	{#each items as item (item.id)}
		<div class="rail__item">
			<button
				class="rail-btn {item.active ? 'rail-btn--active' : ''}"
				title={`${item.label}  (${item.key})`}
				onclick={() => onOpen?.(item.id)}
			>
				{@render item.icon()}
			</button>
			<div class="rail-label">{item.label}</div>
			<div class="rail-key">{item.key}</div>
		</div>
	{/each}
	<div class="rail-spacer"></div>
	{#if footer}{@render footer()}{/if}
</div>

<style>
	.rail__item {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2px;
	}
</style>
