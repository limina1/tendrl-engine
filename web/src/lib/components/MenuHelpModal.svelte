<script lang="ts">
	// Event-menu reference. Opened by the `?` affordance on the event menu
	// (next to its W walkthrough chip), mirroring the search / mode-line /
	// composer `?` modals. Read-only: it explains the keyboard-chord model and
	// names each section. The W chip is the *guided* counterpart; this is the
	// cheat-sheet. Sits above the event modal (which is z-index 100).

	import { menuHelpUI, closeMenuHelp } from '$lib/wm/menu-help.svelte';

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeMenuHelp();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet row(token: string, desc: string)}
	<div class="mhm-row">
		<code class="mhm-token">{token}</code>
		<span class="mhm-desc">{desc}</span>
	</div>
{/snippet}

{#if menuHelpUI.open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="mhm-backdrop" onclick={closeMenuHelp} role="presentation">
		<div
			class="mhm-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			tabindex="-1"
		>
			<header class="mhm-header">
				<h3 class="mhm-title">The event menu</h3>
				<button class="mhm-close" onclick={closeMenuHelp} aria-label="Close">×</button>
			</header>

			<p class="mhm-blurb">
				Everything you can do with one event. It's keyboard-driven: press a
				section's letter to arm it, then the inner key. <strong>c i</strong>
				copies the id; <strong>a r</strong> reads it. Or just click. Tap
				<strong>W</strong> for a guided walk.
			</p>

			<div class="mhm-scroll">
				<div class="mhm-group">c · Copy as</div>
				{@render row('i', 'hex event id')}
				{@render row('e', 'nevent1… (bech32m event id)')}
				{@render row('a', 'naddr1… — replaceables (publications, articles)')}
				{@render row('n', "author's npub1…")}

				<div class="mhm-group">a · Actions</div>
				{@render row('r', 'read — open in the reader')}
				{@render row('f', 'find publications that contain this section')}
				{@render row('i', 'insert into the current draft')}
				{@render row('b', 'broadcast to your relays (deliberate, per-event)')}

				<div class="mhm-group">p · Pool</div>
				{@render row('c', 'route into chat context')}
				{@render row('m', 'route into compose')}
				{@render row('r', 'hold in refs (no routing)')}
				{@render row('i', 'lock — imported / claimed')}
				{@render row('x', 'drop from every pool')}

				<div class="mhm-group">Found on</div>
				{@render row('chips', 'relays this event was seen on + local cache')}

				<div class="mhm-group">Global</div>
				{@render row('t', 'toggle the raw tag block')}
				{@render row('Esc', 'cancel a chord / close the menu')}
				{@render row('W · ?', 'this guided tour · this reference')}
			</div>
		</div>
	</div>
{/if}

<style>
	.mhm-backdrop {
		position: fixed;
		inset: 0;
		z-index: 330; /* above the event modal (100) and the tour overlay (290) */
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 24px;
	}
	.mhm-modal {
		width: min(560px, 100%);
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-md);
		box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
		font-family: var(--font-mono);
	}
	.mhm-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 12px 14px 8px;
	}
	.mhm-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-md);
		color: var(--base7);
	}
	.mhm-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-lg);
		line-height: 1;
		cursor: pointer;
		padding: 0 4px;
	}
	.mhm-close:hover {
		color: var(--fg);
	}
	.mhm-blurb {
		margin: 0;
		padding: 0 14px 10px;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.55;
	}
	.mhm-scroll {
		overflow-y: auto;
		padding: 0 14px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.mhm-group {
		margin: 12px 0 5px;
		font-size: var(--t-xs);
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.mhm-row {
		display: flex;
		gap: 10px;
		padding: 3px 0;
		align-items: baseline;
	}
	.mhm-token {
		flex: 0 0 60px;
		color: var(--affordance-help);
		font-size: var(--t-xs);
		white-space: nowrap;
	}
	.mhm-desc {
		flex: 1;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.5;
	}
</style>
