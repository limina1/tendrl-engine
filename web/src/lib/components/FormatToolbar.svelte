<script lang="ts">
	// The composer's formatting toolbar. Every button writes the current
	// style's own syntax into the source (markup.ts); nothing is converted.
	// Buttons keep focus in the editor (mousedown is prevented).
	import type { EditorView } from '@codemirror/view';
	import type { Style } from '$lib/editor/markup';
	import {
		cmdCodeBlock,
		cmdHeading,
		cmdInline,
		cmdInsert,
		cmdLink,
		cmdList,
		cmdQuote,
		headingLevelAt
	} from '$lib/editor/wysiwyg';
	import { promptText } from '$lib/wm/text-prompt.svelte';

	let {
		view,
		style,
		onref,
		version = 0
	}: {
		view: EditorView | null;
		style: Style;
		/** Open the reference builder (the `{{ }}` button). */
		onref: () => void;
		/** Bumped by the host on every editor update so the active heading
		 *  level re-reads. */
		version?: number;
	} = $props();

	const level = $derived.by(() => {
		void version;
		return view ? headingLevelAt(view.state, style) : 0;
	});
	const plain = $derived(style === 'text/plain');

	const keep = (e: MouseEvent) => e.preventDefault();

	async function link() {
		if (!view) return;
		const url = await promptText({ title: 'Link', placeholder: 'https://…', confirmLabel: 'Insert' });
		if (url && view) cmdLink(view, style, url.trim());
	}
</script>

<div class="fmt-bar" role="toolbar" aria-label="Formatting">
	<div class="grp">
		{#each [1, 2, 3, 4] as n (n)}
			<button
				class:on={level === n}
				title={n === 1
					? 'Heading 1 — the title'
					: n === 2
						? 'Heading 2 — a section'
						: n === 3
							? 'Heading 3 — a sub-section (its H2 becomes a nested index)'
							: 'Heading 4 — stays inside the section text'}
				aria-label="Heading {n}"
				aria-pressed={level === n}
				disabled={!view}
				onmousedown={keep}
				onclick={() => view && cmdHeading(view, style, n)}>H{n}</button
			>
		{/each}
	</div>
	{#if !plain}
		<div class="grp">
			<button title="Bold" aria-label="Bold" disabled={!view} onmousedown={keep} onclick={() => view && cmdInline(view, style, 'bold')}><b>B</b></button>
			<button title="Italic" aria-label="Italic" disabled={!view} onmousedown={keep} onclick={() => view && cmdInline(view, style, 'italic')}><i>I</i></button>
			<button title="Inline code" aria-label="Inline code" class="mono" disabled={!view} onmousedown={keep} onclick={() => view && cmdInline(view, style, 'code')}>&lt;/&gt;</button>
			<button title="Link" aria-label="Link" disabled={!view} onmousedown={keep} onclick={link}>Link</button>
		</div>
		<div class="grp">
			<button title="List" aria-label="List" disabled={!view} onmousedown={keep} onclick={() => view && cmdList(view, style)}>• List</button>
			<button title="Quote" aria-label="Quote" disabled={!view} onmousedown={keep} onclick={() => view && cmdQuote(view, style)}>❝</button>
			<button title="Code block" aria-label="Code block" class="mono" disabled={!view} onmousedown={keep} onclick={() => view && cmdCodeBlock(view, style)}>{'{ }'}</button>
		</div>
	{/if}
	<div class="grp">
		<button
			class="mono"
			title="Insert a reference — ref / wiki / embed / slot / quote / mention"
			aria-label="Insert reference"
			data-tour="compose-ref"
			disabled={!view}
			onmousedown={keep}
			onclick={onref}>&lbrace;&lbrace; &rbrace;&rbrace;</button
		>
		<button
			class="mono"
			title="Wikilink — [[topic]]"
			aria-label="Wikilink"
			disabled={!view}
			onmousedown={keep}
			onclick={() => view && cmdInsert(view, '[[]]', 2)}>[[ ]]</button
		>
	</div>
</div>

<style>
	.fmt-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		align-items: center;
		padding: 4px 8px;
		border-bottom: 1px solid var(--border);
		background: var(--bg-alt);
	}
	.grp {
		display: flex;
		gap: 2px;
		padding-right: 6px;
		margin-right: 2px;
		border-right: 1px solid var(--border);
	}
	.grp:last-child {
		border-right: 0;
	}
	button {
		font: inherit;
		font-size: var(--t-sm);
		min-width: 30px;
		min-height: 28px;
		padding: 0 7px;
		background: transparent;
		color: var(--fg);
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	button:hover:not(:disabled) {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	button.on {
		border-color: var(--id-yours);
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
	}
	button:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.mono {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
</style>
