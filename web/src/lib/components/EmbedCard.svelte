<script lang="ts">
	// One field-driven reference card, shared by every surface so they look
	// identical: the reader's inline embeds, the reader's ref/wiki hover preview,
	// and the composer's click preview (mounted into a CodeMirror tooltip). It
	// renders whatever the resolved event carries — image (avatar for profiles,
	// cover for media, thumb for articles), title, byline, and a text body.
	import { getProfile } from '$lib/api';
	import type { ResolvedRef } from '$lib/nostr/nostrdown';

	let { ref, onopen }: { ref: ResolvedRef; onopen?: (ref: ResolvedRef) => void } = $props();

	const KIND_LABEL: Record<number, string> = {
		0: 'profile',
		1: 'note',
		30023: 'article',
		30040: 'publication',
		30041: 'section',
		30818: 'wiki'
	};
	const isEntityLabel = (s: string) =>
		/^(nostr:)?(naddr1|nevent1|note1|npub1|nprofile1)/i.test(s);

	// Don't dump a raw bech32 entity as the title — fall back to a kind name.
	const title = $derived(
		ref.title ||
			(ref.kind === 'quote'
				? 'quote'
				: isEntityLabel(ref.label)
					? KIND_LABEL[ref.event_kind ?? -1] ?? 'embed'
					: ref.label)
	);
	// A quote card uses a quotation glyph; everything else the transclusion mark.
	const glyph = $derived(ref.kind === 'quote' ? '❝' : '⧉');
	const body = $derived(ref.content || ref.summary || '');
	const canOpen = $derived(!!ref.coord || (ref.event_kind === 0 && !!ref.author_pubkey));

	// Quote a long transcluded body (a whole section is huge) as a collapsed
	// excerpt with a show-more toggle, so the embed stays a readable quotation.
	const QUOTE_CAP = 360;
	let expanded = $state(false);
	const isLong = $derived(body.length > QUOTE_CAP);
	const shownBody = $derived(isLong && !expanded ? body.slice(0, QUOTE_CAP).trimEnd() + '…' : body);

	// Byline for non-profile cards: the author tag, else the publisher's kind-0
	// name (resolved), else a short pubkey. (Profiles put the name in the title.)
	let authorName = $state<string | undefined>(undefined);
	$effect(() => {
		authorName = undefined;
		if (!ref.found || ref.event_kind === 0 || ref.author || !ref.author_pubkey) return;
		const pk = ref.author_pubkey;
		let cancelled = false;
		getProfile(pk)
			.then((p) => {
				if (!cancelled) authorName = p.display_name || p.name || undefined;
			})
			.catch(() => {});
		return () => {
			cancelled = true;
		};
	});
	const byline = $derived(
		ref.event_kind === 0
			? ''
			: ref.author || authorName || (ref.author_pubkey ? ref.author_pubkey.slice(0, 10) + '…' : '')
	);

	function openTitle(): string {
		if (!ref.found) return `Unresolved ${ref.kind}: ${ref.target}`;
		const k = ref.event_kind ? ` (kind ${ref.event_kind})` : '';
		const frag = ref.fragment ? ` #${ref.fragment}` : '';
		return `${ref.kind}: ${ref.target}${k}${frag}`;
	}
</script>

<span class="nd-embed" class:nd-unresolved={!ref.found}><span class="nd-embed__head">{#if ref.image && (ref.event_kind === 0 || body)}<img class="nd-embed__img" class:nd-embed__img--avatar={ref.event_kind === 0} src={ref.image} alt="" referrerpolicy="no-referrer" loading="lazy" />{/if}<span class="nd-embed__titles"><span class="nd-embed__label">{glyph} {title}</span>{#if byline}<span class="nd-embed__by">{byline}</span>{/if}</span>{#if canOpen && onopen}<button class="nd-embed__open" onclick={() => onopen?.(ref)} title={openTitle()}>{ref.event_kind === 0 ? 'profile' : 'open'}</button>{/if}</span>{#if ref.found}{#if body}<blockquote class="nd-embed__body">{shownBody}</blockquote>{#if isLong}<button class="nd-embed__more" onclick={() => (expanded = !expanded)}>{expanded ? 'show less' : 'show more'}</button>{/if}{:else if ref.image && ref.event_kind !== 0}<img class="nd-embed__cover" src={ref.image} alt="" referrerpolicy="no-referrer" loading="lazy" />{/if}{:else}<span class="nd-embed__missing">embed unavailable — {ref.target}</span>{/if}</span>

<style>
	.nd-embed {
		display: block;
		margin: 8px 0;
		max-width: 100%;
		overflow: hidden;
		border-left: 3px solid var(--id-yours);
		border-radius: var(--r-sm, 3px);
		background: color-mix(in srgb, var(--id-yours) 5%, transparent);
		padding: 6px 10px;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.5;
	}
	.nd-embed__head {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 4px;
	}
	.nd-embed__img {
		flex: 0 0 auto;
		width: 36px;
		height: 36px;
		object-fit: cover;
		border-radius: var(--r-sm, 3px);
		border: 1px solid var(--border);
	}
	.nd-embed.nd-unresolved {
		border-left-color: var(--fg-muted);
		background: none;
		color: var(--fg-muted);
	}
	.nd-embed__img--avatar {
		border-radius: 50%;
	}
	.nd-embed__titles {
		display: flex;
		flex-direction: column;
		gap: 1px;
		flex: 1;
		min-width: 0;
	}
	.nd-embed__label {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--id-yours);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nd-embed__by {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
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
	/* The transcluded text reads as a quotation: a slim rule + slight indent,
	   set apart from the source attribution in the header above. */
	.nd-embed__body {
		display: block;
		margin: 4px 0 0;
		padding-left: 8px;
		border-left: 2px solid color-mix(in srgb, var(--id-yours) 35%, transparent);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
		color: var(--fg);
		font-style: italic;
	}
	.nd-embed__more {
		margin-top: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		background: none;
		border: none;
		color: var(--id-yours);
		cursor: pointer;
		padding: 0;
	}
	.nd-embed__more:hover {
		text-decoration: underline;
	}
	.nd-embed__cover {
		display: block;
		max-width: 100%;
		max-height: 180px;
		object-fit: contain;
		border-radius: var(--r-sm, 3px);
		border: 1px solid var(--border);
	}
	.nd-embed__missing {
		display: block;
		font-style: italic;
		color: var(--fg-muted);
		font-size: var(--t-2xs);
		overflow-wrap: anywhere;
	}
</style>
