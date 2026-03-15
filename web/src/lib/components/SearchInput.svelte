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
		const trimmed = value.trim();
		if (!trimmed || disabled) return;
		onsearch(trimmed);
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
</div>

<style>
	.search-input {
		padding: 8px 12px;
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
</style>
