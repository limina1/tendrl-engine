<script lang="ts">
	import '$lib/styles/tokens.css';
	import '../app.css';
	import { createAppState } from '$lib/state.svelte';
	import SearchActionModal from '$lib/components/SearchActionModal.svelte';
	import EventViewModal from '$lib/components/EventViewModal.svelte';
	import type { SearchResult } from '$lib/types';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';

	let { children } = $props();

	const app = createAppState();

	let initialized = $state(false);
	$effect(() => {
		if (initialized) return;
		initialized = true;
		app.initialize();
		const cleanup = app.startNetworkPoll();
		return cleanup;
	});

	function spawnReader(pubkey: string, d_tag: string, label: string | null) {
		try {
			const store = getActiveStore();
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:30040:${pubkey}:${d_tag}`,
					kind: 'reader',
					label: 'reader',
					kicker: label ?? d_tag
				}
			});
		} catch {
			// No active store (legacy chrome) — fall back to AppState navigation.
			app.navigateToPublication?.(pubkey, d_tag);
		}
	}

	function spawnEventReader(eventId: string, label: string | null) {
		try {
			const store = getActiveStore();
			store.openBuffer({
				className: 'work',
				buffer: {
					id: `reader:event:${eventId}`,
					kind: 'reader',
					label: 'section',
					kicker: label ?? eventId.slice(0, 8)
				}
			});
		} catch {
			// No active store — silently no-op; the legacy chrome doesn't
			// have a one-section reader.
		}
	}

	function onReadEvent(r: SearchResult) {
		app.actionModalResult = null;
		if (r.kind === 30040 && r.addr) {
			// Read publication: full reader with all sections.
			spawnReader(r.addr.pubkey, r.addr.d_tag, r.title);
			return;
		}
		// Sections (30041) and other events: open just this event in a
		// single-section reader, paginated view. No parent walk, no TOC.
		spawnEventReader(r.event_id, r.title);
	}

	async function onFindContaining(r: SearchResult) {
		app.actionModalResult = null;
		if (!r.addr) return;
		// Populate the visible search buffer with everything that references
		// this address — collections for 30040s, parent indexes for 30041s,
		// notes that quoted it, etc. Skip the by:me scope (parents may live
		// under any author).
		const aRef = `${r.kind}:${r.addr.pubkey}:${r.addr.d_tag}`;
		await app.handleSearch(`a:${aRef}`, { scopeToMe: false });
		// Make sure the search slot is visible.
		try {
			const store = getActiveStore();
			const searchSlot = store.findSlotForClass('research');
			if (searchSlot) {
				store.focusSlot(searchSlot);
				if (store.effectiveState(searchSlot) === 'rail') store.toggleSlot(searchSlot);
			}
		} catch {
			// no store — legacy chrome
		}
	}

	async function onInsert(r: SearchResult, mode: 'cursor' | 'append') {
		app.actionModalResult = null;
		// Make sure the composer buffer is on screen so the user can see
		// what they just inserted. If the WM store isn't available (legacy
		// chrome), handleInsertEvent will navigate via docMode.
		try {
			const store = getActiveStore();
			store.openBuffer({
				className: 'work',
				buffer: { id: 'composer:current', kind: 'composer', label: 'composer', kicker: 'draft' }
			});
		} catch {
			// fall through
		}
		await app.handleInsertEvent(r, mode);
	}

	function onOpenSettings() {
		app.actionModalResult = null;
		try {
			const store = getActiveStore();
			store.openBuffer({
				className: 'work',
				buffer: { id: 'settings', kind: 'settings', label: 'settings', kicker: 'settings' }
			});
		} catch {
			// No-op — legacy chrome doesn't have a settings buffer yet.
		}
	}
</script>

{@render children()}

{#if app.actionModalResult}
	<SearchActionModal
		result={app.actionModalResult}
		insertMode={app.editorInsertMode}
		onclose={() => (app.actionModalResult = null)}
		onread={onReadEvent}
		onfindcontaining={onFindContaining}
		oninsert={onInsert}
		onopensettings={onOpenSettings}
	/>
{/if}

{#if app.eventModalData}
	<EventViewModal
		event={app.eventModalData}
		onclose={() => (app.eventModalData = null)}
		onspawnreader={spawnReader}
	/>
{/if}

{#if app.jsonModalData}
	<!-- Legacy <pre> dump for the M-x buffer-inspector and PublishProgressBuffer
	     raw-event paths. Will get its own structured viewer later if needed. -->
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="json-modal-backdrop" onclick={() => (app.jsonModalData = null)} role="presentation">
		<div class="json-modal" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
			<div class="json-modal-header">
				<span>Raw JSON</span>
				<button onclick={() => (app.jsonModalData = null)}>Close</button>
			</div>
			<pre class="json-modal-body">{JSON.stringify(app.jsonModalData, null, 2)}</pre>
		</div>
	</div>
{/if}

<style>
	.json-modal-backdrop {
		position: fixed;
		/* Leave the bottom modeline visible so the search-history pill
		   stays clickable while a modal is open. */
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 100;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.json-modal {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: 90vw;
		max-width: 720px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
	}

	.json-modal-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
		font-weight: 600;
		font-size: 0.85rem;
	}

	.json-modal-body {
		flex: 1;
		overflow: auto;
		padding: 14px;
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.8rem;
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-all;
	}
</style>
