<script lang="ts">
	// Canonical inline display for a user pubkey, with two states:
	//
	//   unresolved — no kind-0 cached yet: renders the truncated pubkey
	//     as one big click target; pressing it force-fetches the kind-0
	//     from relays (bypassing offline mode — an explicit user okay).
	//   resolved — kind-0 known: renders the display name; clicking it
	//     opens the profile modal (kind-0 detail + "view profile").
	import { getProfile, refreshProfiles, onProfileUpdate, type Profile } from '$lib/api';
	import ProfileModal from './ProfileModal.svelte';

	let { pubkey, onviewprofile }: { pubkey: string; onviewprofile?: (pubkey: string) => void } =
		$props();

	let profile = $state<Profile | null>(null);
	let name = $state<string | null>(null);
	let refreshing = $state(false);
	let showModal = $state(false);

	function resolve(pk: string) {
		getProfile(pk)
			.then((p) => {
				if (p.found) {
					profile = p;
					name = p.display_name || p.name || null;
				}
			})
			.catch(() => {});
	}

	$effect(() => {
		const pk = pubkey;
		name = null;
		profile = null;
		showModal = false;
		resolve(pk);

		const unsub = onProfileUpdate(() => {
			if (!name) resolve(pk);
		});

		return unsub;
	});

	function openModal(e: MouseEvent) {
		e.stopPropagation();
		if (profile) showModal = true;
	}

	// Force-fetch this pubkey's kind-0 from relays, then re-resolve. The
	// force fetch is an explicit user action, so the engine lets it
	// reach relays even in offline mode.
	async function handleRefresh(e: MouseEvent) {
		e.stopPropagation();
		if (refreshing) return;
		refreshing = true;
		try {
			await refreshProfiles([pubkey]);
			resolve(pubkey);
		} finally {
			refreshing = false;
		}
	}
</script>

{#if name}
	<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
	<span class="profile-name" title={pubkey} onclick={openModal}>{name}</span>
{:else}
	<button
		class="profile-ref"
		onclick={handleRefresh}
		disabled={refreshing}
		title={refreshing ? 'Refreshing…' : `Fetch ${pubkey.slice(0, 12)}… from relays`}
	>
		<span class="profile-pubkey">{pubkey.slice(0, 12)}…</span>
		<span class="profile-refresh" class:spinning={refreshing} aria-hidden="true">⟳</span>
	</button>
{/if}

{#if showModal && profile}
	<ProfileModal {profile} onclose={() => (showModal = false)} {onviewprofile} />
{/if}

<style>
	.profile-name {
		font-weight: 500;
		cursor: pointer;
	}

	.profile-name:hover {
		text-decoration: underline;
	}

	/* Unresolved: the whole pill is one easy refresh target. */
	.profile-ref {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		vertical-align: baseline;
		padding: 1px 6px;
		border: none;
		border-radius: var(--radius, 4px);
		background: none;
		font: inherit;
		color: inherit;
		cursor: pointer;
	}

	.profile-ref:hover {
		background: color-mix(in srgb, var(--accent) 13%, transparent);
	}

	.profile-ref:disabled {
		cursor: default;
	}

	.profile-pubkey {
		font-family: var(--font-mono);
		opacity: 0.75;
	}

	.profile-refresh {
		flex-shrink: 0;
		font-size: 1.15em;
		line-height: 1;
		color: var(--fg-muted);
	}

	.profile-ref:hover .profile-refresh {
		color: var(--accent);
	}

	.profile-refresh.spinning {
		display: inline-block;
		color: var(--accent);
		animation: profile-refresh-spin 0.7s linear infinite;
	}

	@keyframes profile-refresh-spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
