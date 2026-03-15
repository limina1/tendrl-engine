<script lang="ts">
	let {
		editMode = false,
		loading = false,
		systemExpanded = false,
		contextExpanded = false,
		onedit,
		onreset,
		ontogglesystem,
		ontogglecontext
	}: {
		editMode?: boolean;
		loading?: boolean;
		systemExpanded?: boolean;
		contextExpanded?: boolean;
		onedit: () => void;
		onreset: () => void;
		ontogglesystem: () => void;
		ontogglecontext: () => void;
	} = $props();
</script>

<div class="toolbar">
	<div class="left">
		<span class="title">tendrl chat</span>
	</div>
	<div class="right">
		<button onclick={ontogglesystem} class:active={systemExpanded} disabled={loading}>
			System
		</button>
		<button onclick={ontogglecontext} class:active={contextExpanded} disabled={loading}>
			Context
		</button>
		<button onclick={onedit} disabled={loading || editMode}>
			Edit
		</button>
		<button onclick={onreset} disabled={loading}>
			Reset
		</button>
	</div>
</div>

<style>
	.toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 16px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-surface);
	}

	.title {
		font-weight: 600;
		font-size: 0.95rem;
	}

	.right {
		display: flex;
		gap: 6px;
	}

	.active {
		background: var(--accent);
		color: white;
		border-color: var(--accent);
	}
</style>
