<script lang="ts">
	import { getProfile, onProfileUpdate } from '$lib/api';

	let { pubkey }: { pubkey: string } = $props();

	let name = $state<string | null>(null);

	function resolve(pk: string) {
		getProfile(pk).then(p => {
			if (p.found) {
				name = p.display_name || p.name;
			}
		}).catch(() => {});
	}

	$effect(() => {
		const pk = pubkey;
		name = null;
		resolve(pk);

		// Re-resolve when batch prefetch completes
		const unsub = onProfileUpdate(() => {
			if (!name) resolve(pk);
		});

		return unsub;
	});
</script>

{#if name}
	<span class="profile-name" title={pubkey}>{name}</span>
{:else}
	<span class="profile-pubkey">{pubkey.slice(0, 12)}...</span>
{/if}

<style>
	.profile-name {
		font-weight: 500;
	}

	.profile-pubkey {
		font-family: var(--font-mono);
		opacity: 0.7;
	}
</style>
