<script lang="ts">
	import type { ComposeEntry, TagEntry } from '$lib/types';
	import TagEditor from './TagEditor.svelte';

	let {
		section,
		index,
		onupdate,
		onremove
	}: {
		section: ComposeEntry;
		index: number;
		onupdate: (index: number, section: ComposeEntry) => void;
		onremove: (index: number) => void;
	} = $props();

	function updateField(field: keyof ComposeEntry, value: string | TagEntry[]) {
		onupdate(index, { ...section, [field]: value });
	}
</script>

<div class="compose-section">
	<div class="compose-section-header">
		<input
			class="compose-section-title"
			value={section.title}
			oninput={(e) => updateField('title', e.currentTarget.value)}
			placeholder="Section title"
		/>
		<button onclick={() => onremove(index)}>Remove</button>
	</div>
	<textarea
		value={section.content}
		oninput={(e) => updateField('content', e.currentTarget.value)}
		placeholder="Section content..."
		rows="6"
	></textarea>
	<TagEditor tags={section.tags} onupdate={(tags) => updateField('tags', tags)} />
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

	.compose-section-header {
		display: flex;
		gap: 8px;
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
</style>
