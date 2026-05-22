<script lang="ts">
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getRelayInfo, normalizeRelayUrl, type Nip11Status, type Nip11Doc } from '$lib/relay/nip11';
	import ProfileName from '$lib/components/ProfileName.svelte';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	// Per docs/relay-classes-and-info-port.md, a relay row carries the
	// role-agnostic shell (URL + runtime metadata + NIP-11 derived
	// flags) while role membership (read/write) lives in role-specific
	// list events. Auth here is a placeholder for the eventual
	// blocked/auth-required taxonomy; toggles don't persist yet.
	type RelayRow = {
		url: string;
		read: boolean;
		write: boolean;
		auth: boolean;
	};

	let rows = $state<RelayRow[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(new Set<string>());
	// Map<normalizedUrl, Nip11Status> — refreshed reactively as fetches
	// resolve. Fresh object each update so $derived sees a change.
	let infoMap = $state<Record<string, Nip11Status>>({});

	async function load(force = false) {
		loading = true;
		try {
			const cfg = await api.getRelayConfig();
			const map = new Map<string, RelayRow>();
			const ensure = (url: string): RelayRow => {
				let r = map.get(url);
				if (!r) {
					r = { url, read: false, write: false, auth: false };
					map.set(url, r);
				}
				return r;
			};
			for (const url of cfg.general?.urls ?? []) {
				const r = ensure(url);
				r.read = true;
				r.write = true;
			}
			for (const url of cfg.fetch?.urls ?? []) ensure(url).read = true;
			for (const url of cfg.publish?.urls ?? []) ensure(url).write = true;
			rows = [...map.values()].sort((a, b) => a.url.localeCompare(b.url));
			// Kick off NIP-11 fetches up-front so the badges fill in
			// without the user expanding each row.
			for (const r of rows) primeInfo(r.url, force);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	function primeInfo(url: string, force = false) {
		const key = normalizeRelayUrl(url);
		if (force) infoMap = { ...infoMap, [key]: { state: 'loading' } };
		const status = getRelayInfo(
			url,
			(s) => {
				infoMap = { ...infoMap, [key]: s };
			},
			{ force }
		);
		if (!force) infoMap = { ...infoMap, [key]: status };
	}

	$effect(() => {
		load();
	});

	async function toggle(url: string, field: 'read' | 'write' | 'auth') {
		const row = rows.find((r) => r.url === url);
		if (!row) return;
		const next = { ...row, [field]: !row[field] };
		rows = rows.map((r) => (r.url === url ? next : r)); // optimistic

		// `auth` has no config home yet — keep it cosmetic.
		if (field === 'auth') return;

		try {
			// Reconcile the row's read/write into explicit fetch + publish set
			// membership, and drop it from the legacy `general` set (which means
			// read+write) so a toggle-off actually takes effect after restart.
			await api.removeRelay('general', url);
			await (next.read ? api.addRelay('fetch', url) : api.removeRelay('fetch', url));
			await (next.write ? api.addRelay('publish', url) : api.removeRelay('publish', url));
			app.pushToast('Relay config saved — restart engine to apply', 'info', 3000);
		} catch (e) {
			rows = rows.map((r) => (r.url === url ? row : r)); // revert on failure
			app.pushToast(
				`Couldn't save relay config: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		}
	}

	function toggleExpanded(url: string) {
		const next = new Set(expanded);
		if (next.has(url)) next.delete(url);
		else next.add(url);
		expanded = next;
	}

	function shorten(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/$/, '');
	}

	function statusFor(url: string): Nip11Status {
		return infoMap[normalizeRelayUrl(url)] ?? { state: 'pending' };
	}

	function docFor(url: string): Nip11Doc | null {
		const s = statusFor(url);
		return s.state === 'loaded' ? s.doc : null;
	}
</script>

<div class="relays-view">
	<div class="relays-header">
		<span>Relay configuration</span>
		<span class="relays-hint">read/write persist (restart to apply) · auth is cosmetic</span>
	</div>

	{#if loading}
		<p class="empty">Loading…</p>
	{:else if error}
		<p class="empty error">{error}</p>
	{:else if rows.length === 0}
		<p class="empty">No relays configured</p>
	{:else}
		<div class="relays-list">
			{#each rows as row (row.url)}
				{@const status = statusFor(row.url)}
				{@const doc = docFor(row.url)}
				{@const lim = doc?.limitation}
				<div class="relay-card" class:relay-card--expanded={expanded.has(row.url)}>
					<div class="relay-row">
						<button
							class="relay-disclosure"
							onclick={() => toggleExpanded(row.url)}
							aria-expanded={expanded.has(row.url)}
							title={expanded.has(row.url) ? 'Collapse' : 'Show NIP-11 details'}
						>{expanded.has(row.url) ? '▾' : '▸'}</button>

						<div class="relay-id">
							<span class="relay-url">{shorten(row.url)}</span>
							<div class="relay-flags">
								{#if status.state === 'loading'}
									<span class="pill pill--ghost"><span class="dot dot--fetching"></span>info</span>
								{:else if status.state === 'failed'}
									<span class="pill pill--ghost" title={status.error}>info: {status.error.slice(0, 24)}</span>
								{:else if doc}
									{#if lim?.payment_required}
										<span class="pill pill--draft" title="Payment required">paid</span>
									{/if}
									{#if lim?.auth_required}
										<span class="pill pill--imported" title="NIP-42 auth required">auth</span>
									{/if}
									{#if lim?.restricted_writes}
										<span class="pill pill--diverged" title="Writes restricted">restricted</span>
									{/if}
									{#if doc.software}
										<span class="pill pill--ghost" title="{doc.software}{doc.version ? ` ${doc.version}` : ''}">{doc.software.split('/').pop()}</span>
									{/if}
								{/if}
							</div>
						</div>

						<div class="relay-toggles">
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.read}
								onclick={() => toggle(row.url, 'read')}
								title="Read from this relay"
							>read</button>
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.write}
								onclick={() => toggle(row.url, 'write')}
								title="Publish to this relay"
							>write</button>
							<button
								class="pill toggle-pill"
								class:toggle-pill--on={row.auth}
								onclick={() => toggle(row.url, 'auth')}
								title="Authenticate (NIP-42) when this relay challenges"
							>auth</button>
						</div>
					</div>

					{#if expanded.has(row.url)}
						<div class="relay-detail">
							{#if status.state === 'loading'}
								<p class="empty muted">Fetching NIP-11…</p>
							{:else if status.state === 'failed'}
								<div class="failed-detail">
									<p class="empty error">Couldn't load NIP-11: {status.error}</p>
									<button class="btn-refresh" onclick={() => primeInfo(row.url, true)}>Retry</button>
								</div>
							{:else if doc}
								{#if doc.name || doc.description}
									<section class="info-section">
										{#if doc.name}<h3 class="info-title">{doc.name}</h3>{/if}
										{#if doc.description}<p class="info-desc">{doc.description}</p>{/if}
									</section>
								{/if}

								{#if doc.software || doc.version || doc.contact || doc.pubkey}
									<section class="info-section">
										<div class="info-section-title">Software</div>
										<dl class="kv">
											{#if doc.software}<dt>software</dt><dd class="mono">{doc.software}</dd>{/if}
											{#if doc.version}<dt>version</dt><dd class="mono">{doc.version}</dd>{/if}
											{#if doc.contact}<dt>contact</dt><dd>{doc.contact}</dd>{/if}
											{#if doc.pubkey}<dt>operator</dt><dd><ProfileName pubkey={doc.pubkey} /></dd>{/if}
										</dl>
									</section>
								{/if}

								{#if doc.supported_nips && doc.supported_nips.length > 0}
									<section class="info-section">
										<div class="info-section-title">Supported NIPs</div>
										<div class="nip-chips">
											{#each doc.supported_nips as nip (nip)}
												<a
													class="nip-chip"
													href={`https://github.com/nostr-protocol/nips/blob/master/${String(nip).padStart(2, '0')}.md`}
													target="_blank"
													rel="noopener noreferrer"
													title="Open NIP-{nip} in a new tab"
												>{nip}</a>
											{/each}
										</div>
									</section>
								{/if}

								{#if lim}
									<section class="info-section">
										<div class="info-section-title">Limitations</div>
										{#if lim.max_message_length || lim.max_event_tags || lim.max_content_length || lim.max_subscriptions || lim.max_limit || lim.min_pow_difficulty}
											<div class="info-subtitle">Sizes &amp; throughput</div>
											<dl class="kv">
												{#if lim.max_message_length}
													<dt title="Maximum bytes in any single client→relay message">max message</dt>
													<dd>{lim.max_message_length.toLocaleString()} bytes</dd>
												{/if}
												{#if lim.max_event_tags}
													<dt title="Maximum tags on a single event">max tags</dt>
													<dd>{lim.max_event_tags}</dd>
												{/if}
												{#if lim.max_content_length}
													<dt title="Maximum bytes in an event's content field">max content</dt>
													<dd>{lim.max_content_length.toLocaleString()} bytes</dd>
												{/if}
												{#if lim.max_subscriptions}
													<dt title="Maximum concurrent REQ subscriptions on one connection (not per second)">max subscriptions</dt>
													<dd>{lim.max_subscriptions}</dd>
												{/if}
												{#if lim.max_limit}
													<dt title="Largest value the relay accepts in a filter's `limit` field">max filter limit</dt>
													<dd>{lim.max_limit}</dd>
												{/if}
												{#if lim.min_pow_difficulty}
													<dt title="Minimum NIP-13 proof-of-work difficulty (leading zero bits)">min PoW</dt>
													<dd>{lim.min_pow_difficulty} bits</dd>
												{/if}
											</dl>
										{/if}

										{#if lim.auth_required || lim.payment_required || lim.restricted_writes}
											<div class="info-subtitle">Access</div>
											<dl class="kv">
												{#if lim.auth_required}
													<dt title="Relay challenges connections with NIP-42 auth before serving">auth required</dt>
													<dd>yes</dd>
												{/if}
												{#if lim.payment_required}
													<dt title="Relay requires payment (see Fees) before accepting events">payment required</dt>
													<dd>yes</dd>
												{/if}
												{#if lim.restricted_writes}
													<dt title="Anyone can read; only members can publish">restricted writes</dt>
													<dd>yes</dd>
												{/if}
											</dl>
										{/if}

										{#if (lim.created_at_lower_limit ?? 0) > 0 || (lim.created_at_upper_limit ?? 0) > 0}
											<div class="info-subtitle">Event time bounds</div>
											<dl class="kv">
												{#if (lim.created_at_lower_limit ?? 0) > 0}
													<dt title="Events with `created_at` older than this (Unix seconds) are rejected">created_at min</dt>
													<dd>{lim.created_at_lower_limit}</dd>
												{/if}
												{#if (lim.created_at_upper_limit ?? 0) > 0}
													<dt title="Events with `created_at` newer than this (Unix seconds) are rejected">created_at max</dt>
													<dd>{lim.created_at_upper_limit}</dd>
												{/if}
											</dl>
										{/if}
									</section>
								{/if}

								{#if doc.fees && (doc.fees.admission?.length || doc.fees.subscription?.length || doc.fees.publication?.length)}
									<section class="info-section">
										<div class="info-section-title">Fees</div>
										<dl class="kv">
											{#each doc.fees.admission ?? [] as fee, i (`a${i}`)}
												<dt>admission</dt><dd>{fee.amount} {fee.unit}</dd>
											{/each}
											{#each doc.fees.subscription ?? [] as fee, i (`s${i}`)}
												<dt>subscription</dt><dd>{fee.amount} {fee.unit}{fee.period ? ` / ${fee.period}s` : ''}</dd>
											{/each}
											{#each doc.fees.publication ?? [] as fee, i (`p${i}`)}
												<dt>publication{fee.kinds ? ` (k:${fee.kinds.join(',')})` : ''}</dt>
												<dd>{fee.amount} {fee.unit}</dd>
											{/each}
										</dl>
									</section>
								{/if}

								{#if (doc.tags && doc.tags.length) || (doc.relay_countries && doc.relay_countries.length) || (doc.language_tags && doc.language_tags.length)}
									<section class="info-section">
										<div class="info-section-title">Audience</div>
										<dl class="kv">
											{#if doc.tags?.length}<dt>tags</dt><dd>{doc.tags.join(', ')}</dd>{/if}
											{#if doc.relay_countries?.length}<dt>countries</dt><dd>{doc.relay_countries.join(', ')}</dd>{/if}
											{#if doc.language_tags?.length}<dt>languages</dt><dd>{doc.language_tags.join(', ')}</dd>{/if}
										</dl>
									</section>
								{/if}

								{#if doc.privacy_policy || doc.terms_of_service || doc.posting_policy}
									<section class="info-section">
										<div class="info-section-title">Policies</div>
										<dl class="kv">
											{#if doc.privacy_policy}<dt>privacy</dt><dd><a href={doc.privacy_policy} target="_blank" rel="noopener noreferrer">{doc.privacy_policy}</a></dd>{/if}
											{#if doc.terms_of_service}<dt>terms</dt><dd><a href={doc.terms_of_service} target="_blank" rel="noopener noreferrer">{doc.terms_of_service}</a></dd>{/if}
											{#if doc.posting_policy}<dt>posting</dt><dd><a href={doc.posting_policy} target="_blank" rel="noopener noreferrer">{doc.posting_policy}</a></dd>{/if}
										</dl>
									</section>
								{/if}
							{:else}
								<p class="empty muted">No NIP-11 fetched yet.</p>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
		</div>

		<div class="relays-footer">
			<button class="btn-add" disabled title="Will prompt for a relay URL">+ Add relay</button>
			<button class="btn-refresh" onclick={() => load(true)}>Refresh</button>
		</div>
	{/if}
</div>

<style>
	.relays-view {
		flex: 1;
		overflow-y: auto;
		min-height: 0;
		padding: 0 0 24px;
	}

	.relays-header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}

	.relays-hint {
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
		color: var(--base5);
		font-style: italic;
	}

	.empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.empty.error { color: var(--id-draft); }
	.empty.muted { color: var(--base5); }

	.failed-detail {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 6px;
	}
	.failed-detail .empty {
		padding: 8px 0;
		text-align: left;
	}

	.relays-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 6px 0;
	}

	.relay-card {
		border-bottom: 1px solid var(--panel-border);
	}
	.relay-card--expanded {
		background: var(--bg-surface);
	}

	.relay-row {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 8px 14px;
	}

	.relay-disclosure {
		background: transparent;
		border: none;
		color: var(--fg-muted);
		font-size: 0.8rem;
		min-width: 18px;
		cursor: pointer;
		padding: 0;
	}
	.relay-disclosure:hover { color: var(--fg); }

	.relay-id {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}

	.relay-url {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.relay-flags {
		display: flex;
		gap: 4px;
		flex-wrap: wrap;
	}

	.relay-toggles {
		display: flex;
		gap: 4px;
		flex-shrink: 0;
	}

	/* Toggle pills: ghost outline when off, filled-tinted when on. The
	   "on" tints reuse pill--online so all three toggles read as "this
	   relay carries this role." */
	.toggle-pill {
		border: 1px solid var(--base3);
		background: transparent;
		color: var(--base6);
		cursor: pointer;
		font-family: var(--font-mono);
		padding: 1px 8px;
	}
	.toggle-pill:hover {
		color: var(--fg);
	}
	.toggle-pill--on {
		background: rgba(180, 190, 130, 0.14);
		color: var(--state-online);
		border-color: color-mix(in srgb, var(--state-online) 50%, transparent);
	}
	.toggle-pill--on:hover {
		filter: brightness(1.15);
	}

	.relay-detail {
		padding: 4px 14px 16px 38px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.info-section {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.info-section-title {
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
	}
	.info-subtitle {
		font-size: var(--t-xs);
		color: var(--base6);
		margin-top: 4px;
	}
	.info-title {
		font-size: var(--t-md);
		margin: 0;
	}
	.info-desc {
		font-size: var(--t-sm);
		margin: 0;
		color: var(--fg);
	}

	.kv {
		display: grid;
		grid-template-columns: 110px 1fr;
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
	.kv .mono {
		font-family: var(--font-mono);
	}
	.kv a {
		color: var(--accent);
	}

	.nip-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.nip-chip {
		display: inline-block;
		padding: 1px 8px;
		border-radius: var(--r-md);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: rgba(137, 184, 194, 0.12);
		color: var(--id-remote);
		text-decoration: none;
	}
	.nip-chip:hover {
		filter: brightness(1.15);
	}

	.relays-footer {
		display: flex;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
		margin-top: 8px;
	}
	.btn-add,
	.btn-refresh {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
	}
	.btn-add[disabled] {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
