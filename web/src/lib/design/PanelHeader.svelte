<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from '../Icon.svelte';

	type Props = {
		title: string;
		subtitle?: string;
		icon?: Snippet;
		actions?: Snippet;
		onCollapse?: () => void;
	};

	let { title, subtitle, icon, actions, onCollapse }: Props = $props();
</script>

<div class="panel-header">
	{#if icon}
		<span class="panel-header__icon">
			{@render icon()}
		</span>
	{/if}
	<span class="panel-header__title">{title}</span>
	{#if subtitle}
		<span class="panel-header__subtitle">{subtitle}</span>
	{/if}
	<div class="panel-header__spacer"></div>
	<div class="panel-header__actions">
		{#if actions}{@render actions()}{/if}
		{#if onCollapse}
			<button class="btn btn--ghost btn--icon" title="Collapse" onclick={onCollapse}>
				<Icon name="x" size={11} />
			</button>
		{/if}
	</div>
</div>

<style>
	.panel-header__icon {
		color: var(--base5);
		display: flex;
	}
	.panel-header__subtitle {
		color: var(--base5);
		text-transform: none;
		letter-spacing: 0;
		font-family: var(--font-sans);
	}
</style>
