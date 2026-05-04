<script lang="ts">
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import SectionCard from '$lib/components/SectionCard.svelte';
	import type { ComposeState, LazySection, ViewMode, ContextItem } from '$lib/types';
	import { sectionState, segmentSections } from '$lib/compose/state';

	let {
		compose,
		ontogglereadonly,
		onremove,
		onunlockall,
		onlockall
	}: {
		compose: ComposeState;
		ontogglereadonly?: (id: string) => void;
		onremove?: (id: string) => void;
		onunlockall?: () => void;
		onlockall?: () => void;
	} = $props();

	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);

	// Adapter: ComposeState.sections (ContextItem[]) → LazySection[].
	// New sections without a source_addr get a synthetic addr so the
	// keyed-each in nested views stays stable across reorders.
	const sections = $derived<LazySection[]>(
		compose.sections.map((s, i) => ({
			addr: s.source_addr ?? {
				kind: 30041,
				pubkey: '',
				d_tag: s.id
			},
			title: s.title || null,
			content: s.content,
			position: i,
			status: 'loaded' as const
		}))
	);

	const segments = $derived(segmentSections(compose));

	function itemAt(index: number): ContextItem | null {
		return compose.sections[index] ?? null;
	}

	function stateAt(index: number) {
		const item = itemAt(index);
		return item ? sectionState(item) : 'original';
	}

	function toggleLock(index: number) {
		const item = itemAt(index);
		if (item && ontogglereadonly) ontogglereadonly(item.id);
	}

	function removeAt(index: number) {
		const item = itemAt(index);
		if (item && onremove) onremove(item.id);
	}

	const anyUnlocked = $derived(
		compose.sections.some((s) => s.source_addr && !s.readonly)
	);
	const anyLockable = $derived(
		compose.sections.some((s) => s.source_addr && s.readonly)
	);
</script>

