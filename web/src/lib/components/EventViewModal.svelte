<script lang="ts">
	import type { NostrEvent, SearchResult, EditorInsertMode } from '$lib/types';
	import ProfileName from './ProfileName.svelte';
	import { getAppState } from '$lib/state.svelte';
	import {
		encodeNpub,
		encodeNevent,
		encodeNaddr,
		isHex64,
		stripNostrPrefix
	} from '$lib/nostr/nip19';
	import * as api from '$lib/api';
	import { getRelayInfo, normalizeRelayUrl, type Nip11Status } from '$lib/relay/nip11';
	import { requestRelayFocus } from '$lib/relay/focus.svelte';
	import { getActiveStore } from '$lib/wm/buffer-store.svelte';

	let {
		event,
		insertMode = 'append',
		onclose,
		onspawnreader,
		onspawneventreader,
		onfindcontaining,
		oninsert
	}: {
		event: NostrEvent | SearchResult;
		/** Compose insert mode for the "Insert into compose" action. */
		insertMode?: EditorInsertMode;
		onclose: () => void;
		onspawnreader?: (pubkey: string, d_tag: string, label: string | null) => void;
		onspawneventreader?: (eventId: string, label: string | null) => void;
		onfindcontaining?: (kind: number, pubkey: string, d_tag: string) => void;
		oninsert?: (event: NostrEvent | SearchResult, mode: EditorInsertMode) => void;
	} = $props();

	const app = getAppState();

	// Breadcrumb of events visited via chained nav within this modal session.
	// Reset when the displayed event id doesn't match the most recent nav
	// target (i.e. the event was set externally — e.g. history popover replay
	// or a fresh "View JSON" click). Each entry stores the original event so
	// breadcrumb-back can restore it without a refetch.
	type Crumb = { event: NostrEvent | SearchResult; label: string; id: string };
	let breadcrumb: Crumb[] = $state([]);
	let pendingNavTarget: string | null = null;

	type Normalized = {
		id: string;
		pubkey: string;
		kind: number;
		tags: string[][];
		content: string;
		created_at: number;
		title: string | null;
	};

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
	const dTag = $derived(n.tags.find((t) => t[0] === 'd')?.[1] ?? null);
	const addrRef = $derived(dTag ? `${n.kind}:${n.pubkey}:${dTag}` : null);
	let tagsOpen = $state(false);
	let rawOpen = $state(false);

	// POOL row — live view of the reference pool. The viewed event may or
	// may not have a ContextItem yet; `poolItem` is null until the user
	// first toggles a square (which creates it via addToPool). Squares,
	// lock, and drop all reflect and mutate the real `app.items` array.
	const poolItem = $derived(app.findPoolItem(event));
	const inPool = $derived(poolItem != null);
	const inContext = $derived(poolItem?.in_context ?? false);
	const inCompose = $derived(poolItem?.in_compose ?? false);
	const isHeld = $derived(poolItem?.held ?? false);
	// `readonly` only carries meaning once the item exists. When nothing's
	// in the pool yet the lock button is disabled — there's nothing to
	// lock — but we show the *would-be* default for affordance: imports
	// default locked, everything else unlocked.
	const wouldBeLocked = $derived(n.kind === 30041 || n.kind === 30040);
	const locked = $derived(poolItem ? poolItem.readonly : wouldBeLocked);

	// ===== Chord system =====
	// Top-level keys c / a / p enter a prefix; the next key dispatches to
	// the prefix's sub-action and clears the prefix. Esc clears an active
	// prefix (or closes the modal if none). t and r are bare toggles.
	let chordPrefix: null | 'c' | 'a' | 'p' = $state(null);

	function copy(kind: 'id' | 'nevent' | 'naddr' | 'npub'): void {
		try {
			if (kind === 'id') copyText(n.id, 'id');
			else if (kind === 'nevent') copyText(encodeNevent(n.id), 'nevent');
			else if (kind === 'naddr') {
				if (!dTag) return;
				copyText(encodeNaddr({ kind: n.kind, pubkey: n.pubkey, dTag }), 'naddr');
			} else if (kind === 'npub') copyText(encodeNpub(n.pubkey), 'npub');
		} catch {
			app.pushToast(`Couldn't encode ${kind}`, 'error');
		}
	}

	let tagsContainer: HTMLElement | null = $state(null);

	function focusFirstTagChip() {
		// Tags are conditionally rendered; wait for the DOM to settle.
		queueMicrotask(() => {
			const btn = tagsContainer?.querySelector<HTMLButtonElement>('button');
			btn?.focus();
		});
	}

	function navTagChips(e: KeyboardEvent) {
		if (!(e.target instanceof HTMLButtonElement)) return;
		if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
		const buttons = Array.from(
			tagsContainer?.querySelectorAll<HTMLButtonElement>('button') ?? []
		);
		const idx = buttons.indexOf(e.target);
		if (idx < 0) return;
		e.preventDefault();
		const next = e.key === 'ArrowRight' ? idx + 1 : idx - 1;
		buttons[(next + buttons.length) % buttons.length]?.focus();
	}

	// Containing publications — kind-30040 indexes that reference the
	// currently-displayed event by `#a` (preferred, for replaceable kinds)
	// or `#e`. Refetched on every event swap; the app-level cache makes
	// repeat visits cheap. Hidden entirely for kinds where the lookup
	// isn't meaningful (anything outside 30041/30818/30040/30023).
	const ZETTEL_KINDS = new Set([30041, 30818, 30040, 30023]);
	const containingApplicable = $derived(ZETTEL_KINDS.has(n.kind));
	const isZettel = $derived(ZETTEL_KINDS.has(n.kind));

	// Kind-aware label for the primary "Read" action.
	const readLabel = $derived(
		n.kind === 30040
			? 'Read publication'
			: n.kind === 30041
				? 'Read section'
				: n.kind === 30023
					? 'Read article'
					: n.kind === 30818
						? 'Read wiki page'
						: 'Read event'
	);
	let containingIndexes: { id: string; pubkey: string; d_tag: string; title: string }[] = $state([]);

	$effect(() => {
		// Re-run when the event id changes. `event` is read inside the
		// async call below — capture it locally so the closure sees the
		// version that was current when this effect fired (avoids races
		// when the user clicks chips quickly).
		const currentEvent = event;
		const currentId = n.id.toLowerCase();
		containingIndexes = [];
		if (!containingApplicable) return;
		app.findContainingIndexes(currentEvent).then((r) => {
			// Drop stale results if the user navigated away in the meantime.
			if (n.id.toLowerCase() !== currentId) return;
			containingIndexes = r.indexes;
		});
	});

	// Clicking a containing publication recurses into *its* JSON within this
	// same modal — chained via the breadcrumb so the reader can climb the
	// reference graph one index at a time and step back down. The per-row
	// "read" button keeps the old escape-to-reader behaviour.
	function onRecurseContaining(idx: { id: string }) {
		pushBreadcrumb();
		pendingNavTarget = idx.id.toLowerCase();
		app.getEventForModal(idx.id);
	}

	function onReadContaining(idx: { pubkey: string; d_tag: string; title: string }) {
		onclose();
		onspawnreader?.(idx.pubkey, idx.d_tag, idx.title);
	}

	// ===== Primary actions =====

	// IMPORTANT — order matters here.  onclose() sets app.eventModalData
	// to null on the parent, which makes the `event` prop go null
	// synchronously in this component.  Anything that reads `event` or
	// `n` (the derived normalize(event)) after that point will crash —
	// normalize(null) throws on `'event_id' in input`.  So: snapshot
	// what we need into locals, run the action, *then* close.

	function onReadAction() {
		const kind = n.kind;
		const pubkey = n.pubkey;
		const id = n.id;
		const title = n.title;
		const d = dTag;
		onclose();
		if (kind === 30040 && d) {
			onspawnreader?.(pubkey, d, title);
		} else {
			onspawneventreader?.(id, title);
		}
	}

	function onFindAction() {
		if (!dTag) return;
		const kind = n.kind;
		const pubkey = n.pubkey;
		const d = dTag;
		onclose();
		onfindcontaining?.(kind, pubkey, d);
	}

	function onInsertAction() {
		if (!isZettel) return;
		const ev = event;
		const mode = insertMode;
		onclose();
		oninsert?.(ev, mode);
	}

	// Per-event broadcast. Explicit, user-initiated — pushes the signed
	// event to the configured broadcast set (nostr.land etc.) without
	// going through the auto-publish path. Aligns with [[project-
	// publishing-philosophy]]: deliberate, per-event opt-in.
	//
	// SearchResults arrive with truncated `preview` and no `sig`, so we
	// always re-fetch the full event by id before broadcasting.
	let broadcasting = $state(false);
	async function onBroadcastAction() {
		if (broadcasting) return;
		broadcasting = true;
		try {
			const [fullResp, cfg] = await Promise.all([
				api.getEvent(n.id),
				api.getRelayConfig()
			]);
			const fullEvent = fullResp.event;
			if (!fullEvent || typeof fullEvent !== 'object') {
				throw new Error('Engine returned no event JSON');
			}
			const targets = cfg.broadcast?.urls ?? [];
			if (targets.length === 0) {
				app.pushToast(
					'No broadcast relays configured. Toggle some as "broadcast" in the relays buffer first.',
					'error',
					5000
				);
				return;
			}
			const resp = await api.broadcastEvent({
				event: fullEvent,
				relays: targets
			});
			app.pushToast(
				`Broadcast to ${resp.successful}/${resp.total} relay${resp.total === 1 ? '' : 's'}`,
				resp.successful > 0 ? 'success' : 'error',
				4000
			);
		} catch (e) {
			app.pushToast(
				`Broadcast failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			broadcasting = false;
		}
	}

	function onModalKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			if (chordPrefix !== null) {
				chordPrefix = null;
				return;
			}
			onclose();
			return;
		}
		const k = e.key.toLowerCase();

		// In a prefix — dispatch and clear.
		if (chordPrefix === 'c') {
			chordPrefix = null;
			if (k === 'i' || k === 'e' || k === 'n') {
				e.preventDefault();
				copy(k === 'i' ? 'id' : k === 'e' ? 'nevent' : 'npub');
			} else if (k === 'a' && addrRef && dTag) {
				e.preventDefault();
				copy('naddr');
			}
			return;
		}
		if (chordPrefix === 'a') {
			chordPrefix = null;
			if (k === 'r') {
				e.preventDefault();
				onReadAction();
			} else if (k === 'f' && dTag) {
				e.preventDefault();
				onFindAction();
			} else if (k === 'i' && isZettel) {
				e.preventDefault();
				onInsertAction();
			} else if (k === 'b') {
				e.preventDefault();
				onBroadcastAction();
			}
			return;
		}
		if (chordPrefix === 'p') {
			chordPrefix = null;
			if (k === 'c') {
				e.preventDefault();
				app.togglePoolMembership(event, 'context');
			} else if (k === 'm') {
				e.preventDefault();
				app.togglePoolMembership(event, 'compose');
			} else if (k === 'r') {
				e.preventDefault();
				app.togglePoolMembership(event, 'held');
			} else if (k === 'i') {
				e.preventDefault();
				if (inPool) app.togglePoolReadonly(event);
			} else if (k === 'x') {
				e.preventDefault();
				if (inPool) app.dropFromPool(event);
			}
			return;
		}

		// Top-level — prefixes (c/a/p) and bare toggles (t/r). The modal
		// has no text inputs, so bare letters are safe.
		if (k === 'c') {
			e.preventDefault();
			chordPrefix = 'c';
		} else if (k === 'a') {
			e.preventDefault();
			chordPrefix = 'a';
		} else if (k === 'p') {
			e.preventDefault();
			chordPrefix = 'p';
		} else if (k === 't') {
			e.preventDefault();
			tagsOpen = !tagsOpen;
			if (tagsOpen) focusFirstTagChip();
		} else if (k === 'r') {
			e.preventDefault();
			rawOpen = !rawOpen;
		}
	}

	function focusModal(el: HTMLElement) {
		el.focus();
	}

	// Breadcrumb reset: when n.id changes, if it isn't the expected chained
	// target, the user came in via external nav (popover replay, fresh open)
	// and the breadcrumb is stale. The check runs after each prop change.
	$effect(() => {
		const id = n.id.toLowerCase();
		if (pendingNavTarget !== null && id === pendingNavTarget) {
			pendingNavTarget = null;
		} else if (pendingNavTarget === null && breadcrumb.length > 0) {
			const top = breadcrumb[breadcrumb.length - 1];
			// Back-step landed: do nothing (breadcrumb already trimmed).
			if (top.id === id) return;
			// External nav — chain is broken.
			breadcrumb = [];
		}
	});

	function pushBreadcrumb() {
		breadcrumb = [
			...breadcrumb,
			{ event, label: n.title ?? shortHex(n.id, 6, 4), id: n.id.toLowerCase() }
		];
	}

	function gotoBreadcrumb(idx: number) {
		if (idx < 0 || idx >= breadcrumb.length) return;
		const target = breadcrumb[idx];
		breadcrumb = breadcrumb.slice(0, idx);
		pendingNavTarget = target.id;
		app.eventModalData = target.event;
	}

	const KIND_LABEL: Record<number, string> = {
		0: 'profile',
		1: 'note',
		3: 'contacts',
		1111: 'comment',
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

	function copyText(s: string, label = 'value'): void {
		try {
			navigator.clipboard?.writeText(s);
			app.pushToast(`${label} copied`, 'success');
		} catch {
			app.pushToast(`Couldn't copy ${label}`, 'error');
		}
	}

	function shortHex(s: string, head = 8, tail = 6): string {
		if (s.length <= head + tail + 1) return s;
		return `${s.slice(0, head)}…${s.slice(-tail)}`;
	}

	// ===== Tag click dispatch =====

	type TagAction =
		| { kind: 'none' }
		| { kind: 'event'; eventId: string }
		| { kind: 'reader'; pubkey: string; d_tag: string; label: string | null }
		| { kind: 'addr-nonindex'; addr: string }
		| { kind: 'search'; query: string };

	function tagAction(tag: string[]): TagAction {
		if (!Array.isArray(tag) || tag.length < 1) return { kind: 'none' };
		const key = tag[0];
		const rawValue = tag[1] ?? '';
		const value = stripNostrPrefix(rawValue).trim();
		if (!value) return { kind: 'none' };

		if (key === 'e' || key === 'q' || key === 'note') {
			if (isHex64(value)) return { kind: 'event', eventId: value.toLowerCase() };
			return { kind: 'none' };
		}
		if (key === 'p') {
			if (!isHex64(value)) return { kind: 'none' };
			try {
				const npub = encodeNpub(value.toLowerCase());
				return { kind: 'search', query: `by:${npub}` };
			} catch { return { kind: 'none' }; }
		}
		if (key === 'a') {
			const parts = value.split(':');
			if (parts.length < 3) return { kind: 'none' };
			const kind = Number(parts[0]);
			const pubkey = parts[1];
			const d_tag = parts.slice(2).join(':');
			if (!Number.isFinite(kind) || !isHex64(pubkey)) return { kind: 'none' };
			if (kind === 30040) {
				return { kind: 'reader', pubkey, d_tag, label: null };
			}
			return { kind: 'addr-nonindex', addr: value };
		}
		if (key === 'd') return { kind: 'search', query: `d:${value}` };
		if (key === 't') return { kind: 'search', query: `t:${value}` };
		// Generic tag filter — both single-char (NIP-01 short tags) and
		// multi-char names (author, client, imeta, alt, …) are accepted by
		// the parser (src/search.rs:506). The value must be whitespace-free
		// and not start with `/`; otherwise fall through to a plain chip.
		const validName = /^[A-Za-z][A-Za-z0-9_]*$/.test(key);
		const validValue = !value.startsWith('/') && /^[^\s]+$/.test(value);
		if (validName && validValue) {
			return { kind: 'search', query: `${key}:${value}` };
		}
		return { kind: 'none' };
	}

	async function onTagClick(tag: string[]) {
		const action = tagAction(tag);
		if (action.kind === 'none') return;
		if (action.kind === 'event') {
			// Chained nav — keep modal open, swap content via getEventForModal.
			pushBreadcrumb();
			pendingNavTarget = action.eventId;
			app.getEventForModal(action.eventId);
			return;
		}
		if (action.kind === 'reader') {
			onclose();
			onspawnreader?.(action.pubkey, action.d_tag, action.label);
			return;
		}
		if (action.kind === 'addr-nonindex') {
			// Version-aware: query all kinds-by-pubkey-by-d, show newest in modal.
			const parts = action.addr.split(':');
			const k = Number(parts[0]);
			const pk = parts[1];
			const d = parts.slice(2).join(':');
			try {
				const resp = await api.queryEvents(
					[{ kinds: [k], authors: [pk], '#d': [d] }],
					'local_only'
				);
				const evts = (resp?.events ?? []) as NostrEvent[];
				evts.sort((a, b) => b.created_at - a.created_at);
				if (evts[0]) {
					pushBreadcrumb();
					pendingNavTarget = evts[0].id.toLowerCase();
					app.eventModalData = evts[0];
					app.pushHistoryEntry({
						kind: 'naddr',
						coord: { kind: k, pubkey: pk, d_tag: d },
						title: evts[0].tags.find((t) => t[0] === 'title')?.[1],
						lastRunAt: Date.now()
					});
				}
			} catch (e) {
				console.error('a-tag non-index lookup failed:', e);
			}
			return;
		}
		// search
		onclose();
		app.handleSearch(action.query, { scopeToMe: false });
	}

	// Tags shown in the chip block — exclude the title tag (already in the
	// header) and the d tag (rendered as the addr identifier above).
	const tagChips = $derived(
		n.tags.filter((t) => t[0] !== 'title' && t[0] !== 'd')
	);

	// ===== Found on =====
	// Relays this event has been seen on / broadcast to. The modal can't
	// open without the event being locally cached, so the local-cache
	// chip is always present. Network relays come from `event.relays`
	// (only on full `NostrEvent`, not on `SearchResult` — that's by
	// design per the provenance plan). Insertion order from nostrdb is
	// preserved; no sort.
	const eventRelays = $derived(
		'event_id' in event ? [] : (event as NostrEvent).relays ?? []
	);
	const RELAY_COLLAPSE_THRESHOLD = 5;
	let relaysExpanded = $state(false);
	const visibleRelays = $derived(
		relaysExpanded || eventRelays.length <= RELAY_COLLAPSE_THRESHOLD
			? eventRelays
			: eventRelays.slice(0, RELAY_COLLAPSE_THRESHOLD)
	);
	const hiddenRelayCount = $derived(
		Math.max(0, eventRelays.length - visibleRelays.length)
	);

	// NIP-11 lookups for each chip — populated lazily on hover/focus so we
	// don't fire one fetch per relay just to render the modal. The
	// `getRelayInfo` helper dedups across components within the tab.
	let relayInfo: Record<string, Nip11Status> = $state({});
	function primeRelayInfo(url: string) {
		const key = normalizeRelayUrl(url);
		if (relayInfo[key]?.state === 'loaded' || relayInfo[key]?.state === 'loading') return;
		const s = getRelayInfo(url, (next) => {
			relayInfo = { ...relayInfo, [key]: next };
		});
		relayInfo = { ...relayInfo, [key]: s };
	}
	function relayTooltip(url: string): string {
		const status = relayInfo[normalizeRelayUrl(url)];
		if (!status || status.state === 'pending') return url;
		if (status.state === 'loading') return `${url}\n(loading NIP-11…)`;
		if (status.state === 'failed') return `${url}\n(NIP-11: ${status.error})`;
		const doc = status.doc;
		const parts: string[] = [url];
		if (doc.name) parts.push(doc.name);
		if (doc.description) parts.push(doc.description);
		if (doc.software) parts.push(doc.software + (doc.version ? ` ${doc.version}` : ''));
		return parts.join('\n');
	}
	// Open the relays buffer focused on the clicked relay — sets a
	// one-shot focus signal that RelaysBuffer consumes on mount/update
	// to expand and scroll that specific row into view.
	function openRelayInfo(url: string) {
		try {
			requestRelayFocus(url);
			getActiveStore().openBuffer({
				className: 'work',
				buffer: { id: 'relays', kind: 'relays', label: 'relays', kicker: 'config' }
			});
			onclose();
		} catch {
			copyText(url, 'relay url');
		}
	}
	function shortenRelay(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="evm-backdrop" onclick={onclose} role="presentation">
	<div
		class="evm"
		onclick={(e) => e.stopPropagation()}
		onkeydown={onModalKeydown}
		role="dialog"
		tabindex="-1"
		use:focusModal
	>
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

		{#if breadcrumb.length > 0}
			<nav class="evm__crumbs" aria-label="In-modal navigation">
				{#each breadcrumb as crumb, i (crumb.id + ':' + i)}
					<button class="evm__crumb" onclick={() => gotoBreadcrumb(i)} title="Back to {crumb.label}">
						{crumb.label}
					</button>
					<span class="evm__crumb-sep">›</span>
				{/each}
				<span class="evm__crumb evm__crumb--current">{n.title ?? shortHex(n.id, 6, 4)}</span>
			</nav>
		{/if}

		<section class="evm__section" class:evm__section--active={chordPrefix === 'c'}>
			<h3 class="evm__heading">
				<span class="evm__key evm__key--head" class:evm__key--active={chordPrefix === 'c'}>c</span>
				Copy as
			</h3>
			<div class="evm__copy-bar">
				<button
					class="evm__copy-pill"
					onclick={() => copy('id')}
					title="Copy hex id ({n.id})"
				>
					<span class="evm__key">i</span>
					<span class="evm__copy-label">id</span>
				</button>
				<button
					class="evm__copy-pill"
					onclick={() => copy('nevent')}
					title="Copy as nevent1… (bech32m event id)"
				>
					<span class="evm__key">e</span>
					<span class="evm__copy-label">nevent</span>
				</button>
				{#if addrRef && dTag}
					<button
						class="evm__copy-pill"
						onclick={() => copy('naddr')}
						title="Copy as naddr1… (bech32m {n.kind}:{shortHex(n.pubkey, 6, 4)}:{dTag})"
					>
						<span class="evm__key">a</span>
						<span class="evm__copy-label">naddr</span>
					</button>
				{/if}
				<button
					class="evm__copy-pill"
					onclick={() => copy('npub')}
					title="Copy author npub1… (bech32 pubkey)"
				>
					<span class="evm__key">n</span>
					<span class="evm__copy-label">npub</span>
				</button>
			</div>
		</section>

		<section class="evm__section" class:evm__section--active={chordPrefix === 'a'}>
			<h3 class="evm__heading">
				<span class="evm__key evm__key--head" class:evm__key--active={chordPrefix === 'a'}>a</span>
				Actions
			</h3>
			<div class="evm__actions">
				<button class="evm__action" onclick={onReadAction}>
					<span class="evm__key">r</span>
					<span class="evm__action-label">{readLabel}</span>
				</button>
				{#if dTag}
					<button class="evm__action" onclick={onFindAction}>
						<span class="evm__key">f</span>
						<span class="evm__action-label">Find containing publications</span>
					</button>
				{/if}
				{#if isZettel}
					<button class="evm__action" onclick={onInsertAction}>
						<span class="evm__key">i</span>
						<span class="evm__action-label">Insert into compose</span>
					</button>
				{/if}
				<button
					class="evm__action"
					onclick={onBroadcastAction}
					disabled={broadcasting}
					title="Push this event to your configured broadcast relays (aggregators like nostr.land). Deliberate per-event push — never auto-fires."
				>
					<span class="evm__key">b</span>
					<span class="evm__action-label">{broadcasting ? 'Broadcasting…' : 'Broadcast'}</span>
				</button>
			</div>
		</section>

		<!-- POOL — live view of the reference pool. The three squares show
		     which memberships this event currently has; clicking toggles
		     them on/off (creating the ContextItem on first touch).
		     Lock reflects the item's `readonly` and is only interactive
		     once the item is in the pool. Drop removes every flag and
		     gc()s the item out. -->
		<section class="evm__section" class:evm__section--active={chordPrefix === 'p'}>
			<h3 class="evm__heading">
				<span class="evm__key evm__key--head" class:evm__key--active={chordPrefix === 'p'}>p</span>
				Pool
				{#if !inPool}
					<span class="evm__heading-meta">not in pool</span>
				{/if}
			</h3>
			<div class="evm__pool">
				<div class="evm__pool-members">
					<button
						class="evm__pool-sq"
						class:evm__pool-sq--on={inContext}
						onclick={() => app.togglePoolMembership(event, 'context')}
						title={inContext ? 'In context — click to remove' : 'Add to chat context'}
					>
						<span class="evm__key">c</span>
						<span class="evm__pool-box">{inContext ? '▣' : '▢'}</span>
						context
					</button>
					<button
						class="evm__pool-sq"
						class:evm__pool-sq--on={inCompose}
						onclick={() => app.togglePoolMembership(event, 'compose')}
						title={inCompose ? 'In compose — click to remove' : 'Add to compose'}
					>
						<span class="evm__key">m</span>
						<span class="evm__pool-box">{inCompose ? '▣' : '▢'}</span>
						compose
					</button>
					<button
						class="evm__pool-sq"
						class:evm__pool-sq--on={isHeld}
						onclick={() => app.togglePoolMembership(event, 'held')}
						title={isHeld ? 'Held in refs — click to release' : 'Hold in refs (no routing)'}
					>
						<span class="evm__key">r</span>
						<span class="evm__pool-box">{isHeld ? '▣' : '▢'}</span>
						refs
					</button>
				</div>
				<div class="evm__pool-state">
					<button
						class="evm__pool-lock"
						class:evm__pool-lock--locked={locked}
						class:evm__pool-lock--disabled={!inPool}
						onclick={() => inPool && app.togglePoolReadonly(event)}
						disabled={!inPool}
						title={!inPool
							? 'Lock applies once the item is in the pool'
							: locked
								? 'Imported — locked to source; click to claim'
								: 'Claimed — click to re-lock as imported'}
					>
						<span class="evm__key">i</span>
						<svg class="evm__lock" viewBox="0 0 16 16" aria-hidden="true">
							<rect x="3" y="7.2" width="10" height="6.8" rx="1.6" />
							<path
								class="evm__lock-shackle"
								d={locked
									? 'M5.5 7.2 V5 a2.5 2.5 0 0 1 5 0 V7.2'
									: 'M5.5 7.2 V5 a2.5 2.5 0 0 1 5 0'}
							/>
						</svg>
						{locked ? 'imported' : 'claimed'}
					</button>
					<button
						class="evm__pool-drop"
						class:evm__pool-drop--disabled={!inPool}
						onclick={() => inPool && app.dropFromPool(event)}
						disabled={!inPool}
						title={inPool ? 'Drop from every pool' : 'Nothing to drop'}
					>
						<span class="evm__key">x</span>
						drop
					</button>
				</div>
			</div>
		</section>

		<!-- FOUND ON — relays this event id has been seen on or successfully
		     broadcast to, plus a "Local cache" chip (always shown — the modal
		     can't open without the event being locally cached). Built from
		     `event.relays` (only present on full NostrEvent; absent on
		     SearchResult, deliberately). Order matches nostrdb insertion;
		     no sort. Collapsed past five chips with an "expand all" link. -->
		<section class="evm__section">
			<h3 class="evm__heading">
				Found on
				<span class="evm__heading-meta">
					{#if eventRelays.length === 0}
						(local cache only)
					{:else}
						({eventRelays.length + 1} {eventRelays.length + 1 === 1 ? 'source' : 'sources'})
					{/if}
				</span>
			</h3>
			{#if eventRelays.length === 0}
				<div class="evm__found-empty">
					<span class="evm__relay-chip evm__relay-chip--local" title="Locally cached only — not seen on any relay yet">
						<span class="evm__relay-dot"></span>
						Local cache
					</span>
					<span class="evm__placeholder">Only in local cache — not seen on any relay yet.</span>
				</div>
			{:else}
				<div class="evm__found-row">
					<span
						class="evm__relay-chip evm__relay-chip--local"
						title="This event is in the local nostrdb cache"
					>
						<span class="evm__relay-dot"></span>
						Local cache
					</span>
					{#each visibleRelays as url (url)}
						<button
							class="evm__relay-chip evm__relay-chip--remote"
							onclick={() => openRelayInfo(url)}
							onmouseenter={() => primeRelayInfo(url)}
							onfocus={() => primeRelayInfo(url)}
							title={relayTooltip(url)}
						>
							<span class="evm__relay-dot evm__relay-dot--remote"></span>
							{shortenRelay(url)}
						</button>
					{/each}
					{#if hiddenRelayCount > 0}
						<button
							class="evm__relay-more"
							onclick={() => (relaysExpanded = true)}
							title="Show every relay this event has been seen on"
						>+ {hiddenRelayCount} more</button>
					{:else if relaysExpanded && eventRelays.length > RELAY_COLLAPSE_THRESHOLD}
						<button
							class="evm__relay-more"
							onclick={() => (relaysExpanded = false)}
							title="Collapse to first {RELAY_COLLAPSE_THRESHOLD}"
						>show fewer</button>
					{/if}
				</div>
			{/if}
		</section>

		<section class="evm__section">
			<button class="evm__raw-toggle" onclick={() => (tagsOpen = !tagsOpen)}>
				<span class="evm__raw-arrow" class:open={tagsOpen}>{tagsOpen ? '▾' : '▸'}</span>
				<span class="evm__key evm__key--head">t</span>
				Tags <span class="evm__heading-meta">({tagChips.length})</span>
			</button>
			{#if tagsOpen}
				{#if tagChips.length === 0}
					<div class="evm__placeholder">No tags.</div>
				{:else}
					<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
					<div class="evm__chips" bind:this={tagsContainer} onkeydown={navTagChips} role="group">
						{#each tagChips as tag, i (i)}
							{@const action = tagAction(tag)}
							{@const clickable = action.kind !== 'none'}
							{#if clickable}
								<!-- svelte-ignore a11y_click_events_have_key_events -->
								<button
									class="evm__chip evm__chip--{tag[0]} evm__chip--clickable"
									onclick={() => onTagClick(tag)}
									title="{tag[0]}: {tag[1] ?? ''}"
								>
									<span class="evm__chip-key">{tag[0]}</span>
									<span class="evm__chip-val">{tag[1] ?? ''}</span>
								</button>
							{:else}
								<span
									class="evm__chip evm__chip--{tag[0]}"
									title="{tag[0]}: {tag[1] ?? ''}"
								>
									<span class="evm__chip-key">{tag[0]}</span>
									<span class="evm__chip-val">{tag[1] ?? ''}</span>
								</span>
							{/if}
						{/each}
					</div>
				{/if}
			{/if}
		</section>

		<!-- Only rendered when there are results — discovery when empty is
		     the "Find containing publications" action's job. -->
		{#if containingIndexes.length > 0}
			<section class="evm__section">
				<h3 class="evm__heading">
					Containing publications
					<span class="evm__heading-meta">({containingIndexes.length})</span>
				</h3>
				<div class="evm__containing">
					{#each containingIndexes as idx (idx.id)}
						<div class="evm__containing-row">
							<button
								class="evm__containing-btn"
								onclick={() => onRecurseContaining(idx)}
								title="View JSON for {idx.title} — climb to this index"
							>
								<span class="evm__containing-title">{idx.title}</span>
								<span class="evm__containing-dtag">{idx.d_tag}</span>
							</button>
							<button
								class="evm__containing-read"
								onclick={() => onReadContaining(idx)}
								title="Open {idx.title} in the reader"
							>read</button>
						</div>
					{/each}
				</div>
			</section>
		{/if}

		<section class="evm__section">
			<button class="evm__raw-toggle" onclick={() => (rawOpen = !rawOpen)}>
				<span class="evm__raw-arrow" class:open={rawOpen}>{rawOpen ? '▾' : '▸'}</span>
				<span class="evm__key evm__key--head">r</span>
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
		/* One scroll container for the whole modal — sections flow at their
		   natural height; Raw JSON doesn't get a cramped scroll box of its
		   own (it's often tiny). */
		overflow-y: auto;
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

	/* Chained-nav breadcrumb. Each crumb is the title (or shortened id) of an
	   event we navigated away from via an e/q/a click. Click to pop back. */
	.evm__crumbs {
		display: flex;
		align-items: center;
		gap: 4px;
		padding: 6px 14px;
		border-bottom: 1px solid var(--border);
		background: color-mix(in srgb, var(--id-yours) 6%, transparent);
		font-size: 0.72rem;
		flex-wrap: wrap;
	}
	.evm__crumb {
		background: none;
		border: none;
		color: var(--id-yours);
		font-family: inherit;
		font-size: inherit;
		padding: 1px 4px;
		border-radius: var(--r-sm);
		cursor: pointer;
		max-width: 200px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evm__crumb:hover {
		background: color-mix(in srgb, var(--id-yours) 18%, transparent);
	}
	.evm__crumb--current {
		color: var(--fg);
		cursor: default;
	}
	.evm__crumb--current:hover {
		background: none;
	}
	.evm__crumb-sep {
		color: var(--fg-muted);
		font-size: 0.85rem;
	}

	.evm__section {
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}

	.evm__section:last-child {
		border-bottom: none;
	}

	.evm__heading {
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
		margin-bottom: 6px;
		font-weight: 600;
	}
	.evm__heading-meta {
		color: var(--fg-muted);
		font-weight: 400;
	}

	.evm__placeholder {
		font-size: 0.75rem;
		color: var(--fg-muted);
		font-style: italic;
	}

	/* Containing publications block */
	.evm__containing {
		display: flex;
		flex-direction: column;
		gap: 4px;
		margin-bottom: 6px;
	}
	.evm__containing-row {
		display: flex;
		gap: 4px;
	}
	.evm__containing-btn {
		flex: 1;
		min-width: 0;
		display: flex;
		align-items: center;
		gap: 10px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 6px 10px;
		text-align: left;
		cursor: pointer;
		color: var(--fg);
		font-size: 0.78rem;
	}
	.evm__containing-btn:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-color: var(--id-yours);
	}
	.evm__containing-read {
		flex-shrink: 0;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 0 10px;
		cursor: pointer;
		color: var(--fg-muted);
		font-size: 0.72rem;
	}
	.evm__containing-read:hover {
		color: var(--id-yours);
		border-color: var(--id-yours);
	}
	.evm__containing-title {
		flex: 1;
		font-weight: 500;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evm__containing-dtag {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: var(--fg-muted);
		max-width: 200px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	/* Actions fill the section as one horizontal row of equal-width
	   buttons. The Pool config row (Phase B) sits below as a sibling. */
	.evm__actions {
		display: flex;
		gap: 6px;
	}
	.evm__action {
		flex: 1;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 6px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 8px 6px;
		text-align: center;
		cursor: pointer;
		color: var(--fg);
		font-size: 0.75rem;
		line-height: 1.3;
	}
	.evm__action:hover {
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
		border-color: var(--id-yours);
	}
	/* Keycap — small monochrome key hint used everywhere a chord key
	   exists (section headings, action buttons, copy pills, pool buttons).
	   `--head` is for section-heading keycaps; `--active` lights up while
	   that section's prefix is held. */
	.evm__action-key,
	.evm__key {
		flex-shrink: 0;
		font-family: var(--font-mono);
		font-size: 0.6rem;
		color: var(--fg-muted);
		border: 1px solid var(--border);
		border-radius: 3px;
		padding: 0 3px;
		line-height: 1.5;
	}
	.evm__key--head {
		font-size: 0.62rem;
		font-weight: 600;
	}
	.evm__key--active {
		color: var(--id-yours);
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 22%, transparent);
	}
	/* While a prefix is active, the relevant section gets a subtle tint
	   so it's obvious which sub-keys are now armed. */
	.evm__section--active {
		background: color-mix(in srgb, var(--id-yours) 5%, transparent);
	}

	/* Pool row — membership squares (left) + provenance lock & drop
	   (right). Static preview; Phase B wires it to the reference pool. */
	.evm__pool {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
	}
	.evm__pool-members,
	.evm__pool-state {
		display: flex;
		gap: 6px;
	}
	.evm__pool-sq {
		display: flex;
		align-items: center;
		gap: 5px;
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 4px 9px;
		cursor: pointer;
		color: var(--fg-muted);
		font-size: 0.75rem;
	}
	.evm__pool-sq:hover {
		border-color: var(--id-yours);
	}
	.evm__pool-sq--on {
		color: var(--fg);
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}
	.evm__pool-box {
		font-size: 0.85rem;
		line-height: 1;
		color: var(--id-yours);
	}
	.evm__pool-lock,
	.evm__pool-drop {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 4px 9px;
		cursor: pointer;
		font-size: 0.72rem;
		color: var(--fg-muted);
	}
	.evm__pool-lock:hover,
	.evm__pool-drop:hover {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.evm__pool-lock {
		display: inline-flex;
		align-items: center;
		gap: 5px;
	}
	/* Imported (locked) gets the --id-imported accent so the state reads
	   at a glance; claimed falls back to the muted default. */
	.evm__pool-lock--locked {
		color: var(--id-imported);
		border-color: color-mix(in srgb, var(--id-imported) 45%, var(--border));
	}
	/* Lock/drop are nonsense until the item exists in the pool. We keep
	   them visible (so the keycaps stay legible) but render them as
	   clearly inert: muted text, dashed border, no pointer affordance. */
	.evm__pool-lock--disabled,
	.evm__pool-drop--disabled {
		opacity: 0.45;
		border-style: dashed;
		cursor: not-allowed;
	}
	.evm__pool-lock--disabled:hover,
	.evm__pool-drop--disabled:hover {
		border-color: var(--border);
		color: var(--fg-muted);
	}
	.evm__lock {
		width: 12px;
		height: 12px;
		flex-shrink: 0;
	}
	.evm__lock rect {
		fill: currentColor;
	}
	.evm__lock-shackle {
		fill: none;
		stroke: currentColor;
		stroke-width: 1.7;
	}

	/* Identifiers block — compact "Copy as" pill bar. Each pill is a
	   clipboard icon + format label; click copies that encoding to the
	   clipboard. Replaces the older per-row layout (id / addr / author
	   each on its own row with separate copy buttons). */
	.evm__copy-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.evm__copy-pill {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-yours) 30%, transparent);
		color: var(--id-yours);
		font-family: var(--font-mono);
		font-size: 0.72rem;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.evm__copy-pill:hover {
		background: color-mix(in srgb, var(--id-yours) 20%, transparent);
		border-color: var(--id-yours);
	}
	.evm__copy-pill:active {
		background: color-mix(in srgb, var(--id-yours) 30%, transparent);
	}
	.evm__copy-label {
		font-weight: 500;
	}

	/* Tag chips */
	.evm__chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.evm__chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		background: var(--border);
		color: var(--fg);
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		padding: 2px 6px;
		font-family: var(--font-mono);
		font-size: 0.7rem;
		max-width: 100%;
	}
	.evm__chip--clickable {
		cursor: pointer;
	}
	.evm__chip--clickable:hover {
		background: color-mix(in srgb, var(--id-yours) 18%, var(--border));
		border-color: var(--id-yours);
	}
	.evm__chip-key {
		color: var(--fg-muted);
		font-size: 0.65rem;
		font-weight: 600;
	}
	.evm__chip-val {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 240px;
	}
	/* Per-kind tints for the most common tags */
	.evm__chip--e .evm__chip-key,
	.evm__chip--q .evm__chip-key,
	.evm__chip--note .evm__chip-key { color: var(--id-yours); }
	.evm__chip--a .evm__chip-key { color: var(--id-imported); }
	.evm__chip--p .evm__chip-key { color: var(--id-remote); }
	.evm__chip--t .evm__chip-key,
	.evm__chip--d .evm__chip-key { color: var(--cyan); }

	/* Found on — relay provenance row. Local-cache chip is rendered
	   distinctly (no host, --id-local tint) from the network-relay chips.
	   Insertion order from nostrdb is preserved; no sort. */
	.evm__found-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
	}
	.evm__found-empty {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.evm__relay-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: 0.72rem;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		border: 1px solid var(--border);
		background: none;
		color: var(--fg);
		cursor: default;
		max-width: 100%;
	}
	.evm__relay-chip--remote {
		cursor: pointer;
		border-color: color-mix(in srgb, var(--id-remote, var(--id-yours)) 35%, transparent);
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 8%, transparent);
		color: var(--id-remote, var(--fg));
	}
	.evm__relay-chip--remote:hover {
		border-color: var(--id-remote, var(--id-yours));
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 18%, transparent);
	}
	/* Local-cache chip — distinct token so it never looks like a network
	   relay. Falls back to id-imported / id-yours if the local token is
	   missing in a given theme. */
	.evm__relay-chip--local {
		border-color: color-mix(in srgb, var(--id-local, var(--id-imported, var(--id-yours))) 45%, transparent);
		background: color-mix(in srgb, var(--id-local, var(--id-imported, var(--id-yours))) 10%, transparent);
		color: var(--id-local, var(--id-imported, var(--fg)));
	}
	.evm__relay-dot {
		display: inline-block;
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--id-local, var(--id-imported, var(--fg-muted)));
	}
	.evm__relay-dot--remote {
		background: var(--id-remote, var(--id-yours));
	}
	.evm__relay-more {
		font-family: var(--font-mono);
		font-size: 0.72rem;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		background: color-mix(in srgb, var(--accent, var(--id-yours)) 10%, transparent);
		border: 1px solid color-mix(in srgb, var(--accent, var(--id-yours)) 40%, transparent);
		color: var(--accent, var(--id-yours));
		cursor: pointer;
		font-weight: 600;
	}
	.evm__relay-more:hover {
		background: color-mix(in srgb, var(--accent, var(--id-yours)) 22%, transparent);
		border-color: var(--accent, var(--id-yours));
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
