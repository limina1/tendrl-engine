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
		expandAll = false,
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
		/** Force every node unpacked — the corner tabs become no-ops.
		 *  Used by the design artboard to show the whole graph at once. */
		expandAll?: boolean;
		onnavigate: (addr: NAddr) => void;
		onclose: () => void;
	} = $props();

	// Depth runs horizontally — one column per level. COL_W is the column
	// pitch; NODE_W/H the node box.
	const COL_W = 168;
	const NODE_W = 134;
	const NODE_H = 38;
	const TAB_W = 22;
	const TAB_H = 15;
	// Cycles route as orthogonal wires through the inter-row gaps — every
	// back-edge runs in the gap beside its target row, and the gap grows to
	// fit the lanes packed into it. LANE_PITCH stacks lanes, GAP_PAD is the
	// clearance from a row to the nearest lane, GAP_MIN the height of an
	// empty gap, LANE_MARGIN the horizontal room two cycles need to share a
	// lane, CORNER_R the turn radius.
	const LANE_PITCH = 4;
	const GAP_PAD = 12;
	const GAP_MIN = 26;
	const LANE_MARGIN = 18;
	const CORNER_R = 7;
	// Where several cycles meet one node, their endpoints fan out into
	// separate ports across the node edge — PORT_PITCH is the step.
	const PORT_PITCH = 22;

	const pathSet = $derived(new Set(pathKeys));
	const hasChildren = (k: string) => (nodes[k]?.childKeys.length ?? 0) > 0;

	// Which index nodes are unpacked. Collapsed by default — a node's child
	// indexes show only once it (and its ancestors) are expanded.
	let expanded = $state(new Set<string>());

	// Hover state — the edge or node currently under the pointer. A lit edge
	// goes solid; everything else recedes so the one path stands out.
	let hoverEdge = $state<string | null>(null);
	let hoverNode = $state<string | null>(null);
	const hovering = $derived(hoverEdge !== null || hoverNode !== null);

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

	// Lay the graph out left-to-right: BFS sets each node's column; nodes
	// within a column stack into rows. Cycles (edges to a shallower-or-equal
	// depth) route as orthogonal wires through the inter-row gaps, and each
	// gap grows to fit the lanes packed into it.
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
			if (!expandAll && !expanded.has(k)) continue; // collapsed — children stay packed
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
		// Column + row index for every laid-out node.
		const col = new Map<string, number>();
		const row = new Map<string, number>();
		for (const [d, keys] of byDepth)
			keys.forEach((k, i) => {
				col.set(k, d);
				row.set(k, i);
			});
		const maxRow = Math.max(0, ...[...byDepth.values()].map((ks) => ks.length - 1));
		const colX = (k: string) => col.get(k)! * COL_W + COL_W / 2;

		const edges: { from: string; to: string; back: boolean }[] = [];
		for (const k of order) {
			const fd = depth.get(k) ?? 0;
			for (const ck of nodes[k].childKeys) {
				if (!depth.has(ck)) continue;
				edges.push({ from: k, to: ck, back: (depth.get(ck) ?? 0) <= fd });
			}
		}
		const key = (e: { from: string; to: string }) => e.from + '->' + e.to;
		const backEdges = edges.filter((e) => e.back);
		const sameRow = (e: { from: string; to: string }) => row.get(e.from) === row.get(e.to);

		// Same-row cycles alternate sides by x-span — the shortest below the
		// row, the next above, and so on.
		const srSide = new Map<string, 'above' | 'below'>();
		const srRows = new Map<number, typeof backEdges>();
		for (const e of backEdges) {
			if (!sameRow(e)) continue;
			const r = row.get(e.from)!;
			(srRows.get(r) ?? srRows.set(r, []).get(r)!).push(e);
		}
		const span = (e: { from: string; to: string }) => Math.abs(colX(e.from) - colX(e.to));
		for (const list of srRows.values()) {
			list.sort((a, b) => span(a) - span(b));
			list.forEach((e, i) => srSide.set(key(e), i % 2 === 0 ? 'below' : 'above'));
		}

		// Every cycle runs in the inter-row gap beside its target row, on the
		// side facing its source. The target attaches on that side; the
		// source attaches on the side facing the run between them.
		const sideOf = (e: { from: string; to: string }): 'above' | 'below' => {
			if (sameRow(e)) return srSide.get(key(e)) ?? 'below';
			return row.get(e.from)! > row.get(e.to)! ? 'below' : 'above';
		};
		const sourceEnd = (e: { from: string; to: string }): 'above' | 'below' => {
			const s = sideOf(e);
			if (sameRow(e)) return s;
			return s === 'below' ? 'above' : 'below';
		};
		// Gap id: `g${r}` is the gap below row r; 'top' above row 0, 'bot'
		// below the last row.
		const gapOf = (e: { from: string; to: string }): string => {
			const t = row.get(e.to)!;
			if (sideOf(e) === 'below') return t < maxRow ? `g${t}` : 'bot';
			return t > 0 ? `g${t - 1}` : 'top';
		};

		// Fan each node's cycle endpoints into separate ports spread across
		// the node edge, ordered by the opposite end's column so wires don't
		// cross — without this, many cycles converge on a single point.
		const portOf = new Map<string, number>();
		const portGroups = new Map<string, { port: string; oppX: number; tie: string }[]>();
		for (const e of backEdges) {
			const k = key(e);
			const tSide: 'top' | 'bottom' = sideOf(e) === 'below' ? 'bottom' : 'top';
			const sSide: 'top' | 'bottom' = sourceEnd(e) === 'below' ? 'bottom' : 'top';
			const ends: [string, 'from' | 'to', string, 'top' | 'bottom'][] = [
				[e.from, 'from', e.to, sSide],
				[e.to, 'to', e.from, tSide]
			];
			for (const [node, end, opp, side] of ends) {
				const gk = node + '|' + side;
				(portGroups.get(gk) ?? portGroups.set(gk, []).get(gk)!).push({
					port: k + '|' + end,
					oppX: colX(opp),
					tie: k
				});
			}
		}
		for (const [gk, list] of portGroups) {
			const cx = colX(gk.slice(0, gk.lastIndexOf('|')));
			list.sort((a, b) => a.oppX - b.oppX || (a.tie < b.tie ? -1 : 1));
			const n = list.length;
			const spread = Math.min(NODE_W - 30, n * PORT_PITCH);
			list.forEach((it, i) =>
				portOf.set(it.port, cx - spread / 2 + (spread * (i + 0.5)) / n)
			);
		}
		const portX = (e: { from: string; to: string }, end: 'from' | 'to') =>
			portOf.get(key(e) + '|' + end) ?? colX(end === 'from' ? e.from : e.to);

		// Pack each gap's cycles into stacked lanes: two whose x-spans don't
		// overlap share a lane (greedy interval colouring, widest first), so
		// the gap stays as shallow as it can.
		const laneOf = new Map<string, number>();
		const gapLanes = new Map<string, number>();
		const gapEdges = new Map<string, typeof backEdges>();
		for (const e of backEdges) {
			const g = gapOf(e);
			(gapEdges.get(g) ?? gapEdges.set(g, []).get(g)!).push(e);
		}
		for (const [g, list] of gapEdges) {
			const items = list
				.map((e) => {
					const lo = Math.min(portX(e, 'from'), portX(e, 'to'));
					const hi = Math.max(portX(e, 'from'), portX(e, 'to'));
					return { e, lo, hi };
				})
				.sort((x, y) => y.hi - y.lo - (x.hi - x.lo));
			const right: number[] = [];
			for (const { e, lo, hi } of items) {
				let lane = right.findIndex((r) => r + LANE_MARGIN < lo);
				if (lane < 0) {
					lane = right.length;
					right.push(0);
				}
				right[lane] = hi;
				laneOf.set(key(e), lane);
			}
			gapLanes.set(g, right.length);
		}

		// A gap's height follows its lane count; an empty inter-row gap keeps
		// GAP_MIN so rows never touch.
		const gapHeight = (g: string, inter: boolean) => {
			const n = gapLanes.get(g) ?? 0;
			if (n > 0) return 2 * GAP_PAD + (n - 1) * LANE_PITCH;
			return inter ? GAP_MIN : 0;
		};
		const rowY: number[] = [gapHeight('top', false) + NODE_H / 2];
		for (let r = 1; r <= maxRow; r++)
			rowY[r] = rowY[r - 1] + NODE_H + gapHeight(`g${r - 1}`, true);
		const height = rowY[maxRow] + NODE_H / 2 + gapHeight('bot', false);

		const pos = new Map<string, { x: number; y: number }>();
		for (const k of order) pos.set(k, { x: colX(k), y: rowY[row.get(k)!] });

		// Run-Y: the y of each cycle's horizontal segment — its lane within
		// its gap.
		const gapTopY = (g: string) => {
			if (g === 'top') return 0;
			if (g === 'bot') return rowY[maxRow] + NODE_H / 2;
			return rowY[Number(g.slice(1))] + NODE_H / 2;
		};
		const runY = new Map<string, number>();
		for (const e of backEdges)
			runY.set(
				key(e),
				gapTopY(gapOf(e)) + GAP_PAD + (laneOf.get(key(e)) ?? 0) * LANE_PITCH
			);

		return { pos, edges, runY, portOf, width: byDepth.size * COL_W, height };
	});

	function label(k: string): string {
		const n = nodes[k];
		const t = (n?.title || n?.addr.d_tag || 'index').trim();
		return t.length > 17 ? t.slice(0, 16) + '…' : t;
	}

	const ekey = (e: { from: string; to: string }) => e.from + '->' + e.to;

	// An endpoint's fanned-out x, or the node centre if it has no port.
	function portX(e: { from: string; to: string }, end: 'from' | 'to'): number {
		return (
			layout.portOf.get(ekey(e) + '|' + end) ??
			layout.pos.get(end === 'from' ? e.from : e.to)!.x
		);
	}

	// An edge is lit when it — or a node it touches — is hovered.
	function edgeLit(e: { from: string; to: string }): boolean {
		if (hoverEdge === ekey(e)) return true;
		return hoverNode !== null && (e.from === hoverNode || e.to === hoverNode);
	}

	function edgePath(e: { from: string; to: string; back: boolean }): string {
		const a = layout.pos.get(e.from)!;
		const b = layout.pos.get(e.to)!;
		if (e.back) {
			// Cycle: an orthogonal wire with rounded turns. It rises/drops
			// from the source into the gap beside the target row, runs level
			// there, and rises/drops into the target. Each end attaches at
			// its fanned-out port; the final segment is straight so the
			// arrowhead orients cleanly into the node.
			const ax = portX(e, 'from');
			const bx = portX(e, 'to');
			const L = layout.runY.get(ekey(e)) ?? a.y;
			// The node edge each end attaches to is whichever side faces the
			// run between them.
			const sEdge = a.y + Math.sign(L - a.y) * (NODE_H / 2);
			const tEdge = b.y + Math.sign(L - b.y) * (NODE_H / 2);
			const dir = bx < ax ? -1 : 1;
			const sSign = Math.sign(sEdge - L) || 1;
			const tSign = Math.sign(tEdge - L) || 1;
			const r = Math.max(
				2,
				Math.min(
					CORNER_R,
					Math.abs(bx - ax) / 2 - 1,
					Math.abs(L - sEdge),
					Math.abs(L - tEdge)
				)
			);
			return (
				`M ${ax} ${sEdge} L ${ax} ${L + sSign * r} ` +
				`Q ${ax} ${L} ${ax + dir * r} ${L} ` +
				`L ${bx - dir * r} ${L} ` +
				`Q ${bx} ${L} ${bx} ${L + tSign * r} ` +
				`L ${bx} ${tEdge}`
			);
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
			<svg
				class="fg__svg"
				width={layout.width}
				height={layout.height}
				viewBox="0 0 {layout.width} {layout.height}"
			>
				<defs>
					<marker
						id="fg-arrow-fwd"
						markerUnits="userSpaceOnUse"
						markerWidth="10"
						markerHeight="10"
						refX="10"
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
						refX="10"
						refY="5"
						orient="auto"
					>
						<path class="fg__arrowhead--back" d="M0 0 L10 5 L0 10 Z" />
					</marker>
				</defs>
				<!-- Hover layer: a fat invisible stroke under every edge,
				     easy to point at without hitting a hairline. -->
				{#each layout.edges as e (e.from + '->' + e.to)}
					<path
						class="fg__edge-hit"
						role="presentation"
						d={edgePath(e)}
						onmouseenter={() => (hoverEdge = e.from + '->' + e.to)}
						onmouseleave={() => (hoverEdge = null)}
					/>
				{/each}
				{#each layout.edges as e (e.from + '->' + e.to)}
					{@const lit = edgeLit(e)}
					<path
						class="fg__edge"
						class:fg__edge--back={e.back}
						class:fg__edge--fwd={!e.back}
						class:fg__edge--lit={lit}
						class:fg__edge--dim={hovering && !lit}
						d={edgePath(e)}
						marker-end="url(#{e.back ? 'fg-arrow-back' : 'fg-arrow-fwd'})"
					/>
				{/each}
				{#each [...layout.pos] as [k, p] (k)}
					{@const node = nodes[k]}
					{@const expandable = hasChildren(k)}
					{@const open = expandAll || expanded.has(k)}
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
							onmouseenter={() => (hoverNode = k)}
							onmouseleave={() => (hoverNode = null)}
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
		font-size: var(--t-3xs);
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
		opacity: 0.5;
		pointer-events: none;
		transition:
			opacity 0.12s ease,
			stroke-width 0.12s ease;
	}
	/* Invisible fat stroke that catches the pointer for hover. */
	.fg__edge-hit {
		fill: none;
		stroke: transparent;
		stroke-width: 14;
		pointer-events: stroke;
		cursor: pointer;
	}
	/* The hovered edge — or every edge on a hovered node — goes solid… */
	.fg__edge--lit {
		opacity: 1;
		stroke-width: 2.6;
	}
	/* …and the rest recede so the lit path stands out. */
	.fg__edge--dim {
		opacity: 0.14;
	}
	/* Forward edge — parent → nested child. */
	.fg__edge--fwd { stroke: var(--green); }
	.fg__arrowhead--fwd { fill: var(--green); }
	/* Back-edge — a 30040 referencing one of its ancestors (a cycle).
	   Routed through the inter-row gap beside its target row. */
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
		font-size: var(--t-xs);
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
		font-size: var(--t-base);
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