<div class="draft-reader">
	<div class="toolbar">
		<button class:active={viewMode === 'outline'} onclick={() => (viewMode = 'outline')}>Outline</button>
		<button class:active={viewMode === 'continuous'} onclick={() => (viewMode = 'continuous')}>Continuous</button>
		<button class:active={viewMode === 'paginated'} onclick={() => (viewMode = 'paginated')}>Paginated</button>
		<span class="sp"></span>
		{#if onunlockall}
			<button
				class="bulk"
				onclick={onunlockall}
				disabled={!anyLockable}
				title="Unlock all imported sections (yellow — claimed for reorder/edit)"
			>Unlock all</button>
		{/if}
		{#if onlockall}
			<button
				class="bulk"
				onclick={onlockall}
				disabled={!anyUnlocked}
				title="Lock all unlocked sections (green — transcluded as-is)"
			>Lock all</button>
		{/if}
	</div>

	{#if sections.length === 0}
		<div class="empty"><p>No sections in draft. Add content from the Plain or Full tab.</p></div>
	{:else if viewMode === 'outline'}
		<div class="outline-overlay">
			{#each segments as seg, segIdx (segIdx + ':' + seg.indices.join(','))}
				<div
					class="segment"
					class:segment--imported={seg.state === 'imported'}
					class:segment--claimed={seg.state === 'claimed'}
					class:segment--forked={seg.state === 'forked'}
					class:segment--original={seg.state === 'original'}
					class:segment--group={seg.indices.length > 1}
				>
					{#each seg.indices as i (i)}
						{@const item = compose.sections[i]}
						{@const st = stateAt(i)}
						{@const isLast = seg.indices[seg.indices.length - 1] === i}
						<div
							class="entry"
							class:entry--imported={st === 'imported'}
							class:entry--claimed={st === 'claimed'}
							class:entry--forked={st === 'forked'}
							class:entry--original={st === 'original'}
						>
							<div class="rail" aria-hidden="true">
								{#if seg.indices.length > 1}
									<span class="rail-glyph" title="Locked group — moves together">{isLast ? '└' : (seg.indices[0] === i ? '┌' : '│')}</span>
								{/if}
							</div>
							{#if item && item.source_addr}
								<button
									class="lock"
									class:lock--unlocked={st === 'claimed' || st === 'forked'}
									onclick={() => toggleLock(i)}
									title={st === 'imported'
										? 'Unlock — claim for reorder / fork'
										: st === 'forked'
											? 'Forked (content diverged) — re-lock blocked'
											: 'Lock — restore as transcluded'}
									disabled={st === 'forked'}
								>{st === 'imported' ? '🔒' : '🔓'}</button>
							{:else}
								<span class="lock lock--placeholder" title="Original — no source to lock against">·</span>
							{/if}
							<div class="entry-body">
								<SectionCard
									section={sections[i]}
									preview
									index={i + 1}
									onclick={() => {
										viewMode = 'paginated';
										currentSection = i;
									}}
								/>
							</div>
							{#if onremove && st !== 'imported' && item}
								<button
									class="remove"
									onclick={() => removeAt(i)}
									title="Remove from draft"
								>✕</button>
							{/if}
						</div>
					{/each}
				</div>
			{/each}
			<p class="hint">
				Green = transcluded as-is. Yellow = claimed (unlocked, no edits).
				Purple = forked (content diverged). No border = original.
			</p>
		</div>
	{:else if viewMode === 'continuous'}
		<ContinuousView {sections} publication={null} />
	{:else}
		<PaginatedView
			{sections}
			{currentSection}
			onnavigate={(i) => (currentSection = i)}
		/>
	{/if}
</div>

<style>
	.draft-reader { display: flex; flex-direction: column; flex: 1; min-height: 0; height: 100%; }
	.toolbar {
		display: flex;
		gap: 4px;
		padding: 6px 12px;
		border-bottom: 1px solid var(--panel-border);
		background: var(--panel-bg-soft);
		flex-shrink: 0;
	}
	.toolbar button {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.toolbar button.active {
		background: var(--id-yours);
		color: var(--bg);
		border-color: var(--id-yours);
	}
	.toolbar .sp { flex: 1; }
	.toolbar .bulk:disabled { opacity: 0.4; cursor: not-allowed; }

	.empty {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}

	.outline-overlay {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 8px;
	}

	/* Segment wrapper: groups consecutive imported entries visually so it
	   reads as a single movable unit in the reorder model. */
	.segment {
		margin-bottom: 6px;
	}
	.segment--group.segment--imported {
		border-left: 2px solid var(--green);
		padding-left: 4px;
	}

	.entry {
		display: grid;
		grid-template-columns: 14px auto 1fr auto;
		gap: 6px;
		align-items: flex-start;
		padding: 4px 6px;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		margin-bottom: 2px;
	}

	/* State-derived borders.
	   - imported: green; transcluded as-is, no new event on publish.
	   - claimed:  yellow; unlocked, content unchanged.
	   - forked:   violet; content diverged, will publish a fork.
	   - original: no border; authored fresh, plain 30041 on publish.

	   Iceberg notes: --id-draft is RED (unsigned-draft semantic), so we
	   use --yellow directly here. --id-imported is magenta (read-only-
	   import) which conflicts with the design model where imported = "as
	   the original author wrote it"; we use --green to convey the
	   attribution-clean semantic. */
	.entry--imported {
		border-color: var(--green);
		background: color-mix(in srgb, var(--green) 6%, transparent);
	}
	.entry--claimed {
		border-color: var(--yellow);
		background: color-mix(in srgb, var(--yellow) 7%, transparent);
	}
	.entry--forked {
		border-color: var(--id-forked);
		background: color-mix(in srgb, var(--id-forked) 8%, transparent);
	}
	.entry--original { /* no border on purpose */ }

	.rail {
		font-family: var(--font-mono);
		color: var(--green);
		font-size: 14px;
		line-height: 1;
		padding-top: 6px;
	}

	.lock {
		flex-shrink: 0;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		font-size: 12px;
		padding: 0 6px;
		cursor: pointer;
		color: var(--base6);
		align-self: flex-start;
	}
	.lock--unlocked {
		border-color: var(--yellow);
		color: var(--yellow);
	}
	.lock--placeholder {
		opacity: 0.3;
		cursor: default;
	}
	.lock:hover:not(:disabled):not(.lock--placeholder) {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.lock:disabled { opacity: 0.6; cursor: not-allowed; }

	.entry-body { min-width: 0; }

	.remove {
		flex-shrink: 0;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		font-size: 11px;
		padding: 0 6px;
		cursor: pointer;
		color: var(--base5);
	}
	.remove:hover {
		border-color: var(--red);
		color: var(--red);
	}

	.hint {
		padding: 12px;
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
		text-align: center;
		margin: 0;
	}
</style>
