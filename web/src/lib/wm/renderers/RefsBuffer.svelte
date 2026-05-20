<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import type { Buffer } from '../types';
	import type { ContextItem } from '$lib/types';

	// The refs buffer is the held-filtered view of the unified item pool.
	// "held" means: in the pool with no active routing intent yet — the
	// neutral bookmark state that Phase 7 of workbench-architecture.org
	// introduces. From here the user routes onward (→ compose, → chat,
	// cite, → search) — but those routes are deferred to a later pass.
	// This pass: list + open + drop. That alone makes the held flag useful.

	// The refs view is global — it reads app.heldEntries, not anything
	// keyed on the active buffer. We accept the prop to match the
	// renderer contract; we read its label inside a derived so the
	// linter doesn't flag a static prop read.
	let { buffer }: { buffer: Buffer } = $props();
	const bufferLabel = $derived(buffer.label ?? 'references');

	const app = getAppState();

	const KIND_LABEL: Record<number, string> = {
		1: 'note',
		1111: 'comment',
		9802: 'highlight',
		30023: 'article',
		30040: 'publication',
		30041: 'section',
		30818: 'wiki'
	};

	function kindLabel(item: ContextItem): string {
		const k = item.source_addr?.kind;
		if (k == null) return '—';
		return KIND_LABEL[k] ?? `kind ${k}`;
	}

	function openItem(item: ContextItem) {
		// Prefer the addressable coordinate when we have one — it resolves
		// to the latest version of a replaceable event. Fall back to the
		// pinned event id otherwise.
		if (item.source_addr) {
			app.openAddressableInModal(item.source_addr);
		} else if (item.source_event_id) {
			app.getEventForModal(item.source_event_id);
		}
	}
</script>

<div class="refs">
	<div class="refs__header">
		<span>{bufferLabel} ({app.heldEntries.length})</span>
	</div>
	{#if app.heldEntries.length === 0}
		<div class="empty">
			<p>Nothing held.</p>
			<p class="hint">
				Open an event's menu (<kbd>m</kbd>) and toggle the <strong>refs</strong> square to hold
				it here without routing it into chat or compose.
			</p>
		</div>
	{:else}
		<div class="refs__list">
			{#each app.heldEntries as item (item.id)}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div
					class="row"
					onclick={() => openItem(item)}
					onkeydown={(e) => {
						if (e.key === 'Enter' || e.key === 'm') openItem(item);
						else if (e.key === 'x') app.releaseHeldItem(item.id);
					}}
					role="button"
					tabindex="0"
				>
					<div class="row-body">
						<div class="row-head">
							<span class="title">{item.title}</span>
							<span class="kind">{kindLabel(item)}</span>
							{#if item.in_context}<span class="loc">context</span>{/if}
							{#if item.in_compose}<span class="loc">compose</span>{/if}
						</div>
						{#if item.source_addr?.pubkey}
							<div class="row-foot">
								<ProfileName pubkey={item.source_addr.pubkey} onviewprofile={app.handleViewProfile} />
							</div>
						{/if}
					</div>
					<button
						class="drop"
						onclick={(e) => {
							e.stopPropagation();
							app.releaseHeldItem(item.id);
						}}
						title="Release from refs (x)"
					>
						drop
					</button>
				</div>
			{/each}
		</div>
	{/if}
</div>

<style>
	.refs { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.refs__header {
		padding: 8px 12px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.refs__list { flex: 1; overflow-y: auto; }
	.empty {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 6px;
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.hint { max-width: 42ch; font-size: var(--t-xs); color: var(--base5); }
	.hint kbd {
		font-family: var(--font-mono);
		font-size: 0.85em;
		padding: 1px 5px;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		background: var(--panel-bg-soft);
	}

	.row {
		padding: 8px 12px;
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		align-items: flex-start;
		gap: 8px;
		cursor: pointer;
		border-left: 3px solid var(--id-imported);
	}
	.row:hover { background: var(--panel-bg-soft); }
	.row-body { flex: 1; min-width: 0; }
	.row-head { display: flex; align-items: center; gap: 8px; margin-bottom: 2px; }
	.title {
		font-size: var(--t-sm);
		font-weight: 600;
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.kind {
		font-size: 0.65rem;
		padding: 1px 6px;
		border-radius: 4px;
		background: var(--border);
		color: var(--fg-muted);
		white-space: nowrap;
	}
	/* Memberships beyond `held` get a small chip so a held item that's
	   *also* in context/compose isn't invisible from this view. */
	.loc {
		font-size: 0.6rem;
		padding: 0 5px;
		border-radius: 3px;
		background: color-mix(in srgb, var(--id-yours) 20%, transparent);
		color: var(--id-yours);
		white-space: nowrap;
		font-weight: 600;
	}
	.row-foot {
		display: flex;
		gap: 8px;
		font-size: var(--t-xs);
		color: var(--base5);
		margin-top: 4px;
	}
	.drop {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 2px 8px;
		background: transparent;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
		color: var(--base6);
		cursor: pointer;
	}
	.drop:hover { color: var(--fg); border-color: var(--id-imported); }
</style>
