<script lang="ts">
	// `value` is bindable so external surfaces (the Refs tab's "→ search"
	// route action) can push coordinate tokens into the query without
	// running it. The user then edits and presses Enter to commit.
	let {
		onsearch,
		disabled = false,
		value = $bindable<string>('')
	}: { onsearch: (query: string) => void; disabled?: boolean; value?: string } = $props();

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			submit();
			// Blur after commit so the global focusout listener flips
			// modal nav back to 'normal' — same loop as vim `/` search:
			// query → RET commits & exits insert → j/k walks results →
			// i/o re-enters the field.
			(e.currentTarget as HTMLInputElement).blur();
		}
	}

	function submit() {
		if (disabled) return;
		const trimmed = value.trim();
		if (!trimmed) return;
		onsearch(trimmed);
	}

	function clear() {
		value = '';
		onsearch('');
	}
</script>

<div class="search-input">
	<input
		type="text"
		bind:value
		onkeydown={handleKeydown}
		placeholder={'t:tag k:30041 "exact" words'}
		data-entry
		{disabled}
	/>
	{#if value.length > 0}
		<button class="clear-btn" onclick={clear} title="Clear search">x</button>
	{/if}
</div>

<style>
	.search-input {
		padding: 8px 12px;
		position: relative;
	}

	input {
		width: 100%;
		font-family: inherit;
		font-size: var(--t-xs);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 8px 12px;
		outline: none;
	}

	input:focus {
		border-color: var(--accent);
	}

	.clear-btn {
		position: absolute;
		right: 20px;
		top: 50%;
		transform: translateY(-50%);
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-2xs);
		padding: 2px 6px;
	}
</style>
