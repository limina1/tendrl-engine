<script lang="ts">
	import type { ContextItem, SyncMode } from '$lib/types';
	import { sectionState, sourceKindLabel } from '$lib/compose/state';

	let {
		item,
		syncMode,
		panel,
		ontogglereadonly,
		onlocksource,
		oncrosspanelcopy
	}: {
		item: ContextItem;
		syncMode: SyncMode;
		panel: 'context' | 'compose' | 'search';
		ontogglereadonly?: (id: string) => void;
		onlocksource?: (id: string) => void;
		oncrosspanelcopy?: (id: string, fromPanel: string) => void;
	} = $props();

	const inBoth = $derived(item.in_context && item.in_compose);
	const contentMatch = $derived(item.content === item.context_content);

	// Cross-panel badge: context panel shows [compose], compose panel shows [context]
	// Chat-origin items only show cross-panel badge when in_context (chat badge hides)
	const crossLabel = $derived(
		panel === 'search'
			? item.in_context && item.in_compose
				? 'ctx+doc'
				: item.in_context
					? 'context'
					: item.in_compose
						? 'compose'
						: null
			: panel === 'context' && item.in_compose
				? 'compose'
				: panel === 'compose' && item.in_context
					? 'context'
					: null
	);

	// --- Armed copy state (like trash two-step) ---
	let copyArmed = $state(false);
	let copyTimer: ReturnType<typeof setTimeout> | null = $state(null);
	let copyCountdown = $state(0);
	let copyInterval: ReturnType<typeof setInterval> | null = $state(null);

	// Cross-panel color: armed=pulsing, readonly=blue, matched=green, diverged=yellow
	const crossColor = $derived(
		copyArmed ? 'armed' : item.readonly ? 'readonly' : contentMatch ? 'synced' : 'modified'
	);

	// Origin badge color
	const originColor = $derived(item.readonly ? 'readonly' : 'synced');

	// Provenance badge for sourced items (imported via nevent/naddr or pulled
	// from search): kind label + lock state. Drives the user-facing
	// imported/claimed/forked signal in the compose Detected panel.
	const srcState = $derived(item.source_addr ? sectionState(item) : null);
	const srcKind = $derived(item.source_addr ? sourceKindLabel(item.source_addr.kind) : '');
	const srcIcon = $derived(
		srcState === 'imported' ? '🔒' : srcState === 'claimed' ? '🔓' : '⑂'
	);
	const srcColor = $derived(
		srcState === 'imported' ? 'readonly' : srcState === 'claimed' ? 'modified' : 'forked'
	);
	const srcTitle = $derived(
		srcState === 'imported'
			? `Locked to original ${srcKind} — publishes a reference to the source event (kept under its author). Click to unlock.`
			: srcState === 'claimed'
				? `Unlocked but unchanged — still publishes as a reference to the original ${srcKind}. Click to re-lock.`
				: `Diverged from the original ${srcKind} — publishes as your fork with lineage tags. Click to reset to the original and re-lock.`
	);

	// Show chat badge only when not in_context (once in context, chat badge disappears)
	const showChatBadge = $derived(item.origin === 'chat' && !item.in_context);

	function clearArmed() {
		copyArmed = false;
		copyCountdown = 0;
		if (copyTimer) clearTimeout(copyTimer);
		copyTimer = null;
		if (copyInterval) clearInterval(copyInterval);
		copyInterval = null;
	}

	function handleCrossClick() {
		if (copyArmed) {
			// Second click — copy content from this panel to other
			oncrosspanelcopy?.(item.id, panel);
			clearArmed();
		} else {
			// First click — lock readonly + arm for copy
			ontogglereadonly?.(item.id);
			copyArmed = true;
			copyCountdown = 10;
			copyTimer = setTimeout(clearArmed, 10000);
			copyInterval = setInterval(() => {
				copyCountdown--;
				if (copyCountdown <= 0) clearArmed();
			}, 1000);
		}
	}
</script>

<span class="badges">
	{#if showChatBadge}
		<span class="badge badge-chat">chat</span>
	{/if}
	{#if srcState}
		<button
			class="badge badge-{srcColor}"
			onclick={() => onlocksource?.(item.id)}
			title={srcTitle}
		>{srcKind} {srcIcon}</button>
	{:else if item.origin === 'search'}
		<button
			class="badge badge-{originColor}"
			onclick={() => onlocksource?.(item.id)}
			title={item.readonly ? 'Unlock from source' : 'Lock to source (reset)'}
		>search{#if item.readonly} 🔒{/if}</button>
	{/if}
	{#if item.origin === 'compose'}
		<span class="badge badge-{originColor}">compose</span>
	{/if}
	{#if crossLabel}
		<button
			class="badge badge-{crossColor}"
			onclick={handleCrossClick}
			title={copyArmed ? `Copy to ${crossLabel} (${copyCountdown}s)` : item.readonly ? 'Unlock' : 'Lock'}
		>{crossLabel}{#if item.readonly && !copyArmed} 🔒{/if}{#if copyArmed} ⇄ {copyCountdown}s{/if}</button>
	{/if}
</span>

<style>
	.badges {
		display: inline-flex;
		gap: 3px;
		flex-shrink: 0;
	}

	.badge {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 4px;
		white-space: nowrap;
		font-weight: 600;
		line-height: 1.6;
		border: none;
		cursor: default;
	}

	button.badge {
		cursor: pointer;
	}

	.badge-synced {
		background: color-mix(in srgb, var(--badge-synced) 20%, transparent);
		color: var(--badge-synced);
	}

	.badge-modified {
		background: color-mix(in srgb, var(--badge-modified) 20%, transparent);
		color: var(--badge-modified);
	}

	.badge-chat {
		background: color-mix(in srgb, var(--badge-chat) 20%, transparent);
		color: var(--badge-chat);
	}

	.badge-readonly {
		background: color-mix(in srgb, var(--badge-readonly) 20%, transparent);
		color: var(--badge-readonly);
	}

	.badge-forked {
		background: color-mix(in srgb, var(--id-forked) 20%, transparent);
		color: var(--id-forked);
	}

	.badge-armed {
		background: color-mix(in srgb, var(--badge-modified) 40%, transparent);
		color: var(--badge-modified);
		animation: pulse 1s ease-in-out infinite;
	}

	@keyframes pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.6; }
	}
</style>
