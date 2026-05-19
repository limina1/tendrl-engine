<script lang="ts">
	import { untrack } from 'svelte';
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
	const TAB_W = 22;
	const TAB_H = 15;
	// How far below the node band a back-edge bows, so a cycle routes
	// clear of the indexes between its source and target.
	const BACK_DIP = 54;

	const pathSet = $derived(new Set(pathKeys));
	const hasChildren = (k: string) => (nodes[k]?.childKeys.length ?? 0) > 0;

	// Which index nodes are unpacked. Collapsed by default — a node's child
	// indexes show only once it (and its ancestors) are expanded.
	let expanded = $state(new Set<string>());

	// Full-graph BFS, collapse-blind: gives every node a structural parent so
	// the path to the current focus can be auto-unpacked.
	const fullParent = $derived.by(() => {
		const parent = new Map<string, string>();
		const seen = new Set<string>([rootKey]);
		const queue: string[] = nodes[rootKey] ? [rootKey] : [];
		while (queue.length) {
			const k = queue.shift()!;
			for (const ck of nodes[k]?.childKeys ?? []) {
				if (!nodes[ck] || seen.has(ck)) continue;
				seen.add(ck);
				parent.set(ck, k);
				queue.push(ck);
			}
		}
		return parent;
	});

	// Keep the root and the chain of ancestors above the current focus
	// unpacked, so refocusing always reveals where the reader is. Everything
	// off that path stays collapsed until the reader opens it.
	$effect(() => {
		currentKey;
		rootKey;
		fullParent;
		untrack(() => {
			const next = new Set(expanded);
			next.add(rootKey);
			let cur: string | undefined = currentKey;
			let guard = 0;
			while (cur && guard++ < 64) {
				const p = fullParent.get(cur);
				if (!p) break;
				next.add(p);
				cur = p;
			}
			if (next.size !== expanded.size) expanded = next;
		});
	});

	function toggle(k: string) {
		const next = new Set(expanded);
		if (next.has(k)) next.delete(k);
		else next.add(k);
		expanded = next;
	}

	// Lay the graph out left-to-right: BFS from the root sets the column,
	// the indexes within a level stack down the column. A collapsed node is
	// a dead end — its children are not laid out. Edges to a shallower-or-
	// equal depth are back-edges — the cycles the loader's guard stops at.
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
			if (!expanded.has(k)) continue; // collapsed — children stay packed
			for (const ck of nodes[k]?.childKeys ?? []) {
				if (!nodes[ck] || depth.has(ck)) continue;
				depth.set(ck, (depth.get(k) ?? 0) + 1);
				queue.push(ck);
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
		for (const k of pos.keys()) {
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
			height: maxLevel * ROW_H,
			hasBack: edges.some((e) => e.back)
		};
	});

	function label(k: string): string {
		const n = nodes[k];
		const t = (n?.title || n?.addr.d_tag || 'index').trim();
		return t.length > 17 ? t.slice(0, 16) + '…' : t;
	}

	function edgePath(e: { from: string; to: string; back: boolean }): string {
		const a = layout.pos.get(e.from)!;
		const b = layout.pos.get(e.to)!;
		if (e.back) {
			// Cycle edge: bow it below the node band so it never crosses the
			// indexes sitting between source and target. Leaves the source's
			// bottom, dips under everything, rises into the target's bottom.
			const ay = a.y + NODE_H / 2;
			const by = b.y + NODE_H / 2;
			const dip = layout.height + BACK_DIP * 0.72;
			return `M ${a.x} ${ay} C ${a.x} ${dip}, ${b.x} ${dip}, ${b.x} ${by}`;
		}
		// Forward edge: source's right edge → target's left edge.
		const ax = a.x + NODE_W / 2;
		const bx = b.x - NODE_W / 2;
		const mx = (ax + bx) / 2;
		return `M ${ax} ${a.y} C ${mx} ${a.y}, ${mx} ${b.y}, ${bx} ${b.y}`;
	}
</script>

