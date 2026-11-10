<script lang="ts">
	// The player's controls, rendered from the store alone — no <audio>
	// here. NarrationPlayer owns the one audio element and mounts these in
	// its fixed dock; the reader mounts them inline, right above the pager,
	// while the loaded playlist is that publication's (the dock hides
	// itself then). Seeking goes through `nar.seek`, which the audio
	// element applies.
	import * as nar from '$lib/narration.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';

	let {
		variant = 'dock',
		open = true,
		ontoggleopen = undefined
	}: {
		variant?: 'dock' | 'inline';
		/** Dock only: collapsed to the transport buttons. */
		open?: boolean;
		ontoggleopen?: (() => void) | undefined;
	} = $props();

	const item = $derived(nar.current());
	const narrator = $derived(item ? getAuthorDisplayName(item.recording.pubkey) : '');
	const piece = $derived(nar.currentPiece());
	const max = $derived(nar.player.duration || item?.recording.duration_secs || 0);
</script>

{#if item}
	<div class="npc" class:npc--inline={variant === 'inline'} class:npc--collapsed={!open}>
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
					{#if nar.player.pubTitle && variant === 'dock'}<span class="np-pub">{nar.player.pubTitle}</span> · {/if}
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
						{max}
						step="1"
						value={nar.player.position}
						oninput={(e) => nar.seek(Number((e.currentTarget as HTMLInputElement).value))}
						aria-label="Seek"
					/>
					<span class="np-time">{nar.fmtTime(max)}</span>
				</div>
				{#if nar.player.error}
					<div class="np-error">{nar.player.error}</div>
				{/if}
			</div>
			<button class="np-btn np-rate" onclick={() => nar.cycleRate()} title="Playback speed">{nar.player.rate}×</button>
		{/if}
		{#if variant === 'dock' && ontoggleopen}
			<button class="np-btn" onclick={ontoggleopen} title={open ? 'Collapse' : 'Expand'} aria-label={open ? 'Collapse player' : 'Expand player'}>{open ? '▾' : '▴'}</button>
		{/if}
		<button class="np-btn" onclick={() => nar.close()} title="Stop and close the player" aria-label="Close player">×</button>
	</div>
{/if}

<style>
	.npc {
		display: flex;
		align-items: center;
		gap: 4px;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
	}
	.npc--inline {
		width: 100%;
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
	.npc--inline .np-meta {
		flex-direction: row;
		align-items: center;
		gap: 10px;
	}
	.npc--inline .np-seek {
		flex: 1 1 auto;
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
</style>
