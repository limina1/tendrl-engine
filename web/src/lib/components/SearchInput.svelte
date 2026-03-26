<script lang="ts">
	let { onsearch, disabled = false }: { onsearch: (query: string) => void; disabled?: boolean } =
		$props();

	let value = $state('');

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter') {
			e.preventDefault();
			submit();
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
		font-size: 0.85rem;
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
		font-size: 0.8rem;
		padding: 2px 6px;
	}
</style>
