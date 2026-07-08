<script lang="ts">
	import type { SpellClause } from '$lib/api';

	// The spell preview IS the search DSL: one clause per line, with a dim
	// annotation only where a clause isn't literal (variables, relative
	// times, relay-side search). Shared by spell cards and the composer.
	let { clauses }: { clauses: SpellClause[] } = $props();
</script>

{#if clauses.length > 0}
	<div class="clauses">
		{#each clauses as c (c.clause + (c.annotation ?? ''))}
			<div class="clause-line">
				<code class="clause">{c.clause}</code>
				{#if c.annotation}
					<span class="clause-note">— {c.annotation}</span>
				{/if}
			</div>
		{/each}
	</div>
{/if}

<style>
	.clauses {
		display: flex;
		flex-direction: column;
		gap: 1px;
		padding: 4px 0;
	}
	.clause-line {
		display: flex;
		align-items: baseline;
		gap: 6px;
		min-width: 0;
	}
	.clause {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: color-mix(in srgb, var(--accent) 8%, transparent);
		border-radius: var(--radius);
		padding: 0 4px;
		white-space: nowrap;
	}
	.clause-note {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
