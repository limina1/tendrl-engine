<script lang="ts">
	// Saved drafts, grouped by publication (d-tag) into an expandable version
	// list. Each older version expands to its diff *vs the latest* — the 30040
	// title/tag changes on top, the contained 30041 sections beneath (indented
	// by heading level so nested sub-indexes read as nested). The diff itself is
	// computed engine-side (POST /api/v1/drafts/diff); this only renders it.
	import * as api from '$lib/api';
	import type { DraftSummary, VersionDiff } from '$lib/api';

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
	let expandedVersions = $state(new Set<string>());
	let diffs = $state<Record<string, VersionDiff | 'loading' | 'error'>>({});

	function togglePub(dTag: string) {
		const next = new Set(expandedPubs);
		next.has(dTag) ? next.delete(dTag) : next.add(dTag);
		expandedPubs = next;
	}

	async function toggleVersion(group: Group, v: DraftSummary, isLatest: boolean) {
		const id = v.draft_id;
		const next = new Set(expandedVersions);
		if (next.has(id)) {
			next.delete(id);
			expandedVersions = next;
			return;
		}
		next.add(id);
		expandedVersions = next;
		// The latest version is the baseline — nothing to diff. Otherwise fetch
		// its diff against the latest once, then cache.
		if (isLatest || diffs[id]) return;
		diffs = { ...diffs, [id]: 'loading' };
		try {
			const d = await api.draftDiff(id, group.versions[0].draft_id);
			diffs = { ...diffs, [id]: d };
		} catch {
			diffs = { ...diffs, [id]: 'error' };
		}
	}

	function fmt(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}
	// Indent step per heading level beyond 2 (the flat baseline).
	function indent(level: number): string {
		return `${Math.max(0, level - 2) * 14}px`;
	}
</script>

