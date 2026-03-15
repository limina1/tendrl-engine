<script lang="ts">
	import type { ChatResponse } from '$lib/types';
	import ChatLog from './ChatLog.svelte';
	import ChatInput from './ChatInput.svelte';
	import EditView from './EditView.svelte';
	import SystemPrompt from './SystemPrompt.svelte';
	import ContextPanel from './ContextPanel.svelte';

	let {
		chat,
		loading = false,
		systemExpanded = false,
		contextExpanded = false,
		ontogglesystem,
		ontogglecontext,
		onsend,
		onreset,
		onedit,
		onapplyedit,
		oncanceledit,
		onsetsystem,
		oninjectcontext
	}: {
		chat: ChatResponse | null;
		loading?: boolean;
		systemExpanded?: boolean;
		contextExpanded?: boolean;
		ontogglesystem: () => void;
		ontogglecontext: () => void;
		onsend: (content: string) => void;
		onreset: () => void;
		onedit: () => void;
		onapplyedit: (buffer: string) => void;
		oncanceledit: () => void;
		onsetsystem: (prompt: string) => void;
		oninjectcontext: (title: string, content: string) => void;
	} = $props();
</script>

<div class="chat-panel">
	<div class="chat-toolbar">
		<button onclick={ontogglesystem} class:active={systemExpanded} disabled={loading}>
			System
		</button>
		<button onclick={ontogglecontext} class:active={contextExpanded} disabled={loading}>
			Context
		</button>
		<button onclick={onedit} disabled={loading || (chat?.edit_mode ?? false)}>Edit</button>
		<button onclick={onreset} disabled={loading}>Reset</button>
	</div>

	{#if systemExpanded}
		<SystemPrompt
			currentPrompt={chat?.system_prompt ?? null}
			onset={onsetsystem}
			disabled={loading}
		/>
	{/if}

	{#if contextExpanded}
		<ContextPanel
			contextCount={chat?.context_count ?? 0}
			oninject={oninjectcontext}
			disabled={loading}
		/>
	{/if}

	{#if chat}
		{#if chat.edit_mode && chat.edit_buffer != null}
			<EditView
				editBuffer={chat.edit_buffer}
				onapply={onapplyedit}
				oncancel={oncanceledit}
			/>
		{:else}
			<ChatLog fragments={chat.fragments} />
		{/if}
	{:else}
		<div class="chat-empty">Loading...</div>
	{/if}

	{#if !chat?.edit_mode}
		<ChatInput {onsend} disabled={loading || !chat} />
	{/if}
</div>

<style>
	.chat-panel {
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.chat-toolbar {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		padding: 8px 12px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.chat-empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
