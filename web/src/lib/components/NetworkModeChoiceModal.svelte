<script lang="ts">
	// First-run, one-time modal. Shown when the engine reports
	// `mode_chosen: false` (fresh install) — BEFORE any relay fetch — so the
	// user deliberately picks how the app talks to the network. The choice
	// persists to config.toml (engine flips `mode_chosen` true), so it never
	// re-appears once picked. The X / backdrop is "decide later": a
	// session-only dismiss (mode_chosen stays false, so the modal returns
	// next launch and the cold-cache fetch stays suppressed) — on a phone a
	// modal with no escape is a wall, so the hard gate became a soft one.

	import type { NetworkMode } from '$lib/types';

	let {
		onchoose,
		ondismiss
	}: {
		/** Persist the picked mode + close, and either start or suppress the
		 *  contextual walkthrough per the toggle. Wired to app.chooseNetworkMode. */
		onchoose: (mode: NetworkMode, runWalkthrough: boolean) => void;
		/** "Decide later" — hide for this session without persisting a mode. */
		ondismiss: () => void;
	} = $props();

	let submitting = $state(false);
	// Default ON — most first-run users benefit from the guided tour. Unchecking
	// means "never show tips" until re-armed from Settings or the W button.
	let runWalkthrough = $state(true);

	function pick(mode: NetworkMode) {
		if (submitting) return;
		submitting = true;
		onchoose(mode, runWalkthrough);
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="nm-backdrop" role="presentation" onclick={ondismiss}>
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<div
		class="nm-modal"
		role="dialog"
		aria-modal="true"
		aria-labelledby="nm-title"
		tabindex="-1"
		onclick={(e) => e.stopPropagation()}
	>
		<header class="nm-header">
			<h3 class="nm-title" id="nm-title">Select your default network mode</h3>
			<button
				class="nm-x"
				onclick={ondismiss}
				title="Decide later — nothing is fetched until you pick a mode; this asks again next launch"
				aria-label="Decide later"
			>×</button>
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
				<span class="nm-choice-tip">New here and want to explore? Pick Confirm — the walkthrough is built around it.</span>
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
			<button
				type="button"
				class="nm-walk"
				class:nm-walk--on={runWalkthrough}
				aria-pressed={runWalkthrough}
				onclick={() => (runWalkthrough = !runWalkthrough)}
				disabled={submitting}
				title="Run a short, click-through walkthrough of the interface as you go. Always dismissable; you can re-run it any time from the W button or Settings."
			>{runWalkthrough ? '✓' : '○'} Run walkthrough</button>
			<span class="nm-foot-note">Pick a mode to continue — no fetching happens until you do.</span>
		</footer>
	</div>
</div>

<style>
	.nm-backdrop {
		position: fixed;
		inset: 0;
		background: var(--scrim);
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
		max-height: 92dvh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		overflow-y: auto;
	}
	.nm-header {
		position: relative;
		padding: 16px 18px 10px;
		border-bottom: 1px solid var(--panel-border);
	}
	.nm-x {
		position: absolute;
		top: 10px;
		right: 10px;
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-lg);
		line-height: 1;
		padding: 4px 8px;
		cursor: pointer;
	}
	.nm-x:hover {
		color: var(--fg);
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
	/* Short, friendly nudge for first-timers — the walkthrough's dull-grey hue
	   so it reads as a guided-tour aside, not part of the mode description. */
	.nm-choice-tip {
		margin-top: 2px;
		color: var(--affordance-walkthrough);
		font-size: calc(var(--t-xs) - 1px);
		font-style: italic;
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
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 10px 18px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.nm-foot-note {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	/* "Run walkthrough" toggle — mirrors the ✓/○ "General feed" toggle pattern
	   from the fetch-confirm modal: transparent at rest, lifts to the
	   walkthrough's dull-grey role hue when on. */
	.nm-walk {
		font: inherit;
		font-size: var(--t-xs);
		padding: 4px 10px;
		background: transparent;
		border: 1px solid var(--panel-border);
		color: var(--base6);
		border-radius: var(--r-sm);
		cursor: pointer;
		white-space: nowrap;
	}
	.nm-walk:hover:not(:disabled) {
		border-color: var(--affordance-walkthrough);
		color: var(--affordance-walkthrough);
	}
	.nm-walk--on {
		background: color-mix(in srgb, var(--affordance-walkthrough) 14%, transparent);
		color: var(--affordance-walkthrough);
		border-color: color-mix(in srgb, var(--affordance-walkthrough) 50%, transparent);
	}
	.nm-walk--on:hover:not(:disabled) {
		background: color-mix(in srgb, var(--affordance-walkthrough) 22%, transparent);
	}
	.nm-walk:disabled {
		opacity: 0.55;
		cursor: default;
	}
</style>
