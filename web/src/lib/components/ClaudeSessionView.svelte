<script lang="ts">
	import type { ClaudeSessionSummary, ClaudeSessionMessage } from '$lib/types';

	let {
		sessions = [],
		selectedSession = null,
		loading = false,
		onselect,
		onback,
		onload
	}: {
		sessions: ClaudeSessionSummary[];
		selectedSession: { id: string; messages: ClaudeSessionMessage[]; count: number } | null;
		loading: boolean;
		onselect?: (id: string) => void;
		onback?: () => void;
		onload?: () => void;
	} = $props();

	function textContent(msg: ClaudeSessionMessage): string {
		return msg.blocks
			.filter(b => b.type === 'text')
			.map(b => b.text ?? '')
			.join('\n');
	}

	function hasThinking(msg: ClaudeSessionMessage): boolean {
		return msg.blocks.some(b => b.type === 'thinking');
	}

	function thinkingContent(msg: ClaudeSessionMessage): string {
		return msg.blocks
			.filter(b => b.type === 'thinking')
			.map(b => b.thinking ?? '')
			.join('\n');
	}

	function hasToolUse(msg: ClaudeSessionMessage): boolean {
		return msg.blocks.some(b => b.type === 'tool_use');
	}

	function toolBlocks(msg: ClaudeSessionMessage) {
		return msg.blocks.filter(b => b.type === 'tool_use');
	}

	function hasToolResult(msg: ClaudeSessionMessage): boolean {
		return msg.blocks.some(b => b.type === 'tool_result');
	}

	function toolResultContent(msg: ClaudeSessionMessage): string {
		return msg.blocks
			.filter(b => b.type === 'tool_result')
			.map(b => b.content ?? '')
			.join('\n');
	}

	// Track which tool output panels are open (by message index)
	let openOutputs: Set<number> = $state(new Set());

	function toggleOutput(idx: number) {
		const next = new Set(openOutputs);
		if (next.has(idx)) next.delete(idx);
		else next.add(idx);
		openOutputs = next;
	}

	// Find the tool_result message that follows a tool_use message.
	// Skips over other tool_use messages (parallel calls) to find the matching result.
	function findResultForTool(messages: ClaudeSessionMessage[], toolIdx: number): string | null {
		// Count how many tool_use messages precede this one in the current batch
		let batchOffset = 0;
		for (let j = toolIdx - 1; j >= 0; j--) {
			if (hasToolUse(messages[j]) && !hasToolResult(messages[j])) batchOffset++;
			else break;
		}

		// Look ahead, skipping tool_use messages, collecting tool_results
		let resultsSeen = 0;
		for (let i = toolIdx + 1; i < messages.length; i++) {
			const msg = messages[i];
			if (hasToolUse(msg)) continue; // skip parallel tool calls
			if (hasToolResult(msg)) {
				if (resultsSeen === batchOffset) return toolResultContent(msg);
				resultsSeen++;
				continue;
			}
			// Stop at text/thinking messages
			break;
		}
		return null;
	}
</script>

