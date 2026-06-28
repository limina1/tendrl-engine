<script lang="ts">
	// Renders a section's plain-text content with two overlay layers merged onto
	// one pass: NIP-84 highlights (engine-resolved spans) and nostrdown
	// `{{ref|wiki|embed:…}}` references (engine-resolved refs). Both arrive as
	// UTF-16 offsets into the same `content`; `buildSegments` slices them into
	// renderable runs. Purely presentational — resolution happens in the parent
	// (which calls `api.resolveHighlights` / `api.resolveNostrdown`).
	import { getAppState } from '$lib/state.svelte';
	import EmbedCard from './EmbedCard.svelte';
	import { pubkeyToHighlightFill, pubkeyToHighlightStroke } from '$lib/discussions/colors';
	import type { HighlightSpan } from '$lib/discussions/highlights';
	import { buildSegments, type ResolvedRef } from '$lib/nostr/nostrdown';

	const app = getAppState();

	let {
		content,
		spans = [],
		refs = [],
		focusedHighlightId = null,
		muted = false
	}: {
		content: string;
		spans?: HighlightSpan[];
		refs?: ResolvedRef[];
		focusedHighlightId?: string | null;
		muted?: boolean;
	} = $props();

	const segments = $derived(buildSegments(content, spans, refs, focusedHighlightId));

	function styleFor(pubkey: string, focused: boolean): string {
		const fill = pubkeyToHighlightFill(pubkey);
		const stroke = pubkeyToHighlightStroke(pubkey);
		if (focused) {
			return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke}, 0 0 0 2px var(--state-online);`;
		}
		return `background: ${fill}; box-shadow: inset 3px 0 0 ${stroke};`;
	}

	// Open a resolved reference: an addressable in the reader (publication /
	// section / article / wiki), or a user (npub embed) in the profile view.
	// nevent/note embeds have no addressable coordinate — preview only, for now.
	function openRef(ref: ResolvedRef) {
		if (ref.coord) app.openCoord(ref.coord);
		else if (ref.event_kind === 0 && ref.author_pubkey) app.navigateToProfile(ref.author_pubkey);
	}

	function refTitle(ref: ResolvedRef): string {
		if (!ref.found) return `Unresolved ${ref.kind}: ${ref.target}`;
		const kind = ref.event_kind ? ` (kind ${ref.event_kind})` : '';
		const frag = ref.fragment ? ` #${ref.fragment}` : '';
		return `${ref.kind}: ${ref.target}${kind}${frag}`;
	}

	// Portal the hover popover to <body> so a scrolling (continuous) view or a
	// transformed ancestor can't clip the fixed-position card.
	function portal(node: HTMLElement) {
		document.body.appendChild(node);
		return { destroy: () => node.remove() };
	}

	// Hover preview for ref/wiki links — the same EmbedCard the reader renders
	// inline, floated beside the link. Click still navigates; hover just peeks.
	let preview = $state<{ ref: ResolvedRef; x: number; y: number } | null>(null);
	let previewTimer: ReturnType<typeof setTimeout> | undefined;
	function showPreview(e: MouseEvent | FocusEvent, ref: ResolvedRef) {
		if (!ref.found) return;
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		clearTimeout(previewTimer);
		preview = { ref, x: Math.max(8, Math.min(r.left, window.innerWidth - 348)), y: r.bottom + 4 };
	}
	function cancelHide() {
		clearTimeout(previewTimer);
	}
	function hidePreview() {
		clearTimeout(previewTimer);
		previewTimer = setTimeout(() => (preview = null), 120);
	}
</script>

<pre class="section-content" class:muted>{#each segments as seg, i (i)}{#if seg.type === 'highlight'}<mark class="hl-overlay" data-hl-ids={seg.highlight.id} style={styleFor(seg.highlight.pubkey, seg.highlight.focused)} title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…">{seg.text}</mark>{:else if seg.type === 'ref'}{#if seg.ref.kind === 'embed' || seg.ref.kind === 'quote'}<EmbedCard ref={seg.ref} onopen={openRef} />{:else}<button class="nd-ref nd-ref--{seg.ref.kind}" class:nd-unresolved={!seg.ref.found} onclick={() => openRef(seg.ref)} onmouseenter={(e) => showPreview(e, seg.ref)} onmouseleave={hidePreview} onfocus={(e) => showPreview(e, seg.ref)} onblur={hidePreview} disabled={!seg.ref.coord} title={refTitle(seg.ref)}>{seg.ref.label}{#if seg.ref.fragment}<span class="nd-ref__frag">#{seg.ref.fragment}</span>{/if}</button>{/if}{:else}{seg.text}{/if}{/each}</pre>
{#if preview}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="nd-preview"
		style="left:{preview.x}px; top:{preview.y}px"
		use:portal
		onmouseenter={cancelHide}
		onmouseleave={hidePreview}
		role="tooltip"
	><EmbedCard ref={preview.ref} onopen={openRef} /></div>
{/if}

<style>
	.section-content {
		white-space: pre-wrap;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.5;
		color: var(--fg);
		margin: 0;
	}
	.section-content.muted {
		color: var(--fg-muted);
	}

	.hl-overlay {
		color: inherit;
		padding: 1px 2px;
		border-radius: 2px;
	}
	@keyframes hl-flash {
		0%, 100% { filter: brightness(1) saturate(1); }
		30%      { filter: brightness(1.5) saturate(1.6); }
	}
	:global(.hl-overlay.hl-flash) {
		animation: hl-flash 1.2s ease-in-out;
	}

	/* Nostrdown inline ref / wiki link. Rendered in place of the `{{…}}` token;
	   resolved targets are clickable, unresolved ones read as muted dotted text. */
	.nd-ref {
		font: inherit;
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 8%, transparent);
		border: none;
		border-radius: var(--r-sm, 3px);
		padding: 0 3px;
		cursor: pointer;
	}
	.nd-ref--wiki {
		color: var(--accent, var(--id-yours));
		background: color-mix(in srgb, var(--accent, var(--id-yours)) 8%, transparent);
	}
	.nd-ref:hover:not(:disabled) {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
		text-decoration: underline;
	}
	.nd-ref__frag {
		opacity: 0.65;
		font-size: 0.9em;
	}
	.nd-ref.nd-unresolved {
		color: var(--fg-muted);
		background: none;
		border-bottom: 1px dotted var(--fg-muted);
		cursor: default;
	}

	/* Floating wrapper for the ref/wiki hover preview (portaled to <body>). The
	   card itself is EmbedCard; this just positions + lifts it. */
	.nd-preview {
		position: fixed;
		z-index: 200;
		width: min(340px, 90vw);
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		overflow: hidden;
	}
	/* Neutralize EmbedCard's own block margin inside the popover. */
	.nd-preview :global(.nd-embed) {
		margin: 0;
	}
</style>
