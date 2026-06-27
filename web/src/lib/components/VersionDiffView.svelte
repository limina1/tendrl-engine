<script lang="ts">
	// Renders an engine VersionDiff (A → B): the 30040 index changes on top, then
	// the contained 30041 sections indented by heading level, each annotated
	// added / removed / matched-with-changes. Shared by the draft-version compare
	// modal and the "current vs published" modal.
	import type { VersionDiff } from '$lib/api';

	let { diff }: { diff: VersionDiff } = $props();

	function indent(level: number): string {
		return `${Math.max(0, level - 2) * 14}px`;
	}
</script>

<div class="vdv">
	<!-- 30040 index changes -->
	<div class="vdv-index">
		<span class="vdv-kind">30040 index</span>
		{#if diff.titleChanged}
			<span class="vdv-title-change">
				title: <s>{diff.titleChanged.old}</s> → <b>{diff.titleChanged.new}</b>
			</span>
		{/if}
		{#if diff.indexTags?.added?.length}
			{#each diff.indexTags.added as [n, val] (n + val)}
				<span class="vdv-tag vdv-tag--add">+{n}:{val}</span>
			{/each}
		{/if}
		{#if diff.indexTags?.removed?.length}
			{#each diff.indexTags.removed as [n, val] (n + val)}
				<span class="vdv-tag vdv-tag--rem">−{n}:{val}</span>
			{/each}
		{/if}
		{#if !diff.titleChanged && !diff.indexTags?.added?.length && !diff.indexTags?.removed?.length}
			<span class="vdv-muted">index unchanged</span>
		{/if}
	</div>

	<!-- contained 30041 sections, indented by heading level -->
	{#if diff.sections.length === 0}
		<span class="vdv-muted">no sections</span>
	{:else}
		{#each diff.sections as s (s.t + s.status)}
			<div class="vdv-sec" style:margin-left={indent(s.level)}>
				<span class="vdv-status vdv-status--{s.status}">{s.status}</span>
				<span class="vdv-sec-title">{s.title || '[untitled]'}</span>
				{#if s.contentChanged}<span class="vdv-chip vdv-chip--content">content</span>{/if}
				{#if s.levelChanged}<span class="vdv-chip">level</span>{/if}
				{#if s.tags?.added?.length}
					{#each s.tags.added as [n, val] (n + val)}
						<span class="vdv-tag vdv-tag--add">+{n}:{val}</span>
					{/each}
				{/if}
				{#if s.tags?.removed?.length}
					{#each s.tags.removed as [n, val] (n + val)}
						<span class="vdv-tag vdv-tag--rem">−{n}:{val}</span>
					{/each}
				{/if}
			</div>
		{/each}
	{/if}
</div>

<style>
	.vdv {
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: var(--t-2xs);
	}
	.vdv-index,
	.vdv-sec {
		display: flex;
		align-items: center;
		gap: 5px;
		flex-wrap: wrap;
	}
	.vdv-kind {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		border: 1px solid var(--border);
		border-radius: 3px;
		padding: 0 4px;
	}
	.vdv-sec-title {
		font-weight: 500;
	}
	.vdv-title-change s {
		color: var(--fg-muted);
	}
	.vdv-muted {
		color: var(--fg-muted);
		font-style: italic;
	}
	.vdv-status {
		font-size: var(--t-3xs);
		font-weight: 600;
		border-radius: 3px;
		padding: 0 4px;
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}
	.vdv-status--matched {
		color: var(--fg-muted);
		border: 1px solid var(--border);
	}
	.vdv-status--added {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 14%, transparent);
	}
	.vdv-status--removed {
		color: var(--id-draft, crimson);
		background: color-mix(in srgb, var(--id-draft, crimson) 14%, transparent);
	}
	.vdv-chip {
		font-size: var(--t-3xs);
		color: var(--id-diverged, var(--id-yours));
		background: color-mix(in srgb, var(--id-diverged, var(--id-yours)) 14%, transparent);
		border-radius: 3px;
		padding: 0 4px;
	}
	.vdv-chip--content {
		color: var(--id-diverged, #e2a478);
		background: color-mix(in srgb, var(--id-diverged, #e2a478) 16%, transparent);
	}
	.vdv-tag {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		border-radius: 3px;
		padding: 0 4px;
	}
	.vdv-tag--add {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 12%, transparent);
	}
	.vdv-tag--rem {
		color: var(--id-draft, crimson);
		background: color-mix(in srgb, var(--id-draft, crimson) 12%, transparent);
	}
</style>
