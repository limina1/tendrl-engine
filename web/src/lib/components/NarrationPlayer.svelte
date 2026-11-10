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

	let audio: HTMLAudioElement | undefined = $state();
	let open = $state(true);

	const item = $derived(nar.current());
	const url = $derived(nar.currentUrl());
	const narrator = $derived(item ? getAuthorDisplayName(item.recording.pubkey) : '');
	const piece = $derived(nar.currentPiece());

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
		nar.stop();
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

	function seek(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		if (audio && Number.isFinite(v)) audio.currentTime = v;
	}

	const RATES = [0.8, 1, 1.25, 1.5, 1.75, 2];
	function cycleRate() {
		const i = RATES.indexOf(nar.player.rate);
		nar.setRate(RATES[(i + 1) % RATES.length]);
	}
</script>

{#if item}
	<div class="nar-player" class:nar-player--mobile={shell.mode === 'mobile'} class:collapsed={!open} role="region" aria-label="Narration player">
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
		<button class="np-btn" onclick={() => nar.prev()} disabled={nar.player.index <= 0} title="Previous section" aria-label="Previous">⏮</button>
		<button class="np-btn np-btn--main" onclick={() => nar.toggle()} title={nar.player.playing ? 'Pause' : 'Play'} aria-label={nar.player.playing ? 'Pause' : 'Play'}>
			{nar.player.playing ? '⏸' : '▶'}
		</button>
		<button class="np-btn" onclick={() => nar.next()} disabled={nar.player.index + 1 >= nar.player.playlist.length} title="Next section" aria-label="Next">⏭</button>
		{#if open}
			<div class="np-meta">
				<div class="np-title" title={item.title}>
					<span class="np-idx">{nar.player.index + 1}/{nar.player.playlist.length}</span>
					{item.title}
					{#if piece.pieces > 1}<span class="np-idx" title="This section spans several blobs"> · part {piece.piece + 1}/{piece.pieces}</span>{/if}
				</div>
				<div class="np-sub">
					{#if nar.player.pubTitle}<span class="np-pub">{nar.player.pubTitle}</span> · {/if}
					<span title={item.recording.pubkey}>{narrator}</span>
					{#if item.recording.voice}<span class="np-voice"> · {item.recording.voice}</span>{/if}
					{#if item.recording.version_match !== 'exact'}
						<span class="np-warn" title="This recording was made from a different version of the section text">· {nar.matchLabel(item.recording.version_match)}</span>
					{/if}
				</div>
				<div class="np-seek">
					<span class="np-time">{nar.fmtTime(nar.player.position)}</span>
					<input
						class="np-range"
						type="range"
						min="0"
						max={nar.player.duration || item.recording.duration_secs || 0}
						step="1"
						value={nar.player.position}
						oninput={seek}
						aria-label="Seek"
					/>
					<span class="np-time">{nar.fmtTime(nar.player.duration || item.recording.duration_secs || 0)}</span>
				</div>
				{#if nar.player.error}
					<div class="np-error">{nar.player.error}</div>
				{/if}
			</div>
			<button class="np-btn np-rate" onclick={cycleRate} title="Playback speed">{nar.player.rate}×</button>
		{/if}
		<button class="np-btn" onclick={() => (open = !open)} title={open ? 'Collapse' : 'Expand'} aria-label={open ? 'Collapse player' : 'Expand player'}>{open ? '▾' : '▴'}</button>
		<button class="np-btn" onclick={closePlayer} title="Stop and close the player" aria-label="Close player">×</button>
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
	.np-btn {
		background: none;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--fg-muted);
		cursor: pointer;
		padding: 2px 6px;
		font: inherit;
		line-height: 1.4;
	}
	.np-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--panel-border);
	}
	.np-btn:disabled {
		opacity: 0.35;
		cursor: default;
	}
	.np-btn--main {
		font-size: 1.1em;
		color: var(--state-online, var(--accent));
	}
	.np-rate {
		min-width: 3.2em;
	}
	.np-meta {
		display: flex;
		flex-direction: column;
		min-width: 0;
		flex: 1 1 auto;
		gap: 2px;
		padding: 0 4px;
	}
	.np-title {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.np-idx {
		color: var(--fg-muted);
		margin-right: 4px;
	}
	.np-sub {
		color: var(--fg-muted);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.np-warn {
		color: var(--state-warn, var(--warning, #c90));
	}
	.np-seek {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.np-range {
		flex: 1 1 auto;
		min-width: 80px;
		accent-color: var(--state-online, var(--accent));
		height: 4px;
	}
	.np-time {
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
	}
	.np-error {
		color: var(--state-error, var(--error, #c33));
	}
	.collapsed .np-meta {
		display: none;
	}
</style>
