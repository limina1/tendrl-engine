<script lang="ts">
	import type { ContextItem, TagEntry } from '$lib/types';
	import TagEditor from './TagEditor.svelte';

	let {
		section,
		checked,
		oncheck,
		onupdate,
		onupdatetags,
		onreset,
		onremove
	}: {
		section: ContextItem;
		checked: boolean;
		oncheck: (id: string) => void;
		onupdate: (id: string, title: string, content: string) => void;
		onupdatetags: (id: string, tags: TagEntry[]) => void;
		onreset: (id: string) => void;
		onremove: (id: string) => void;
	} = $props();
</script>

<div class="compose-section" class:modified={section.modified}>
	<div class="compose-section-header">
		<label class="check">
			<input
				type="checkbox"
				{checked}
				onchange={() => oncheck(section.id)}
			/>
		</label>
		<input
			class="compose-section-title"
			value={section.title}
			oninput={(e) => onupdate(section.id, e.currentTarget.value, section.content)}
			placeholder="Section title"
		/>
		<button onclick={() => onremove(section.id)}>Remove</button>
	</div>
	<textarea
		value={section.content}
		oninput={(e) => onupdate(section.id, section.title, e.currentTarget.value)}
		placeholder="Section content..."
		rows="6"
	></textarea>
	<TagEditor tags={section.tags} onupdate={(tags) => onupdatetags(section.id, tags)} />
	{#if section.modified}
		<div class="modified-banner">
			<span>Modified</span>
			<button class="reset-btn" onclick={() => onreset(section.id)}>Reset</button>
		</div>
	{/if}
</div>

<style>
	.compose-section {
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.compose-section.modified {
		border-color: var(--modified-border);
		background: var(--modified-bg);
	}

	.compose-section-header {
		display: flex;
		gap: 8px;
		align-items: center;
	}

	.check {
		display: flex;
		align-items: center;
	}

	.compose-section-title {
		flex: 1;
		font-family: inherit;
		font-size: 0.9rem;
		font-weight: 600;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 6px 10px;
		outline: none;
	}

	.compose-section-title:focus {
		border-color: var(--accent);
	}

	textarea {
		width: 100%;
		font-size: 0.85rem;
		line-height: 1.5;
	}

	.modified-banner {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 8px;
		border-radius: 4px;
		background: var(--modified-bg);
		color: var(--modified-fg);
		font-size: 0.75rem;
		font-weight: 600;
		border: 1px solid var(--modified-border);
	}

	.reset-btn {
		font-size: 0.7rem;
		padding: 2px 8px;
	}
</style>
