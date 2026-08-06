<script lang="ts">
	// Shows the diff between the current compose (live, possibly unsaved) and the
	// last *published* (signed) version of the article — so you can see exactly
	// what you've changed since publishing. Diff direction: published → current.
	import type { VersionDiff } from '$lib/api';
	import VersionDiffView from './VersionDiffView.svelte';

	let { diff, onclose }: { diff: VersionDiff; onclose: () => void } = $props();

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			onclose();
		}
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="pdm-backdrop" onclick={onclose} role="presentation">
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		class="pdm"
		onclick={(e) => e.stopPropagation()}
		onkeydown={onKey}
		role="dialog"
		aria-label="Current draft vs last published"
		tabindex="-1"
	>
		<header class="pdm-head">
			<span class="pdm-title">Current draft vs last published</span>
			<button class="pdm-close" onclick={onclose} title="Close (Esc)">✕</button>
		</header>
		<p class="pdm-direction">
			Changes from <b>last published</b> → <b>current draft</b> (added / removed are
			relative to what you published)
		</p>
		<div class="pdm-body">
			<VersionDiffView {diff} />
		</div>
	</div>
</div>

<style>
	.pdm-backdrop {
		position: fixed;
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 120;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.pdm {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius, 6px);
		width: min(680px, 92vw);
		max-height: 82dvh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.pdm-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 9px 13px;
		border-bottom: 1px solid var(--border);
	}
	.pdm-title {
		font-weight: 600;
		font-size: var(--t-sm);
	}
	.pdm-close {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-sm);
	}
	.pdm-close:hover {
		color: var(--fg);
	}
	.pdm-direction {
		margin: 0;
		padding: 8px 13px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		border-bottom: 1px solid var(--border);
	}
	.pdm-body {
		padding: 10px 13px 13px;
		overflow-y: auto;
	}
</style>
