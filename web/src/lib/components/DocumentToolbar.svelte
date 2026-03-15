<script lang="ts">
	import type { ViewMode, DocMode } from '$lib/types';

	let {
		viewMode,
		docMode,
		previewVisible = false,
		onviewmode,
		ontogglepreview,
		oncompose
	}: {
		viewMode: ViewMode;
		docMode: DocMode;
		previewVisible?: boolean;
		onviewmode: (mode: ViewMode) => void;
		ontogglepreview: () => void;
		oncompose: () => void;
	} = $props();
</script>

<div class="doc-toolbar">
	<div class="doc-toolbar-left">
		{#if docMode === 'reading'}
			<button class:active={viewMode === 'outline'} onclick={() => onviewmode('outline')}>
				Outline
			</button>
			<button
				class:active={viewMode === 'continuous'}
				onclick={() => onviewmode('continuous')}
			>
				Continuous
			</button>
			<button
				class:active={viewMode === 'paginated'}
				onclick={() => onviewmode('paginated')}
			>
				Paginated
			</button>
		{/if}
	</div>
	<div class="doc-toolbar-right">
		{#if docMode === 'reading'}
			<button class:active={previewVisible} onclick={ontogglepreview}>JSON</button>
		{/if}
		{#if docMode !== 'compose'}
			<button onclick={oncompose}>Compose</button>
		{/if}
	</div>
</div>

<style>
	.doc-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		gap: 8px;
	}

	.doc-toolbar-left,
	.doc-toolbar-right {
		display: flex;
		gap: 6px;
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
