<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { getAppState } from '$lib/state.svelte';
	import { getProfile, onProfileUpdate, type Profile } from '$lib/api';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import ProfileModal from '$lib/components/ProfileModal.svelte';
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import JsonPreview from '$lib/components/JsonPreview.svelte';

	const app = getAppState();

	let authorProfile = $state<Profile | null>(null);
	let showAuthorModal = $state(false);

	// Load publication when route params change
	$effect(() => {
		const params = $page.params;
		if (!params) return;
		const { pubkey, d_tag } = params;
		if (pubkey && d_tag) {
			app.openPublication(pubkey, d_tag);
		}
	});

	// Resolve author profile when publication changes
	$effect(() => {
		const pk = app.publication?.author_pubkey;
		authorProfile = null;
		if (!pk) return;

		function resolve() {
			getProfile(pk!).then(p => {
				if (p.found) authorProfile = p;
			}).catch(() => {});
		}
		resolve();

		const unsub = onProfileUpdate(() => {
			if (!authorProfile) resolve();
		});

		return unsub;
	});

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}
</script>

<svelte:head>
	<title>{app.publication?.title ?? 'Publication'} - tendrl</title>
</svelte:head>

<div class="document-panel">
	<div class="doc-toolbar">
		<div class="doc-toolbar-left">
			<button class:active={app.viewMode === 'outline'} onclick={() => app.handleViewMode('outline')}>Outline</button>
			<button class:active={app.viewMode === 'continuous'} onclick={() => app.handleViewMode('continuous')}>Continuous</button>
			<button class:active={app.viewMode === 'paginated'} onclick={() => app.handleViewMode('paginated')}>Paginated</button>
		</div>
		<div class="doc-toolbar-right">
			<button class="icon-btn" onclick={app.handleDocToChat} title="Copy to chat">◂</button>
			<button class="icon-btn" onclick={app.handleDocPublish} title="Publish locally">▸</button>
			<button class:active={app.previewVisible} onclick={app.handleTogglePreview}>JSON</button>
			<button onclick={() => app.handleCompose()}>Compose</button>
		</div>
	</div>

	{#if app.publication}
		{#if app.publication.title}
			<div class="doc-title">{app.publication.title}</div>
		{/if}
		<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
		<div class="doc-author-bar" onclick={() => { if (authorProfile) showAuthorModal = true; }}>
			{#if authorProfile?.picture}
				<img class="doc-author-avatar" src={authorProfile.picture} alt="" />
			{:else}
				<div class="doc-author-avatar placeholder">?</div>
			{/if}
			<span class="doc-author-name">
				<ProfileName pubkey={app.publication.author_pubkey} onviewprofile={(pk) => goto(`/profile/${pk}`)} />
			</span>
			<span class="doc-author-time">{formatTime(app.publication.created_at)}</span>
		</div>
	{/if}

	{#if showAuthorModal && authorProfile}
		<ProfileModal profile={authorProfile} onclose={() => showAuthorModal = false} onviewprofile={(pk) => goto(`/profile/${pk}`)} />
	{/if}

	<div class="doc-content">
		{#if app.docLoading}
			<div class="doc-empty"><p>Loading...</p></div>
		{:else if app.viewMode === 'outline'}
			<OutlineView sections={app.sections} onload={app.handleLoadSection} onselect={(index) => { app.handleLoadSection(index); app.handleViewMode('paginated'); app.handleNavigate(index); }} />
		{:else if app.viewMode === 'continuous'}
			<ContinuousView sections={app.sections} publication={app.publication ? { title: app.publication.title, summary: app.publication.summary } : null} onload={app.handleLoadSection} />
		{:else}
			<PaginatedView sections={app.sections} currentSection={app.currentSection} onnavigate={app.handleNavigate} onload={app.handleLoadSection} />
		{/if}

		{#if app.previewVisible && app.publication}
			<JsonPreview data={app.publication} onclose={app.handleTogglePreview} />
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

	.doc-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
		gap: 8px;
	}

	.doc-toolbar-left, .doc-toolbar-right { display: flex; gap: 6px; }
	.active { background: var(--accent); color: white; border-color: var(--accent); }
	.icon-btn { padding: 4px 8px; font-size: 0.85rem; min-width: 28px; }

	.doc-title {
		padding: 10px 16px;
		font-size: 1.1rem;
		font-weight: 700;
		border-bottom: 1px solid var(--border);
	}

	.doc-author-bar {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 16px;
		border-bottom: 1px solid var(--border);
		cursor: pointer;
	}
	.doc-author-bar:hover { background: var(--bg-surface); }

	.doc-author-avatar {
		width: 28px;
		height: 28px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}
	.doc-author-avatar.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: 0.75rem;
	}
	.doc-author-name { font-size: 0.85rem; }
	.doc-author-time { font-size: 0.75rem; color: var(--fg-muted); margin-left: auto; }

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
