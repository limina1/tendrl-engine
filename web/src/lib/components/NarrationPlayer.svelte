<script lang="ts">
	// The one audio player. A fixed strip above the modeline (bottom-left,
	// mirroring the toast stack on the right) that renders only while a
	// playlist is loaded. Audio bytes stream straight from the Blossom
	// server (BUD-01, open CORS, range requests); the engine never proxies
	// them. On a load error the store advances to the next candidate URL
	// for the same blob, then to the next section.
	import { untrack } from 'svelte';
	import * as nar from '$lib/narration.svelte';
	import * as amedia from '$lib/media-android';
	import * as history from '$lib/history.svelte';
	import { shell } from '$lib/wm/shell.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import NarrationControls from './NarrationControls.svelte';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';

	let audio: HTMLAudioElement | undefined = $state();
	let open = $state(true);

	const item = $derived(nar.current());
	const url = $derived(nar.currentUrl());
	const narrator = $derived(item ? getAuthorDisplayName(item.recording.pubkey) : '');
	/** The work pane is showing the reader of the loaded publication, which
	 *  renders the controls inline above its pager: keep the audio element,
	 *  hide the dock. Read straight off the buffer store — no counter, no
	 *  effect writes (a host-registration effect looped the batch). */
	const docked = $derived.by(() => {
		const pc = nar.player.pubCoord;
		if (!pc) return true;
		let id: string | null = null;
		try {
			const store = getActiveStore();
			const pos = store.findSlotForClass('work');
			if (pos && store.slotStates[pos] === 'open') id = store.focusedLeaf(pos)?.buffer.id ?? null;
		} catch {
			return true;
		}
		const m = id?.match(/^reader:30040:([0-9a-f]{64}):([^?]*)/);
		return !(m && `30040:${m[1]}:${m[2]}` === pc);
	});

	// Seeks asked for through the store (controls, inline or docked).
	$effect(() => {
		const v = nar.player.seekTo;
		if (v === null || !audio) return;
		nar.player.seekTo = null;
		if (Number.isFinite(audio.duration)) audio.currentTime = Math.min(v, audio.duration);
		else audio.currentTime = v;
	});

	// Drive the element from store state: src, play/pause, rate.
	$effect(() => {
		if (!audio) return;
		const u = url;
		if (!u) {
			audio.removeAttribute('src');
			audio.load();
			return;
		}
		if (audio.src !== u) {
			audio.src = u;
			audio.load();
			// load() resets playbackRate to defaultPlaybackRate — keep the
			// chosen speed across sections and pieces.
			audio.playbackRate = nar.player.rate;
		}
	});
	$effect(() => {
		if (!audio || !url) return;
		if (nar.player.playing) {
			audio.play().catch(() => {
				// Autoplay refused or the element is not ready yet; the
				// user's next click on ▶ retries.
				nar.player.playing = false;
			});
		} else {
			audio.pause();
		}
	});
	$effect(() => {
		if (!audio) return;
		audio.defaultPlaybackRate = nar.player.rate;
		audio.playbackRate = nar.player.rate;
	});

	function onLoadedMetadata() {
		if (!audio) return;
		audio.playbackRate = nar.player.rate;
		const seek = nar.player.pendingSeek;
		if (seek !== null) {
			nar.player.pendingSeek = null;
			if (Number.isFinite(audio.duration) && seek < audio.duration - 2) audio.currentTime = seek;
		}
	}

	// Listening history: where the player is in which publication, so
	// "listen" resumes there next time. Throttled in the store; flushed on
	// pause / track change / stop / page hide.
	function recordListen(flush = false) {
		const it = nar.current();
		const pc = nar.player.pubCoord;
		if (!it || !pc || !audio) return;
		const parts = history.parseCoord(pc);
		history.recordListen(
			pc,
			{ title: nar.player.pubTitle, author: parts?.pubkey ?? '' },
			{
				coord: it.coord,
				title: it.title,
				recording_id: it.recording.id,
				piece: it.piece,
				position_secs: audio.currentTime,
				duration_secs: audio.duration || it.recording.duration_secs || 0
			},
			flush
		);
	}
	$effect(() => {
		const onHide = () => recordListen(true);
		window.addEventListener('pagehide', onHide);
		document.addEventListener('visibilitychange', onHide);
		return () => {
			window.removeEventListener('pagehide', onHide);
			document.removeEventListener('visibilitychange', onHide);
		};
	});
	function closePlayer() {
		recordListen(true);
		nar.close();
	}

	// Media Session: lock-screen / notification controls + metadata where
	// the platform offers them (desktop browsers, Android WebView).
	$effect(() => {
		if (!('mediaSession' in navigator)) return;
		const ms = navigator.mediaSession;
		if (!item) {
			ms.metadata = null;
			return;
		}
		ms.metadata = new MediaMetadata({
			title: item.title,
			artist: narrator || item.recording.pubkey.slice(0, 12),
			album: nar.player.pubTitle ?? 'tendrl narration'
		});
		ms.setActionHandler('play', () => (nar.player.playing = true));
		ms.setActionHandler('pause', () => (nar.player.playing = false));
		ms.setActionHandler('previoustrack', () => nar.prev());
		ms.setActionHandler('nexttrack', () => void nar.next());
		ms.setActionHandler('seekbackward', (d) => {
			if (audio) audio.currentTime = Math.max(0, audio.currentTime - (d.seekOffset ?? 15));
		});
		ms.setActionHandler('seekforward', (d) => {
			if (audio) audio.currentTime = Math.min(audio.duration || Infinity, audio.currentTime + (d.seekOffset ?? 15));
		});
		return () => {
			for (const a of ['play', 'pause', 'previoustrack', 'nexttrack', 'seekbackward', 'seekforward'] as MediaSessionAction[]) {
				try {
					ms.setActionHandler(a, null);
				} catch {
					// unsupported action on this platform
				}
			}
		};
	});
	$effect(() => {
		if ('mediaSession' in navigator) navigator.mediaSession.playbackState = nar.player.playing ? 'playing' : 'paused';
	});

	// Android host: mirror the player into the system media session so audio
	// survives screen-off / app switch and the lock screen gets controls.
	// Position is read untracked (it ticks every ~250 ms) and refreshed on
	// seek + a slow timer; the session extrapolates in between.
	function syncAndroid() {
		if (!amedia.available()) return;
		const it = nar.current();
		if (!it) return;
		void amedia.setState({
			playing: nar.player.playing,
			title: it.title,
			subtitle: narrator || it.recording.pubkey.slice(0, 12),
			album: nar.player.pubTitle ?? 'tendrl narration',
			position: audio?.currentTime ?? 0,
			duration: audio?.duration || it.recording.duration_secs || 0,
			rate: nar.player.rate,
			hasNext: nar.player.index + 1 < nar.player.playlist.length,
			hasPrev: nar.player.index > 0
		});
	}
	$effect(() => {
		if (!amedia.available()) return;
		// Dependencies: track + play state + rate + playlist shape + duration.
		void item;
		void nar.player.playing;
		void nar.player.rate;
		void nar.player.index;
		void nar.player.playlist.length;
		void nar.player.duration;
		void narrator;
		if (!item) {
			void amedia.stop();
			return;
		}
		untrack(syncAndroid);
	});
	$effect(() => {
		if (!amedia.available() || !nar.player.playing) return;
		const t = setInterval(() => untrack(syncAndroid), 20_000);
		return () => clearInterval(t);
	});
	$effect(() => {
		if (!amedia.available()) return;
		return amedia.onAction((action, position) => {
			switch (action) {
				case 'play':
					nar.player.playing = true;
					break;
				case 'pause':
					nar.player.playing = false;
					break;
				case 'next':
					void nar.next();
					break;
				case 'prev':
					nar.prev();
					break;
				case 'stop':
					closePlayer();
					break;
				case 'seek':
					if (audio && position !== undefined && Number.isFinite(position)) audio.currentTime = position;
					break;
				case 'forward':
					if (audio) audio.currentTime = Math.min(audio.duration || Infinity, audio.currentTime + 15);
					break;
				case 'rewind':
					if (audio) audio.currentTime = Math.max(0, audio.currentTime - 15);
					break;
			}
			untrack(syncAndroid);
		});
	});

	function onError() {
		const r = nar.failover();
		if (r === 'retry' && audio) {
			audio.load();
			if (nar.player.playing) audio.play().catch(() => {});
		}
	}

	function onEnded() {
		if (nar.next()) return;
		// End of the playlist: the book is finished — forget the listening
		// place so the next ▶ starts from the top.
		if (nar.player.pubCoord) history.clearListen(nar.player.pubCoord);
		if (audio) audio.currentTime = 0;
	}

