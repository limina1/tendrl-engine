// Narration — recordings discovered per publication + the one global
// player. Module-level $state (no context): buffer renderers may not use
// Svelte lifecycle/context APIs (see project_wm_buffer_render_constraints),
// and the player outlives any buffer — audio keeps playing across buffer
// switches, like a music app.
//
// Engine owns discovery + ranking (`GET /narration/for`): this store only
// caches the result per publication coordinate and holds playback view
// state (playlist, index, playing, position). Recording choice per node is
// view state too — the first ranked recording unless the user picked one.
import * as api from '$lib/api';
import * as history from '$lib/history.svelte';

export interface PlaylistItem {
	/** Node coordinate (30041:pk:d) this item narrates. */
	coord: string;
	title: string;
	recording: api.NarrationRecording;
	/** Which `b` piece is playing (a long section may span several blobs). */
	piece: number;
	/** Which URL of that piece's candidates is in use (fallback cursor). */
	urlIndex: number;
}

function pieceUrls(rec: api.NarrationRecording): string[][] {
	return rec.piece_urls?.length ? rec.piece_urls : rec.urls.length ? [rec.urls] : [];
}

const recordingsByPub = $state<Record<string, api.NarrationDiscovery>>({});
const loadingByPub = $state<Record<string, boolean>>({});
/** User-picked recording id per node coordinate (else the top-ranked). */
const chosen = $state<Record<string, string>>({});

// Declared before `player`: its initializer reads the persisted rate.
const RATE_KEY = 'tendrl.narration.rate';
function loadRate(): number {
	try {
		const v = Number(localStorage.getItem(RATE_KEY));
		return v > 0 && v <= 4 ? v : 1;
	} catch {
		return 1;
	}
}

const player = $state({
	playlist: [] as PlaylistItem[],
	index: -1,
	playing: false,
	position: 0,
	duration: 0,
	/** Publication coordinate the playlist came from — the strip's title link. */
	pubCoord: null as string | null,
	pubTitle: null as string | null,
	error: null as string | null,
	/** Set by the player component; drives seek/pause from outside. */
	rate: loadRate(),
	/** Seconds to seek to once the current item's metadata loads (resume). */
	pendingSeek: null as number | null,
	/** A seek the controls asked for; the audio element applies and clears it. */
	seekTo: null as number | null
});

export function seek(secs: number) {
	if (Number.isFinite(secs)) player.seekTo = Math.max(0, secs);
}

const RATES = [0.8, 1, 1.25, 1.5, 1.75, 2];
export function cycleRate() {
	const i = RATES.indexOf(player.rate);
	setRate(RATES[(i + 1) % RATES.length]);
}

/** Stop and close: flush the listening place first so ▶ resumes here. */
export function close() {
	const it = current();
	const pc = player.pubCoord;
	if (it && pc) {
		const parts = history.parseCoord(pc);
		history.recordListen(
			pc,
			{ title: player.pubTitle, author: parts?.pubkey ?? '' },
			{
				coord: it.coord,
				title: it.title,
				recording_id: it.recording.id,
				piece: it.piece,
				position_secs: player.position,
				duration_secs: player.duration || it.recording.duration_secs || 0
			},
			true
		);
	}
	stop();
}


export function pubKey(pubkey: string, d_tag: string): string {
	return `30040:${pubkey}:${d_tag}`;
}

export function discoveryFor(pubkey: string, d_tag: string): api.NarrationDiscovery | null {
	return recordingsByPub[pubKey(pubkey, d_tag)] ?? null;
}

export function isLoading(pubkey: string, d_tag: string): boolean {
	return loadingByPub[pubKey(pubkey, d_tag)] ?? false;
}

/** Recordings for one node coordinate, from whichever publication has it
 *  cached, else from a one-node discovery (`refreshNode`). */
export function recordingsFor(coord: string): api.NarrationRecording[] {
	for (const d of Object.values(recordingsByPub)) {
		const n = d.nodes.find((x) => x.coord === coord);
		if (n) return n.recordings;
	}
	return recordingsByNode[coord]?.nodes[0]?.recordings ?? [];
}

