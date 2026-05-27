<script lang="ts">
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { getRelayInfo, normalizeRelayUrl, type Nip11Status, type Nip11Doc } from '$lib/relay/nip11';
	import { relayFocus, consumeRelayFocus } from '$lib/relay/focus.svelte';
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
	let initialRelays = $state<string[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(new Set<string>());
	// Map<normalizedUrl, Nip11Status> — refreshed reactively as fetches
	// resolve. Fresh object each update so $derived sees a change.
	let infoMap = $state<Record<string, Nip11Status>>({});
	// Per-row DOM refs so we can scroll a focused row into view when
	// the EventViewModal hands us a URL via the relayFocus signal.
	let rowEls: Record<string, HTMLDivElement | undefined> = {};
	// Pulled suggestions from the user's kind 10002. Surfaced as
	// suggestions only — never auto-applied. The user picks per relay.
	type PulledRelay = { url: string; read: boolean; write: boolean };
	let pulled = $state<PulledRelay[] | null>(null);
	let pulling = $state(false);
	let pullError = $state<string | null>(null);
	let pullCreatedAt = $state<number | null>(null);

	async function load(force = false) {
		loading = true;
		try {
			const cfg = await api.getRelayConfig();
			initialRelays = cfg.initial_relays ?? [];
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

	// Consume the one-shot focus signal once rows have populated: expand
	// the matching row and scroll it into view. Matched by normalized URL
	// so trailing-slash / case / port differences don't miss.
	$effect(() => {
		const focus = relayFocus.url;
		if (!focus || rows.length === 0) return;
		const target = normalizeRelayUrl(focus);
		const row = rows.find((r) => normalizeRelayUrl(r.url) === target);
		if (!row) return;
		consumeRelayFocus();
		const next = new Set(expanded);
		next.add(row.url);
		expanded = next;
		primeInfo(row.url);
		// Wait a frame so the {#if expanded} detail block is in the DOM
		// before scrolling — gives a smoother "lands at the right place".
		queueMicrotask(() => {
			rowEls[row.url]?.scrollIntoView({ behavior: 'smooth', block: 'center' });
		});
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
			app.pushToast('Relay config saved', 'success', 2000);
		} catch (e) {
			rows = rows.map((r) => (r.url === url ? row : r)); // revert on failure
			app.pushToast(
				`Couldn't save relay config: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		}
	}

	// Pull the user's kind 10002 (NIP-65 read/write relays) from the
	// configured `initial_relays` and surface them as **suggestions** —
	// never auto-applied, never re-published. The user picks per-relay
	// what to import into their working sets. See
	// `project_publishing_philosophy.md`.
	async function pullFromProfile() {
		const pubkey = app.myPubkey;
		if (!pubkey) {
			pullError = 'Sign in first — no pubkey to look up.';
			return;
		}
		if (initialRelays.length === 0) {
			pullError = 'No initial relays configured in config.toml. Add `initial_relays = [...]` under `[relay]` to seed.';
			return;
		}
		pulling = true;
		pullError = null;
		try {
			// 1. Pull the user's kind 10002 from the seed relays. Confirm-mode
			//    will gate this; otherwise it goes through silently.
			await api.fetchFromRelay(initialRelays, [10002], [pubkey], 5);
			// 2. Read it back from the local cache. The newest one wins.
			const result = await api.search(`by:${pubkey} k:10002`, 3, pubkey, 'local_only');
			const newest = (result.results ?? []).sort(
				(a, b) => (b.created_at ?? 0) - (a.created_at ?? 0)
			)[0];
			if (!newest) {
				pullError = "No kind 10002 found on those relays for your pubkey. (If you've never published your relay list, there's nothing to pull yet.)";
				pulled = [];
				return;
			}
			pullCreatedAt = newest.created_at ?? null;
			const entries: PulledRelay[] = (newest.tags ?? [])
				.filter((t) => t[0] === 'r' && typeof t[1] === 'string')
				.map((t) => {
					const marker = (t[2] ?? '').toLowerCase();
					return {
						url: t[1] as string,
						read: marker === 'read' || marker === '',
						write: marker === 'write' || marker === ''
					};
				});
			pulled = entries;
		} catch (e) {
			pullError = e instanceof Error ? e.message : String(e);
		} finally {
			pulling = false;
		}
	}

	function rowKeyFor(url: string): string {
		return normalizeRelayUrl(url);
	}

	function alreadyConfigured(url: string): RelayRow | undefined {
		const key = rowKeyFor(url);
		return rows.find((r) => rowKeyFor(r.url) === key);
	}

	async function importSuggestion(s: PulledRelay, role: 'fetch' | 'publish' | 'both') {
		try {
			if (role === 'fetch' || role === 'both') await api.addRelay('fetch', s.url);
			if (role === 'publish' || role === 'both') await api.addRelay('publish', s.url);
			app.pushToast(`Added ${shorten(s.url)} to ${role}`, 'success', 2500);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't add ${shorten(s.url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	function dismissPulled() {
		pulled = null;
		pullError = null;
		pullCreatedAt = null;
	}

	// Add a new relay via the prompt — defaults to read+write so the
	// relay is fully active; user can toggle either side off after.
	async function promptAdd() {
		const raw = window.prompt('Relay URL (bare hostname OK — wss:// is added if missing):');
		if (!raw) return;
		const trimmed = raw.trim();
		if (!trimmed) return;
		// Client-side normalization for nice display; the engine
		// normalizes again on the receiving end, so this is purely UX.
		const url = normalizeRelayUrl(trimmed);
		if (rows.some((r) => normalizeRelayUrl(r.url) === url)) {
			app.pushToast(`${shorten(url)} is already configured`, 'info', 2500);
			return;
		}
		try {
			await api.addRelay('fetch', url);
			await api.addRelay('publish', url);
			app.pushToast(`Added ${shorten(url)} (read + write)`, 'success', 2500);
			await load();
		} catch (e) {
			app.pushToast(
				`Couldn't add ${shorten(url)}: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				4000
			);
		}
	}

	let snapshotting = $state(false);
	async function snapshotToConfig() {
		snapshotting = true;
		try {
			const resp = await api.snapshotConfig();
			app.pushToast(resp.message, 'success', 3500);
		} catch (e) {
			app.pushToast(
				`Snapshot failed: ${e instanceof Error ? e.message : String(e)}`,
				'error',
				5000
			);
		} finally {
			snapshotting = false;
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
		<span class="relays-hint">read/write apply live and persist · auth is cosmetic</span>
	</div>

	<!-- Pull-from-profile: fetches the user's kind 10002 (NIP-65) from
	     the configured initial_relays and surfaces it as suggestions.
	     Suggestions never auto-apply; the user picks per relay. -->
	<div class="pull-bar">
		{#if !pulled && !pulling && !pullError}
			<button
				class="btn-pull"
				onclick={pullFromProfile}
				disabled={!app.myPubkey || initialRelays.length === 0}
				title={!app.myPubkey
					? 'Sign in first'
					: initialRelays.length === 0
						? 'No initial_relays in config.toml'
						: `Fetch your published relay list (kind 10002) from ${initialRelays.length} initial relay${initialRelays.length === 1 ? '' : 's'}`}
			>
				Pull from your profile
			</button>
			{#if !app.myPubkey}
				<span class="pull-hint pull-hint--warn">Sign in first to fetch your relay list.</span>
			{:else if initialRelays.length === 0}
				<span class="pull-hint pull-hint--warn">
					No <code>initial_relays</code> configured. Add a few in <code>config.toml</code> under <code>[relay]</code> (e.g. <code>initial_relays = ["wss://relay.damus.io", "wss://nos.lol"]</code>) and restart — or add relays directly below.
				</span>
			{:else}
				<span class="pull-hint">Reads your kind 10002 from <code>initial_relays</code> ({initialRelays.length} configured); you choose what to import.</span>
			{/if}
		{:else if pulling}
			<span class="pull-status">Fetching your relay list…</span>
		{:else if pullError}
			<span class="pull-status pull-status--err">{pullError}</span>
			<button class="btn-pull btn-pull--small" onclick={dismissPulled}>dismiss</button>
			<button class="btn-pull btn-pull--small" onclick={pullFromProfile}>retry</button>
		{:else if pulled}
			<span class="pull-status">
				Found {pulled.length} relay{pulled.length === 1 ? '' : 's'} in your kind 10002{#if pullCreatedAt}
					· {new Date(pullCreatedAt * 1000).toLocaleDateString()}
				{/if}
			</span>
			<button class="btn-pull btn-pull--small" onclick={dismissPulled}>dismiss</button>
		{/if}
	</div>

	{#if pulled && pulled.length > 0}
		<div class="pulled-list">
			<div class="pulled-label">From your profile · suggestions</div>
			{#each pulled as s (s.url)}
				{@const existing = alreadyConfigured(s.url)}
				<div class="pulled-row">
					<span class="pulled-url" title={s.url}>{shorten(s.url)}</span>
					<span class="pulled-marker">
						{#if s.read && s.write}read+write
						{:else if s.read}read
						{:else if s.write}write
						{/if}
					</span>
					{#if existing}
						<span class="pulled-state">already configured</span>
					{:else}
						<div class="pulled-actions">
							{#if s.read}
								<button class="pull-add" onclick={() => importSuggestion(s, 'fetch')}>+ fetch</button>
							{/if}
							{#if s.write}
								<button class="pull-add" onclick={() => importSuggestion(s, 'publish')}>+ publish</button>
							{/if}
							{#if s.read && s.write}
								<button class="pull-add pull-add--strong" onclick={() => importSuggestion(s, 'both')}>+ both</button>
							{/if}
						</div>
					{/if}
				</div>
			{/each}
		</div>
	{/if}

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
				<div class="relay-card" class:relay-card--expanded={expanded.has(row.url)} bind:this={rowEls[row.url]}>
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
			<button class="btn-add" onclick={promptAdd} title="Add a new relay (defaults to read + write — toggle either off after)">+ Add relay</button>
			<button class="btn-refresh" onclick={() => load(true)}>Refresh</button>
			<button
				class="btn-snapshot"
				onclick={snapshotToConfig}
				disabled={snapshotting || rows.length === 0}
				title="Write the current relay set into config.toml's `initial_relays` — a portable bootstrap seed for another machine or a fresh data dir. relays.json stays the runtime source of truth."
			>
				{snapshotting ? 'Saving…' : 'Save settings'}
			</button>
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
		/* Pin to the bottom of the scrollable buffer so the action row
		   (especially "Save settings") is always reachable even when
		   the relay list scrolls. Background prevents row text from
		   showing through. */
		position: sticky;
		bottom: 0;
		background: var(--panel-bg, var(--bg));
		z-index: 1;
	}
	.btn-add,
	.btn-refresh {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
	}
	.btn-snapshot[disabled] {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.btn-snapshot {
		font-size: var(--t-xs);
		padding: 4px 10px;
		font-family: var(--font-mono);
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-yours) 35%, transparent);
		color: var(--id-yours);
		cursor: pointer;
		margin-left: auto;
		border-radius: var(--r-sm);
	}
	.btn-snapshot:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-yours) 24%, transparent);
	}

	/* Pull-from-profile bar + suggestion list. Suggestions are deliberately
	   chip-styled (not row-styled like configured relays) so they read as
	   "external suggestion, click to accept" rather than "live config."
	   Per project_publishing_philosophy.md, suggestions never auto-apply. */
	.pull-bar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		padding: 8px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.btn-pull {
		font-size: var(--t-xs);
		padding: 3px 10px;
		font-family: var(--font-mono);
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 14%, transparent);
		border: 1px solid color-mix(in srgb, var(--id-remote, var(--id-yours)) 35%, transparent);
		color: var(--id-remote, var(--fg));
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.btn-pull:hover:not([disabled]) {
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 24%, transparent);
	}
	.btn-pull[disabled] {
		opacity: 0.45;
		cursor: not-allowed;
	}
	.btn-pull--small {
		font-size: 0.7rem;
		padding: 2px 8px;
	}
	.pull-hint {
		font-size: var(--t-xs);
		color: var(--base5);
		font-style: italic;
	}
	.pull-hint--warn {
		color: var(--id-draft);
		font-style: normal;
	}
	.pull-hint code {
		font-family: var(--font-mono);
		font-style: normal;
	}
	.pull-status {
		font-size: var(--t-xs);
		color: var(--base6);
	}
	.pull-status--err {
		color: var(--id-draft);
	}
	.pulled-list {
		padding: 8px 14px 10px 14px;
		display: flex;
		flex-direction: column;
		gap: 4px;
		background: color-mix(in srgb, var(--id-remote, var(--id-yours)) 4%, transparent);
		border-bottom: 1px solid var(--panel-border);
	}
	.pulled-label {
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		margin-bottom: 2px;
	}
	.pulled-row {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: var(--t-xs);
	}
	.pulled-url {
		font-family: var(--font-mono);
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.pulled-marker {
		font-family: var(--font-mono);
		color: var(--base5);
		font-size: 0.7rem;
	}
	.pulled-state {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--base5);
		font-style: italic;
	}
	.pulled-actions {
		display: flex;
		gap: 4px;
	}
	.pull-add {
		font-size: 0.7rem;
		padding: 2px 7px;
		font-family: var(--font-mono);
		background: none;
		border: 1px solid var(--base3);
		color: var(--base6);
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.pull-add:hover {
		color: var(--fg);
		border-color: var(--fg-muted);
	}
	.pull-add--strong {
		background: color-mix(in srgb, var(--state-online) 14%, transparent);
		border-color: color-mix(in srgb, var(--state-online) 40%, transparent);
		color: var(--state-online);
	}
</style>
