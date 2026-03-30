<script lang="ts">
	import { getProfile, onProfileUpdate, type Profile } from '$lib/api';
	import ProfileModal from './ProfileModal.svelte';

	let { pubkey }: { pubkey: string } = $props();

	let profile = $state<Profile | null>(null);
	let name = $state<string | null>(null);
	let showModal = $state(false);

	function resolve(pk: string) {
		getProfile(pk).then(p => {
			if (p.found) {
				profile = p;
				name = p.display_name || p.name;
			}
		}).catch(() => {});
	}

	$effect(() => {
		const pk = pubkey;
		name = null;
		profile = null;
		resolve(pk);

		const unsub = onProfileUpdate(() => {
			if (!name) resolve(pk);
		});

		return unsub;
	});

	function handleClick(e: MouseEvent) {
		e.stopPropagation();
		if (profile) showModal = true;
	}
</script>

{#if name}
	<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
	<span class="profile-name clickable" title={pubkey} onclick={handleClick}>{name}</span>
{:else}
	<span class="profile-pubkey">{pubkey.slice(0, 12)}...</span>
{/if}

{#if showModal && profile}
	<ProfileModal {profile} onclose={() => showModal = false} />
{/if}

<style>
	.profile-name {
		font-weight: 500;
	}

	.profile-name.clickable {
		cursor: pointer;
	}

	.profile-name.clickable:hover {
		text-decoration: underline;
	}

	.profile-pubkey {
		font-family: var(--font-mono);
		opacity: 0.7;
	}
</style>
