<script lang="ts">
	// Search-result row for kind-0 (profile metadata) events. A kind-0
	// hit isn't a document — it's an *author match* — so instead of the
	// generic "[Untitled] / raw-JSON preview" row it renders the profile
	// itself: avatar, names, about, a "View profile" action, and a
	// fold-out raw-JSON inspector underneath.

	import type { SearchResult } from '$lib/types';
	import { getProfile, getEvent, onProfileUpdate, type Profile } from '$lib/api';

	let {
		result,
		checked = false,
		ontogglecheck,
		onviewprofile,
		onignorepubkey,
		localPubkeys = new Set<string>()
	}: {
		result: SearchResult;
		checked?: boolean;
		ontogglecheck?: () => void;
		onviewprofile?: (pubkey: string) => void;
		onignorepubkey?: (result: SearchResult) => void;
		localPubkeys?: Set<string>;
	} = $props();

	// For a kind-0 event the author *is* the profile's pubkey.
	let profile = $state<Profile | null>(null);
	let jsonOpen = $state(false);
	let rawEvent = $state<unknown>(null);
	let jsonLoading = $state(false);

	$effect(() => {
		const pk = result.author;
		profile = null;
		getProfile(pk)
			.then((p) => {
				if (p.found) profile = p;
			})
			.catch(() => {});
		// Re-read if the profile lands in cache after this mount.
		return onProfileUpdate(() => {
			if (!profile) getProfile(pk).then((p) => p.found && (profile = p)).catch(() => {});
		});
	});

	const displayName = $derived(
		profile?.display_name || profile?.name || `${result.author.slice(0, 12)}…`
	);

	async function toggleJson() {
		jsonOpen = !jsonOpen;
		if (jsonOpen && rawEvent === null && !jsonLoading) {
			jsonLoading = true;
			try {
				const r = await getEvent(result.event_id);
				rawEvent = r.event;
			} catch (e) {
				rawEvent = { error: e instanceof Error ? e.message : String(e) };
			} finally {
				jsonLoading = false;
			}
		}
	}
</script>

<div class="profile-result">
	<div class="pr-main">
		{#if ontogglecheck}
			<label class="pr-check" onclick={(e) => e.stopPropagation()}>
				<input type="checkbox" {checked} onchange={ontogglecheck} />
			</label>
		{/if}

		{#if profile?.picture}
			<img class="pr-avatar" src={profile.picture} alt="" />
		{:else}
			<div class="pr-avatar pr-avatar--placeholder">{displayName.slice(0, 1)}</div>
		{/if}

		<div class="pr-identity">
			<div class="pr-name-row">
				<span class="pr-name">{displayName}</span>
				<span class="pr-kind">profile</span>
				{#if localPubkeys.has(result.author)}
					<span class="pr-local">local</span>
				{/if}
			</div>
			{#if profile?.name || profile?.nip05}
				<div class="pr-sub">
					{#if profile?.name}<span class="pr-handle">@{profile.name}</span>{/if}
					{#if profile?.nip05}<span class="pr-nip05">{profile.nip05}</span>{/if}
				</div>
			{/if}
			{#if profile?.about}
				<p class="pr-about">{profile.about}</p>
			{/if}
		</div>
	</div>

	<div class="pr-actions">
		<button
			class="pr-btn pr-btn--primary"
			onclick={(e) => {
				e.stopPropagation();
				onviewprofile?.(result.author);
			}}
		>
			View profile
		</button>
		<button class="pr-btn" onclick={(e) => { e.stopPropagation(); toggleJson(); }}>
			<span class="pr-arrow" class:open={jsonOpen}>{jsonOpen ? '▾' : '▸'}</span> json
		</button>
		{#if onignorepubkey}
			<button
				class="pr-btn pr-btn--danger"
				onclick={(e) => {
					e.stopPropagation();
					onignorepubkey?.(result);
				}}
				title="Hide this author"
			>
				Hide
			</button>
		{/if}
	</div>

	{#if jsonOpen}
		<div class="pr-json">
			{#if jsonLoading}
				<span class="pr-json-loading">Loading event…</span>
			{:else}
				<pre>{JSON.stringify(rawEvent, null, 2)}</pre>
			{/if}
		</div>
	{/if}
</div>

<style>
	.profile-result {
		padding: 10px 12px;
		border-bottom: 1px solid var(--border);
		border-right: 3px solid #a093c7;
	}

	.pr-main {
		display: flex;
		align-items: flex-start;
		gap: 10px;
	}

	.pr-check {
		display: flex;
		align-items: center;
		flex-shrink: 0;
		padding-top: 2px;
	}

	.pr-avatar {
		width: 40px;
		height: 40px;
		border-radius: 50%;
		object-fit: cover;
		flex-shrink: 0;
	}

	.pr-avatar--placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		background: var(--bg-surface);
		border: 1px solid var(--border);
		color: var(--fg-muted);
		font-size: var(--t-md);
		text-transform: uppercase;
	}

	.pr-identity {
		flex: 1;
		min-width: 0;
	}

	.pr-name-row {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.pr-name {
		font-size: var(--t-xs);
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.pr-kind {
		font-size: var(--t-3xs);
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.pr-local {
		font-size: var(--t-3xs);
		padding: 0 5px;
		border-radius: 3px;
		background: #f9731633;
		color: #f97316;
		font-weight: 600;
	}

	.pr-sub {
		display: flex;
		gap: 8px;
		font-size: var(--t-3xs);
		margin-top: 1px;
	}

	.pr-handle {
		color: var(--fg-muted);
	}

	.pr-nip05 {
		color: var(--accent);
	}

	.pr-about {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 4px 0 0;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.pr-actions {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-top: 8px;
		padding-left: 50px;
	}

	.pr-btn {
		font-size: var(--t-3xs);
		padding: 3px 10px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		border: none;
		cursor: pointer;
	}

	.pr-btn:hover {
		color: var(--fg);
	}

	.pr-btn--primary {
		background: none;
		border: 1px solid var(--accent);
		color: var(--accent);
	}

	.pr-btn--primary:hover {
		background: var(--accent);
		color: white;
	}

	.pr-btn--danger {
		margin-left: auto;
		color: #ef4444;
	}

	.pr-btn--danger:hover {
		background: #ef444415;
		color: #ef4444;
	}

	.pr-arrow {
		font-size: var(--t-3xs);
	}

	.pr-json {
		margin: 8px 0 0 50px;
		border: 1px solid var(--border);
		border-radius: 4px;
		background: var(--bg-surface);
		max-height: 240px;
		overflow: auto;
	}

	.pr-json pre {
		margin: 0;
		padding: 8px 10px;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		white-space: pre-wrap;
		word-break: break-word;
	}

	.pr-json-loading {
		display: block;
		padding: 8px 10px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
	}
</style>
