<script lang="ts">
	import OutlineView from '$lib/components/OutlineView.svelte';
	import ContinuousView from '$lib/components/ContinuousView.svelte';
	import PaginatedView from '$lib/components/PaginatedView.svelte';
	import type { ComposeState, LazySection, ViewMode, ContextItem } from '$lib/types';

	let {
		compose,
		ontogglereadonly
	}: {
		compose: ComposeState;
		ontogglereadonly?: (id: string) => void;
	} = $props();

	let viewMode = $state<ViewMode>('outline');
	let currentSection = $state(0);

	// Adapter: ComposeState.sections (ContextItem[]) → LazySection[].
	// New sections without a source_addr get a synthetic addr so the
	// keyed-each in OutlineView stays stable across reorders.
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

	// Map LazySection back to ContextItem so the lock controls can reach
	// the AppState handlers. Index match (sections / compose.sections).
	function itemAt(index: number): ContextItem | null {
		return compose.sections[index] ?? null;
	}

	// Outline-overlay lock UI. The outline lists sections; each gets a
	// 🔒/🔓 toggle that flips ContextItem.readonly via the standard handler.
	function isUnlocked(index: number): boolean {
		return !(compose.sections[index]?.readonly ?? false);
	}

	function toggleLock(index: number) {
		const item = itemAt(index);
		if (item && ontogglereadonly) ontogglereadonly(item.id);
	}
</script>

<div class="draft-reader">
	<div class="toolbar">
		<button class:active={viewMode === 'outline'} onclick={() => (viewMode = 'outline')}>Outline</button>
		<button class:active={viewMode === 'continuous'} onclick={() => (viewMode = 'continuous')}>Continuous</button>
		<button class:active={viewMode === 'paginated'} onclick={() => (viewMode = 'paginated')}>Paginated</button>
	</div>

	{#if sections.length === 0}
		<div class="empty"><p>No sections in draft. Add content from the Plain or Full tab.</p></div>
	{:else if viewMode === 'outline'}
		<div class="outline-overlay">
			{#each sections as _section, i (_section.addr.d_tag + ':' + i)}
				{@const unlocked = isUnlocked(i)}
				<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
				<div class="entry" class:entry--unlocked={unlocked}>
					<button
						class="lock"
						class:lock--unlocked={unlocked}
						onclick={() => toggleLock(i)}
						title={unlocked ? 'Lock section in place' : 'Unlock to reorder / remove'}
					>{unlocked ? '🔓' : '🔒'}</button>
					<div class="entry-body">
						<OutlineView
							sections={[_section]}
							onselect={() => {
								viewMode = 'paginated';
								currentSection = i;
							}}
						/>
					</div>
				</div>
			{/each}
			<div class="hint">
				Click 🔒 to unlock a section. Unlocked sections get a yellow accent — drag-to-reorder and transclude affordances land in the next pass.
			</div>
		</div>
	{:else if viewMode === 'continuous'}
		<ContinuousView
			{sections}
			publication={null}
		/>
	{:else}
		<PaginatedView
			{sections}
			{currentSection}
			onnavigate={(i) => (currentSection = i)}
		/>
	{/if}
</div>

<style>
	.draft-reader { display: flex; flex-direction: column; height: 100%; min-height: 0; }
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
	.entry {
		display: flex;
		gap: 8px;
		padding: 6px;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		margin-bottom: 4px;
	}
	.entry--unlocked {
		border-color: var(--badge-modified, var(--id-draft));
		background: color-mix(in srgb, var(--badge-modified, var(--id-draft)) 6%, transparent);
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
		border-color: var(--badge-modified, var(--id-draft));
		color: var(--badge-modified, var(--id-draft));
	}
	.lock:hover { border-color: var(--id-yours); color: var(--fg); }
	.entry-body { flex: 1; min-width: 0; }
	.hint {
		padding: 12px;
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
		text-align: center;
	}
</style>