<div class="fg">
	<div class="fg__head">
		<span class="fg__title">Reference graph</span>
		<span class="fg__hint"
			>click a node to refocus · ⊞ corner tab unpacks · green → forward,
			yellow ⤺ cycle</span
		>
		<button class="fg__close" onclick={onclose} title="Close the graph panel">✕</button>
	</div>
	<div class="fg__body">
		{#if layout.pos.size === 0}
			<p class="fg__empty">No nested indexes loaded yet.</p>
		{:else}
			{@const svgH = layout.height + (layout.hasBack ? BACK_DIP : 0)}
			<svg
				class="fg__svg"
				width={layout.width}
				height={svgH}
				viewBox="0 0 {layout.width} {svgH}"
			>
				<defs>
					<marker
						id="fg-arrow-fwd"
						markerUnits="userSpaceOnUse"
						markerWidth="10"
						markerHeight="10"
						refX="8.5"
						refY="5"
						orient="auto"
					>
						<path class="fg__arrowhead--fwd" d="M0 0 L10 5 L0 10 Z" />
					</marker>
					<marker
						id="fg-arrow-back"
						markerUnits="userSpaceOnUse"
						markerWidth="10"
						markerHeight="10"
						refX="8.5"
						refY="5"
						orient="auto"
					>
						<path class="fg__arrowhead--back" d="M0 0 L10 5 L0 10 Z" />
					</marker>
				</defs>
				{#each layout.edges as e (e.from + '->' + e.to)}
					<path
						class="fg__edge"
						class:fg__edge--back={e.back}
						class:fg__edge--fwd={!e.back}
						d={edgePath(e)}
						marker-end="url(#{e.back ? 'fg-arrow-back' : 'fg-arrow-fwd'})"
					/>
				{/each}
				{#each [...layout.pos] as [k, p] (k)}
					{@const node = nodes[k]}
					{@const expandable = hasChildren(k)}
					{@const open = expanded.has(k)}
					<g
						class="fg__node"
						class:fg__node--current={k === currentKey}
						class:fg__node--path={k !== currentKey && pathSet.has(k)}
						class:fg__node--stub={!node?.title}
					>
						<rect
							class="fg__box"
							x={p.x - NODE_W / 2}
							y={p.y - NODE_H / 2}
							width={NODE_W}
							height={NODE_H}
							rx="6"
							role="button"
							tabindex="0"
							aria-label="Refocus on {label(k)}"
							onclick={() => node && onnavigate(node.addr)}
							onkeydown={(ev) => {
								if ((ev.key === 'Enter' || ev.key === ' ') && node) {
									ev.preventDefault();
									onnavigate(node.addr);
								}
							}}
						/>
						<text
							class="fg__label"
							x={p.x}
							y={p.y}
							dominant-baseline="central"
							text-anchor="middle">{label(k)}</text
						>
						{#if expandable}
							<!-- Bottom-right corner tab: unpacks this index's
							     nested indexes. Collapsed by default. -->
							<g
								class="fg__tab"
								class:fg__tab--open={open}
								role="button"
								tabindex="0"
								aria-label="{open ? 'Collapse' : 'Expand'} {label(k)}"
								onclick={(ev) => {
									ev.stopPropagation();
									toggle(k);
								}}
								onkeydown={(ev) => {
									if (ev.key === 'Enter' || ev.key === ' ') {
										ev.preventDefault();
										ev.stopPropagation();
										toggle(k);
									}
								}}
							>
								<rect
									x={p.x + NODE_W / 2 - TAB_W}
									y={p.y + NODE_H / 2 - TAB_H}
									width={TAB_W}
									height={TAB_H}
									rx="3"
								/>
								<text
									x={p.x + NODE_W / 2 - TAB_W / 2}
									y={p.y + NODE_H / 2 - TAB_H / 2 + 0.5}
									dominant-baseline="central"
									text-anchor="middle">{open ? '−' : '+'}</text
								>
							</g>
						{/if}
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
		stroke-width: 1.7;
	}
	/* Forward edge — parent → nested child. */
	.fg__edge--fwd { stroke: var(--green); }
	.fg__arrowhead--fwd { fill: var(--green); }
	/* Back-edge — a 30040 referencing one of its ancestors (a cycle).
	   Routed in a bow below the node band so the loop stays visible. */
	.fg__edge--back {
		stroke: var(--yellow);
		stroke-dasharray: 5 3;
	}
	.fg__arrowhead--back { fill: var(--yellow); }

	.fg__box {
		fill: var(--panel-bg);
		stroke: var(--base4);
		stroke-width: 1.5;
		cursor: pointer;
	}
	.fg__label {
		fill: var(--fg);
		font-family: var(--font-mono);
		font-size: 11px;
		pointer-events: none;
	}
	.fg__node:hover .fg__box { stroke: var(--id-yours); }
	.fg__node--stub .fg__box { stroke-dasharray: 3 3; }
	.fg__node--stub .fg__label { fill: var(--base5); }
	/* On the breadcrumb path to the current focus. */
	.fg__node--path .fg__box {
		stroke: var(--id-yours);
		fill: color-mix(in srgb, var(--id-yours) 10%, var(--panel-bg));
	}
	/* The current focus. */
	.fg__node--current .fg__box {
		stroke: var(--id-yours);
		stroke-width: 2.5;
		fill: color-mix(in srgb, var(--id-yours) 24%, var(--panel-bg));
	}
	.fg__node--current .fg__label { fill: var(--base6); font-weight: 700; }

	/* Corner expand tab. */
	.fg__tab { cursor: pointer; }
	.fg__tab rect {
		fill: var(--panel-bg-soft);
		stroke: var(--base4);
		stroke-width: 1;
	}
	.fg__tab text {
		fill: var(--base5);
		font-family: var(--font-mono);
		font-size: 13px;
		font-weight: 700;
		pointer-events: none;
	}
	.fg__tab:hover rect { stroke: var(--id-yours); }
	.fg__tab:hover text { fill: var(--id-yours); }
	.fg__tab--open rect {
		fill: color-mix(in srgb, var(--id-yours) 18%, var(--panel-bg));
		stroke: var(--id-yours);
	}
	.fg__tab--open text { fill: var(--id-yours); }
</style>
