<script lang="ts">
	// The annotations table — one buffer kind (`annotations:<root>`) with its
	// own views: by section · by author · timeline · free · collective. Beside
	// the reader or filling the window is the WM's split state, not a mode
	// of the table. Contract: docs/annotations-table-contract.org.
	//
	// Track 0 stub: resolves its root and renders the shell classes the
	// contract names, so the split/open wiring (Track B) and the mobile
	// shell (Track D) can be exercised before the table itself (Track C)
	// lands. No Svelte lifecycle hooks — props + $effect teardown only.
	import type { Buffer } from '../types';
	import { rootFromAnnotationsId } from '$lib/components/annotations/table-model';

	let { buffer }: { buffer: Buffer } = $props();
	const root = $derived(rootFromAnnotationsId(buffer.id));
</script>

<div class="abuf" data-root={root ?? ''}>
	<div class="spell">
		<input class="spell__input" value={root ? `A:${root} k:30041 count:by count:a` : ''} readonly aria-label="Table query" />
	</div>
	<div class="atable" role="region" aria-label="Annotations table">
		<div class="empty">
			{#if root}
				annotations table for <code>{root}</code> — table view not built yet
			{:else}
				not an annotations buffer: <code>{buffer.id}</code>
			{/if}
		</div>
	</div>
</div>

<style>
	.abuf { display: flex; flex-direction: column; min-height: 0; height: 100%; }
	.spell { display: flex; gap: var(--s-1); padding: var(--s-1) var(--s-2); border-bottom: 1px solid var(--panel-border); }
	.spell__input { flex: 1; min-width: 0; font-family: var(--font-mono); font-size: var(--t-xs); color: var(--fg); background: var(--base1); border: 1px solid var(--panel-border); border-radius: 3px; padding: 4px 8px; }
	.atable { flex: 1; overflow: auto; position: relative; }
	.empty { padding: var(--s-3); font-family: var(--font-mono); font-size: var(--t-xs); color: var(--base5); font-style: italic; }
	code { font-family: var(--font-mono); color: var(--base6); }
</style>
