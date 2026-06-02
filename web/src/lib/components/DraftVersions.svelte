<script lang="ts">
	// Saved drafts, grouped by publication (d-tag) into an expandable version
	// list. Each version row can Compare — opening a modal that diffs any two
	// versions of that publication with an A/B swap. Load resumes a version;
	// the × deletes that snapshot.
	import type { DraftSummary } from '$lib/api';
	import DraftCompareModal from './DraftCompareModal.svelte';

	let {
		drafts,
		onload,
		ondelete
	}: {
		drafts: DraftSummary[];
		onload: (draftId: string) => void;
		ondelete: (draftId: string) => void;
	} = $props();

	type Group = { dTag: string; title: string; versions: DraftSummary[] };

	// Group by d-tag; versions newest-first; groups by most-recent activity.
	const groups = $derived.by<Group[]>(() => {
		const map = new Map<string, DraftSummary[]>();
		for (const d of drafts) {
			const arr = map.get(d.d_tag) ?? [];
			arr.push(d);
			map.set(d.d_tag, arr);
		}
		const out: Group[] = [];
		for (const [dTag, vs] of map) {
			vs.sort((a, b) => b.modified_at - a.modified_at);
			out.push({ dTag, title: vs[0].title || '[untitled]', versions: vs });
		}
		out.sort((a, b) => b.versions[0].modified_at - a.versions[0].modified_at);
		return out;
	});

	let expandedPubs = $state(new Set<string>());
	let compare = $state<{ versions: DraftSummary[]; aId: string; bId: string } | null>(null);

	function togglePub(dTag: string) {
		const next = new Set(expandedPubs);
		next.has(dTag) ? next.delete(dTag) : next.add(dTag);
		expandedPubs = next;
	}

	// Open the compare modal for this publication. A defaults to the clicked
	// version, B to the latest; if they're the same (clicked the latest), A
	// drops to the next-newest so the two sides differ.
	function openCompare(g: Group, vId: string) {
		const latest = g.versions[0].draft_id;
		let aId = vId;
		if (aId === latest && g.versions.length > 1) aId = g.versions[1].draft_id;
		compare = { versions: g.versions, aId, bId: latest };
	}

	// created_at/modified_at are unix milliseconds (finer than the events' secs).
	function fmt(ts: number): string {
		return new Date(ts).toLocaleString();
	}
</script>

<div class="dv">
	{#each groups as g (g.dTag)}
		<div class="dv-pub">
			<button
				class="dv-pub-head"
				onclick={() => togglePub(g.dTag)}
				aria-expanded={expandedPubs.has(g.dTag)}
			>
				<span class="dv-ptr">{expandedPubs.has(g.dTag) ? '▾' : '▸'}</span>
				<span class="dv-pub-title">{g.title}</span>
				<!-- Short d-tag so two articles that share a title read as distinct
				     publications (identity is the d-tag, not the title). -->
				<span class="dv-dtag" title={`d-tag ${g.dTag}`}>{g.dTag.slice(0, 6)}</span>
				<span class="dv-count">{g.versions.length} version{g.versions.length === 1 ? '' : 's'}</span>
			</button>

			{#if expandedPubs.has(g.dTag)}
				<ul class="dv-versions">
					{#each g.versions as v, i (v.draft_id)}
						{@const isLatest = i === 0}
						<li class="dv-version-row">
							<div class="dv-meta">
								{#if isLatest}<span class="dv-latest">latest</span>{/if}
								<span class="dv-when">{fmt(v.modified_at)}</span>
								<span class="dv-secs">{v.section_count} sec</span>
							</div>
							<button
								class="dv-btn"
								onclick={() => openCompare(g, v.draft_id)}
								disabled={g.versions.length < 2}
								title={g.versions.length < 2
									? 'Save another version to compare'
									: 'Compare versions (A/B diff)'}>compare</button
							>
							<button class="dv-btn dv-load" onclick={() => onload(v.draft_id)} title="Resume this version">load</button>
							<button class="dv-btn dv-del" onclick={() => ondelete(v.draft_id)} title="Delete this version">✕</button>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/each}
</div>

{#if compare}
	<DraftCompareModal
		versions={compare.versions}
		aId={compare.aId}
		bId={compare.bId}
		onclose={() => (compare = null)}
	/>
{/if}

<style>
	.dv {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.dv-ptr {
		color: var(--fg-muted);
		font-size: 0.7rem;
		width: 0.9em;
		display: inline-block;
	}
	.dv-pub-head {
		display: flex;
		align-items: center;
		gap: 6px;
		width: 100%;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		padding: 5px 8px;
		cursor: pointer;
		color: var(--fg);
		text-align: left;
	}
	.dv-pub-head:hover {
		border-color: var(--accent, var(--id-yours));
	}
	.dv-pub-title {
		flex: 1;
		min-width: 0;
		font-size: 0.82rem;
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.dv-dtag {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		color: var(--fg-muted);
		border: 1px solid var(--border);
		border-radius: 3px;
		padding: 0 4px;
	}
	.dv-count {
		font-size: 0.68rem;
		color: var(--fg-muted);
	}
	.dv-versions {
		list-style: none;
		margin: 2px 0 0;
		padding: 0 0 0 10px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.dv-version-row {
		display: flex;
		gap: 4px;
		align-items: center;
	}
	.dv-meta {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.74rem;
		overflow: hidden;
	}
	.dv-latest {
		font-size: 0.62rem;
		color: var(--state-online);
		border: 1px solid color-mix(in srgb, var(--state-online) 40%, transparent);
		border-radius: 3px;
		padding: 0 4px;
	}
	.dv-secs {
		color: var(--fg-muted);
		font-size: 0.66rem;
	}
	.dv-btn {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		color: var(--fg-muted);
		cursor: pointer;
		padding: 1px 7px;
		font-size: 0.7rem;
	}
	.dv-btn:hover:not(:disabled) {
		color: var(--accent, var(--id-yours));
		border-color: var(--accent, var(--id-yours));
	}
	.dv-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}
	.dv-del:hover:not(:disabled) {
		color: var(--id-draft, crimson);
		border-color: var(--id-draft, crimson);
	}
</style>
