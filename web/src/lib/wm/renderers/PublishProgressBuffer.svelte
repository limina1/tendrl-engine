<script lang="ts">
	import {
		getStore,
		aggregateAcceptRatio,
		eventAcceptRatio,
		ratioColor,
		type PublishEventStatus,
		type PublishRelayStatus,
		type RelayResult
	} from '../publish-progress.svelte';
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const store = getStore();
	const app = getAppState();
	const progress = $derived(store.current);

	// naddr forms for each event's `kind:pubkey:d_tag` coordinate, encoded
	// engine-side. Populated async into a map keyed by the raw `a`-tag so the
	// render path stays synchronous — it just reads `naddrByATag[aTag]`, which
	// is undefined until the encode resolves. Re-encodes when the progress (and
	// thus its set of coordinates) changes; cleanup cancels stale in-flight work.
	let naddrByATag = $state<Record<string, string>>({});

	$effect(() => {
		const p = store.current;
		if (!p) {
			naddrByATag = {};
			return;
		}
		const aTags = [
			...new Set([
				...(p.aTag ? [p.aTag] : []),
				...p.events.flatMap((ev) => (ev.aTag ? [ev.aTag] : []))
			])
		];
		if (aTags.length === 0) {
			naddrByATag = {};
			return;
		}
		let cancelled = false;
		(async () => {
			const pairs = await Promise.all(
				aTags.map(async (a) => {
					try {
						return [a, await api.encode({ kind: 'atag', a_tag: a })] as const;
					} catch {
						return null;
					}
				})
			);
			if (cancelled) return;
			const next: Record<string, string> = {};
			for (const pair of pairs) if (pair) next[pair[0]] = pair[1];
			naddrByATag = next;
		})();
		return () => {
			cancelled = true;
		};
	});

	let expanded = $state(new Set<string>());
	// Tracks the just-copied identifier so we can flash a "copied"
	// label next to the value the user clicked. Cleared after 1.4 s.
	let copiedKey = $state<string | null>(null);

	function toggleExpanded(id: string) {
		const next = new Set(expanded);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		expanded = next;
	}

	function shortenUrl(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}

	function shortenId(id: string): string {
		return id.length > 16 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;
	}

	function kindLabel(kind: number): string {
		if (kind === 30040) return 'index';
		if (kind === 30041) return 'section';
		return `kind:${kind}`;
	}

	async function copyValue(key: string, value: string) {
		try {
			await navigator.clipboard.writeText(value);
			copiedKey = key;
			setTimeout(() => {
				if (copiedKey === key) copiedKey = null;
			}, 1400);
		} catch (e) {
			console.warn('[PublishProgress] clipboard write failed', e);
		}
	}

	function showRawEvent(ev: PublishEventStatus) {
		if (ev.rawEvent != null) app.jsonModalData = { rawEvent: ev.rawEvent };
	}

	function inspectAll() {
		if (!progress) return;
		app.openEventsModal(
			progress.title ? `Published — ${progress.title}` : 'Published events',
			progress.events.map((ev) => ({
				label: ev.title ?? '[Untitled]',
				kind: ev.kind,
				id: ev.eventId,
				json: ev.rawEvent ?? { id: ev.eventId, kind: ev.kind }
			}))
		);
	}

	function naddrFor(ev: PublishEventStatus): string | null {
		return ev.aTag ? naddrByATag[ev.aTag] ?? null : null;
	}

	function dotClass(state: RelayResult): string {
		switch (state) {
			case 'accepted':
				return 'dot--ok';
			case 'rejected':
				return 'dot--err';
			case 'timeout':
				return 'dot--err';
			case 'sending':
				return 'dot--fetching';
			case 'pending':
			default:
				return 'dot--idle';
		}
	}

	function stateLabel(state: RelayResult): string {
		switch (state) {
			case 'accepted':
				return 'accepted';
			case 'rejected':
				return 'rejected';
			case 'timeout':
				return 'timeout';
			case 'sending':
				return 'sending…';
			case 'pending':
			default:
				return 'pending';
		}
	}

