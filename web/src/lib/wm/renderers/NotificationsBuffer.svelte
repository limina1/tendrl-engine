<script lang="ts">
	// Notifications — the list behind the 🔔: what is for the user
	// (docs/zettel/feat-notifications.org). Three groups: asks (the fixed
	// rules, they need an act), activity (the kinds the user chose in
	// Settings › Notifications), mine (the 818s I sent, with verdicts). Rows
	// reuse the annotations-list card idiom, coloured per author; an ask
	// wears the ask colour. Clicking a row opens its target: a publication
	// in the reader, a section in the event modal, anything else as the
	// event itself. No Svelte lifecycle hooks (buffer renderers unmount on
	// every switch); the store is module-level and refreshes itself.
	import type { Buffer } from '../types';
	import { getAppState } from '$lib/state.svelte';
	import { getActiveStore } from '../buffer-store.svelte';
	import { notifications } from '$lib/notifications.svelte';
	import { pubkeyToColor } from '$lib/discussions/colors';
	import { relTime, authorFallback } from '$lib/components/annotations/list-model';
	import type { NotificationRow } from '$lib/api';

	// The buffer is a singleton (`notifications`); its id carries nothing.
	let { buffer: _buffer }: { buffer: Buffer } = $props();
	const app = getAppState();
	const store = getActiveStore();

	const st = $derived(notifications.state);

	// Re-read on mount (teardown-free: the store coalesces).
	$effect(() => {
		void notifications.refresh();
	});

	type Group = { key: string; label: string; rows: NotificationRow[] };
	function byRoot(rows: NotificationRow[]): Group[] {
		const map = new Map<string, Group>();
		for (const r of rows) {
			const root = r.anchor?.root ?? '';
			const key = root || '—';
			let g = map.get(key);
			if (!g) {
				g = { key, label: rootLabel(root), rows: [] };
				map.set(key, g);
			}
			g.rows.push(r);
		}
		return [...map.values()];
	}
	function rootLabel(root: string): string {
		if (!root) return 'elsewhere';
		const parts = root.split(':');
		return parts.length >= 3 ? parts.slice(2).join(':') : root;
	}
	const askGroups = $derived(byRoot(st.acts));
	const activityGroups = $derived(byRoot(st.activity));
	const mineGroups = $derived(byRoot(st.mine));
	const activityOn = $derived(st.rules.some((r) => !r.fixed && r.on));

	function badge(r: NotificationRow): { text: string; cls: string } {
		const done = r.status && r.status !== 'open';
		switch (r.key) {
			case '818':
				return { text: `${done ? '✓' : '?'} merge request${done ? ' · ' + r.status : ''}`, cls: 'ask' };
			case 'request-on-mine':
				return { text: `${done ? '✓' : '?'} annotation request${done ? ' · ' + r.status : ''}`, cls: 'ask' };
			case '7-on-my-818':
				return { text: verdictText(r), cls: verdictCls(r) };
			case '9802':
				return { text: 'highlight', cls: 'hl' };
			case '1111':
				return { text: 'comment', cls: 'cmt' };
			case '7':
				return { text: 'reaction', cls: 'muted' };
			case '9735':
				return { text: 'zap', cls: 'hl' };
			case '30045':
				return { text: 'shelved', cls: 'shelf' };
			default:
				return { text: `kind ${r.kind}`, cls: 'muted' };
		}
	}
	function verdictText(r: NotificationRow): string {
		const c = (r.event?.content ?? '').trim();
		return c === '+' ? '✓ merged' : c === '-' ? '− declined' : 'verdict';
	}
	function verdictCls(r: NotificationRow): string {
		const c = (r.event?.content ?? '').trim();
		return c === '+' ? 'ok' : c === '-' ? 'bad' : 'muted';
	}
	function mineBadge(r: NotificationRow): { text: string; cls: string } {
		const s = r.status ?? 'open';
		return s === 'merged'
			? { text: '✓ merged', cls: 'ok' }
			: s === 'declined'
				? { text: '− declined', cls: 'bad' }
				: { text: '818 · open', cls: 'ask' };
	}
	function au(pubkey: string): string {
		return `--au:${pubkeyToColor(pubkey)};--au-soft:${pubkeyToColor(pubkey, 0.14)}`;
	}
	function coordParts(c: string): { kind: number; pubkey: string; d_tag: string } | null {
		const i = c.indexOf(':');
		const j = c.indexOf(':', i + 1);
		if (i < 0 || j < 0) return null;
		const kind = Number(c.slice(0, i));
		if (!Number.isFinite(kind)) return null;
		return { kind, pubkey: c.slice(i + 1, j), d_tag: c.slice(j + 1) };
	}
	/** Open the row's target: an 818's / request's publication in the
	 *  reader; a section-anchored activity in the event modal; else the
	 *  event itself. */
	function open(r: NotificationRow) {
		const root = r.anchor?.root ? coordParts(r.anchor.root) : null;
		const parent = r.anchor?.parent ? coordParts(r.anchor.parent) : null;
		if ((r.key === '818' || r.key === 'request-on-mine' || r.kind === 818) && root && root.kind === 30040) {
			void app.openPublication(root.pubkey, root.d_tag);
			return;
		}
		if (parent && parent.kind === 30041) {
			void app.openAddressableInModal(parent);
			return;
		}
		if (root && root.kind === 30040) {
			void app.openPublication(root.pubkey, root.d_tag);
			return;
		}
		if (r.event) app.eventModalData = r.event;
	}
	function openSettings() {
		store.openBuffer({
			className: 'work',
			buffer: { id: 'settings', kind: 'settings', label: 'settings', kicker: 'notifications' }
		});
	}
	let fetching = $state(false);
	async function fetchNow() {
		if (fetching) return;
		fetching = true;
		try {
			await notifications.fetchFromRelays('fetch_always');
			app.pushToast('Notifications fetched', 'success');
		} catch (e) {
			app.pushToast(e instanceof Error ? e.message : String(e), 'error');
		} finally {
			fetching = false;
		}
	}
