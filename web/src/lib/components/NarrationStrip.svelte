<script lang="ts">
	// A slim player strip at the top of any content event that has audio
	// attached — a chapter in the reader, an article / wiki page / spec in
	// the doc view. Unintrusive: one line (▶, narrator, voice, duration,
	// progress while it plays) and, when there is more to see, a button
	// that opens the recordings drawer the way comments / highlights open
	// theirs. Renders nothing when no recording is bound to `coord`.
	//
	// Playback goes through the one global player (narration.svelte.ts):
	// inside a publication the whole book plays from this chapter; on its
	// own the document resumes from its history entry.
	import * as nar from '$lib/narration.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';

	let {
		coord,
		title,
		pubTitle = null,
		publicationAtag = undefined,
		onopenrecordings = undefined
	}: {
		/** Node coordinate (`kind:pubkey:d`) the audio is bound to. */
		coord: string;
		title: string;
		/** Containing publication's title (the player strip's subtitle). */
		pubTitle?: string | null;
		/** Containing publication coordinate — play the book from here. */
		publicationAtag?: string | undefined;
		/** Open the recordings drawer (all recordings, sources, mirror, narrate). */
		onopenrecordings?: (() => void) | undefined;
	} = $props();

	const recording = $derived(nar.activeRecording(coord));
	const count = $derived(nar.recordingsFor(coord).length);
	const playingHere = $derived(nar.isPlayingCoord(coord));
	const current = $derived(nar.current()?.coord === coord);
	const narrator = $derived(recording ? getAuthorDisplayName(recording.pubkey) : '');
	const duration = $derived(current && nar.player.duration ? nar.player.duration : (recording?.duration_secs ?? 0));
	const frac = $derived(current && duration > 0 ? Math.min(1, nar.player.position / duration) : 0);

	function toggle() {
		if (!recording) return;
		if (playingHere) {
			nar.pause();
			return;
		}
		if (current) {
			nar.toggle();
			return;
		}
		const m = publicationAtag?.match(/^30040:([0-9a-f]{64}):(.+)$/);
		if (m && nar.playPublication(m[1], m[2], pubTitle, coord)) return;
		// On its own: the document is its own history key (30023 / 30818 /
		// 30817); a bare section is not remembered.
		const kind = Number(coord.split(':')[0]);
		const historyCoord = kind === 30041 ? null : coord;
		nar.resumeOrPlayRecording(coord, title, recording, pubTitle ?? title, historyCoord);
	}
</script>

{#if recording}
	<div class="nstrip" class:nstrip--on={playingHere} role="group" aria-label="Narration of {title}">
		<button
			class="nstrip__play"
			onclick={(e) => {
				e.stopPropagation();
				toggle();
			}}
			title={playingHere ? 'Pause' : `Listen — ${recording.version_match === 'exact' ? 'recorded from this version' : recording.version_match === 'other_version' ? 'recorded from another version of the text' : 'not pinned to a version'}`}
			aria-label={playingHere ? 'Pause narration' : 'Play narration'}
		>{playingHere ? '⏸' : '▶'}</button>
		<span class="nstrip__who" title={recording.pubkey}>{recording.mine ? 'you' : narrator}</span>
		{#if recording.voice}<span class="nstrip__dim">· {recording.voice}</span>{/if}
		{#if duration}<span class="nstrip__dim">· {current ? `${nar.fmtTime(nar.player.position)} / ` : ''}{nar.fmtTime(duration)}</span>{/if}
		{#if recording.version_match !== 'exact'}
			<span class="nstrip__warn" title={recording.version_match === 'other_version' ? 'Recorded from another version of this text' : 'Not pinned to a version'}>{nar.matchLabel(recording.version_match)}</span>
		{/if}
		{#if current}
			<span class="nstrip__bar" aria-hidden="true"><span class="nstrip__fill" style="width: {frac * 100}%"></span></span>
		{:else}
			<span class="nstrip__sp"></span>
		{/if}
		{#if onopenrecordings}
			<button
				class="nstrip__more"
				onclick={(e) => {
					e.stopPropagation();
					onopenrecordings?.();
				}}
				title="Recordings — every narration of this document (play, sources, mirror, narrate your own)"
			>{count > 1 ? `${count} recordings` : 'recordings'}</button>
		{/if}
	</div>
{/if}

<style>
	.nstrip {
		display: flex;
		align-items: center;
		gap: 6px;
		margin: 2px 0 10px;
		padding: 3px 8px 3px 4px;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--bg-alt, var(--bg)) 70%, transparent);
		font-size: var(--t-xs);
		color: var(--fg-muted);
		line-height: 1.4;
		min-width: 0;
	}
	.nstrip--on {
		border-color: color-mix(in srgb, var(--id-yours) 55%, var(--border));
	}
	.nstrip__play {
		flex-shrink: 0;
		width: 24px;
		height: 24px;
		border: 1px solid var(--border);
		border-radius: 50%;
		background: var(--bg);
		color: var(--fg);
		cursor: pointer;
		font-size: var(--t-2xs);
		line-height: 1;
		display: inline-flex;
		align-items: center;
		justify-content: center;
	}
	.nstrip__play:hover,
	.nstrip--on .nstrip__play {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.nstrip__who {
		color: var(--fg);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		max-width: 12em;
	}
	.nstrip__dim {
		white-space: nowrap;
	}
	.nstrip__warn {
		font-size: var(--t-3xs);
		padding: 0 5px;
		border-radius: var(--r-sm);
		border: 1px solid color-mix(in srgb, var(--warn, orange) 50%, var(--border));
		color: var(--warn, orange);
		white-space: nowrap;
	}
	.nstrip__bar {
		flex: 1;
		min-width: 24px;
		height: 3px;
		border-radius: 2px;
		background: var(--border);
		overflow: hidden;
	}
	.nstrip__fill {
		display: block;
		height: 100%;
		background: var(--id-yours);
		transition: width 0.5s linear;
	}
	.nstrip__sp {
		flex: 1;
	}
	.nstrip__more {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 0 7px;
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		cursor: pointer;
		white-space: nowrap;
	}
	.nstrip__more:hover {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
</style>
