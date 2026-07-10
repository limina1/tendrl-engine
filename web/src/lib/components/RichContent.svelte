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
	import { buildSegments, type ResolvedRef, type ParsedToken } from '$lib/nostr/nostrdown';
	import type { ResolutionTracker } from '$lib/nostr/resolution-progress.svelte';

	const app = getAppState();

	let {
		content,
		spans = [],
		refs = [],
		tokens = [],
		resolution = undefined,
		focusedHighlightId = null,
		muted = false
	}: {
		content: string;
		spans?: HighlightSpan[];
		refs?: ResolvedRef[];
		/** Engine-parsed token spans for the pre-resolution "resolving" chips
		 *  (from `api.parseNostrdown`); superseded by `refs` once `/resolve` lands. */
		tokens?: ParsedToken[];
		/** The enclosing reader's resolution tracker (threaded as a prop, not
		 *  context — see resolution-progress). Absent → inert. */
		resolution?: ResolutionTracker;
		focusedHighlightId?: string | null;
		muted?: boolean;
	} = $props();

	const segments = $derived(buildSegments(content, spans, refs, tokens, focusedHighlightId));

	// Progress is reported by the embed/quote/slot cards themselves (each knows
	// when it's still fetching a not-local event from relays — the actually-slow
	// step), so the tracker is passed straight through to EmbedCard below. Inline
	// ref/wiki/mention links resolve instantly and aren't counted.

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
		// An unresolved wiki link doesn't dead-end — open the search frame seeded
		// with the topic (Auto auto-fetches, Confirm searches local + offers relays).
		else if (ref.kind === 'wiki') app.openSearchFor(`k:30818 d:${ref.target}`, ref.target);
	}

	function refTitle(ref: ResolvedRef): string {
		if (!ref.found)
			return ref.kind === 'wiki'
				? `Search for “${ref.target}”`
				: `Unresolved ${ref.kind}: ${ref.target}`;
		const kind = ref.event_kind ? ` (kind ${ref.event_kind})` : '';
		return `${ref.kind}: ${ref.target}${kind}`;
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

<!-- Every segment element carries its source position (`data-src-start`, plus
     `data-src-end` on atomic chips/cards whose DOM text differs from the
     source span) so selection capture can map DOM positions back to UTF-16
     content offsets. Text runs get a style-free wrapper span for the same
     reason — inline and inert inside the pre-wrap. -->
<pre class="section-content" class:muted>{#each segments as seg, i (i)}{#if seg.type === 'highlight'}<mark class="hl-overlay" data-hl-ids={seg.highlight.id} data-src-start={seg.srcStart} style={styleFor(seg.highlight.pubkey, seg.highlight.focused)} title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…">{seg.text}</mark>{:else if seg.type === 'ref'}{#if seg.ref.kind === 'embed' || seg.ref.kind === 'quote' || seg.ref.kind === 'slot'}<span data-src-start={seg.srcStart} data-src-end={seg.srcEnd}><EmbedCard ref={seg.ref} onopen={openRef} {resolution} /></span>{:else}<button class="nd-ref nd-ref--{seg.ref.kind}" class:nd-unresolved={!seg.ref.found} data-src-start={seg.srcStart} data-src-end={seg.srcEnd} onclick={() => openRef(seg.ref)} onmouseenter={(e) => showPreview(e, seg.ref)} onmouseleave={hidePreview} onfocus={(e) => showPreview(e, seg.ref)} onblur={hidePreview} disabled={!seg.ref.coord && !(seg.ref.event_kind === 0 && seg.ref.author_pubkey) && seg.ref.kind !== 'wiki'} title={refTitle(seg.ref)}>{seg.ref.kind === 'mention' ? '@' : ''}{seg.ref.label}</button>{/if}{:else if seg.type === 'token'}<span class="nd-token nd-token--{seg.kind}" data-src-start={seg.srcStart} data-src-end={seg.srcEnd} title="{seg.kind}: {seg.target} — resolving…">{seg.display || seg.target}</span>{:else}<span data-src-start={seg.srcStart}>{seg.text}</span>{/if}{/each}</pre>
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
	.nd-ref.nd-unresolved {
		color: var(--fg-muted);
		background: none;
		border-bottom: 1px dotted var(--fg-muted);
		cursor: default;
	}

	/* A `{{ }}` token before the engine resolves it: reads as a reference (not
	   plain text) with a faint pulse to signal "resolving", so the syntax never
	   flashes as raw prose. Replaced in place once the resolved ref lands. */
	.nd-token {
		font: inherit;
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 7%, transparent);
		border-radius: var(--r-sm, 3px);
		padding: 0 3px;
		border-bottom: 1px dotted color-mix(in srgb, var(--id-yours) 55%, transparent);
		animation: nd-token-pulse 1.4s ease-in-out infinite;
	}
	.nd-token::before {
		content: '⧉ ';
		opacity: 0.6;
		font-size: 0.85em;
	}
	.nd-token--ref::before,
	.nd-token--wiki::before {
		content: '↗ ';
	}
	.nd-token--quote::before {
		content: '❝ ';
	}
	@keyframes nd-token-pulse {
		0%,
		100% {
			opacity: 0.62;
		}
		50% {
			opacity: 1;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.nd-token {
			animation: none;
			opacity: 0.78;
		}
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