<div class="claude-sessions">
	{#if loading}
		<div class="sessions-empty"><p>Loading...</p></div>
	{:else if selectedSession}
		<div class="session-header">
			<button class="back-btn" onclick={onback}>&larr; Back</button>
			<span class="session-id">{selectedSession.id.slice(0, 8)}</span>
			<span class="session-count">{selectedSession.count} messages</span>
			{#if onload}
				<button class="load-btn" onclick={onload}>Load to chat</button>
			{/if}
		</div>
		<div class="session-messages">
			{#each selectedSession.messages as msg, i}
				{@const text = textContent(msg)}
				{@const thinking = thinkingContent(msg)}
				{@const tools = toolBlocks(msg)}
				{@const hasText = text.length > 0}
				{@const hasThink = thinking.length > 0}
				{@const hasTool = tools.length > 0}
				{@const hasResult = hasToolResult(msg)}
				{#if hasText || hasThink}
					<div class="message" class:user={msg.role === 'user'} class:assistant={msg.role === 'assistant'}>
						<div class="message-role">{msg.role}</div>

						{#if hasThink}
							<details class="thinking-block">
								<summary>Thinking...</summary>
								<pre class="thinking-content">{thinking}</pre>
							</details>
						{/if}

						{#if hasText}
							<pre class="message-content">{text}</pre>
						{/if}
					</div>
				{:else if hasTool}
					{@const result = findResultForTool(selectedSession.messages, i)}
					<div class="tool-message">
						<div class="tool-row-flex">
							<details class="tool-block" style="flex:1; min-width:0;">
								<summary>{tools.map(b => b.name).join(', ')}</summary>
								{#each tools as tool}
									<div class="tool-call">
										<span class="tool-name">{tool.name}</span>
										<pre class="tool-input">{JSON.stringify(tool.input, null, 2)}</pre>
									</div>
								{/each}
							</details>
							{#if result}
								<button
									class="output-toggle"
									class:active={openOutputs.has(i)}
									title="Toggle tool output"
									onclick={() => toggleOutput(i)}
								>&lbrace;&rbrace;</button>
							{/if}
						</div>
						{#if result && openOutputs.has(i)}
							<div class="tool-output-panel">
								<div class="tool-output-header">
									<span>output</span>
									<button class="output-close" onclick={() => toggleOutput(i)}>&times;</button>
								</div>
								<pre class="tool-output-content">{result}</pre>
							</div>
						{/if}
					</div>
				{:else if hasResult}
					<!-- tool results consumed by preceding tool_use row -->
				{/if}
			{/each}
		</div>
	{:else if sessions.length > 0}
		<div class="sessions-header">
			<span>Claude Code Sessions ({sessions.length})</span>
		</div>
		<div class="sessions-list">
			{#each sessions as session (session.id)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div
					class="session-item"
					onclick={() => onselect?.(session.id)}
					onkeydown={(e) => { if (e.key === 'Enter') onselect?.(session.id); }}
					role="button"
					tabindex="0"
				>
					<div class="session-item-header">
						<span class="session-item-id">{session.id.slice(0, 8)}</span>
						<span class="session-item-meta">{session.message_count} msgs</span>
					</div>
					<p class="session-item-prompt">{session.last_message || session.first_prompt}</p>
					<div class="session-item-footer">
						<span class="session-item-date">{session.date}</span>
					</div>
				</div>
			{/each}
		</div>
	{:else}
		<div class="sessions-empty"><p>No Claude Code sessions found</p></div>
	{/if}
</div>

<style>
	.claude-sessions {
		flex: 1;
		display: flex;
		flex-direction: column;
		min-height: 0;
		overflow: hidden;
	}

	.sessions-empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--fg-muted);
		font-size: var(--t-xs);
	}

	/* Session list */

	.sessions-header {
		padding: 10px 16px;
		font-size: var(--t-2xs);
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--border);
	}

	.sessions-list {
		flex: 1;
		overflow-y: auto;
	}

	.session-item {
		padding: 10px 16px;
		border-bottom: 1px solid var(--border);
		cursor: pointer;
		border-left: 3px solid var(--accent);
	}

	.session-item:hover {
		background: var(--bg-surface);
	}

	.session-item-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin-bottom: 2px;
	}

	.session-item-id {
		font-size: var(--t-2xs);
		font-weight: 600;
		font-family: var(--font-mono);
	}

	.session-item-meta {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		white-space: nowrap;
	}

	.session-item-prompt {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		line-height: 1.4;
		margin: 2px 0;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		-webkit-box-orient: vertical;
	}

	.session-item-footer {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		margin-top: 4px;
	}

	/* Session detail */

	.session-header {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 16px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.back-btn {
		font-size: var(--t-2xs);
		padding: 2px 8px;
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		cursor: pointer;
		border-radius: var(--radius);
	}

	.back-btn:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}

	.session-id {
		font-size: var(--t-2xs);
		font-family: var(--font-mono);
		color: var(--fg-muted);
	}

	.session-count {
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		flex: 1;
		text-align: right;
	}

	.load-btn {
		font-size: var(--t-3xs);
		padding: 2px 10px;
		background: var(--accent);
		color: white;
		border: 1px solid var(--accent);
		cursor: pointer;
		border-radius: var(--radius);
		white-space: nowrap;
	}

	.load-btn:hover {
		opacity: 0.9;
	}

	.session-messages {
		flex: 1;
		overflow-y: auto;
		padding: 8px 0;
	}

	.message {
		padding: 8px 16px;
		border-bottom: 1px solid var(--border);
	}

	.message.user {
		background: color-mix(in srgb, var(--accent) 5%, transparent);
	}

	.message-role {
		font-size: var(--t-3xs);
		font-weight: 600;
		text-transform: uppercase;
		color: var(--fg-muted);
		margin-bottom: 4px;
		letter-spacing: 0.05em;
	}

	.message.user .message-role {
		color: var(--accent);
	}

	.message-content {
		font-size: var(--t-xs);
		line-height: 1.5;
		white-space: pre-wrap;
		word-wrap: break-word;
		margin: 0;
		font-family: inherit;
	}

	.tool-message {
		padding: 2px 16px;
		border-bottom: 1px solid var(--border);
	}

	/* Thinking block */

	.thinking-block {
		margin: 4px 0;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		overflow: hidden;
	}

	.thinking-block summary {
		padding: 4px 10px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		cursor: pointer;
		background: var(--bg-surface);
		user-select: none;
	}

	.thinking-block summary:hover {
		color: var(--fg);
	}

	.thinking-content {
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

	/* Tool use/result blocks */

	.tool-block {
		margin: 4px 0;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		overflow: hidden;
	}

	.tool-block.result {
		border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
	}

	.tool-block summary {
		padding: 4px 10px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		cursor: pointer;
		background: var(--bg-surface);
		user-select: none;
		font-family: var(--font-mono);
	}

	.tool-block summary:hover {
		color: var(--fg);
	}

	.tool-call {
		border-top: 1px solid var(--border);
	}

	.tool-name {
		display: block;
		padding: 4px 10px 0;
		font-size: var(--t-3xs);
		font-weight: 600;
		font-family: var(--font-mono);
		color: var(--accent);
	}

	.tool-input {
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
