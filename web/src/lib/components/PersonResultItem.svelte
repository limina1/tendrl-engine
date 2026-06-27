<script lang="ts">
	// One row of the search "People" category — a kind-0 author match.
	// Unlike ProfileResultItem (which adapts a generic kind-0 SearchResult
	// and fetches the profile), a ProfileResult already carries its
	// display fields inline, so this row renders straight from props.

	import type { ProfileResult } from '$lib/types';

	let {
		profile,
		onviewprofile,
		localPubkeys = new Set<string>()
	}: {
		profile: ProfileResult;
		onviewprofile?: (pubkey: string) => void;
		localPubkeys?: Set<string>;
	} = $props();

	const displayName = $derived(
		profile.display_name || profile.name || `${profile.pubkey.slice(0, 12)}…`
	);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	class="person"
	onclick={() => onviewprofile?.(profile.pubkey)}
	onkeydown={(e) => e.key === 'Enter' && onviewprofile?.(profile.pubkey)}
	role="button"
	tabindex="0"
>
	{#if profile.picture}
		<img class="person-avatar" src={profile.picture} alt="" />
	{:else}
		<div class="person-avatar person-avatar--placeholder">{displayName.slice(0, 1)}</div>
	{/if}

	<div class="person-identity">
		<div class="person-name-row">
			<span class="person-name">{displayName}</span>
			{#if profile.name && profile.display_name}
				<span class="person-handle">@{profile.name}</span>
			{/if}
			{#if localPubkeys.has(profile.pubkey)}
				<span class="person-local">local</span>
			{/if}
		</div>
		{#if profile.nip05}
			<div class="person-nip05">{profile.nip05}</div>
		{/if}
		{#if profile.about}
			<p class="person-about">{profile.about}</p>
		{/if}
	</div>

	<button
		class="person-view"
		onclick={(e) => {
			e.stopPropagation();
			onviewprofile?.(profile.pubkey);
		}}
	>
		View profile
	</button>
</div>

<style>
	.person {
		display: flex;
		align-items: flex-start;
		gap: 10px;
		padding: 9px 12px;
		border-bottom: 1px solid var(--border);
		border-right: 3px solid #a093c7;
		cursor: pointer;
		text-align: left;
	}

	.person:hover {
		background: color-mix(in srgb, #a093c7 8%, transparent);
	}

	.person-avatar {
		width: 36px;
		height: 36px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.person-avatar--placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: var(--t-base);
		text-transform: uppercase;
	}

	.person-identity {
		flex: 1;
		min-width: 0;
	}

	.person-name-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.person-name {
		font-size: var(--t-xs);
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.person-handle {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
	}

	.person-local {
		font-size: var(--t-3xs);
		padding: 0 5px;
		border-radius: 3px;
		background: #f9731633;
		color: #f97316;
		font-weight: 600;
	}

	.person-nip05 {
		font-size: var(--t-3xs);
		color: var(--accent);
		margin-top: 1px;
	}

	.person-about {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 3px 0 0;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.person-view {
		flex-shrink: 0;
		font-size: var(--t-3xs);
		padding: 3px 10px;
		border-radius: 4px;
		background: none;
		border: 1px solid var(--accent);
		color: var(--accent);
		cursor: pointer;
		align-self: center;
	}

	.person-view:hover {
		background: var(--accent);
		color: white;
	}
</style>
