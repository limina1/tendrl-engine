<script lang="ts">
	import type { NostrEvent, SearchResult } from '$lib/types';
	import ProfileName from './ProfileName.svelte';

	let {
		event,
		onclose
	}: {
		event: NostrEvent | SearchResult;
		onclose: () => void;
	} = $props();

	type Normalized = {
		id: string;
		pubkey: string;
		kind: number;
		tags: string[][];
		content: string;
		created_at: number;
		title: string | null;
	};

	// SearchResult and NostrEvent diverge: SearchResult uses `event_id`/`author`,
	// lacks `content` (only `preview`), and ships a denormalized `title`.
	// Normalize at the boundary so the rest of the component reads one shape.
	function normalize(input: NostrEvent | SearchResult): Normalized {
		if ('event_id' in input) {
			return {
				id: input.event_id,
				pubkey: input.author,
				kind: input.kind,
				tags: input.tags,
				content: input.preview,
				created_at: input.created_at,
				title: input.title
			};
		}
		const titleTag = input.tags.find((t) => t[0] === 'title');
		return {
			id: input.id,
			pubkey: input.pubkey,
			kind: input.kind,
			tags: input.tags,
			content: input.content,
			created_at: input.created_at,
			title: titleTag?.[1] ?? null
		};
	}

	const n = $derived(normalize(event));
	let rawOpen = $state(false);

	const KIND_LABEL: Record<number, string> = {
		0: 'profile',
		1: 'note',
		3: 'contacts',
		10002: 'relay list',
		30023: 'long-form',
		30040: 'publication index',
		30041: 'publication section',
		9802: 'highlight',
		30817: 'wiki',
		30818: 'wiki page'
	};

	function formatTime(ts: number): string {
		return new Date(ts * 1000).toLocaleString();
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="evm-backdrop" onclick={onclose} role="presentation">
	<div class="evm" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
		<header class="evm__header">
			<div class="evm__title-row">
				<span class="evm__title">{n.title ?? '[Untitled]'}</span>
				<span class="evm__kind">{KIND_LABEL[n.kind] ?? `kind ${n.kind}`}</span>
				<button class="evm__close" onclick={onclose} title="Close (Esc)">✕</button>
			</div>
			<div class="evm__meta">
				<ProfileName pubkey={n.pubkey} />
				<span class="evm__time">{formatTime(n.created_at)}</span>
			</div>
		</header>

		<section class="evm__section">
			<!-- Identifiers block — Slice 5 -->
			<div class="evm__placeholder">Identifiers · pending Slice 5</div>
		</section>

		<section class="evm__section">
			<!-- Tags block — Slice 5 -->
			<div class="evm__placeholder">{n.tags.length} tags · pending Slice 5</div>
		</section>

		<section class="evm__section">
			<!-- Containing publications block — Slice 6 -->
			<div class="evm__placeholder">Containing publications · pending Slice 6</div>
		</section>

		<section class="evm__section evm__section--raw">
			<button class="evm__raw-toggle" onclick={() => (rawOpen = !rawOpen)}>
				<span class="evm__raw-arrow" class:open={rawOpen}>{rawOpen ? '▾' : '▸'}</span>
				Raw JSON
			</button>
			{#if rawOpen}
				<pre class="evm__raw">{JSON.stringify(event, null, 2)}</pre>
			{/if}
		</section>
	</div>
</div>

<style>
	.evm-backdrop {
		position: fixed;
		/* Stop short of the modeline so the search-history pill stays
		   clickable while the modal is open. */
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 100;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.evm {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: min(720px, 90vw);
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.evm__header {
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.evm__title-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.evm__title {
		flex: 1;
		font-weight: 600;
		font-size: 0.95rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.evm__kind {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		text-transform: lowercase;
	}

	.evm__close {
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font-size: 0.9rem;
		padding: 2px 6px;
	}

	.evm__close:hover {
		color: var(--fg);
	}

	.evm__meta {
		display: flex;
		align-items: center;
		gap: 10px;
		margin-top: 4px;
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	.evm__section {
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.evm__section:last-child {
		border-bottom: none;
	}

	.evm__section--raw {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
	}

	.evm__placeholder {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-style: italic;
	}

	.evm__raw-toggle {
		background: none;
		border: none;
		color: var(--fg);
		cursor: pointer;
		font-size: 0.8rem;
		font-weight: 500;
		padding: 0;
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.evm__raw-arrow {
		display: inline-block;
		font-size: 0.7rem;
	}

	.evm__raw {
		margin-top: 8px;
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--fg-muted);
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