</script>

{#if item}
	<div class="nar-player" class:nar-player--mobile={shell.mode === 'mobile'} class:nar-player--hidden={!docked} class:collapsed={!open} role="region" aria-label="Narration player">
		<audio
			bind:this={audio}
			preload="auto"
			onended={onEnded}
			onerror={onError}
			ontimeupdate={() => {
				nar.player.position = audio?.currentTime ?? 0;
				recordListen();
			}}
			ondurationchange={() => (nar.player.duration = audio?.duration ?? 0)}
			onloadedmetadata={onLoadedMetadata}
			onseeked={() => {
				syncAndroid();
				recordListen(true);
			}}
			onplay={() => (nar.player.playing = true)}
			onpause={() => {
				// Only a user-initiated pause should flip the store; the
				// `ended` → next transition pauses transiently.
				if (audio && !audio.ended) nar.player.playing = false;
				recordListen(true);
			}}
		></audio>
		{#if docked}
			<NarrationControls variant="dock" {open} ontoggleopen={() => (open = !open)} />
		{/if}
	</div>
{/if}

<style>
	.nar-player {
		position: fixed;
		left: 16px;
		bottom: calc(var(--modeline-h, 23px) + 12px);
		z-index: 190;
		display: flex;
		align-items: center;
		gap: 4px;
		max-width: min(92vw, 520px);
		padding: 6px 8px;
		background: var(--panel-bg, var(--bg-panel, var(--bg-surface)));
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-md);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
	}
	.nar-player--mobile {
		left: 50%;
		transform: translateX(-50%);
		bottom: calc(56px + env(safe-area-inset-bottom));
		max-width: 96vw;
	}
	.nar-player--hidden {
		/* A reader hosts the controls inline; the audio element stays mounted. */
		visibility: hidden;
		pointer-events: none;
		width: 0;
		height: 0;
		padding: 0;
		border: 0;
		overflow: hidden;
	}
</style>