<div class="dv">
	{#each groups as g (g.dTag)}
		<div class="dv-pub">
			<button class="dv-pub-head" onclick={() => togglePub(g.dTag)} aria-expanded={expandedPubs.has(g.dTag)}>
				<span class="dv-ptr">{expandedPubs.has(g.dTag) ? '▾' : '▸'}</span>
				<span class="dv-pub-title">{g.title}</span>
				<span class="dv-count">{g.versions.length} version{g.versions.length === 1 ? '' : 's'}</span>
			</button>

			{#if expandedPubs.has(g.dTag)}
				<ul class="dv-versions">
					{#each g.versions as v, i (v.draft_id)}
						{@const isLatest = i === 0}
						<li class="dv-version">
							<div class="dv-version-row">
								<button
									class="dv-version-toggle"
									onclick={() => toggleVersion(g, v, isLatest)}
									title={isLatest ? 'Latest version' : 'Show what changed vs the latest version'}
								>
									<span class="dv-ptr">{isLatest ? '●' : expandedVersions.has(v.draft_id) ? '▾' : '▸'}</span>
									{#if isLatest}<span class="dv-latest">latest</span>{/if}
									<span class="dv-when">{fmt(v.modified_at)}</span>
									<span class="dv-secs">{v.section_count} sec</span>
								</button>
								<button class="dv-load" onclick={() => onload(v.draft_id)} title="Resume this version">load</button>
								<button class="dv-del" onclick={() => ondelete(v.draft_id)} title="Delete this version">✕</button>
							</div>

							{#if expandedVersions.has(v.draft_id) && !isLatest}
								{@const diff = diffs[v.draft_id]}
								<div class="dv-diff">
									{#if diff === 'loading'}
										<span class="dv-muted">computing diff…</span>
									{:else if diff === 'error'}
										<span class="dv-muted">diff failed</span>
									{:else if diff}
										<!-- 30040 index changes -->
										<div class="dv-index">
											<span class="dv-kind">30040</span>
											{#if diff.titleChanged}
												<span class="dv-title-change">
													title: <s>{diff.titleChanged.old}</s> → {diff.titleChanged.new}
												</span>
											{/if}
											{#if diff.indexTags?.added?.length}
												{#each diff.indexTags.added as [n, val] (n + val)}
													<span class="dv-tag dv-tag--add">+{n}:{val}</span>
												{/each}
											{/if}
											{#if diff.indexTags?.removed?.length}
												{#each diff.indexTags.removed as [n, val] (n + val)}
													<span class="dv-tag dv-tag--rem">−{n}:{val}</span>
												{/each}
											{/if}
											{#if !diff.titleChanged && !diff.indexTags?.added?.length && !diff.indexTags?.removed?.length}
												<span class="dv-muted">index unchanged</span>
											{/if}
										</div>
										<!-- contained 30041 sections, indented by level -->
										{#each diff.sections as s (s.t + s.status)}
											<div class="dv-sec" style:margin-left={indent(s.level)}>
												<span class="dv-status dv-status--{s.status}">{s.status}</span>
												<span class="dv-sec-title">{s.title || '[untitled]'}</span>
												{#if s.contentChanged}<span class="dv-chip dv-chip--content">content</span>{/if}
												{#if s.levelChanged}<span class="dv-chip">level</span>{/if}
												{#if s.tags?.added?.length}
													{#each s.tags.added as [n, val] (n + val)}
														<span class="dv-tag dv-tag--add">+{n}:{val}</span>
													{/each}
												{/if}
												{#if s.tags?.removed?.length}
													{#each s.tags.removed as [n, val] (n + val)}
														<span class="dv-tag dv-tag--rem">−{n}:{val}</span>
													{/each}
												{/if}
											</div>
										{/each}
									{/if}
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</div>
	{/each}
</div>

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
		align-items: stretch;
	}
	.dv-version-toggle {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 6px;
		background: none;
		border: 1px solid transparent;
		border-radius: var(--r-sm, 4px);
		padding: 3px 6px;
		cursor: pointer;
		color: var(--fg);
		text-align: left;
		font-size: 0.74rem;
	}
	.dv-version-toggle:hover {
		border-color: var(--border);
		background: color-mix(in srgb, var(--accent, var(--id-yours)) 8%, transparent);
	}
	.dv-latest {
		font-size: 0.62rem;
		color: var(--state-online);
		border: 1px solid color-mix(in srgb, var(--state-online) 40%, transparent);
		border-radius: 3px;
		padding: 0 4px;
	}
	.dv-when {
		color: var(--fg);
	}
	.dv-secs {
		color: var(--fg-muted);
		font-size: 0.66rem;
	}
	.dv-load,
	.dv-del {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 4px);
		color: var(--fg-muted);
		cursor: pointer;
		padding: 0 7px;
		font-size: 0.7rem;
	}
	.dv-load:hover {
		color: var(--accent, var(--id-yours));
		border-color: var(--accent, var(--id-yours));
	}
	.dv-del:hover {
		color: var(--id-draft, crimson);
		border-color: var(--id-draft, crimson);
	}
	.dv-diff {
		margin: 2px 0 4px 16px;
		padding: 5px 8px;
		border-left: 2px solid var(--border);
		display: flex;
		flex-direction: column;
		gap: 3px;
		font-size: 0.7rem;
	}
	.dv-index,
	.dv-sec {
		display: flex;
		align-items: center;
		gap: 5px;
		flex-wrap: wrap;
	}
	.dv-kind {
		font-family: var(--font-mono);
		font-size: 0.6rem;
		color: var(--fg-muted);
		border: 1px solid var(--border);
		border-radius: 3px;
		padding: 0 3px;
	}
	.dv-sec-title {
		font-weight: 500;
	}
	.dv-title-change s {
		color: var(--fg-muted);
	}
	.dv-muted {
		color: var(--fg-muted);
		font-style: italic;
	}
	.dv-status {
		font-size: 0.6rem;
		font-weight: 600;
		border-radius: 3px;
		padding: 0 4px;
		text-transform: uppercase;
		letter-spacing: 0.03em;
	}
	.dv-status--matched {
		color: var(--fg-muted);
		border: 1px solid var(--border);
	}
	.dv-status--added {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 14%, transparent);
	}
	.dv-status--removed {
		color: var(--id-draft, crimson);
		background: color-mix(in srgb, var(--id-draft, crimson) 14%, transparent);
	}
	.dv-chip {
		font-size: 0.62rem;
		color: var(--id-diverged, var(--id-yours));
		background: color-mix(in srgb, var(--id-diverged, var(--id-yours)) 14%, transparent);
		border-radius: 3px;
		padding: 0 4px;
	}
	.dv-chip--content {
		color: var(--id-diverged, #e2a478);
		background: color-mix(in srgb, var(--id-diverged, #e2a478) 16%, transparent);
	}
	.dv-tag {
		font-family: var(--font-mono);
		font-size: 0.62rem;
		border-radius: 3px;
		padding: 0 4px;
	}
	.dv-tag--add {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 12%, transparent);
	}
	.dv-tag--rem {
		color: var(--id-draft, crimson);
		background: color-mix(in srgb, var(--id-draft, crimson) 12%, transparent);
	}
</style>
