<script lang="ts">
	import type { Fragment } from '$lib/types';

	let {
		fragment,
		checked = false,
		ontoggle
	}: {
		fragment: Fragment;
		checked?: boolean;
		ontoggle?: (id: number) => void;
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
	<div class="message" class:user={isUser} class:assistant={!isUser}>
		<span class="role">{fragment.role}</span>
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

	.role {
		display: block;
		font-size: 0.75rem;
		color: var(--fg-muted);
		margin-bottom: 4px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: 0.9rem;
		line-height: 1.5;
		margin: 0;
	}
</style>
