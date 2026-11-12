<script lang="ts">
	// Back up to relays: every event connected to the open document (the
	// publication tree + highlights, comments, annotations, narrations, …),
	// listed by the engine from the local store (`GET /backup/plan`). Tick
	// events — all / none act on what the filters show — pick relays, send.
	// The pick is the approval: the engine broadcasts in any network mode,
	// local included, to exactly these relays. Opened from the reader's
	// tools sheet via `app.backupModal`.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();
	const req = $derived(app.backupModal);

	let plan = $state<api.BackupPlan | null>(null);
	let loadErr = $state<string | null>(null);
	let picked = $state<Record<string, boolean>>({});
	let relayOn = $state<Record<string, boolean>>({});
	// Facets: a value maps to false when unchecked (absent = checked).
	let kindOff = $state<Record<string, boolean>>({});
	let authorOff = $state<Record<string, boolean>>({});
	let provOff = $state<Record<string, boolean>>({});
	let sending = $state(false);
	let result = $state<api.BackupResult | null>(null);
	let sendErr = $state<string | null>(null);

	const LOCAL = 'local only';

	$effect(() => {
		const coord = req?.coord;
		if (!coord) return;
		let live = true;
		api
			.getBackupPlan(coord)
			.then((p) => {
				if (!live) return;
				plan = p;
				const sel: Record<string, boolean> = {};
				for (const r of p.rows) sel[r.id] = !r.deleted;
				picked = sel;
				const on: Record<string, boolean> = {};
				for (const r of p.relays) on[r.url] = r.publish;
				relayOn = on;
			})
			.catch((e) => {
				if (live) loadErr = e instanceof Error ? e.message : String(e);
			});
		return () => {
			live = false;
		};
	});

	function authorKey(r: api.BackupRow): string {
		return r.pubkey;
	}
	function authorName(pk: string): string {
		const r = plan?.rows.find((x) => x.pubkey === pk);
		return r?.author || `${pk.slice(0, 10)}…`;
	}
	function kindText(kind: number, name: string | null): string {
		return name ? `${kind} · ${name}` : String(kind);
	}

	type Facet = { key: string; label: string; n: number };
	function tally(keys: (r: api.BackupRow) => string[], label: (k: string) => string): Facet[] {
		const m = new Map<string, number>();
		for (const r of plan?.rows ?? []) for (const k of keys(r)) m.set(k, (m.get(k) ?? 0) + 1);
		return [...m.entries()]
			.map(([key, n]) => ({ key, label: label(key), n }))
			.sort((a, b) => b.n - a.n || a.label.localeCompare(b.label));
	}
	const kinds = $derived(
		tally(
			(r) => [String(r.kind)],
			(k) => {
				const r = plan?.rows.find((x) => String(x.kind) === k);
				return kindText(Number(k), r?.kind_name ?? null);
			}
		)
	);
	const authors = $derived(tally((r) => [authorKey(r)], authorName));
	const provenance = $derived(
		tally(
			(r) => (r.relays.length ? r.relays : [LOCAL]),
			(k) => k.replace(/^wss:\/\//, '')
		)
	);

	function visibleRow(r: api.BackupRow): boolean {
		if (kindOff[String(r.kind)]) return false;
		if (authorOff[authorKey(r)]) return false;
		const prov = r.relays.length ? r.relays : [LOCAL];
		return prov.some((p) => !provOff[p]);
	}
	const rows = $derived((plan?.rows ?? []).filter(visibleRow));
	const sendIds = $derived(rows.filter((r) => picked[r.id]).map((r) => r.id));
	const hiddenPicked = $derived(
		(plan?.rows ?? []).filter((r) => picked[r.id] && !visibleRow(r)).length
	);
	const relays = $derived((plan?.relays ?? []).filter((r) => relayOn[r.url]).map((r) => r.url));

	function setAll(on: boolean) {
		const next = { ...picked };
		for (const r of rows) next[r.id] = on;
		picked = next;
	}
	function toggleFacet(
		map: Record<string, boolean>,
		key: string
	): Record<string, boolean> {
		return { ...map, [key]: !map[key] };
	}

	async function send() {
		if (sending || !sendIds.length || !relays.length) return;
		sending = true;
		sendErr = null;
		result = null;
		try {
			result = await api.backupBroadcast(sendIds, relays);
			app.pushToast(
				`Backed up ${result.landed} of ${result.events} event(s)`,
				result.landed === result.events ? 'success' : 'info',
				3500
			);
			// Provenance changed — re-read so the relay column is current.
			if (req) plan = await api.getBackupPlan(req.coord);
		} catch (e) {
			sendErr = e instanceof Error ? e.message : String(e);
		} finally {
			sending = false;
		}
	}

	function close() {
		if (sending) return;
		app.backupModal = null;
	}
	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') close();
	}
	function when(ts: number): string {
		return new Date(ts * 1000).toLocaleDateString();
	}
