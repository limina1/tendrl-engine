<script lang="ts">
	// Mode-line reference. Opened by the `?` affordance on the mode-line (next
	// to its W walkthrough chip), mirroring the search panel's `?` syntax modal.
	// Read-only: it names each mode-line segment and the global keys, changing
	// no state. The W chip is the *guided* counterpart; this is the cheat-sheet.

	import { modelineHelpUI, closeModelineHelp } from '$lib/wm/modeline-help.svelte';

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeModelineHelp();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet row(token: string, desc: string)}
	<div class="mh-row">
		<code class="mh-token">{token}</code>
		<span class="mh-desc">{desc}</span>
	</div>
{/snippet}

{#if modelineHelpUI.open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="mh-backdrop" onclick={closeModelineHelp} role="presentation">
		<div
			class="mh-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			tabindex="-1"
		>
			<header class="mh-header">
				<h3 class="mh-title">The mode-line</h3>
				<button class="mh-close" onclick={closeModelineHelp} aria-label="Close">×</button>
			</header>

			<p class="mh-blurb">
				The bottom status bar. The left half tells you <em>where you are</em>;
				the right half is <em>live status</em>, much of it clickable. Click
				any empty part of it to open the <em>menu</em> (the <code>SPC</code>
				leader). Tap <code>W</code> for a guided tour of these.
			</p>

			<div class="mh-scroll">
				<div class="mh-group">Where you are</div>
				{@render row('@class', 'focused slot class: work · chat · research')}
				{@render row('buffer', 'the focused buffer — switch with SPC b b')}
				{@render row('SPC- / mb:', 'an open leader prefix / minibuffer mode')}

				<div class="mh-group">Status &amp; toggles</div>
				{@render row('relays', 'relay configuration · read/write · NIP-11')}
				{@render row('🔍 N', 'search history — click to replay a past query')}
				{@render row('auto / confirm', 'fetch mode — click to flip; right-click for relays')}
				{@render row('embeddings', 'index health — click for status / reindex')}
				{@render row('identity', 'your login — click for profile (or to unlock)')}

				<div class="mh-group">Global keys</div>
				{@render row('SPC', 'menu — the leader (which-key popup); also opens by clicking the mode-line')}
				{@render row('SPC b b', 'switch buffer (B = by class, r = recent)')}
				{@render row('SPC :', 'commands — run an app command')}
				{@render row('W · ?', 'this mode-line tour · this reference')}
			</div>
		</div>
	</div>
{/if}

<style>
	.mh-backdrop {
		position: fixed;
		inset: 0;
		z-index: 320;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 24px;
	}
	.mh-modal {
		width: min(560px, 100%);
		max-height: 80dvh;
		display: flex;
		flex-direction: column;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-lg);
		font-family: var(--font-mono);
	}
	.mh-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 12px 14px 8px;
	}
	.mh-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-md);
		color: var(--base7);
	}
	.mh-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-lg);
		line-height: 1;
		cursor: pointer;
		padding: 0 4px;
	}
	.mh-close:hover {
		color: var(--fg);
	}
	.mh-blurb {
		margin: 0;
		padding: 0 14px 10px;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.55;
	}
	.mh-blurb code {
		font-family: inherit;
		color: var(--affordance-help);
		background: color-mix(in srgb, var(--affordance-help) 12%, transparent);
		padding: 0 3px;
		border-radius: var(--r-sm);
		font-weight: 600;
	}
	.mh-blurb em {
		font-style: normal;
		color: var(--base7);
		font-weight: 600;
	}
	.mh-scroll {
		overflow-y: auto;
		padding: 0 14px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.mh-group {
		margin: 12px 0 5px;
		font-size: var(--t-xs);
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.mh-row {
		display: flex;
		gap: 10px;
		padding: 3px 0;
		align-items: baseline;
	}
	.mh-token {
		flex: 0 0 124px;
		color: var(--affordance-help);
		font-size: var(--t-xs);
		white-space: nowrap;
	}
	.mh-desc {
		flex: 1;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.5;
	}
</style>
