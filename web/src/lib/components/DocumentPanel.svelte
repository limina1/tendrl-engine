<script lang="ts">
	import type { Section, ViewMode, DocMode, PublicationDetail, ComposeState, ContextItem, SyncMode } from '$lib/types';
	import DocumentToolbar from './DocumentToolbar.svelte';
	import OutlineView from './OutlineView.svelte';
	import ContinuousView from './ContinuousView.svelte';
	import PaginatedView from './PaginatedView.svelte';
	import JsonPreview from './JsonPreview.svelte';
	import ComposeView from './ComposeView.svelte';

	let {
		docMode,
		publication,
		sections,
		viewMode,
		currentSection,
		previewVisible,
		compose,
		loading,
		onviewmode,
		ontogglepreview,
		oncompose,
		onnavigate,
		oncomposeupdate,
		oncancelcompose,
		onsendtochat,
		onpublishcompose,
		ondeletecompose,
		ondeletepermanentcompose,
		ondoctochat,
		ondocpublish,
		syncMode,
		onsenditemtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy
	}: {
		docMode: DocMode;
		publication: PublicationDetail | null;
		sections: Section[];
		viewMode: ViewMode;
		currentSection: number;
		previewVisible: boolean;
		compose: ComposeState;
		loading: boolean;
		onviewmode: (mode: ViewMode) => void;
		ontogglepreview: () => void;
		oncompose: () => void;
		onnavigate: (index: number) => void;
		oncomposeupdate: (state: ComposeState) => void;
		oncancelcompose: () => void;
		onsendtochat: (items: ContextItem[]) => void;
		onpublishcompose: (items: ContextItem[]) => void;
		ondeletecompose: (items: ContextItem[]) => void;
		ondeletepermanentcompose: (items: ContextItem[]) => void;
		ondoctochat: () => void;
		ondocpublish: () => void;
		syncMode: SyncMode;
		onsenditemtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
	} = $props();
</script>

<div class="document-panel">
	<DocumentToolbar
		{viewMode}
		{docMode}
		{previewVisible}
		{onviewmode}
		{ontogglepreview}
		{oncompose}
		onsendtochat={ondoctochat}
		onpublish={ondocpublish}
	/>

	{#if docMode === 'reading' && publication?.title}
		<div class="doc-title">{publication.title}</div>
	{/if}

	<div class="doc-content">
		{#if docMode === 'empty'}
			<div class="doc-empty">
				<p>Select a publication from search results</p>
			</div>
		{:else if docMode === 'reading'}
			{#if loading}
				<div class="doc-empty"><p>Loading...</p></div>
			{:else if viewMode === 'outline'}
				<OutlineView {sections} />
			{:else if viewMode === 'continuous'}
				<ContinuousView {sections} />
			{:else}
				<PaginatedView {sections} {currentSection} {onnavigate} />
			{/if}
		{:else if docMode === 'compose'}
			<ComposeView
				{compose}
				onupdate={oncomposeupdate}
				oncancel={oncancelcompose}
				{onsendtochat}
				onpublish={onpublishcompose}
				ondelete={ondeletecompose}
				ondeletepermanent={ondeletepermanentcompose}
				{syncMode}
				{onsenditemtochat}
				{ontogglereadonly}
				{onlocksource}
				{oncrosspanelcopy}
			/>
		{/if}

		{#if previewVisible && publication}
			<JsonPreview data={publication} onclose={ontogglepreview} />
		{/if}
	</div>
</div>

<style>
	.document-panel {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.doc-title {
		padding: 10px 16px;
		font-size: 1.1rem;
		font-weight: 700;
		border-bottom: 1px solid var(--border);
	}

	.doc-content {
		flex: 1;
		position: relative;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.doc-empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
		font-size: 0.85rem;
	}
</style>
