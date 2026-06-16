<script lang="ts">
	// Composer reference. Opened by the `?` affordance on the compose mode-bar
	// (next to its W walkthrough chip), mirroring the search and mode-line `?`
	// modals. Read-only: it names the modes, the section model, and the
	// draft → sign → broadcast lifecycle, changing no state. The W chip is the
	// *guided* counterpart; this is the cheat-sheet.

	import { composeHelpUI, closeComposeHelp } from '$lib/wm/compose-help.svelte';

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeComposeHelp();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet row(token: string, desc: string)}
	<div class="ch-row">
		<code class="ch-token">{token}</code>
		<span class="ch-desc">{desc}</span>
	</div>
{/snippet}

{#if composeHelpUI.open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="ch-backdrop" onclick={closeComposeHelp} role="presentation">
		<div
			class="ch-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			tabindex="-1"
		>
			<header class="ch-header">
				<h3 class="ch-title">The composer</h3>
				<button class="ch-close" onclick={closeComposeHelp} aria-label="Close">×</button>
			</header>

			<p class="ch-blurb">
				You're assembling a publication — a <code>kind-30040</code> index over
				an ordered list of <code>kind-30041</code> sections. Tap
				<code>W</code> for a guided walk through these.
			</p>

			<div class="ch-scroll">
				<div class="ch-group">View modes</div>
				{@render row('Full', 'each section as an editable card')}
				{@render row('Plain', 'one text buffer for the whole draft')}
				{@render row('Read', 'preview the rendered result')}
				{@render row('h / l', 'cycle between Full and Plain')}

				<div class="ch-group">Plain-mode structure</div>
				{@render row('= Title', 'level 1 — the publication itself (one, at top)')}
				{@render row('== Heading', 'level 2 — starts a section')}
				{@render row('=== Sub', 'level 3+ — nested sub-publication (optional)')}
				{@render row(':key: value', 'a tag under the preceding heading')}
				{@render row('delim', 'the heading delimiter character (= by default)')}
				{@render row('nest', 'how deep headings fold into nested 30040 indices')}
				{@render row('+ Section', 'append a new section (Full mode)')}

				<div class="ch-group">Selection toolbar</div>
				{@render row('All / Inv', 'select all sections / invert the selection')}
				{@render row('◂', 'send selected sections to chat')}
				{@render row('▸', 'publish selected sections locally')}
				{@render row('🗑', 'remove from compose (arm again to delete everywhere)')}
				{@render row('▸ all', 'collapse / expand every section')}

				<div class="ch-group">Draft → sign → broadcast</div>
				{@render row('Save draft', 'unsigned local copy — survives a refresh')}
				{@render row('Preview events', 'inspect the 30040/30041 JSON first')}
				{@render row('Sign', 'sign a snapshot — the only way into the db')}
				{@render row('broadcast', 'sending to relays is a separate, later step')}
			</div>
		</div>
	</div>
{/if}

<style>
	.ch-backdrop {
		position: fixed;
		inset: 0;
		z-index: 320;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 24px;
	}
	.ch-modal {
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
	.ch-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 12px 14px 8px;
	}
	.ch-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-md);
		color: var(--base7);
	}
	.ch-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-lg);
		line-height: 1;
		cursor: pointer;
		padding: 0 4px;
	}
	.ch-close:hover {
		color: var(--fg);
	}
	.ch-blurb {
		margin: 0;
		padding: 0 14px 10px;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.55;
	}
	.ch-blurb code {
		font-family: inherit;
		color: var(--affordance-help);
		background: color-mix(in srgb, var(--affordance-help) 12%, transparent);
		padding: 0 3px;
		border-radius: var(--r-sm);
		font-weight: 600;
	}
	.ch-scroll {
		overflow-y: auto;
		padding: 0 14px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.ch-group {
		margin: 12px 0 5px;
		font-size: var(--t-xs);
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.ch-row {
		display: flex;
		gap: 10px;
		padding: 3px 0;
		align-items: baseline;
	}
	.ch-token {
		flex: 0 0 124px;
		color: var(--affordance-help);
		font-size: var(--t-xs);
		white-space: nowrap;
	}
	.ch-desc {
		flex: 1;
		color: var(--base6);
		font-size: var(--t-xs);
		line-height: 1.5;
	}
</style>
