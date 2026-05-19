<script lang="ts">
	import FocusGraph, { type GraphNode } from '$lib/components/FocusGraph.svelte';
	import type { NAddr } from '$lib/types';

	// ── Collatz test structure (mirrors scripts/publish-collatz.py) ──────
	// The odd-Collatz tree: each number maps to the predecessors that step
	// into it. Root is 1. A parent → child pair is an `a`-tag reference.
	const TREE: Record<number, number[]> = {
		1: [5, 21, 85],
		5: [13, 3, 53],
		13: [17, 69],
		17: [11, 45],
		11: [7, 29],
		7: [9, 37],
		37: [49, 197],
		49: [65, 261],
		65: [43, 173],
		43: [57],
		29: [19, 77],
		19: [25, 101],
		25: [33],
		101: [67],
		67: [89, 357],
		89: [59, 237],
		59: [39],
		53: [35],
		35: [23, 93],
		23: [15, 61],
		85: [113],
		113: [75, 301],
		77: [51, 205],
		197: [131],
		173: [115],
		61: [81],
		301: [201],
		205: [137],
		131: [175],
		115: [153]
	};
	// Artificial back-edges — each number also references an ancestor,
	// closing a cycle the graph panel draws as a yellow dashed edge.
	const CYCLES: Record<number, number> = {
		// Cycles whose endpoints share a row — routed in the row gutter.
		43: 5,
		9: 11,
		153: 65,
		57: 7,
		37: 29,
		53: 21,
		29: 21,
		33: 19,
		61: 35,
		59: 19,
		25: 113,
		67: 77,
		77: 75,
		// Cycles spanning rows — routed in the lanes below the band.
		39: 1,
		21: 1,
		89: 19,
		101: 5,
		175: 37,
		237: 59,
		173: 43,
		131: 7,
		197: 11,
		93: 53,
		81: 23,
		51: 29,
		75: 113,
		45: 17,
		115: 49,
		69: 13,
		201: 85,
		137: 67,
		65: 45,
		49: 69,
		35: 21,
		113: 5,
		205: 51,
		19: 13
	};

	const PUBKEY = 'e079f18a8b63037e47d8a111039b1a30511791b0f14ad140392787060c28209a';
	const dtag = (n: number) => `collatz-${n}`;
	const keyOf = (n: number) => `30040:${PUBKEY}:${dtag(n)}`;

	// A number is a 30040 index if it references anything (a tree child or
	// a cycle back-edge); the graph contains only indexes.
	const isIndex = (n: number) => n in TREE || n in CYCLES;
	function childIndexes(n: number): number[] {
		const kids = [...(TREE[n] ?? [])];
		if (n in CYCLES) kids.push(CYCLES[n]);
		return kids.filter(isIndex);
	}

	const allNumbers = (() => {
		const s = new Set<number>(Object.keys(TREE).map(Number));
		for (const ks of Object.values(TREE)) for (const k of ks) s.add(k);
		for (const k of Object.keys(CYCLES).map(Number)) s.add(k);
		return [...s].sort((a, b) => a - b);
	})();

	// The node record FocusGraph consumes — one entry per index.
	const nodes: Record<string, GraphNode> = {};
	for (const n of allNumbers) {
		if (!isIndex(n)) continue;
		nodes[keyOf(n)] = {
			addr: { kind: 30040, pubkey: PUBKEY, d_tag: dtag(n) },
			title: `n = ${n}`,
			childKeys: childIndexes(n).map(keyOf)
		};
	}

	// Structural parent map (forward tree only) → the breadcrumb path.
	const parent: Record<number, number> = {};
	for (const [p, kids] of Object.entries(TREE))
		for (const k of kids) parent[k] = Number(p);
	function pathTo(n: number): number[] {
		const path: number[] = [];
		let cur: number | undefined = n;
		let guard = 0;
		while (cur !== undefined && guard++ < 64) {
			path.unshift(cur);
			cur = parent[cur];
		}
		return path;
	}

	let currentNum = $state(1);
	let expandAll = $state(true);
	let closed = $state(false);

	const currentKey = $derived(keyOf(currentNum));
	const pathKeys = $derived(pathTo(currentNum).filter(isIndex).map(keyOf));

	function navigate(addr: NAddr) {
		const m = addr.d_tag.match(/^collatz-(\d+)$/);
		if (m) currentNum = Number(m[1]);
	}

	const indexCount = Object.keys(nodes).length;
	const cycleCount = Object.keys(CYCLES).length;
	const quickFocus = [1, 17, 19, 89, 39];
