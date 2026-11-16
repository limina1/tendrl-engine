// Per-publication tree depth — the reader's `tree` knob, remembered.
//
// A buffer keeps its depth in its own view state, but a fresh open of the
// same publication used to start at the default 2 again. The depth the
// user last chose for a publication is kept here, per device: view state,
// never engine config, never an event. Only non-default depths are stored.
//
// Leaf module (no wm/ or state imports).

const STORAGE_KEY = 'tendrl.reader.depth';
/** Oldest entries go first past this many publications. */
const MAX_ENTRIES = 200;
export const DEFAULT_TREE_DEPTH = 2;

function read(): Record<string, number> {
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		const v = raw ? JSON.parse(raw) : null;
		return v && typeof v === 'object' ? v : {};
	} catch {
		return {};
	}
}

/** Remembered depth for a publication coordinate (`30040:<pubkey>:<d>`). */
export function rememberedDepth(coord: string): number | undefined {
	const d = read()[coord];
	return Number.isInteger(d) && d >= 1 ? d : undefined;
}

export function rememberDepth(coord: string, depth: number) {
	const all = read();
	delete all[coord]; // re-insert last = most recent
	if (depth !== DEFAULT_TREE_DEPTH) all[coord] = depth;
	const keys = Object.keys(all);
	for (const k of keys.slice(0, Math.max(0, keys.length - MAX_ENTRIES))) delete all[k];
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify(all));
	} catch {
		// Storage unavailable — the buffer's own view state still holds it.
	}
}
