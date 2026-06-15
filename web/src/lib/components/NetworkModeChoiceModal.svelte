<script lang="ts">
	// First-run, one-time modal. Shown when the engine reports
	// `mode_chosen: false` (fresh install) — BEFORE any relay fetch — so the
	// user deliberately picks how the app talks to the network. The choice
	// persists to config.toml (engine flips `mode_chosen` true), so it never
	// re-appears. There is no dismiss / backdrop-close: picking a mode is the
	// only way out, by design.

	import type { NetworkMode } from '$lib/types';

	let {
		onchoose
	}: {
		/** Persist the picked mode + close. Wired to app.chooseNetworkMode. */
		onchoose: (mode: NetworkMode) => void;
	} = $props();

	let submitting = $state(false);

	function pick(mode: NetworkMode) {
		if (submitting) return;
		submitting = true;
		onchoose(mode);
	}
</script>

<div class="nm-backdrop" role="presentation">
	<div class="nm-modal" role="dialog" aria-modal="true" aria-labelledby="nm-title" tabindex="-1">
		<header class="nm-header">
			<h3 class="nm-title" id="nm-title">Select your default network mode</h3>
			<p class="nm-sub">
				How should the app reach Nostr relays? You can change this any time in Settings.
			</p>
		</header>

		<div class="nm-choices">
			<button class="nm-choice" onclick={() => pick('confirm')} disabled={submitting}>
				<span class="nm-choice-head">
					<span class="nm-choice-name">Confirm</span>
					<span class="nm-badge nm-badge--rec">recommended</span>
				</span>
				<span class="nm-choice-desc">
					Every relay request first <span class="nm-vb">raises a menu</span> showing
					exactly what is about to be <span class="nm-vb">fetched</span> — which
					<span class="nm-kw">events</span>, <span class="nm-kw">descriptions</span>,
					and which <span class="nm-kw">relays</span> — and
					<span class="nm-vb">waits for your approval</span>.
					<span class="nm-kw">Local-first</span> and <span class="nm-kw">private</span>;
					nothing leaves your machine unprompted. Best for
					<span class="nm-kw">fine-grained control</span> and for building an
					<span class="nm-kw">understanding of how Nostr works</span> under the hood.
				</span>
			</button>

			<button class="nm-choice" onclick={() => pick('auto')} disabled={submitting}>
				<span class="nm-choice-head">
					<span class="nm-choice-name">Auto</span>
				</span>
				<span class="nm-choice-desc">
					The app <span class="nm-vb">fetches</span> from your default relays
					<span class="nm-kw">automatically</span>, with no per-request prompt — aiming
					for a <span class="nm-kw">smoother experience</span> with
					<span class="nm-kw">sensible defaults</span>. Smoothest for browsing; relay
					traffic <span class="nm-vb">happens in the background</span>.
					<span class="nm-cv">Less fine-grained control</span> over what gets fetched
					and from where.
				</span>
			</button>
		</div>

		<footer class="nm-footer">
			<span class="nm-foot-note">Pick one to continue — no fetching happens until you do.</span>
		</footer>
	</div>
</div>

<style>
	.nm-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.62);
		z-index: 300; /* above the per-fetch confirm modal (250) */
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.nm-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 90vw;
		max-width: 560px;
		max-height: 86vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		overflow-y: auto;
	}
	.nm-header {
		padding: 16px 18px 10px;
		border-bottom: 1px solid var(--panel-border);
	}
	.nm-title {
		margin: 0;
		font-size: var(--t-md);
		color: var(--base7);
	}
	.nm-sub {
		margin: 6px 0 0;
		color: var(--base5);
		font-size: var(--t-xs);
		line-height: 1.5;
	}
	.nm-choices {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 14px 18px;
	}
	.nm-choice {
		display: flex;
		flex-direction: column;
		gap: 6px;
		text-align: left;
		padding: 12px 14px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg);
		font: inherit;
		/* Buttons default to white-space:nowrap in some UAs — force the
		   description to wrap instead of overflowing the card. */
		white-space: normal;
		overflow-wrap: break-word;
		cursor: pointer;
		transition:
			border-color 0.12s ease,
			background 0.12s ease;
	}
	.nm-choice:hover:not(:disabled) {
		border-color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 8%, var(--bg-surface));
	}
	.nm-choice:disabled {
		opacity: 0.55;
		cursor: default;
	}
	.nm-choice-head {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.nm-choice-name {
		font-size: var(--t-sm);
		font-weight: 600;
		color: var(--base7);
	}
	.nm-badge {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 2px);
		padding: 1px 6px;
		border-radius: var(--r-sm);
	}
	.nm-badge--rec {
		color: var(--state-online);
		border: 1px solid color-mix(in srgb, var(--state-online) 45%, transparent);
	}
	.nm-choice-desc {
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.55;
		white-space: normal;
		overflow-wrap: break-word;
		word-break: break-word;
	}
	/* Keyword highlighting: verbs/actions in the accent, key concepts in the
	   "online" green, the trade-off phrase in the muted draft tone. Kept
	   weight-500 so they read as emphasis, not links. */
	.nm-vb {
		color: var(--id-yours);
		font-weight: 500;
	}
	.nm-kw {
		color: var(--state-online);
		font-weight: 500;
	}
	.nm-cv {
		color: var(--id-draft);
		font-weight: 500;
	}
	.nm-footer {
		padding: 10px 18px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.nm-foot-note {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
</style>
