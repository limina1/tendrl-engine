<script lang="ts">
	import type { Profile } from '$lib/api';

	let { profile, onclose, onviewprofile }: { profile: Profile; onclose: () => void; onviewprofile?: (pubkey: string) => void } = $props();

	const npub = $derived(profile.pubkey.slice(0, 16) + '...');

	function handleBackdrop(e: MouseEvent) {
		e.stopPropagation();
		if (e.target === e.currentTarget) onclose();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') onclose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="modal-backdrop" onclick={handleBackdrop}>
	<div class="modal-card">
		<button class="modal-close" onclick={onclose}>✕</button>

		<div class="profile-header">
			{#if profile.picture}
				<img class="profile-avatar" src={profile.picture} alt="" />
			{:else}
				<div class="profile-avatar placeholder">?</div>
			{/if}

			<div class="profile-identity">
				{#if profile.display_name}
					<div class="display-name">{profile.display_name}</div>
				{/if}
				{#if profile.name}
					<div class="username">@{profile.name}</div>
				{/if}
				{#if profile.nip05}
					<div class="nip05">{profile.nip05}</div>
				{/if}
			</div>
		</div>

		{#if profile.about}
			<div class="profile-about">{profile.about}</div>
		{/if}

		{#if onviewprofile}
			<div class="profile-actions">
				<button class="view-profile-btn" onclick={(e) => { e.stopPropagation(); onviewprofile?.(profile.pubkey); onclose(); }}>View profile</button>
			</div>
		{/if}

		<div class="profile-pubkey">
			<span class="pubkey-label">Pubkey</span>
			<code class="pubkey-value">{profile.pubkey}</code>
		</div>
	</div>
</div>

<style>
	.modal-backdrop {
		position: fixed;
		/* Leave the modeline visible for history pill + network pill access. */
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 100;
		display: flex;
		align-items: center;
		justify-content: center;
		background: rgba(0, 0, 0, 0.5);
	}

	.modal-card {
		position: relative;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: 8px;
		padding: 24px;
		max-width: 420px;
		width: 90%;
		max-height: 80vh;
		overflow-y: auto;
	}

	.modal-close {
		position: absolute;
		top: 8px;
		right: 12px;
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: 1.1rem;
		cursor: pointer;
		padding: 4px 8px;
	}

	.modal-close:hover {
		color: var(--fg);
	}

	.profile-header {
		display: flex;
		gap: 16px;
		align-items: center;
		margin-bottom: 16px;
	}

	.profile-avatar {
		width: 64px;
		height: 64px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.profile-avatar.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: 1.5rem;
	}

	.profile-identity {
		min-width: 0;
	}

	.display-name {
		font-size: 1.15rem;
		font-weight: 600;
	}

	.username {
		color: var(--fg-muted);
		font-size: 0.9rem;
	}

	.nip05 {
		color: var(--accent);
		font-size: 0.85rem;
		margin-top: 2px;
	}

	.profile-about {
		margin-bottom: 16px;
		font-size: 0.9rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-word;
	}

	.profile-actions {
		margin-bottom: 12px;
	}

	.view-profile-btn {
		width: 100%;
		padding: 8px;
		font-size: 0.85rem;
		background: none;
		border: 1px solid var(--accent);
		border-radius: var(--radius);
		color: var(--accent);
		cursor: pointer;
	}

	.view-profile-btn:hover {
		background: var(--accent);
		color: white;
	}

	.profile-pubkey {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding-top: 12px;
		border-top: 1px solid var(--border);
	}

	.pubkey-label {
		font-size: 0.75rem;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.pubkey-value {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		word-break: break-all;
		color: var(--fg-muted);
	}
</style>
