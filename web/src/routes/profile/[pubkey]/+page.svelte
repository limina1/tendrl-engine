<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { getAppState } from '$lib/state.svelte';
	import ProfileView from '$lib/components/ProfileView.svelte';

	const app = getAppState();

	const pubkey = $derived($page.params?.pubkey);
</script>

<svelte:head>
	<title>Profile - tendrl</title>
</svelte:head>

<div class="document-panel">
	<div class="doc-toolbar">
		<div class="doc-toolbar-left"></div>
		<div class="doc-toolbar-right">
			<button onclick={() => app.handleCompose()}>Compose</button>
		</div>
	</div>

	<div class="doc-content">
		{#if pubkey}
			<ProfileView
				{pubkey}
				onopenpub={(pub) => goto(`/p/${pub.addr.pubkey}/${pub.addr.d_tag}`)}
				onback={() => goto('/')}
			/>
		{/if}
	</div>
</div>

<style>
	.document-panel {
		flex: 1; display: flex; flex-direction: column; min-height: 0; overflow: hidden;
	}
	.doc-toolbar {
		display: flex; align-items: center; justify-content: space-between;
		padding: 8px 12px; border-bottom: 1px solid var(--border); background: var(--bg-surface); gap: 8px;
	}
	.doc-toolbar-left, .doc-toolbar-right { display: flex; gap: 6px; }
	.doc-content { flex: 1; position: relative; display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
</style>
