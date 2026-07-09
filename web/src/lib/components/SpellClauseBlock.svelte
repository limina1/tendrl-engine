<script lang="ts">
	import type { SpellClause } from '$lib/api';

	// The spell preview IS the search DSL: rendered as one wrapped
	// monospace line, exactly like a query string. Non-literal clauses
	// (variables, relative times, relay-side search) carry their
	// annotation as a hover tooltip — dotted underline marks them.
	let { clauses }: { clauses: SpellClause[] } = $props();
</script>

{#if clauses.length > 0}
	<div class="clauses">
		{#each clauses as c (c.clause + (c.annotation ?? ''))}<!--
		--><span
				class="clause"
				class:clause--annotated={!!c.annotation}
				title={c.annotation}
			>{c.clause}</span>{' '}<!--
		-->{/each}
	</div>
{/if}

<style>
	.clauses {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1.6;
		padding: 2px 0;
		overflow-wrap: anywhere;
	}
	.clause {
		color: var(--fg);
		background: color-mix(in srgb, var(--accent) 8%, transparent);
		border-radius: var(--radius);
		padding: 0 3px;
	}
	.clause--annotated {
		text-decoration: underline dotted var(--fg-muted);
		text-underline-offset: 3px;
		cursor: help;
	}
</style>
