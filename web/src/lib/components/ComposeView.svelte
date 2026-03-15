<script lang="ts">
	import type { ComposeState, ComposeEntry, TagEntry } from '$lib/types';
	import ComposeSection from './ComposeSection.svelte';
	import TagEditor from './TagEditor.svelte';

	let {
		compose,
		onupdate,
		oncancel
	}: {
		compose: ComposeState;
		onupdate: (state: ComposeState) => void;
		oncancel: () => void;
	} = $props();

	function updateTitle(e: Event) {
		onupdate({ ...compose, title: (e.target as HTMLInputElement).value });
	}

	function updateTags(tags: TagEntry[]) {
		onupdate({ ...compose, tags });
	}

	function updateSection(index: number, section: ComposeEntry) {
		const sections = compose.sections.map((s, i) => (i === index ? section : s));
		onupdate({ ...compose, sections });
	}

	function removeSection(index: number) {
		onupdate({ ...compose, sections: compose.sections.filter((_, i) => i !== index) });
	}

	function addSection() {
		onupdate({
			...compose,
			sections: [...compose.sections, { title: '', content: '', tags: [] }]
		});
	}
</script>

<div class="compose-view">
	<div class="compose-header">
		<input
			class="compose-title"
			value={compose.title}
			oninput={updateTitle}
			placeholder="Publication title"
		/>
		<TagEditor tags={compose.tags} onupdate={updateTags} />
	</div>

	<div class="compose-sections">
		{#each compose.sections as section, i}
			<ComposeSection {section} index={i} onupdate={updateSection} onremove={removeSection} />
		{/each}
	</div>

	<div class="compose-actions">
		<button onclick={addSection}>+ Section</button>
		<button onclick={oncancel}>Cancel</button>
	</div>
</div>

<style>
	.compose-view {
		flex: 1;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 16px;
	}

	.compose-header {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.compose-title {
		font-family: inherit;
		font-size: 1.1rem;
		font-weight: 700;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 10px 12px;
		outline: none;
	}

	.compose-title:focus {
		border-color: var(--accent);
	}

	.compose-sections {
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.compose-actions {
		display: flex;
		gap: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--border);
	}
</style>
