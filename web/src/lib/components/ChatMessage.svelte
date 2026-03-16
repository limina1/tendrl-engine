<script lang="ts">
	import type { Fragment } from '$lib/types';

	let {
		fragment,
		checked = false,
		ontoggle,
		inCompose = false,
		composeModified = false
	}: {
		fragment: Fragment;
		checked?: boolean;
		ontoggle?: (id: number) => void;
		inCompose?: boolean;
		composeModified?: boolean;
	} = $props();

	const isUser = $derived(fragment.role === 'user');
</script>

<div class="message-row">
	{#if ontoggle}
		<label class="msg-check">
			<input
				type="checkbox"
				{checked}
				onchange={() => ontoggle?.(fragment.id)}
			/>
		</label>
	{/if}
	<div class="message" class:user={isUser} class:assistant={!isUser} class:in-compose={inCompose} class:compose-modified={composeModified}>
		<div class="message-header">
			<span class="role">{fragment.role}</span>
			{#if inCompose}
				<span class="compose-badge" class:modified={composeModified}>compose{#if composeModified} ✎{/if}</span>
			{/if}
		</div>
		<pre class="content">{fragment.content}</pre>
	</div>
</div>

<style>
	.message-row {
		display: flex;
		align-items: flex-start;
		gap: 6px;
	}

	.msg-check {
		display: flex;
		align-items: center;
		padding-top: 10px;
		flex-shrink: 0;
	}

	.message {
		flex: 1;
		padding: 10px 14px;
		border-radius: var(--radius);
		max-width: 85%;
		word-wrap: break-word;
	}

	.user {
		align-self: flex-end;
		background: var(--user-bg);
		margin-left: auto;
	}

	.assistant {
		align-self: flex-start;
		background: var(--assistant-bg);
	}

	.in-compose {
		border: 1px solid var(--badge-synced);
	}

	.compose-modified {
		border-color: var(--badge-modified);
	}

	.message-header {
		display: flex;
		align-items: center;
		gap: 6px;
		margin-bottom: 4px;
	}

	.role {
		font-size: 0.75rem;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.compose-badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 4px;
		font-weight: 600;
		line-height: 1.6;
		background: color-mix(in srgb, var(--badge-synced) 20%, transparent);
		color: var(--badge-synced);
	}

	.compose-badge.modified {
		background: color-mix(in srgb, var(--badge-modified) 20%, transparent);
		color: var(--badge-modified);
	}

	.content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: 0.9rem;
		line-height: 1.5;
		margin: 0;
	}
</style>