</script>

<div class="pp-view">
	{#if !progress}
		<div class="empty">
			<p>No publish in progress.</p>
			<p class="muted">
				This buffer renders progress when you publish an NKBIP-01 publication.
				The bar shows accepted-relay-cells across every event × relay; expand a row to
				see per-relay status, accept/reject reasons, and the event's address.
			</p>
			<p class="muted">
				Run the <code>tendrl-demo-publish-progress</code> command (<code>SPC :</code>) to load a representative
				snapshot of mock data while we're not signed in.
			</p>
		</div>
	{:else}
		{@const agg = aggregateAcceptRatio(progress)}
		{@const aggColor = ratioColor(agg.ratio)}

		{@const headerNaddr = progress.aTag ? naddrByATag[progress.aTag] ?? null : null}
		<header class="pp-header">
			<div class="pp-title-row">
				<span class="pp-title">{progress.title ?? 'Publishing'}</span>
				{#if progress.completed}
					<span class="pill pill--ghost">done</span>
				{:else}
					<span class="pill pill--ghost"><span class="dot dot--fetching"></span>publishing</span>
				{/if}
			</div>
			{#if headerNaddr}
				{@render copyable('publication-naddr', headerNaddr, 'pp-naddr', 'naddr — click to copy')}
			{/if}
			<div class="pp-summary">
				{progress.events.length} event{progress.events.length === 1 ? '' : 's'} ·
				{agg.accepted} / {agg.total} relay-cells accepted
				<button class="pp-inspect-all" onclick={inspectAll} title="Open every event in the JSON inspector (expand all / each)">
					inspect all JSON
				</button>
			</div>
		</header>

		<div
			class="pp-bar"
			style:--bar-fg={aggColor.fg}
			style:--bar-bg={aggColor.bg}
		>
			<div class="pp-bar-fill" style:width={`${agg.ratio * 100}%`}></div>
			<span class="pp-bar-label">
				{Math.round(agg.ratio * 100)}%
			</span>
		</div>

		<ul class="pp-events">
			{#each progress.events as ev (ev.eventId)}
				{@const ratio = eventAcceptRatio(ev)}
				{@const c = ratioColor(ratio.ratio)}
				{@const open = expanded.has(ev.eventId)}
				<li class="pp-event">
					<button
						class="pp-event-row"
						onclick={() => toggleExpanded(ev.eventId)}
						aria-expanded={open}
					>
						<span class="pp-disclosure">{open ? '▾' : '▸'}</span>
						<span class="pill pill--ghost kind-pill">{kindLabel(ev.kind)}</span>
						<span class="pp-event-title">{ev.title ?? '[Untitled]'}</span>
						<span class="pp-event-mini-bar" style:--bar-fg={c.fg} style:--bar-bg={c.bg}>
							<span class="pp-event-mini-fill" style:width={`${ratio.ratio * 100}%`}></span>
						</span>
						<span class="pp-event-count">
							{ratio.accepted}/{ratio.total}
						</span>
					</button>

					{#if open}
						{@const evNaddr = naddrFor(ev)}
						<div class="pp-event-detail">
							{#if ev.contentPreview}
								<p class="pp-content-preview">{ev.contentPreview}</p>
							{/if}
							<div class="pp-detail-actions">
								{#if ev.rawEvent != null}
									<button
										class="btn-link"
										onclick={() => showRawEvent(ev)}
										title="Show full JSON"
									>view raw JSON</button>
								{/if}
							</div>

							<dl class="kv">
								{#if evNaddr}
									<dt>naddr</dt>
									<dd>
										{@render copyValue_('event-naddr-' + ev.eventId, evNaddr, true)}
									</dd>
								{/if}
								{#if ev.aTag}
									<dt>a tag</dt>
									<dd>
										{@render copyValue_('event-atag-' + ev.eventId, ev.aTag, true)}
									</dd>
								{/if}
								<dt>event id</dt>
								<dd>
									{@render copyValue_('event-id-' + ev.eventId, ev.eventId, true)}
								</dd>
								<dt>author</dt>
								<dd>
									{@render copyValue_('event-author-' + ev.eventId, ev.author, true)}
								</dd>
								<dt>kind</dt>
								<dd>{ev.kind} ({kindLabel(ev.kind)})</dd>
							</dl>

							<div class="pp-relay-list">
								{#each ev.relays as relay (relay.url)}
									{@render relayRow(relay, ev.eventId)}
								{/each}
							</div>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

{#snippet relayRow(r: PublishRelayStatus, eventId: string)}
	<div class="pp-relay" class:pp-relay--local={r.isLocal}>
		<span class="dot {dotClass(r.state)}"></span>
		<button
			class="pp-relay-url mono copy-link"
			onclick={() => copyValue(`relay-${eventId}-${r.url}`, r.url)}
			title="{r.url} — click to copy"
		>
			{shortenUrl(r.url)}{copiedKey === `relay-${eventId}-${r.url}` ? ' ✓' : ''}
		</button>
		{#if r.isLocal}
			<span class="pill pill--local pp-relay-tag">local</span>
		{/if}
		<span class="pp-relay-state">{stateLabel(r.state)}</span>
		{#if r.message}
			<span class="pp-relay-msg" title={r.message}>{r.message}</span>
		{/if}
		{#if r.durationMs != null}
			<span class="pp-relay-dur">{r.durationMs}ms</span>
		{/if}
	</div>
{/snippet}

{#snippet copyable(key: string, value: string, klass: string, hint: string)}
	<button
		class="copy-link {klass} mono"
		onclick={() => copyValue(key, value)}
		title="{hint}: {value}"
	>
		{value}{copiedKey === key ? ' ✓ copied' : ''}
	</button>
{/snippet}

{#snippet copyValue_(key: string, value: string, short: boolean)}
	<button
		class="copy-link mono"
		onclick={() => copyValue(key, value)}
		title="{value} — click to copy"
	>
		{short ? shortenId(value) : value}{copiedKey === key ? ' ✓' : ''}
	</button>
{/snippet}

<style>
	.pp-view {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 0 0 24px;
		display: flex;
		flex-direction: column;
	}

	.empty {
		padding: 32px 24px;
		color: var(--base5);
		font-size: var(--t-sm);
		max-width: 60ch;
	}
	.empty p {
		margin: 0 0 12px;
	}
	.empty .muted {
		color: var(--base5);
	}
	.empty code {
		background: var(--bg-surface);
		padding: 1px 6px;
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}

	.pp-header {
		padding: 12px 16px 8px;
		border-bottom: 1px solid var(--panel-border);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.pp-title-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.pp-title {
		font-weight: 600;
		font-size: var(--t-md);
	}

	.pp-naddr {
		font-size: var(--t-xs);
		color: var(--base6);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		text-align: left;
		max-width: 100%;
	}

	/* Click-to-copy buttons reuse mono font and align with their dt
	   labels visually, but stay clickable + show a brief ✓ confirmation
	   when a value lands on the clipboard. */
	.copy-link {
		background: transparent;
		border: none;
		color: inherit;
		font: inherit;
		padding: 0;
		cursor: pointer;
		text-align: left;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: 100%;
	}
	.copy-link:hover {
		color: var(--accent);
		text-decoration: underline;
		text-decoration-style: dotted;
	}
	.copy-link:focus-visible {
		outline: 1px solid var(--accent);
		outline-offset: 2px;
	}

	.pp-detail-actions {
		display: flex;
		gap: 8px;
	}
	.btn-link {
		background: transparent;
		border: none;
		color: var(--accent);
		font: inherit;
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		cursor: pointer;
		padding: 2px 0;
	}
	.btn-link:hover {
		text-decoration: underline;
	}

	.pp-summary {
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.pp-inspect-all {
		margin-left: 8px;
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 1px 8px;
	}

	/* Top-level progress bar. The fill width is the accept ratio; the
	   color comes from --bar-fg / --bar-bg set per render via inline
	   custom properties so it can shift red→yellow→green smoothly. */
	.pp-bar {
		position: relative;
		height: 14px;
		margin: 12px 16px 8px;
		border-radius: 999px;
		background: var(--base2);
		overflow: hidden;
	}
	.pp-bar-fill {
		position: absolute;
		inset: 0 auto 0 0;
		background: var(--bar-fg, var(--state-online));
		transition: width 200ms ease-out;
	}
	.pp-bar-label {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
		font-family: var(--font-mono);
		font-size: 0.65rem;
		color: var(--fg);
		mix-blend-mode: difference;
	}

	.pp-events {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.pp-event {
		border-bottom: 1px solid var(--panel-border);
	}

	.pp-event-row {
		width: 100%;
		display: grid;
		grid-template-columns: 18px auto 1fr 100px 50px;
		gap: 8px;
		align-items: center;
		padding: 8px 16px;
		background: transparent;
		border: none;
		text-align: left;
		cursor: pointer;
		font: inherit;
	}
	.pp-event-row:hover {
		background: var(--bg-surface);
	}

	.pp-disclosure {
		font-size: 0.75rem;
		color: var(--fg-muted);
	}

	.kind-pill {
		font-size: 0.65rem;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.pp-event-title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--fg);
	}

	.pp-event-mini-bar {
		position: relative;
		height: 8px;
		border-radius: 999px;
		background: var(--base2);
		overflow: hidden;
	}
	.pp-event-mini-fill {
		position: absolute;
		inset: 0 auto 0 0;
		background: var(--bar-fg, var(--state-online));
	}

	.pp-event-count {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
		text-align: right;
	}

	.pp-event-detail {
		padding: 4px 16px 14px 42px;
		display: flex;
		flex-direction: column;
		gap: 12px;
		background: var(--bg-surface);
	}

	.pp-content-preview {
		margin: 0;
		font-size: var(--t-sm);
		color: var(--base7);
		line-height: var(--lh-snug);
		white-space: pre-wrap;
		border-left: 2px solid var(--panel-border);
		padding-left: 10px;
	}

	.kv {
		display: grid;
		grid-template-columns: 90px 1fr;
		gap: 2px 12px;
		margin: 0;
		font-size: var(--t-xs);
	}
	.kv dt {
		color: var(--base5);
		font-family: var(--font-mono);
	}
	.kv dd {
		margin: 0;
		color: var(--fg);
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.pp-relay-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.pp-relay {
		display: grid;
		grid-template-columns: 12px 1fr auto auto auto auto;
		gap: 8px;
		align-items: center;
		font-size: var(--t-xs);
		padding: 4px 0;
		border-bottom: 1px dashed color-mix(in srgb, var(--panel-border) 60%, transparent);
	}
	.pp-relay:last-child { border-bottom: none; }
	.pp-relay--local .pp-relay-url {
		color: var(--id-local);
	}

	.pp-relay-url {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.pp-relay-tag {
		font-size: 0.55rem;
		padding: 0 4px;
		text-transform: uppercase;
	}

	.pp-relay-state {
		font-family: var(--font-mono);
		color: var(--base6);
		min-width: 70px;
	}

	.pp-relay-msg {
		color: var(--id-draft);
		max-width: 22ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		font-style: italic;
	}

	.pp-relay-dur {
		color: var(--base5);
		font-family: var(--font-mono);
	}

	/* Reuse global .dot but add result-specific classes. */
	:global(.dot--ok) {
		background: var(--state-online);
		box-shadow: 0 0 6px rgba(180, 190, 130, 0.5);
	}
	:global(.dot--err) {
		background: var(--id-draft);
	}
	:global(.dot--idle) {
		background: var(--base3);
	}

	.mono {
		font-family: var(--font-mono);
	}
</style>
