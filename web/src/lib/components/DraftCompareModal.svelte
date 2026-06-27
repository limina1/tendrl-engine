<script lang="ts">
	// Compare two draft snapshots of one publication, side-selectable with an
	// A/B swap. Shows the engine-computed diff A → B: the 30040 index changes
	// (title, tags) on top, then the contained 30041 sections indented by
	// heading level (nested sub-indexes read as nested), each annotated
	// added/removed/changed. Swapping A↔B reverses the direction (the engine
	// recomputes, so adds↔removes flip).
	import { untrack } from 'svelte';
	import * as api from '$lib/api';
	import type { DraftSummary, VersionDiff } from '$lib/api';
	import VersionDiffView from './VersionDiffView.svelte';

	let {
		versions,
		aId,
		bId,
		onclose
	}: {
		versions: DraftSummary[];
		aId: string;
		bId: string;
		onclose: () => void;
	} = $props();

	// Seed the A/B sides from the props once; thereafter they're locally owned
	// (the picker + swap mutate them). The modal is remounted per open, so a
	// fresh open re-seeds. untrack makes the one-time read explicit.
	let a = $state(untrack(() => aId));
	let b = $state(untrack(() => bId));
	let diff = $state<VersionDiff | 'loading' | 'error'>('loading');

	const byId = $derived(new Map(versions.map((v) => [v.draft_id, v])));
	const latestId = $derived(
		[...versions].sort((x, y) => y.modified_at - x.modified_at)[0]?.draft_id
	);

	function label(id: string): string {
		const v = byId.get(id);
		if (!v) return id.slice(0, 8);
		const when = new Date(v.modified_at).toLocaleString();
		return `${when}${id === latestId ? ' · latest' : ''} · ${v.section_count} sec`;
	}

	// Refetch whenever either side changes. `a`/`b` are read so the effect
	// re-runs; the fetch is guarded against a stale resolve by capturing them.
	$effect(() => {
		const from = a;
		const to = b;
		diff = 'loading';
		let cancelled = false;
		api
			.draftDiff(from, to)
			.then((d) => {
				if (!cancelled) diff = d;
			})
			.catch(() => {
				if (!cancelled) diff = 'error';
			});
		return () => {
			cancelled = true;
		};
	});

	function swap() {
		[a, b] = [b, a];
	}
	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			onclose();
		} else if (e.key === 's') {
			e.preventDefault();
			swap();
		}
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="dcm-backdrop" onclick={onclose} role="presentation">
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		class="dcm"
		onclick={(e) => e.stopPropagation()}
		onkeydown={onKey}
		role="dialog"
		aria-label="Compare draft versions"
		tabindex="-1"
	>
		<header class="dcm-head">
			<span class="dcm-title">Compare versions</span>
			<button class="dcm-close" onclick={onclose} title="Close (Esc)">✕</button>
		</header>

		<div class="dcm-pickers">
			<div class="dcm-side">
				<span class="dcm-side-label dcm-side-label--a">A</span>
				<select bind:value={a} class="dcm-select">
					{#each versions as v (v.draft_id)}
						<option value={v.draft_id}>{label(v.draft_id)}</option>
					{/each}
				</select>
			</div>
			<button class="dcm-swap" onclick={swap} title="Swap A ⇄ B (s)">⇄</button>
			<div class="dcm-side">
				<span class="dcm-side-label dcm-side-label--b">B</span>
				<select bind:value={b} class="dcm-select">
					{#each versions as v (v.draft_id)}
						<option value={v.draft_id}>{label(v.draft_id)}</option>
					{/each}
				</select>
			</div>
		</div>
		<p class="dcm-direction">Changes from <b>A</b> to <b>B</b></p>

		<div class="dcm-body">
			{#if diff === 'loading'}
				<span class="dcm-muted">computing diff…</span>
			{:else if diff === 'error'}
				<span class="dcm-muted">diff failed</span>
			{:else if a === b}
				<span class="dcm-muted">A and B are the same version — pick two to compare.</span>
			{:else}
				<VersionDiffView {diff} />
			{/if}
		</div>
	</div>
</div>

<style>
	.dcm-backdrop {
		position: fixed;
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 120;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.dcm {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius, 6px);
		width: min(680px, 92vw);
		max-height: 82vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.dcm-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 9px 13px;
		border-bottom: 1px solid var(--border);
	}
	.dcm-title {
		font-weight: 600;
		font-size: var(--t-sm);
	}
	.dcm-close {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: var(--t-sm);
	}
	.dcm-close:hover {
		color: var(--fg);
	}
	.dcm-pickers {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 13px 4px;
	}
	.dcm-side {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.dcm-side-label {
		font-weight: 700;
		font-size: var(--t-3xs);
		border-radius: 3px;
		padding: 1px 6px;
	}
	.dcm-side-label--a {
		color: var(--id-imported, #a093c7);
		background: color-mix(in srgb, var(--id-imported, #a093c7) 16%, transparent);
	}
	.dcm-side-label--b {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 16%, transparent);
	}
	.dcm-select {
		flex: 1;
		min-width: 0;
		background: var(--bg-surface, var(--bg));
		color: var(--fg);
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		padding: 3px 6px;
		font: inherit;
		font-size: var(--t-2xs);
	}
	.dcm-swap {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		color: var(--fg);
		cursor: pointer;
		padding: 3px 9px;
		font-size: var(--t-sm);
	}
	.dcm-swap:hover {
		border-color: var(--accent, var(--id-yours));
		color: var(--accent, var(--id-yours));
	}
	.dcm-direction {
		margin: 0;
		padding: 0 13px 8px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
	}
	.dcm-body {
		padding: 10px 13px 13px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: var(--t-2xs);
		border-top: 1px solid var(--border);
	}
	.dcm-muted {
		color: var(--fg-muted);
		font-style: italic;
	}
</style>
