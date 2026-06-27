<script lang="ts">
	import type { TagEntry } from '$lib/types';

	let { tags, onupdate, disabled = false }: { tags: TagEntry[]; onupdate: (tags: TagEntry[]) => void; disabled?: boolean } = $props();

	function addTag() {
		onupdate([...tags, { name: '', value: '' }]);
	}

	function removeTag(index: number) {
		onupdate(tags.filter((_, i) => i !== index));
	}

	function updateTag(index: number, field: 'name' | 'value', val: string) {
		const updated = tags.map((t, i) => (i === index ? { ...t, [field]: val } : t));
		onupdate(updated);
	}
</script>

<div class="tag-editor">
	{#each tags as tag, i}
		<div class="tag-row">
			<input
				placeholder="name"
				value={tag.name}
				oninput={(e) => updateTag(i, 'name', e.currentTarget.value)}
				{disabled}
			/>
			<input
				placeholder="value"
				value={tag.value}
				oninput={(e) => updateTag(i, 'value', e.currentTarget.value)}
				{disabled}
			/>
			<button class="tag-remove" onclick={() => removeTag(i)} {disabled}>x</button>
		</div>
	{/each}
	<button class="tag-add" onclick={addTag} {disabled}>+ Tag</button>
</div>

<style>
	.tag-editor {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 4px 0;
	}

	.tag-row {
		display: flex;
		gap: 4px;
		align-items: center;
	}

	.tag-row input {
		flex: 1;
		font-family: inherit;
		font-size: var(--t-2xs);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 4px 8px;
		outline: none;
	}

	.tag-row input:focus {
		border-color: var(--accent);
	}

	.tag-remove {
		padding: 2px 8px;
		font-size: var(--t-2xs);
	}

	.tag-add {
		align-self: flex-start;
		font-size: var(--t-2xs);
		padding: 2px 10px;
	}
</style>