// One-node discovery: a document opened outside a publication tree (an
// article, a wiki page, a spec, a section on its own).
const recordingsByNode = $state<Record<string, api.NarrationDiscovery>>({});
const loadingByNode = $state<Record<string, boolean>>({});

export function isNodeLoading(coord: string): boolean {
	return loadingByNode[coord] ?? false;
}

export async function refreshNode(
	coord: string,
	policy: 'local_only' | 'local_first' | 'fetch_always' = 'local_only'
): Promise<api.NarrationDiscovery | null> {
	const m = coord.match(/^(\d+):([0-9a-f]{64}):(.*)$/);
	if (!m) return null;
	loadingByNode[coord] = true;
	try {
		const d = await api.getNarrationNode(Number(m[1]), m[2], m[3], policy);
		recordingsByNode[coord] = d;
		return d;
	} catch (e) {
		console.warn('[narration] node discovery failed', e);
		return null;
	} finally {
		loadingByNode[coord] = false;
	}
}

/** The recording that plays for `coord`: the user's pick, else the top rank. */
export function activeRecording(coord: string): api.NarrationRecording | null {
	const list = recordingsFor(coord);
	if (list.length === 0) return null;
	const id = chosen[coord];
	return (id && list.find((r) => r.id === id)) || list[0];
}

export function choose(coord: string, recordingId: string) {
	chosen[coord] = recordingId;
	// Swap it into the live playlist too, if that node is queued.
	const item = player.playlist.find((p) => p.coord === coord);
	const rec = recordingsFor(coord).find((r) => r.id === recordingId);
	if (item && rec) {
		item.recording = rec;
		item.piece = 0;
		item.urlIndex = 0;
	}
}

/** Total narrated nodes / recordings across the cached tree of a publication. */
export function summary(pubkey: string, d_tag: string): { sections: number; narrated: number; recordings: number } {
	const d = discoveryFor(pubkey, d_tag);
	if (!d) return { sections: 0, narrated: 0, recordings: 0 };
	const leaves = d.nodes.filter((n) => !n.is_index);
	return {
		sections: leaves.length,
		narrated: leaves.filter((n) => n.recordings.length > 0).length,
		// Section recordings only — the root's own 32176 is a container, not
		// something to listen to on its own.
		recordings: leaves.reduce((a, n) => a + n.recordings.length, 0)
	};
}

export async function refresh(
	pubkey: string,
	d_tag: string,
	policy: 'local_only' | 'local_first' | 'fetch_always' = 'local_only'
): Promise<api.NarrationDiscovery | null> {
	const key = pubKey(pubkey, d_tag);
	loadingByPub[key] = true;
	try {
		const d = await api.getNarrations(pubkey, d_tag, policy);
		recordingsByPub[key] = d;
		return d;
	} catch (e) {
		console.warn('[narration] discovery failed', e);
		return null;
	} finally {
		loadingByPub[key] = false;
	}
}

/** Build the play-all list for a publication from its cached discovery:
 *  every leaf with a recording, in tree order. */
export function playlistFor(pubkey: string, d_tag: string): PlaylistItem[] {
	const d = discoveryFor(pubkey, d_tag);
	if (!d) return [];
	const out: PlaylistItem[] = [];
	for (const n of d.nodes) {
		if (n.is_index) continue;
		const rec = activeRecording(n.coord);
		if (rec && pieceUrls(rec).length > 0) out.push({ coord: n.coord, title: n.title, recording: rec, piece: 0, urlIndex: 0 });
	}
	return out;
}

/** Start playing: the whole publication from `startCoord` (or its first
 *  narrated section). */