</script>

<svelte:head><title>tendrl · design · publication graph</title></svelte:head>

<div class="wrap">
	<header>
		<h1>Publication graph</h1>
		<p class="lede">
			The reader's <code>FocusGraph</code> panel, on a test structure: the
			odd-Collatz tree — {indexCount} index nodes — with {cycleCount}
			artificial back-edges closing cycles. Click a node to refocus; the
			⊞ corner tab unpacks an index. Green solid = forward reference,
			yellow dashed = a cyclic back-edge routed through packed lanes.
		</p>
	</header>

	<div class="controls">
		<label class="chk">
			<input type="checkbox" bind:checked={expandAll} />
			expand all
		</label>
		<span class="sep">focus</span>
		{#each quickFocus as n (n)}
			<button class="chip" class:active={currentNum === n} onclick={() => (currentNum = n)}
				>n = {n}</button
			>
		{/each}
		{#if closed}
			<button class="chip reopen" onclick={() => (closed = false)}>reopen graph</button>
		{/if}
	</div>

	<div class="stage">
		{#if closed}
			<p class="closed-note">Graph panel closed — reopen above.</p>
		{:else}
			<FocusGraph
				{nodes}
				rootKey={keyOf(1)}
				{currentKey}
				{pathKeys}
				{expandAll}
				onnavigate={navigate}
				onclose={() => (closed = true)}
			/>
		{/if}
	</div>

	<p class="foot">
		focus <strong>n = {currentNum}</strong>
		<span class="path">{pathTo(currentNum).join(' › ')}</span>
	</p>
</div>

<style>
	.wrap {
		min-height: 100vh;
		background: #161821;
		color: #c6c8d1;
		padding: 40px 32px 80px;
		font-family: var(--font-sans, system-ui, sans-serif);
	}
	header {
		max-width: 720px;
		margin-bottom: 22px;
	}
	h1 {
		font-size: 19px;
		font-weight: 700;
		margin: 0 0 8px;
		color: #e3e4e8;
	}
	.lede {
		font-size: 13px;
		line-height: 1.6;
		color: #9a9ca5;
		margin: 0;
	}
	code {
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 12px;
		background: #1c1e27;
		padding: 1px 5px;
		border-radius: 4px;
		color: #84a0c6;
	}
	.controls {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		margin-bottom: 14px;
	}
	.chk {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: #9a9ca5;
	}
	.sep {
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: #6b7089;
		margin-left: 8px;
	}
	.chip {
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 11px;
		padding: 3px 9px;
		background: #1c1e27;
		border: 1px solid #3d4455;
		border-radius: 5px;
		color: #c6c8d1;
		cursor: pointer;
	}
	.chip:hover {
		border-color: #84a0c6;
	}
	.chip.active {
		background: color-mix(in srgb, #84a0c6 22%, #1c1e27);
		border-color: #84a0c6;
		color: #e3e4e8;
	}
	.reopen {
		margin-left: auto;
	}
	.stage {
		border: 1px solid #3d4455;
		border-radius: 8px;
		overflow: hidden;
		background: #1c1e27;
	}
	.closed-note {
		padding: 40px;
		text-align: center;
		font-size: 12px;
		color: #6b7089;
		margin: 0;
	}
	.foot {
		margin-top: 14px;
		font-family: var(--font-mono, ui-monospace, monospace);
		font-size: 11px;
		color: #6b7089;
	}
	.foot strong {
		color: #84a0c6;
		font-weight: 700;
	}
	.foot .path {
		margin-left: 10px;
		color: #485163;
	}
</style>
