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
	const isTool = $derived(fragment.role === 'tool');

	const thinkingText = $derived(
		fragment.blocks?.filter(b => b.type === 'thinking').map(b => b.thinking ?? '').join('\n') ?? ''
	);
	const toolUseBlocks = $derived(
		fragment.blocks?.filter(b => b.type === 'tool_use') ?? []
	);
	const toolResultBlocks = $derived(
		fragment.blocks?.filter(b => b.type === 'tool_result') ?? []
	);
	const toolResultText = $derived(
		toolResultBlocks.map(b => b.content ?? '').join('\n')
	);

	let outputOpen = $state(false);
</script>

{#if isTool}
	<div class="tool-row">
		<div class="tool-row-flex">
			<details class="chat-tool-block" style="flex:1; min-width:0;">
				<summary>{toolUseBlocks.map(b => b.name).join(', ') || 'tool'}</summary>
				{#each toolUseBlocks as tool}
					<div class="chat-tool-call">
						<span class="chat-tool-name">{tool.name}</span>
						<pre class="chat-tool-input">{JSON.stringify(tool.input, null, 2)}</pre>
					</div>
				{/each}
			</details>
			{#if toolResultText}
				<button
					class="output-toggle"
					class:active={outputOpen}
					title="Toggle tool output"
					onclick={() => outputOpen = !outputOpen}
				>&lbrace;&rbrace;</button>
			{/if}
		</div>
		{#if toolResultText && outputOpen}
			<div class="tool-output-panel">
				<div class="tool-output-header">
					<span>output</span>
					<button class="output-close" onclick={() => outputOpen = false}>&times;</button>
				</div>
				<pre class="tool-output-content">{toolResultText}</pre>
			</div>
		{/if}
	</div>
{:else}
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

			{#if thinkingText}
				<details class="chat-thinking">
					<summary>Thinking...</summary>
					<pre class="chat-thinking-content">{thinkingText}</pre>
				</details>
			{/if}

			{#if toolUseBlocks.length > 0}
				<div class="tool-row-flex">
					<details class="chat-tool-block" style="flex:1; min-width:0;">
						<summary>{toolUseBlocks.map(b => b.name).join(', ')}</summary>
						{#each toolUseBlocks as tool}
							<div class="chat-tool-call">
								<span class="chat-tool-name">{tool.name}</span>
								<pre class="chat-tool-input">{JSON.stringify(tool.input, null, 2)}</pre>
							</div>
						{/each}
					</details>
					{#if toolResultText}
						<button
							class="output-toggle"
							class:active={outputOpen}
							title="Toggle tool output"
							onclick={() => outputOpen = !outputOpen}
						>&lbrace;&rbrace;</button>
					{/if}
				</div>
				{#if toolResultText && outputOpen}
					<div class="tool-output-panel">
						<div class="tool-output-header">
							<span>output</span>
							<button class="output-close" onclick={() => outputOpen = false}>&times;</button>
						</div>
						<pre class="tool-output-content">{toolResultText}</pre>
					</div>
				{/if}
			{/if}

			{#if fragment.content}
				<pre class="content">{fragment.content}</pre>
			{/if}
		</div>
	</div>
{/if}

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
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.compose-badge {
		font-size: var(--t-3xs);
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
		font-size: var(--t-sm);
		line-height: 1.5;
		margin: 0;
	}

	/* Tool-only row (no message bubble) */
	.tool-row {
		padding: 2px 14px;
	}

	/* Thinking block inside message */
	.chat-thinking {
		margin: 4px 0;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		overflow: hidden;
	}

	.chat-thinking summary {
		padding: 4px 10px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		cursor: pointer;
		background: color-mix(in srgb, var(--bg-surface) 50%, transparent);
		user-select: none;
	}

	.chat-thinking-content {
		padding: 8px 10px;
		font-size: var(--t-2xs);
		line-height: 1.4;
		white-space: pre-wrap;
		word-wrap: break-word;
		margin: 0;
		font-family: inherit;
		color: var(--fg-muted);
		max-height: 300px;
		overflow-y: auto;
		border-top: 1px solid var(--border);
	}

	/* Tool use block */
	.chat-tool-block {
		margin: 4px 0;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		overflow: hidden;
	}

	.chat-tool-block summary {
		padding: 4px 10px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		cursor: pointer;
		background: color-mix(in srgb, var(--bg-surface) 50%, transparent);
		user-select: none;
		font-family: var(--font-mono);
	}

	.chat-tool-call {
		border-top: 1px solid var(--border);
	}

	.chat-tool-name {
		display: block;
		padding: 4px 10px 0;
		font-size: var(--t-3xs);
		font-weight: 600;
		font-family: var(--font-mono);
		color: var(--accent);
	}

	.chat-tool-input {
		padding: 4px 10px 8px;
		font-size: var(--t-3xs);
		line-height: 1.3;
		white-space: pre-wrap;
		word-wrap: break-word;
		margin: 0;
		font-family: var(--font-mono);
		color: var(--fg-muted);
		max-height: 200px;
		overflow-y: auto;
	}

	/* Tool output toggle and panel */

	.tool-row-flex {
		display: flex;
		align-items: flex-start;
		gap: 4px;
	}

	.output-toggle {
		flex-shrink: 0;
		padding: 3px 6px;
		font-size: var(--t-3xs);
		font-family: var(--font-mono);
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		cursor: pointer;
		line-height: 1;
		margin-top: 2px;
	}

	.output-toggle:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}

	.output-toggle.active {
		background: color-mix(in srgb, var(--accent) 15%, transparent);
		border-color: var(--accent);
		color: var(--accent);
	}

	.tool-output-panel {
		margin-top: 4px;
		border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
		border-radius: var(--radius);
		overflow: hidden;
	}

	.tool-output-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 3px 10px;
		font-size: var(--t-3xs);
		font-family: var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--fg-muted);
		background: color-mix(in srgb, var(--accent) 8%, var(--bg-surface));
		border-bottom: 1px solid var(--border);
	}

	.output-close {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-xs);
		padding: 0 2px;
		line-height: 1;
	}

	.output-close:hover {
		color: var(--fg);
	}

	.tool-output-content {
		padding: 8px 10px;
		font-size: var(--t-2xs);
		line-height: 1.4;
		white-space: pre-wrap;
		word-wrap: break-word;
		margin: 0;
		font-family: var(--font-mono);
		color: var(--fg);
		max-height: 400px;
		overflow-y: auto;
	}
</style>
