<script lang="ts">
	// Renders a section's plain-text content with two overlay layers merged onto
	// one pass: NIP-84 highlights (engine-resolved spans) and nostrdown
	// `{{ref|wiki|embed:…}}` references (engine-resolved refs). Both arrive as
	// UTF-16 offsets into the same `content`; `buildSegments` slices them into
	// renderable runs. Purely presentational — resolution happens in the parent
	// (which calls `api.resolveHighlights` / `api.resolveNostrdown`).
	import { getAppState } from '$lib/state.svelte';
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

	// Open a resolved reference in the reader (publication reader for a 30040,
	// single-event reader buffer for a section/article/wiki). Fragment-scroll
	// within the target is a follow-up.
	function openRef(ref: ResolvedRef) {
		if (ref.coord) app.openCoord(ref.coord);
	}

	function refTitle(ref: ResolvedRef): string {
		if (!ref.found) return `Unresolved ${ref.kind}: ${ref.target}`;
		const kind = ref.event_kind ? ` (kind ${ref.event_kind})` : '';
		const frag = ref.fragment ? ` #${ref.fragment}` : '';
		return `${ref.kind}: ${ref.target}${kind}${frag}`;
	}
</script>

<pre class="section-content" class:muted>{#each segments as seg, i (i)}{#if seg.type === 'highlight'}<mark class="hl-overlay" data-hl-ids={seg.highlight.id} style={styleFor(seg.highlight.pubkey, seg.highlight.focused)} title="NIP-84 highlight {seg.highlight.id.slice(0, 8)}… by {seg.highlight.pubkey.slice(0, 12)}…">{seg.text}</mark>{:else if seg.type === 'ref'}{#if seg.ref.kind === 'embed'}<span class="nd-embed" class:nd-unresolved={!seg.ref.found}><span class="nd-embed__head"><span class="nd-embed__label">⧉ {seg.ref.label}</span>{#if seg.ref.coord}<button class="nd-embed__open" onclick={() => openRef(seg.ref)} title={refTitle(seg.ref)}>open</button>{/if}</span>{#if seg.ref.found && seg.ref.content}<span class="nd-embed__body">{seg.ref.content}</span>{:else}<span class="nd-embed__missing">embed unavailable — {seg.ref.target}</span>{/if}</span>{:else}<button class="nd-ref nd-ref--{seg.ref.kind}" class:nd-unresolved={!seg.ref.found} onclick={() => openRef(seg.ref)} disabled={!seg.ref.coord} title={refTitle(seg.ref)}>{seg.ref.label}{#if seg.ref.fragment}<span class="nd-ref__frag">#{seg.ref.fragment}</span>{/if}</button>{/if}{:else}{seg.text}{/if}{/each}</pre>

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
	.nd-ref.nd-unresolved,
	.nd-embed.nd-unresolved {
		color: var(--fg-muted);
		background: none;
		border-bottom: 1px dotted var(--fg-muted);
		cursor: default;
	}

	/* Nostrdown embed / transclusion — a block box carrying another event's
	   content inline (depth-1; nested embeds are not expanded yet). */
	.nd-embed {
		display: block;
		margin: 8px 0;
		border-left: 3px solid var(--id-yours);
		border-radius: var(--r-sm, 3px);
		background: color-mix(in srgb, var(--id-yours) 5%, transparent);
		padding: 6px 10px;
	}
	.nd-embed__head {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 4px;
	}
	.nd-embed__label {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--id-yours);
	}
	.nd-embed__open {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		border: 1px solid var(--border);
		background: var(--bg-surface);
		color: var(--id-yours);
		border-radius: var(--radius);
		padding: 0 6px;
		cursor: pointer;
	}
	.nd-embed__open:hover {
		border-color: var(--id-yours);
	}
	.nd-embed__body {
		display: block;
		white-space: pre-wrap;
		color: var(--fg);
	}
	.nd-embed__missing {
		display: block;
		font-style: italic;
		color: var(--fg-muted);
		font-size: var(--t-2xs);
	}
</style>
