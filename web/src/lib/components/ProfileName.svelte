<script lang="ts">
	import { getProfile } from '$lib/api';

	let { pubkey }: { pubkey: string } = $props();

	let name = $state<string | null>(null);

	$effect(() => {
		const pk = pubkey;
		name = null;
		getProfile(pk).then(p => {
			if (p.found) {
				name = p.display_name || p.name;
			}
		}).catch(() => {});
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
