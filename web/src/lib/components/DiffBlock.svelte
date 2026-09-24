<script lang="ts">
	// One section's diff, engine-computed (`DiffHunk[][]`): `- line` red,
	// `+ line` green, context muted, every `{{ref:…}}` run on an added line
	// tinted so the new pointers stand out. Pure rendering — the same rows
	// RelinkModal draws for its preview, as a component the review reuses.
	import type { DiffHunk } from '$lib/api';

	let {
		title,
		sub = '',
		hunks,
		tagsAdded = [],
		tagsRemoved = [],
		coordinate = '',
		bare = false
	}: {
		title: string;
		/** The head's grey line (kind · d · "same d, new version"). */
		sub?: string;
		hunks: DiffHunk[][];
		tagsAdded?: string[][];
		tagsRemoved?: string[][];
		coordinate?: string;
		/** No head row — the caller draws its own (the review's collapsible
		 *  section summary). */
		bare?: boolean;
	} = $props();

	const lineCount = $derived(hunks.reduce((n, h) => n + h.filter((l) => l.kind !== 'context').length, 0));

	/** Split a line around its `{{ref:…}}` runs so the tokens can be tinted. */
	function runs(line: string): { text: string; ref: boolean }[] {
		const out: { text: string; ref: boolean }[] = [];
		const re = /\{\{ref:[^}]*\}\}/g;
		let last = 0;
		let m: RegExpExecArray | null;
		while ((m = re.exec(line))) {
			if (m.index > last) out.push({ text: line.slice(last, m.index), ref: false });
			out.push({ text: m[0], ref: true });
			last = m.index + m[0].length;
		}
		if (last < line.length) out.push({ text: line.slice(last), ref: false });
		return out;
	}
	const sign = (k: DiffHunk['kind']) => (k === 'added' ? '+' : k === 'removed' ? '-' : ' ');
</script>

<div class="diff" class:diff--bare={bare} data-section={coordinate}>
	{#if !bare}
		<div class="diff__head">
			<b>{title}</b>
			{#if sub}<span>{sub}</span>{/if}
			<span class="diff__n">{hunks.length} hunk{hunks.length === 1 ? '' : 's'} · {lineCount} line{lineCount === 1 ? '' : 's'}</span>
		</div>
	{/if}
	{#each hunks as hunk, i (i)}
		{#if i > 0}<div class="diff__sep">…</div>{/if}
		{#each hunk as l, j (j)}
			<div class="diff__ln diff__ln--{l.kind}">{sign(l.kind)} {#if l.kind === 'added'}{#each runs(l.line) as r, k (k)}{#if r.ref}<span class="diff__ref">{r.text}</span>{:else}{r.text}{/if}{/each}{:else}{l.line}{/if}</div>
		{/each}
	{/each}
	{#each tagsAdded as t, i (i)}
		<div class="diff__ln diff__ln--tag">  tags  + [{t.map((v) => JSON.stringify(v)).join(', ')}]</div>
	{/each}
	{#each tagsRemoved as t, i (i)}
		<div class="diff__ln diff__ln--tag diff__ln--tagdel">  tags  − [{t.map((v) => JSON.stringify(v)).join(', ')}]</div>
	{/each}
</div>

<style>
	.diff {
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
		overflow: hidden;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		line-height: 1.5;
	}
	.diff--bare {
		border-top: 0;
		border-radius: 0 0 var(--r-sm, 3px) var(--r-sm, 3px);
	}
	.diff__head {
		background: var(--bg-surface);
		padding: 4px 10px;
		display: flex;
		gap: 10px;
		align-items: baseline;
		flex-wrap: wrap;
		color: var(--base6);
		border-bottom: 1px solid var(--panel-border);
	}
	.diff__head b {
		color: var(--fg);
		font-weight: 600;
	}
	.diff__n {
		margin-left: auto;
		color: var(--base5);
	}
	.diff__sep {
		padding: 0 10px;
		color: var(--base4);
		border-top: 1px dashed var(--panel-border);
		border-bottom: 1px dashed var(--panel-border);
	}
	.diff__ln {
		padding: 1px 10px;
		white-space: pre-wrap;
		word-break: break-word;
		color: var(--base7);
	}
	.diff__ln--context {
		color: var(--base5);
	}
	.diff__ln--removed {
		background: rgba(226, 120, 120, 0.12);
		color: var(--red);
	}
	.diff__ln--added {
		background: rgba(180, 190, 130, 0.12);
		color: var(--green);
	}
	.diff__ref {
		background: rgba(180, 190, 130, 0.22);
		border-radius: 2px;
	}
	.diff__ln--tag {
		color: var(--cyan);
		background: rgba(137, 184, 194, 0.08);
	}
	.diff__ln--tagdel {
		color: var(--red);
		background: rgba(226, 120, 120, 0.08);
	}
</style>
