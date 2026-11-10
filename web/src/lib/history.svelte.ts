// Reading / listening history store. The engine owns the file
// (`/api/v1/history`); this module keeps a reactive mirror, debounces the
// reader's section changes and throttles the player's position ticks so
// the engine sees a write every few seconds at most, and applies each
// update optimistically so the feed's history list moves at once.
import * as api from '$lib/api';

let entries = $state<api.HistoryEntry[]>([]);
let loaded = $state(false);
let loading: Promise<void> | null = null;

export function all(): api.HistoryEntry[] {
	return entries;
}

export function entryFor(coord: string): api.HistoryEntry | null {
	return entries.find((e) => e.coord === coord) ?? null;
}

export function isLoaded(): boolean {
	return loaded;
}

export function ensureLoaded(): Promise<void> {
	if (loaded) return Promise.resolve();
	if (!loading) {
		loading = api
			.getHistory()
			.then((r) => {
				entries = r.entries;
				loaded = true;
			})
			.catch((e) => console.warn('[history] load failed', e))
			.finally(() => (loading = null));
	}
	return loading;
}

export async function reload(): Promise<void> {
	loaded = false;
	await ensureLoaded();
}

/** `30040:<64hex>:<d>` → parts (d may itself contain colons). */
export function parseCoord(coord: string): { kind: number; pubkey: string; d_tag: string } | null {
	const m = coord.match(/^(\d+):([0-9a-f]{64}):(.*)$/);
	return m ? { kind: Number(m[1]), pubkey: m[2], d_tag: m[3] } : null;
}

function now(): number {
	return Math.floor(Date.now() / 1000);
}

/** Optimistic local merge, mirroring the engine's `history::apply`. */
function applyLocal(up: api.HistoryUpdate): void {
	const i = entries.findIndex((e) => e.coord === up.coord);
	const prev = i >= 0 ? entries[i] : null;
	const next: api.HistoryEntry = {
		coord: up.coord,
		title: up.title ?? prev?.title ?? null,
		author: up.author ?? prev?.author ?? parseCoord(up.coord)?.pubkey ?? '',
		section: up.clear_section ? undefined : (up.section ?? prev?.section),
		audio: up.clear_audio ? undefined : (up.audio ?? prev?.audio),
		updated_at: now()
	};
	const rest = i >= 0 ? [...entries.slice(0, i), ...entries.slice(i + 1)] : entries.slice();
	entries = [next, ...rest];
}

async function push(up: api.HistoryUpdate): Promise<void> {
	applyLocal(up);
	try {
		const r = await api.updateHistory(up);
		const i = entries.findIndex((e) => e.coord === r.entry.coord);
		if (i >= 0) entries[i] = r.entry;
	} catch (e) {
		console.warn('[history] update failed', e);
	}
}

const readTimers = new Map<string, ReturnType<typeof setTimeout>>();

/** The reader moved to a section. Debounced per publication (1.5 s) so a
 *  fast page-through writes once. */
export function recordRead(pubCoord: string, meta: { title: string | null; author: string }, section?: api.ReadPlace): void {
	const prev = readTimers.get(pubCoord);
	if (prev) clearTimeout(prev);
	readTimers.set(
		pubCoord,
		setTimeout(() => {
			readTimers.delete(pubCoord);
			void push({ coord: pubCoord, title: meta.title, author: meta.author, section });
		}, 1500)
	);
}

const lastListen = new Map<string, number>();

/** The player ticked. Throttled per publication (5 s) unless `flush`
 *  (pause, track change, stop, page hide). */
export function recordListen(pubCoord: string, meta: { title: string | null; author: string }, audio: api.ListenPlace, flush = false): void {
	const t = Date.now();
	if (!flush && t - (lastListen.get(pubCoord) ?? 0) < 5000) return;
	lastListen.set(pubCoord, t);
	void push({ coord: pubCoord, title: meta.title, author: meta.author, audio });
}

/** The book finished: forget the audio place so the next listen starts over. */
export function clearListen(pubCoord: string): void {
	lastListen.delete(pubCoord);
	void push({ coord: pubCoord, clear_audio: true });
}

/** What an entry is, from its coordinate's kind. */
export function kindLabel(coord: string): string {
	switch (parseCoord(coord)?.kind) {
		case 30040:
			return 'publication';
		case 30023:
			return 'article';
		case 30818:
			return 'wiki';
		case 30041:
			return 'section';
		default:
			return 'event';
	}
}

export async function remove(coord: string): Promise<void> {
	entries = coord === '*' ? [] : entries.filter((e) => e.coord !== coord);
	try {
		await api.removeHistory(coord);
	} catch (e) {
		console.warn('[history] remove failed', e);
	}
}

export function fmtWhen(ts: number): string {
	const d = Date.now() / 1000 - ts;
	if (d < 60) return 'just now';
	if (d < 3600) return `${Math.floor(d / 60)} min ago`;
	if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
	if (d < 7 * 86400) return `${Math.floor(d / 86400)} d ago`;
	return new Date(ts * 1000).toLocaleDateString();
}
