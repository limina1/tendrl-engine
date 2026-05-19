<script lang="ts">
	import type { NAddr } from '$lib/types';

	/** A 30040 node in the publication reference graph. `childKeys` are the
	 *  addr-keys of the nested 30040 indexes it references (30041 leaves are
	 *  not graph nodes). */
	export type GraphNode = { addr: NAddr; title: string | null; childKeys: string[] };

	let {
		nodes,
		rootKey,
		currentKey,
		pathKeys = [],
		onnavigate,
		onclose
	}: {
		nodes: Record<string, GraphNode>;
		/** addr-key of the buffer's own publication — the layout root. */
		rootKey: string;
		/** addr-key of the publication currently in focus. */
		currentKey: string;
		/** addr-keys of the breadcrumb path up to the current focus. */
		pathKeys?: string[];
		onnavigate: (addr: NAddr) => void;
		onclose: () => void;
	} = $props();

	// Depth runs horizontally — one column per level; the indexes within a
	// level stack vertically. COL_W is the horizontal pitch between levels,
	// ROW_H the vertical pitch between sibling indexes.
	const COL_W = 168;
	const ROW_H = 60;
	const NODE_W = 134;
	const NODE_H = 38;

	const pathSet = $derived(new Set(pathKeys));

	// Lay the graph out left-to-right: BFS depth from the root sets the
	// column, the indexes within a level stack down the column. Edges to a
	// shallower-or-equal depth are back-edges — the cycles the loader's
	// guard stops at.
	const layout = $derived.by(() => {
		const depth = new Map<string, number>();
		const order: string[] = [];
		const queue: string[] = [];
		if (nodes[rootKey]) {
			depth.set(rootKey, 0);
			queue.push(rootKey);
		}
		while (queue.length) {
			const k = queue.shift()!;
			order.push(k);
			const n = nodes[k];
			if (!n) continue;
			for (const ck of n.childKeys) {
				if (!depth.has(ck) && nodes[ck]) {
					depth.set(ck, (depth.get(k) ?? 0) + 1);
					queue.push(ck);
				}
			}
		}
		// Nodes never reached from the root (disjoint) land on a trailing row.
		let maxD = 0;
		for (const d of depth.values()) maxD = Math.max(maxD, d);
		for (const k of Object.keys(nodes)) {
			if (!depth.has(k)) {
				depth.set(k, maxD + 1);
				order.push(k);
			}
		}
		const byDepth = new Map<number, string[]>();
		for (const k of order) {
			const d = depth.get(k)!;
			(byDepth.get(d) ?? byDepth.set(d, []).get(d)!).push(k);
		}
		const pos = new Map<string, { x: number; y: number }>();
		let maxLevel = 1;
		for (const [d, keys] of byDepth) {
			maxLevel = Math.max(maxLevel, keys.length);
			keys.forEach((k, i) => {
				// Depth → column (x); position within the level → row (y).
				pos.set(k, { x: d * COL_W + COL_W / 2, y: i * ROW_H + ROW_H / 2 });
			});
		}
		const edges: { from: string; to: string; back: boolean }[] = [];
		for (const k of Object.keys(nodes)) {
			const fd = depth.get(k) ?? 0;
			for (const ck of nodes[k].childKeys) {
				if (!pos.has(ck)) continue;
				edges.push({ from: k, to: ck, back: (depth.get(ck) ?? 0) <= fd });
			}
		}
		return {
			pos,
			edges,
			width: byDepth.size * COL_W,
			height: maxLevel * ROW_H
		};
	});

	function label(k: string): string {
		const n = nodes[k];
		const t = (n?.title || n?.addr.d_tag || 'index').trim();
		return t.length > 17 ? t.slice(0, 16) + '…' : t;
	}

	function edgePath(e: { from: string; to: string }): string {
		const a = layout.pos.get(e.from)!;
		const b = layout.pos.get(e.to)!;
		const mx = (a.x + b.x) / 2;
		return `M ${a.x} ${a.y} C ${mx} ${a.y}, ${mx} ${b.y}, ${b.x} ${b.y}`;
	}
