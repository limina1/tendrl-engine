<script lang="ts">
	import '$lib/styles/tokens.css';
	import '../app.css';
	import { untrack } from 'svelte';
	import { createAppState } from '$lib/state.svelte';
	import EventViewModal from '$lib/components/EventViewModal.svelte';
	import EventsJsonModal from '$lib/components/EventsJsonModal.svelte';
	import ComparePublishModal from '$lib/components/ComparePublishModal.svelte';
	import FetchConfirmModal from '$lib/components/FetchConfirmModal.svelte';
	import PublishConfirmModal from '$lib/components/PublishConfirmModal.svelte';
	import NetworkModeChoiceModal from '$lib/components/NetworkModeChoiceModal.svelte';
	import DiscoveryTip from '$lib/components/DiscoveryTip.svelte';
	import SearchConfigModal from '$lib/components/SearchConfigModal.svelte';
	import SearchHelpModal from '$lib/components/SearchHelpModal.svelte';
	import ModelineHelpModal from '$lib/components/ModelineHelpModal.svelte';
	import ToastStack from '$lib/components/ToastStack.svelte';
	import FetchActivityModal from '$lib/components/FetchActivityModal.svelte';
	// fetch-events self-starts the SSE subscription at module scope; we
	// only need confirmState here to render the modal.
	import { confirmState } from '$lib/network/fetch-events.svelte';
	import { trigger as triggerTip } from '$lib/wm/discovery.svelte';
	import type { NostrEvent, SearchResult } from '$lib/types';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';

	let { children } = $props();

	const app = createAppState();

	// Walkthrough: the "feed sync" confirm modal is the first surface of the
	// login walk. When a fetch intent opens it, point at it — `feed-sync` while
	// the pull is still broad (logged out / unscoped), `general-feed` once the
	// query carries an author scope (signed in), which is the later "now it's
	// optional" beat. Both no-op unless the walkthrough is armed + unseen.
	$effect(() => {
		const intent = confirmState.intent;
		if (intent?.type !== 'intent') return;
		const scoped = (intent.summary?.filters ?? []).some((f) => (f.authors?.length ?? 0) > 0);
		triggerTip(scoped ? 'general-feed' : 'feed-sync');
	});

	// One-time client setup: kick off initialization and start the network
	// poll, returning the poll's teardown for unmount.
	//
	// Implemented as `$effect` + `untrack` rather than `onMount`: in this build
	// the compiler was dropping the `onMount` callback entirely (its body never
	// reached the bundle), so initialization never ran and `networkStatus`
	// stayed null. `untrack` makes the effect register no reactive dependencies,
	// so it runs exactly once on mount (no re-run that would tear down the poll
	// and SSE subscription) — matching the original onMount semantics.
	$effect(() =>
		untrack(() => {
			app.initialize();
			return app.startNetworkPoll();
		})
	);

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

	// The unified event modal works with either a full NostrEvent or a
	// SearchResult. handleInsertEvent wants a SearchResult — flatten a
	// NostrEvent into one (SearchResult is a flat 9-field interface).
	function toSearchResult(ev: NostrEvent | SearchResult): SearchResult {
		if ('event_id' in ev) return ev;
		const d = ev.tags.find((t) => t[0] === 'd')?.[1];
		return {
			addr: d ? { kind: ev.kind, pubkey: ev.pubkey, d_tag: d } : null,
			event_id: ev.id,
			title: ev.tags.find((t) => t[0] === 'title')?.[1] ?? null,
			preview: ev.content,
			author: ev.pubkey,
			kind: ev.kind,
			tags: ev.tags,
			created_at: ev.created_at,
			semantic_score: null
		};
	}

	async function onInsert(ev: NostrEvent | SearchResult, mode: 'cursor' | 'append') {
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
		await app.handleInsertEvent(toSearchResult(ev), mode);
	}
</script>

{@render children()}

<!-- First-run, one-time network-mode choice. Renders above everything and
     gates relay fetching until the user picks (engine: mode_chosen=false). -->
{#if app.needsNetworkModeChoice}
	<NetworkModeChoiceModal
		onchoose={(mode, runWalkthrough) => app.chooseNetworkMode(mode, runWalkthrough)}
	/>
{/if}

<!-- Contextual walkthrough coachmark — floats over the app, points at the
     surface for the active tip. Self-gated: renders nothing unless a tip is
     queued (walkthrough enabled + an unseen surface was triggered). -->
<DiscoveryTip />

<ToastStack />

<FetchActivityModal />

{#if app.eventModalData}
	<EventViewModal
		event={app.eventModalData}
		insertMode={app.editorInsertMode}
		onclose={() => (app.eventModalData = null)}
		onspawnreader={spawnReader}
		onspawneventreader={spawnEventReader}
		oninsert={onInsert}
		onfindcontaining={(kind, pubkey, d_tag) => {
			// Match onFindContaining's behavior — broad `a:K:pk:d` search +
			// pop the search slot into view. Modal closed by the component.
			const aRef = `${kind}:${pubkey}:${d_tag}`;
			app.handleSearch(`a:${aRef}`, { scopeToMe: false });
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
		}}
	/>
{/if}

{#if confirmState.intent?.type === 'intent'}
	<FetchConfirmModal
		intent={confirmState.intent}
		general={app.feedGeneral}
		onToggleGeneral={app.toggleFeedGeneral}
	/>
{:else if confirmState.intent?.type === 'publish_intent'}
	<PublishConfirmModal intent={confirmState.intent} />
{/if}

<SearchConfigModal />

<SearchHelpModal />
<ModelineHelpModal />

<EventsJsonModal />

<ComparePublishModal />

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
