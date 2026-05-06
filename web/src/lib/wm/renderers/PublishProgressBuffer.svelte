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
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const store = getStore();
	const progress = $derived(store.current);

	let expanded = $state(new Set<string>());

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

	function eventDisplayId(ev: PublishEventStatus): string {
		return ev.naddr ?? ev.eventId;
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
				Run <code>M-x tendrl-demo-publish-progress</code> to load a representative
				snapshot of mock data while we're not signed in.
			</p>
		</div>
	{:else}
		{@const agg = aggregateAcceptRatio(progress)}
		{@const aggColor = ratioColor(agg.ratio)}

		<header class="pp-header">
			<div class="pp-title-row">
				<span class="pp-title">{progress.title ?? 'Publishing'}</span>
				{#if progress.completed}
					<span class="pill pill--ghost">done</span>
				{:else}
					<span class="pill pill--ghost"><span class="dot dot--fetching"></span>publishing</span>
				{/if}
			</div>
			{#if progress.naddr}
				<div class="pp-naddr mono">{progress.naddr}</div>
			{/if}
			<div class="pp-summary">
				{progress.events.length} event{progress.events.length === 1 ? '' : 's'} ·
				{agg.accepted} / {agg.total} relay-cells accepted
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
						<div class="pp-event-detail">
							<dl class="kv">
								<dt>address</dt>
								<dd class="mono">{eventDisplayId(ev)}</dd>
								<dt>event id</dt>
								<dd class="mono">{shortenId(ev.eventId)}</dd>
								<dt>author</dt>
								<dd class="mono">{shortenId(ev.author)}</dd>
								<dt>kind</dt>
								<dd>{ev.kind} ({kindLabel(ev.kind)})</dd>
							</dl>

							<div class="pp-relay-list">
								{#each ev.relays as relay (relay.url)}
									{@render relayRow(relay)}
								{/each}
							</div>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

{#snippet relayRow(r: PublishRelayStatus)}
	<div class="pp-relay" class:pp-relay--local={r.isLocal}>
		<span class="dot {dotClass(r.state)}"></span>
		<span class="pp-relay-url mono" title={r.url}>{shortenUrl(r.url)}</span>
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
	}

	.pp-summary {
		font-size: var(--t-xs);
		color: var(--base5);
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