export function playPublication(
	pubkey: string,
	d_tag: string,
	pubTitle: string | null,
	startCoord?: string,
	start?: { piece?: number; position?: number }
) {
	const list = playlistFor(pubkey, d_tag);
	if (list.length === 0) return false;
	let idx = startCoord ? list.findIndex((p) => p.coord === startCoord) : 0;
	if (idx < 0) idx = 0;
	const item = list[idx];
	const pieces = pieceUrls(item.recording).length;
	item.piece = start?.piece !== undefined && start.piece < pieces ? start.piece : 0;
	player.playlist = list;
	player.index = idx;
	player.pubCoord = pubKey(pubkey, d_tag);
	player.pubTitle = pubTitle;
	player.error = null;
	player.position = start?.position ?? 0;
	player.pendingSeek = start?.position && start.position > 1 ? start.position : null;
	player.playing = true;
	return true;
}

/** Continue where the listener left off (history), else from the start.
 *  The remembered recording is re-chosen when it is still among the
 *  section's recordings, so the position lands on the same audio. */
export function resumeOrPlay(pubkey: string, d_tag: string, pubTitle: string | null) {
	const h = history.entryFor(pubKey(pubkey, d_tag));
	const a = h?.audio;
	if (a) {
		if (recordingsFor(a.coord).some((r) => r.id === a.recording_id)) choose(a.coord, a.recording_id);
		if (playPublication(pubkey, d_tag, pubTitle ?? h?.title ?? null, a.coord, { piece: a.piece, position: a.position_secs })) return true;
	}
	return playPublication(pubkey, d_tag, pubTitle);
}

/** Play one specific recording for a node (from the recordings panel).
 *  `opts.historyCoord` is the document coordinate the listen is recorded
 *  under (a 30023 / 30818 played on its own); `opts.position` resumes. */
export function playRecording(
	coord: string,
	title: string,
	rec: api.NarrationRecording,
	pubTitle: string | null,
	opts: { historyCoord?: string | null; piece?: number; position?: number } = {}
) {
	choose(coord, rec.id);
	const existing = player.playlist.findIndex((p) => p.coord === coord);
	if (existing >= 0 && player.pubTitle === pubTitle) {
		player.index = existing;
	} else {
		player.playlist = [{ coord, title, recording: rec, piece: 0, urlIndex: 0 }];
		player.index = 0;
		player.pubCoord = opts.historyCoord ?? null;
		player.pubTitle = pubTitle;
	}
	const item = player.playlist[player.index];
	const pieces = pieceUrls(item.recording).length;
	item.piece = opts.piece !== undefined && opts.piece < pieces ? opts.piece : 0;
	player.error = null;
	player.position = opts.position ?? 0;
	player.pendingSeek = opts.position && opts.position > 1 ? opts.position : null;
	player.playing = true;
}

/** Play a document's recording, resuming where its history entry left off
 *  when the remembered recording is still the one bound to it. */
export function resumeOrPlayRecording(
	coord: string,
	title: string,
	rec: api.NarrationRecording,
	pubTitle: string | null,
	historyCoord: string | null
) {
	const h = historyCoord ? history.entryFor(historyCoord) : null;
	const a = h?.audio;
	if (a && a.coord === coord && a.recording_id === rec.id) {
		playRecording(coord, title, rec, pubTitle, { historyCoord, piece: a.piece, position: a.position_secs });
		return;
	}
	playRecording(coord, title, rec, pubTitle, { historyCoord });
}

/** Play an explicit chapter list (a kind-32176 opened as a book), from
 *  `startIndex`. `historyCoord` = the publication coordinate the listen is
 *  remembered under (null = not remembered). */
export function playChapters(
	items: { coord: string; title: string; recording: api.NarrationRecording }[],
	pubTitle: string | null,
	startIndex = 0,
	historyCoord: string | null = null,
	start?: { piece?: number; position?: number }
) {
	const list: PlaylistItem[] = items
		.filter((i) => pieceUrls(i.recording).length > 0)
		.map((i) => ({ coord: i.coord, title: i.title, recording: i.recording, piece: 0, urlIndex: 0 }));
	if (list.length === 0) return false;
	let idx = Math.min(Math.max(startIndex, 0), list.length - 1);
	// `startIndex` counts over `items`; map it onto the playable subset.
	if (items.length !== list.length) {
		const want = items[startIndex]?.coord;
		const found = list.findIndex((p) => p.coord === want);
		idx = found >= 0 ? found : 0;
	}
	for (const it of list) choose(it.coord, it.recording.id);
	const item = list[idx];
	const pieces = pieceUrls(item.recording).length;
	item.piece = start?.piece !== undefined && start.piece < pieces ? start.piece : 0;
	player.playlist = list;
	player.index = idx;
	player.pubCoord = historyCoord;
	player.pubTitle = pubTitle;
	player.error = null;
	player.position = start?.position ?? 0;
	player.pendingSeek = start?.position && start.position > 1 ? start.position : null;
	player.playing = true;
	return true;
}