</script>

<svelte:window onkeydown={onKey} />

{#if req}
	<div class="bk-backdrop" onclick={close} role="presentation">
		<div
			class="bk-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			aria-label="Back up to relays"
			tabindex="-1"
		>
			<header class="bk-head">
				<span class="bk-title">back up to relays</span>
				<span class="bk-doc" title={req.coord}>{req.title ?? req.coord}</span>
				<button class="bk-x" onclick={close} aria-label="Close" disabled={sending}>×</button>
			</header>

			{#if loadErr}
				<div class="bk-err">{loadErr}</div>
			{:else if !plan}
				<div class="bk-hint">gathering connected events…</div>
			{:else}
				{#if plan.local_mode}
					<div class="bk-note">
						Local mode — nothing reaches a relay except what you send from here, to the relays ticked
						below.
					</div>
				{/if}

				<div class="bk-sec">
					<span class="cap">to</span>
					{#each plan.relays as r (r.url)}
						<label class="bk-chip" title={r.publish ? 'publish relay' : 'broadcast relay'}>
							<input
								type="checkbox"
								checked={!!relayOn[r.url]}
								onchange={() => (relayOn = toggleFacet(relayOn, r.url))}
							/>
							{r.url.replace(/^wss:\/\//, '')}
						</label>
					{:else}
						<span class="bk-hint">no publish or broadcast relays configured — add one in the relays buffer</span>
					{/each}
				</div>

				<div class="bk-bar">
					<span>{sendIds.length} of {rows.length} shown selected</span>
					<button onclick={() => setAll(true)} disabled={!rows.length}>all</button>
					<button onclick={() => setAll(false)} disabled={!rows.length}>none</button>
					{#if rows.length < plan.rows.length}
						<span class="bk-hint">{plan.rows.length - rows.length} hidden by filters</span>
					{/if}
				</div>

				<ul class="bk-list">
					{#each rows as r (r.id)}
						<li class:bk-dim={r.deleted}>
							<label class="bk-row">
								<input
									type="checkbox"
									checked={!!picked[r.id]}
									onchange={() => (picked = toggleFacet(picked, r.id))}
								/>
								<span class="bk-role bk-role--{r.role.replace(/\s/g, '-')}">{r.role}</span>
								<span class="bk-label" title={r.label}>{r.label || '(empty)'}</span>
								<span class="bk-by" title={r.pubkey}>{r.author ?? r.pubkey.slice(0, 8)}</span>
								<span class="bk-when">{when(r.created_at)}</span>
								<span
									class="bk-prov"
									class:bk-prov--local={!r.relays.length}
									title={r.relays.length ? r.relays.join('\n') : 'Only in your local store'}
									>{r.relays.length ? `${r.relays.length} relay${r.relays.length > 1 ? 's' : ''}` : 'local'}</span
								>
								{#if r.deleted}<span class="bk-del" title="You (its author) asked to delete it">deleted</span>{/if}
							</label>
						</li>
					{:else}
						<li class="bk-hint">nothing matches the filters</li>
					{/each}
				</ul>
				{#if plan.truncated}
					<div class="bk-hint">list capped at {plan.rows.length} events</div>
				{/if}

				<div class="bk-facets">
					{#each [{ cap: 'kind', items: kinds, map: 'kind' }, { cap: 'author', items: authors, map: 'author' }, { cap: 'provenance', items: provenance, map: 'prov' }] as g (g.cap)}
						<div class="bk-facet">
							<span class="cap">{g.cap}</span>
							{#each g.items as f (f.key)}
								{@const off = g.map === 'kind' ? kindOff[f.key] : g.map === 'author' ? authorOff[f.key] : provOff[f.key]}
								<label class="bk-chip" title={f.key}>
									<input
										type="checkbox"
										checked={!off}
										onchange={() => {
											if (g.map === 'kind') kindOff = toggleFacet(kindOff, f.key);
											else if (g.map === 'author') authorOff = toggleFacet(authorOff, f.key);
											else provOff = toggleFacet(provOff, f.key);
										}}
									/>
									{f.label} <span class="bk-n">{f.n}</span>
								</label>
							{/each}
						</div>
					{/each}
				</div>

				{#if result}
					<ul class="bk-result">
						{#each result.relays as r (r.relay)}
							<li class:bk-bad={r.rejected > 0}>
								{r.relay.replace(/^wss:\/\//, '')}: {r.accepted} accepted{#if r.rejected}, {r.rejected}
									rejected{#if r.message} — {r.message}{/if}{/if}
							</li>
						{/each}
					</ul>
				{/if}
				{#if sendErr}<div class="bk-err">{sendErr}</div>{/if}

				<footer class="bk-foot">
					{#if hiddenPicked}
						<span class="bk-hint">{hiddenPicked} selected but hidden — not sent</span>
					{/if}
					<span class="sp"></span>
					<button class="bk-go" onclick={send} disabled={sending || !sendIds.length || !relays.length}>
						{sending
							? 'sending…'
							: `broadcast ${sendIds.length} event${sendIds.length === 1 ? '' : 's'} → ${relays.length} relay${relays.length === 1 ? '' : 's'}`}
					</button>
				</footer>
			{/if}
		</div>
	</div>
{/if}

<style>
	.bk-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 6dvh;
	}
	.bk-modal {
		width: min(820px, 96vw);
		max-height: 88dvh;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		overflow: hidden;
	}
	.bk-head {
		display: flex;
		align-items: baseline;
		gap: 8px;
	}
	.bk-title {
		color: var(--blue);
		font-weight: 600;
	}
	.bk-doc {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--base7);
	}
	.bk-x {
		background: none;
		border: none;
		color: var(--base6);
		font-size: 1.2em;
		cursor: pointer;
	}
	.cap {
		color: var(--base5);
		text-transform: uppercase;
		font-size: calc(var(--t-xs) - 1px);
		letter-spacing: 0.05em;
		margin-right: 4px;
	}
	.bk-note {
		padding: 6px 8px;
		border-left: 2px solid var(--id-draft, var(--yellow));
		background: var(--bg-surface);
		color: var(--base7);
	}
	.bk-sec,
	.bk-bar,
	.bk-facet,
	.bk-foot {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
	}
	.bk-bar button,
	.bk-go {
		font: inherit;
		padding: 2px 10px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
		background: var(--bg-surface);
		color: var(--fg);
		cursor: pointer;
	}
	.bk-go {
		border-color: var(--blue);
		color: var(--blue);
	}
	.bk-go:disabled,
	.bk-bar button:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.bk-chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 1px 6px;
		border: 1px solid var(--panel-border);
		border-radius: 10px;
		cursor: pointer;
		white-space: nowrap;
	}
	.bk-n {
		color: var(--base5);
	}
	.bk-list {
		list-style: none;
		margin: 0;
		padding: 0;
		overflow-y: auto;
		flex: 1;
		min-height: 120px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm, 3px);
	}
	.bk-list li {
		border-bottom: 1px solid var(--panel-border);
	}
	.bk-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 3px 6px;
		cursor: pointer;
	}
	.bk-row:hover {
		background: var(--bg-surface);
	}
	.bk-dim {
		opacity: 0.55;
	}
	.bk-role {
		flex-shrink: 0;
		min-width: 9ch;
		color: var(--base6);
	}
	.bk-role--index,
	.bk-role--section {
		color: var(--blue);
	}
	.bk-role--highlight {
		color: var(--yellow);
	}
	.bk-role--comment,
	.bk-role--reply {
		color: var(--green);
	}
	.bk-role--deletion {
		color: var(--red);
	}
	.bk-label {
		flex: 1;
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.bk-by,
	.bk-when {
		color: var(--base5);
		white-space: nowrap;
		max-width: 14ch;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.bk-prov {
		color: var(--base6);
		white-space: nowrap;
	}
	.bk-prov--local {
		color: var(--id-draft, var(--yellow));
	}
	.bk-del {
		color: var(--red);
	}
	.bk-facets {
		display: flex;
		flex-direction: column;
		gap: 4px;
		max-height: 9rem;
		overflow-y: auto;
		flex-shrink: 0;
		border-top: 1px solid var(--panel-border);
		padding-top: 6px;
	}
	.bk-result {
		list-style: none;
		margin: 0;
		padding: 0;
		color: var(--green);
	}
	.bk-result .bk-bad {
		color: var(--yellow);
	}
	.bk-hint {
		color: var(--base5);
		font-style: italic;
	}
	.bk-err {
		color: var(--red);
	}
	.sp {
		flex: 1;
	}
	@media (max-width: 600px) {
		.bk-by,
		.bk-when {
			display: none;
		}
	}
</style>