</script>

<div class="fg">
	<div class="fg__head">
		<span class="fg__title">Reference graph</span>
		<span class="fg__hint">click a node to refocus · ↻ = cyclic reference</span>
		<button class="fg__close" onclick={onclose} title="Close the graph panel">✕</button>
	</div>
	<div class="fg__body">
		{#if layout.pos.size === 0}
			<p class="fg__empty">No nested indexes loaded yet.</p>
		{:else}
			<svg
				class="fg__svg"
				width={layout.width}
				height={layout.height}
				viewBox="0 0 {layout.width} {layout.height}"
			>
				{#each layout.edges as e (e.from + '->' + e.to)}
					<path class="fg__edge" class:fg__edge--back={e.back} d={edgePath(e)} />
				{/each}
				{#each [...layout.pos] as [k, p] (k)}
					{@const node = nodes[k]}
					<g
						class="fg__node"
						class:fg__node--current={k === currentKey}
						class:fg__node--path={k !== currentKey && pathSet.has(k)}
						class:fg__node--stub={!node?.title}
						role="button"
						tabindex="0"
						onclick={() => node && onnavigate(node.addr)}
						onkeydown={(ev) => {
							if ((ev.key === 'Enter' || ev.key === ' ') && node) {
								ev.preventDefault();
								onnavigate(node.addr);
							}
						}}
					>
						<rect
							x={p.x - NODE_W / 2}
							y={p.y - NODE_H / 2}
							width={NODE_W}
							height={NODE_H}
							rx="6"
						/>
						<text x={p.x} y={p.y} dominant-baseline="central" text-anchor="middle"
							>{label(k)}</text
						>
					</g>
				{/each}
			</svg>
		{/if}
	</div>
</div>

<style>
	.fg {
		display: flex;
		flex-direction: column;
		max-height: 320px;
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		flex-shrink: 0;
	}
	.fg__head {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 5px var(--s-3);
		border-bottom: 1px solid var(--panel-border);
	}
	.fg__title {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.07em;
		color: var(--id-yours);
	}
	.fg__hint {
		font-size: 9px;
		color: var(--base5);
	}
	.fg__close {
		margin-left: auto;
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-sm);
		line-height: 1;
		cursor: pointer;
		padding: 0 4px;
	}
	.fg__close:hover { color: var(--fg); }
	.fg__body {
		overflow: auto;
		padding: 12px;
	}
	.fg__empty {
		color: var(--base5);
		font-size: var(--t-xs);
		margin: 8px;
	}
	.fg__svg { display: block; }

	.fg__edge {
		fill: none;
		stroke: var(--base4);
		stroke-width: 1.5;
	}
	/* Back-edge — a 30040 referencing one of its ancestors (a cycle). */
	.fg__edge--back {
		stroke: var(--id-yours);
		stroke-dasharray: 4 3;
		opacity: 0.7;
	}

	.fg__node { cursor: pointer; }
	.fg__node rect {
		fill: var(--panel-bg);
		stroke: var(--base4);
		stroke-width: 1.5;
	}
	.fg__node text {
		fill: var(--fg);
		font-family: var(--font-mono);
		font-size: 11px;
	}
	.fg__node:hover rect { stroke: var(--id-yours); }
	.fg__node--stub rect { stroke-dasharray: 3 3; }
	.fg__node--stub text { fill: var(--base5); }
	/* On the breadcrumb path to the current focus. */
	.fg__node--path rect {
		stroke: var(--id-yours);
		fill: color-mix(in srgb, var(--id-yours) 10%, var(--panel-bg));
	}
	/* The current focus. */
	.fg__node--current rect {
		stroke: var(--id-yours);
		stroke-width: 2.5;
		fill: color-mix(in srgb, var(--id-yours) 24%, var(--panel-bg));
	}
	.fg__node--current text { fill: var(--base6); font-weight: 700; }
</style>