export function current(): PlaylistItem | null {
	return player.playlist[player.index] ?? null;
}

export function currentUrl(): string | null {
	const c = current();
	if (!c) return null;
	return pieceUrls(c.recording)[c.piece]?.[c.urlIndex] ?? null;
}

/** Pieces of the current item: (index, count). */
export function currentPiece(): { piece: number; pieces: number } {
	const c = current();
	return c ? { piece: c.piece, pieces: pieceUrls(c.recording).length } : { piece: 0, pieces: 0 };
}

export function toggle() {
	if (player.index < 0) return;
	player.playing = !player.playing;
}

export function pause() {
	player.playing = false;
}

/** Advance: the next piece of the same section first, then the next section. */
export function next(): boolean {
	const c = current();
	if (c && c.piece + 1 < pieceUrls(c.recording).length) {
		c.piece += 1;
		c.urlIndex = 0;
		player.position = 0;
		player.error = null;
		return true;
	}
	if (player.index + 1 < player.playlist.length) {
		player.index += 1;
		const n = current();
		if (n) {
			n.piece = 0;
			n.urlIndex = 0;
		}
		player.position = 0;
		player.error = null;
		return true;
	}
	player.playing = false;
	return false;
}

export function prev() {
	const c = current();
	if (c && c.piece > 0) {
		c.piece = 0;
		c.urlIndex = 0;
		player.position = 0;
		player.error = null;
		return;
	}
	if (player.index > 0) {
		player.index -= 1;
		const n = current();
		if (n) {
			n.piece = 0;
			n.urlIndex = 0;
		}
		player.position = 0;
		player.error = null;
	}
}

/** The current URL failed: try the next candidate server, else skip ahead. */
export function failover(): 'retry' | 'next' | 'stop' {
	const c = current();
	if (!c) return 'stop';
	if (c.urlIndex + 1 < (pieceUrls(c.recording)[c.piece]?.length ?? 0)) {
		c.urlIndex += 1;
		return 'retry';
	}
	player.error = `no server answered for "${c.title}"`;
	return next() ? 'next' : 'stop';
}

export function stop() {
	player.playing = false;
	player.pendingSeek = null;
	player.seekTo = null;
	player.playlist = [];
	player.index = -1;
	player.position = 0;
	player.duration = 0;
	player.pubCoord = null;
	player.pubTitle = null;
	player.error = null;
}

export function isPlayingCoord(coord: string): boolean {
	return player.playing && current()?.coord === coord;
}

export function setRate(rate: number) {
	player.rate = rate;
	try {
		localStorage.setItem(RATE_KEY, String(rate));
	} catch {
		// private mode / blocked storage: the rate still holds for the session
	}
}

export { player };

export function fmtTime(s: number): string {
	if (!Number.isFinite(s) || s < 0) return '0:00';
	const m = Math.floor(s / 60);
	const r = Math.floor(s % 60);
	const h = Math.floor(m / 60);
	return h > 0 ? `${h}:${String(m % 60).padStart(2, '0')}:${String(r).padStart(2, '0')}` : `${m}:${String(r).padStart(2, '0')}`;
}

export function fmtBytes(b: number): string {
	if (b < 1024 * 1024) return `${Math.max(1, Math.round(b / 1024))} KiB`;
	return `${(b / (1024 * 1024)).toFixed(b < 10 * 1024 * 1024 ? 1 : 0)} MiB`;
}

export function matchLabel(m: api.NarrationVersionMatch): string {
	return m === 'exact' ? 'this version' : m === 'other_version' ? 'other version' : 'unpinned';
}
