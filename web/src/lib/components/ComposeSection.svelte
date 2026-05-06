<script lang="ts">
	import type { ContextItem, TagEntry, SyncMode } from '$lib/types';
	import TagEditor from './TagEditor.svelte';
	import ItemBadge from './ItemBadge.svelte';
	import { sectionState } from '$lib/compose/state';

	let {
		section,
		checked,
		syncMode,
		collapsed,
		oncheck,
		oncollapse,
		onupdate,
		onupdatetags,
		onreset,
		onremove,
		onsendtochat,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy,
		onreorder,
		isFirst = false,
		isLast = false
	}: {
		section: ContextItem;
		checked: boolean;
		syncMode: SyncMode;
		collapsed: boolean;
		oncheck: (id: string) => void;
		oncollapse: (id: string) => void;
		onupdate: (id: string, title: string, content: string) => void;
		onupdatetags: (id: string, tags: TagEntry[]) => void;
		onreset: (id: string) => void;
		onremove: (id: string) => void;
		onsendtochat: (id: string) => void;
		ontogglereadonly: (id: string) => void;
		onlocksource: (id: string) => void;
		oncrosspanelcopy: (id: string, fromPanel: string) => void;
		onreorder?: (id: string, dir: 'up' | 'down') => void;
		isFirst?: boolean;
		isLast?: boolean;
	} = $props();

	const provenance = $derived(sectionState(section));
</script>

<div
	class="compose-section"
	class:modified={section.modified}
	class:section--imported={provenance === 'imported'}
	class:section--claimed={provenance === 'claimed'}
	class:section--forked={provenance === 'forked'}
	class:compose-section--collapsed={collapsed}
>
	<div class="compose-section-header">
		<button
			class="collapse-toggle"
			onclick={() => oncollapse(section.id)}
			title={collapsed ? 'Expand section' : 'Collapse to title only'}
			aria-expanded={!collapsed}
		>{collapsed ? '▸' : '▾'}</button>
		<label class="check">
			<input
				type="checkbox"
				{checked}
				onchange={() => oncheck(section.id)}
			/>
		</label>
		<input
			class="compose-section-title"
			value={section.title}
			oninput={(e) => onupdate(section.id, e.currentTarget.value, section.content)}
			placeholder="Section title"
			disabled={section.readonly}
		/>
		<ItemBadge item={section} {syncMode} panel="compose" {ontogglereadonly} {onlocksource} {oncrosspanelcopy} />
		{#if onreorder}
			<button
				class="icon-btn-sm"
				onclick={() => onreorder(section.id, 'up')}
				disabled={isFirst}
				title="Move section up"
				aria-label="Move section up"
			>↑</button>
			<button
				class="icon-btn-sm"
				onclick={() => onreorder(section.id, 'down')}
				disabled={isLast}
				title="Move section down"
				aria-label="Move section down"
			>↓</button>
		{/if}
		<button class="icon-btn-sm" onclick={() => onsendtochat(section.id)} title="Send to chat">◂</button>
		<button onclick={() => onremove(section.id)}>Remove</button>
	</div>
	{#if !collapsed}
		<textarea
			value={section.content}
			oninput={(e) => onupdate(section.id, section.title, e.currentTarget.value)}
			placeholder="Section content..."
			rows="6"
			disabled={section.readonly}
		></textarea>
		<TagEditor tags={section.tags} onupdate={(tags) => onupdatetags(section.id, tags)} disabled={section.readonly} />
		{#if section.modified}
			<div class="modified-banner">
				<span>Modified</span>
				<button class="reset-btn" onclick={() => onreset(section.id)}>Reset</button>
			</div>
		{/if}
	{/if}
</div>

<style>
	.compose-section {
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.compose-section--collapsed {
		padding: 6px 12px;
	}

	.collapse-toggle {
		font-size: 0.75rem;
		padding: 0 4px;
		min-width: 18px;
		background: transparent;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.collapse-toggle:hover { color: var(--fg); }

	.compose-section.modified {
		border-color: var(--modified-border);
		background: var(--modified-bg);
	}

	/* Provenance-derived borders. Same vocabulary as DraftReader so the
	   read↔edit transition is visually continuous:
	   - imported (green): transcluded as-is, no new event on publish.
	   - claimed (yellow): unlocked but unchanged — UX flag, still publishes
	     as a transclusion unless edited (the user gets a "publish anyway?"
	     popup).
	   - forked (violet): content diverged, will publish a fork-marked 30041.
	   --id-draft is iceberg's red ("unsigned draft"), so we use --yellow
	   directly here; --id-imported is magenta in iceberg, so we use
	   --green for attribution-clean. */
	.section--imported {
		border-color: var(--green);
		background: color-mix(in srgb, var(--green) 5%, transparent);
	}
	.section--claimed {
		border-color: var(--yellow);
		background: color-mix(in srgb, var(--yellow) 6%, transparent);
	}
	.section--forked {
		border-color: var(--id-forked);
		background: color-mix(in srgb, var(--id-forked) 7%, transparent);
	}

	.compose-section-header {
		display: flex;
		gap: 8px;
		align-items: center;
	}

	.check {
		display: flex;
		align-items: center;
	}

	.compose-section-title {
		flex: 1;
		font-family: inherit;
		font-size: 0.9rem;
		font-weight: 600;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		background: var(--bg-surface);
		color: var(--fg);
		padding: 6px 10px;
		outline: none;
	}

	.compose-section-title:focus {
		border-color: var(--accent);
	}

	.icon-btn-sm {
		padding: 2px 6px;
		font-size: 0.75rem;
		min-width: 22px;
	}

	textarea {
		width: 100%;
		font-size: 0.85rem;
		line-height: 1.5;
	}

	.modified-banner {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 8px;
		border-radius: 4px;
		background: var(--modified-bg);
		color: var(--modified-fg);
		font-size: 0.75rem;
		font-weight: 600;
		border: 1px solid var(--modified-border);
	}

	.reset-btn {
		font-size: 0.7rem;
		padding: 2px 8px;
	}
</style>
