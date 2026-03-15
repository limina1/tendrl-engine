<script lang="ts">
	let {
		contextCount = 0,
		oninject,
		disabled = false
	}: {
		contextCount?: number;
		oninject: (title: string, content: string) => void;
		disabled?: boolean;
	} = $props();

	let title = $state('');
	let content = $state('');

	function submit() {
		if (!title.trim() || !content.trim()) return;
		oninject(title.trim(), content.trim());
		title = '';
		content = '';
	}
</script>

<div class="panel">
	<div class="panel-header">
		Context Notes
		{#if contextCount > 0}
			<span class="badge">{contextCount}</span>
		{/if}
	</div>
	<div class="fields">
		<input
			bind:value={title}
			placeholder="Note title"
			{disabled}
		/>
		<textarea
			bind:value={content}
			placeholder="Note content..."
			rows="3"
			{disabled}
		></textarea>
	</div>
	<div class="panel-actions">
		<button class="primary" onclick={submit} disabled={disabled || !title.trim() || !content.trim()}>
			Add
		</button>
	</div>
</div>

<style>
	.panel {
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.panel-header {
		padding: 8px 16px;
		font-size: 0.8rem;
		font-weight: 600;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.badge {
		background: var(--accent);
		color: white;
		font-size: 0.7rem;
		padding: 1px 7px;
		border-radius: 10px;
	}

	.fields {
		padding: 0 16px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	input {
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

	textarea {
		width: 100%;
		font-size: 0.85rem;
	}

	.panel-actions {
		padding: 8px 16px;
		display: flex;
		gap: 8px;
	}
</style>
