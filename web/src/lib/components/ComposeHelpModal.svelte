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
				<code>W</code> for the guided walkthroughs of each piece below.
			</p>

			<div class="ch-scroll">
				<div class="ch-group">Output</div>
				{@render row('kind', 'Publication (30040/41 graph) · Blog (30023) · Wiki (30818) · Custom')}
				{@render row('atomic kinds', 'Blog/Wiki/Custom publish the whole body as one event')}
				{@render row('Publication', 'a title present → sections bind under one 30040 index')}
				{@render row('Notes', 'no title → each section a standalone 30041, no index')}

				<div class="ch-group">View modes</div>
				{@render row('Full', 'each section as an editable card')}
				{@render row('Plain', 'one text buffer + a live detected-section outline')}
				{@render row('Read', 'preview the rendered result in its own buffer')}

				<div class="ch-group">Plain-mode structure</div>
				{@render row('= Title', 'level 1 — the publication itself (one, at top)')}
				{@render row('== Heading', 'level 2 — starts a section')}
				{@render row('=== Sub', 'level 3+ — nested sub-index (when nest > flat)')}
				{@render row('delim', 'the heading delimiter character (= default, # Markdown)')}
				{@render row('nest', 'flat, or fold deeper headings into nested 30040 indices')}
				{@render row(':key: value', 'a ["key","value"] tag — works in every mode')}
				{@render row(':tags: a, b', 'expands to t tags (#a #b) — works in every mode')}
				{@render row('+ Section', 'append a new section (Full mode)')}

				<div class="ch-group">Nostrdown references</div>
				{@render row('{{ref:slug}}', 'link a sibling section by its title-slug')}
				{@render row('{{wiki:topic}}', 'wikilink → kind 30818 / 30023 / 30041 by topic')}
				{@render row('{{embed:target}}', 'transclude a sibling section or naddr inline')}
				{@render row('{{quote:naddr|text}}', 'quote a passage (text inline) — NIP-84-style, attributed')}
				{@render row('{{slot:naddr}}', 'own line: slot an existing 30040/30041 in as a child of the index')}
				{@render row('|Display', 'append to override the link label')}
				{@render row('#heading', 'append to target a heading anchor')}
				{@render row('⌘/Ctrl-click', 'follow a recognized {{…}} reference in the editor')}

				<div class="ch-group">Sections (Full)</div>
				{@render row('locked', 'imported / new sections arrive locked — claim (yellow) to edit')}
				{@render row('Unlock / Lock all', 'bulk claim / re-lock against a source publication')}
				{@render row('drag', 'reorder; collapse a card to its title')}

				<div class="ch-group">Selection toolbar</div>
				{@render row('All / Inv', 'select all sections / invert the selection')}
				{@render row('◂', 'send selected sections to chat')}
				{@render row('▸', 'publish selected sections locally')}
				{@render row('🗑', 'remove from compose (arm again to delete everywhere)')}
				{@render row('▸ all', 'collapse / expand every section')}

				<div class="ch-group">Sign → broadcast</div>
				{@render row('Preview events', 'inspect the exact JSON first')}
				{@render row('Sign', 'sign a snapshot — the only way into the db')}
				{@render row('Sign (N)', 'sign just the checked sections')}
				{@render row('Diff vs published', 'compare to the last published version')}
				{@render row('republish / fork', 'same title reuses identifiers & replaces; else new')}
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
		background: var(--scrim);
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
		box-shadow: var(--shadow-lg);
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
