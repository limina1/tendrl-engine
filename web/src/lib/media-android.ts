// Android host bridge for the narration player. Inside the Tauri app the
// WebView's <audio> keeps playing, but Android suspends it the moment the
// activity leaves the foreground unless a mediaPlayback foreground service
// is running — the `media` plugin (mobile/src-tauri/src/media.rs +
// MediaService.kt) provides that, plus lock-screen / notification / headset
// controls which come back here as `tendrl:media` events. Off-device every
// call is a no-op.

export interface AndroidMediaState {
	playing: boolean;
	title: string;
	subtitle?: string;
	album?: string;
	/** seconds */
	position?: number;
	/** seconds */
	duration?: number;
	rate?: number;
	hasNext?: boolean;
	hasPrev?: boolean;
}

export type MediaAction = 'play' | 'pause' | 'next' | 'prev' | 'stop' | 'seek' | 'forward' | 'rewind';

interface TauriGlobal {
	core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
}

function tauri(): TauriGlobal | undefined {
	return typeof window === 'undefined' ? undefined : (window as unknown as { __TAURI__?: TauriGlobal }).__TAURI__;
}

/** True inside the Android app (the only Tauri host tendrl ships). */
export function available(): boolean {
	return tauri() !== undefined;
}

let lastKey = '';

/** Mirror the player into the system media session. Deduped on content so
 *  effects can call it freely; position is sent as-is (the session
 *  extrapolates from rate + timestamp between calls). */
export async function setState(s: AndroidMediaState): Promise<void> {
	const t = tauri();
	if (!t) return;
	const key = JSON.stringify([s.playing, s.title, s.subtitle, s.album, Math.round(s.position ?? 0), Math.round(s.duration ?? 0), s.rate, s.hasNext, s.hasPrev]);
	if (key === lastKey) return;
	lastKey = key;
	try {
		await t.core.invoke('plugin:media|set_state', {
			playing: s.playing,
			title: s.title,
			subtitle: s.subtitle ?? '',
			album: s.album ?? '',
			position: s.position ?? 0,
			duration: s.duration ?? 0,
			rate: s.rate ?? 1,
			hasNext: s.hasNext ?? false,
			hasPrev: s.hasPrev ?? false
		});
	} catch (e) {
		console.warn('[media] set_state failed', e);
	}
}

/** Player closed: drop the service + notification. */
export async function stop(): Promise<void> {
	const t = tauri();
	if (!t) return;
	lastKey = '';
	try {
		await t.core.invoke('plugin:media|stop');
	} catch (e) {
		console.warn('[media] stop failed', e);
	}
}

/** Transport buttons from the lock screen / notification / headset. */
export function onAction(cb: (action: MediaAction, position?: number) => void): () => void {
	if (typeof window === 'undefined') return () => {};
	const handler = (e: Event) => {
		const d = (e as CustomEvent<{ action?: MediaAction; position?: number }>).detail ?? {};
		if (d.action) cb(d.action, d.position);
	};
	window.addEventListener('tendrl:media', handler);
	return () => window.removeEventListener('tendrl:media', handler);
}