</script>

<div class="nb-root" data-tour="notifications">
	<div class="nb-head">
		<span class="nb-title">🔔 notifications</span>
		<span class="nb-counts mono">
			{st.counts.acts} ask{st.counts.acts === 1 ? '' : 's'}
			{#if st.counts.activity_count}· <span class="nb-new">{st.counts.activity_count} new</span>{/if}
		</span>
		<span class="nb-spacer"></span>
		<button class="nb-btn" onclick={fetchNow} disabled={fetching || st.loading} title="Pull from relays — a filter per enabled rule on your read relays (Confirm-gated)">{fetching ? 'fetching…' : 'fetch'}</button>
		<button class="nb-btn" onclick={() => void notifications.refresh()} disabled={st.loading} title="Re-read the local store">↻</button>
	</div>
	{#if st.error && !st.acts.length && !st.activity.length}
		<div class="nb-note">{st.error.includes('identity') || st.error.includes('401') ? 'sign in to see what is for you' : st.error}</div>
	{/if}

	<div class="alist" role="list">
		<div class="grp">asks · for you <span class="grp__n">{st.acts.length}</span></div>
		{#if !st.acts.length}
			<div class="alist__empty">nothing needs an act from you</div>
		{:else}
			{#each askGroups as g (g.key)}
				<div class="grp grp--sub">{g.label}</div>
				{#each g.rows as r (r.id)}
					{@const b = badge(r)}
					<div class="acard ask" data-id={r.id} style={au(r.pubkey)} role="listitem">
						<button type="button" class="acard__body" onclick={() => open(r)} title="Open the target">
							<span class="acard__t"><span class="badge {b.cls}">{b.text}</span>{r.summary}</span>
							<span class="acard__by"><span class="acard__dot"></span>{authorFallback(r.pubkey)} · {relTime(r.created_at)}</span>
						</button>
					</div>
				{/each}
			{/each}
		{/if}

		<div class="grp">
			activity <span class="grp__n">{st.activity.length}</span>
			<span class="grp__hint">— the kinds you chose</span>
			<span class="nb-spacer"></span>
			<button class="grp__link" onclick={openSettings}>settings › notifications ▸</button>
		</div>
		{#if !st.activity.length}
			<div class="alist__empty">{activityOn ? 'nothing new' : 'no activity kinds chosen yet'}</div>
		{:else}
			{#each activityGroups as g (g.key)}
				<div class="grp grp--sub">{g.label}</div>
				{#each g.rows as r (r.id)}
					{@const b = badge(r)}
					<div class="acard" data-id={r.id} style={au(r.pubkey)} role="listitem">
						<button type="button" class="acard__body" onclick={() => open(r)} title="Open">
							<span class="acard__t"><span class="badge {b.cls}">{b.text}</span>{r.summary}</span>
							<span class="acard__by"><span class="acard__dot"></span>{authorFallback(r.pubkey)} · {relTime(r.created_at)}</span>
						</button>
					</div>
				{/each}
			{/each}
		{/if}

		<div class="grp">mine <span class="grp__n">{st.mine.length}</span><span class="grp__hint">— merge requests you sent</span></div>
		{#if !st.mine.length}
			<div class="alist__empty">you have sent no merge requests</div>
		{:else}
			{#each mineGroups as g (g.key)}
				<div class="grp grp--sub">{g.label}</div>
				{#each g.rows as r (r.id)}
					{@const b = mineBadge(r)}
					<div class="acard" class:ask={b.cls === 'ask'} data-id={r.id} style={au(r.pubkey)} role="listitem">
						<button type="button" class="acard__body" onclick={() => open(r)} title="Open the target">
							<span class="acard__t"><span class="badge {b.cls}">{b.text}</span>{r.summary || r.event?.content || 'merge request'}</span>
							<span class="acard__by"><span class="acard__dot"></span>{relTime(r.created_at)}</span>
						</button>
					</div>
				{/each}
			{/each}
		{/if}
	</div>
</div>

<style>
	.nb-root {
		display: flex;
		flex-direction: column;
		min-height: 0;
		height: 100%;
		background: var(--panel-bg);
		color: var(--fg);
	}
	.nb-head {
		display: flex;
		align-items: center;
		gap: var(--s-2);
		padding: var(--s-2) var(--s-3);
		border-bottom: 1px solid var(--panel-border);
		font: var(--t-xs) / 1.3 var(--font-mono);
		flex-shrink: 0;
	}
	.nb-title {
		font-weight: 600;
	}
	.nb-counts {
		color: var(--base6);
	}
	.nb-new {
		color: var(--cyan);
	}
	.nb-spacer {
		flex: 1;
	}
	.nb-btn {
		font: var(--t-2xs) / 1.2 var(--font-mono);
		min-height: 24px;
		padding: 0 var(--s-2);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--base6);
		cursor: pointer;
	}
	.nb-btn:hover:not(:disabled) {
		border-color: var(--base5);
		color: var(--fg);
	}
	.nb-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.nb-note {
		padding: var(--s-2) var(--s-3);
		font: var(--t-2xs) / 1.4 var(--font-mono);
		color: var(--base5);
	}
	.mono {
		font-family: var(--font-mono);
	}
	.alist {
		flex: 1;
		overflow: auto;
		padding: var(--s-2) var(--s-3) var(--s-10);
	}
	.grp {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
		margin: var(--s-3) 0 var(--s-1);
	}
	.grp--sub {
		text-transform: none;
		letter-spacing: 0;
		color: var(--base6);
		margin-top: var(--s-2);
	}
	.grp__n {
		color: var(--base4);
	}
	.grp__hint {
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
	}
	.grp__link {
		font: var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0;
		text-transform: none;
		min-height: 20px;
		padding: 0 var(--s-2);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: transparent;
		color: var(--base6);
		cursor: pointer;
	}
	.grp__link:hover {
		color: var(--fg);
		border-color: var(--base5);
	}
	.alist__empty {
		padding: var(--s-3);
		text-align: center;
		color: var(--base5);
		font: var(--t-xs) / 1.4 var(--font-mono);
	}
	.acard {
		background: var(--panel-bg-soft);
		border: 1px solid var(--panel-border);
		border-left: 3px solid var(--au, var(--yellow));
		border-radius: 3px;
		margin: var(--s-1) 0;
		font: var(--t-xs) / 1.45 var(--font-sans);
	}
	.acard.ask {
		border-left-color: var(--orange);
	}
	.acard__body {
		display: flex;
		flex-direction: column;
		gap: 2px;
		width: 100%;
		min-height: 44px;
		padding: var(--s-2) var(--s-3);
		text-align: left;
		background: transparent;
		border: 0;
		color: var(--fg);
		cursor: pointer;
	}
	.acard__body:hover {
		background: rgba(255, 255, 255, 0.03);
	}
	.acard__t {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: var(--s-2);
	}
	.acard__by {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font: var(--t-3xs) / 1.2 var(--font-mono);
		color: var(--base5);
	}
	.acard__dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--au, var(--yellow));
		flex: none;
	}
	.badge {
		font: 600 var(--t-3xs) / 1.2 var(--font-mono);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--cyan);
		border: 1px solid rgba(137, 184, 194, 0.5);
		border-radius: 3px;
		padding: 1px 6px;
		white-space: nowrap;
	}
	.badge.ask {
		color: var(--orange);
		border-color: rgba(226, 164, 120, 0.5);
	}
	.badge.ok {
		color: var(--green);
		border-color: rgba(180, 190, 130, 0.5);
	}
	.badge.bad {
		color: var(--red);
		border-color: rgba(226, 120, 120, 0.5);
	}
	.badge.hl {
		color: var(--yellow);
		border-color: rgba(233, 177, 137, 0.5);
	}
	.badge.cmt {
		color: var(--cyan);
	}
	.badge.shelf {
		color: var(--violet);
		border-color: rgba(178, 148, 187, 0.5);
	}
	.badge.muted {
		color: var(--base6);
		border-color: var(--panel-border);
	}
</style>
