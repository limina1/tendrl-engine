// Pointer-event drag for the table's free layout. Svelte action, no deps.
//
// Contract (docs/annotations-table-contract.org): the node moves itself
// while the pointer is down (style.left/top), the layout is NOT re-derived
// mid-drag, and `onEnd` commits the final position to view state (pins).
// A press that never crosses `threshold` px is a click, and a click that
// follows a real drag is swallowed so the card does not also expand.
//
//   <div use:draggable={{ enabled: mode === 'free', ignore: 'button, a, .acard__anchor',
//        onEnd: (x, y) => pin(card.id, x, y) }}>
//
// Touch: `touch-action: none` is set on the node while enabled so the
// browser does not scroll the table instead.

export interface DraggableOptions {
	enabled?: boolean;
	/** Elements inside the node that should not start a drag (buttons, links). */
	ignore?: string;
	/** Movement in px before a press becomes a drag (default 4). */
	threshold?: number;
	onStart?: () => void;
	onMove?: (x: number, y: number) => void;
	/** Fires once, after a real drag, with the node's final left/top in px. */
	onEnd?: (x: number, y: number) => void;
}

export function draggable(node: HTMLElement, opts: DraggableOptions = {}) {
	let o = opts;
	let dragging = false;
	let moved = false;
	let pointerId = -1;
	let x0 = 0, y0 = 0, left0 = 0, top0 = 0;

	function apply() {
		node.style.touchAction = o.enabled ? 'none' : '';
		node.classList.toggle('draggable', !!o.enabled);
	}

	function down(e: PointerEvent) {
		if (!o.enabled || e.button !== 0) return;
		if (o.ignore && (e.target as Element | null)?.closest(o.ignore)) return;
		dragging = true;
		moved = false;
		pointerId = e.pointerId;
		x0 = e.clientX;
		y0 = e.clientY;
		left0 = parseFloat(node.style.left) || 0;
		top0 = parseFloat(node.style.top) || 0;
		node.setPointerCapture(pointerId);
	}
	function move(e: PointerEvent) {
		if (!dragging || e.pointerId !== pointerId) return;
		const dx = e.clientX - x0, dy = e.clientY - y0;
		if (!moved) {
			if (Math.hypot(dx, dy) < (o.threshold ?? 4)) return;
			moved = true;
			node.classList.add('dragging');
			o.onStart?.();
		}
		const x = Math.max(0, left0 + dx), y = Math.max(0, top0 + dy);
		node.style.left = `${x}px`;
		node.style.top = `${y}px`;
		o.onMove?.(x, y);
	}
	function up(e: PointerEvent) {
		if (!dragging || e.pointerId !== pointerId) return;
		dragging = false;
		node.classList.remove('dragging');
		try { node.releasePointerCapture(pointerId); } catch { /* already released */ }
		if (moved) o.onEnd?.(parseFloat(node.style.left) || 0, parseFloat(node.style.top) || 0);
	}
	// A click after a drag would also fire the card's own onclick — swallow it.
	function click(e: MouseEvent) {
		if (moved) {
			e.stopPropagation();
			e.preventDefault();
			moved = false;
		}
	}

	node.addEventListener('pointerdown', down);
	node.addEventListener('pointermove', move);
	node.addEventListener('pointerup', up);
	node.addEventListener('pointercancel', up);
	node.addEventListener('click', click, true);
	apply();

	return {
		update(next: DraggableOptions) {
			o = next;
			apply();
		},
		destroy() {
			node.removeEventListener('pointerdown', down);
			node.removeEventListener('pointermove', move);
			node.removeEventListener('pointerup', up);
			node.removeEventListener('pointercancel', up);
			node.removeEventListener('click', click, true);
			node.style.touchAction = '';
		}
	};
}
