<script lang="ts">
	// Agent services — one modal for the paid model providers (ppq.ai /
	// routstr): signup, funding, and management. Driven by
	// `app.agentServicesModal` ({view, focus, pending}), mounted once in
	// +layout. Structure follows FetchConfirmModal; the modal-yield rule
	// applies twice here (routstr discovery is a Confirm-gated relay fetch;
	// a refused turn raised the funding intent that opened us): while an
	// intent is pending we hide, state alive, and come back after.
	//
	// Design + flows: docs/agent-service-flows.org.

	import { getAppState } from '$lib/state.svelte';
	import * as api from '$lib/api';
	import type { ProviderKind, ProviderModel, RoutstrNodeHealth, PpqInvoice } from '$lib/api';
	import { confirmState } from '$lib/network/fetch-events.svelte';
	import {
		SERVICE_LABEL,
		fmtBalance,
		fmtSats,
		fmtUsd,
		fundedForTurn,
		mintName,
		pillFor,
		pillLabel,
		shortNpub
	} from '$lib/services';
	import QRCode from 'qrcode';

	const app = getAppState();

	type Step =
		| 'home'
		| 'ppq-created'
		| 'ppq-fund'
		| 'ppq-adopt'
		| 'routstr-nodes'
		| 'routstr-token'
		| 'routstr-adopt'
		| 'routstr-topup'
		| 'routstr-withdraw'
		| 'models'
		| 'ledger';

	const modal = $derived(app.agentServicesModal);
	const svc = $derived(app.providersState);
	const yielding = $derived(!!confirmState.intent);

	// svelte-ignore state_referenced_locally
	let step = $state<Step>(app.agentServicesModal?.view === 'fund' ? 'ppq-fund' : 'home');
	let busy = $state<string | null>(null);
	let error = $state<string | null>(null);
	let revealed = $state<Partial<Record<ProviderKind, string>>>({});
	let armed = $state<string | null>(null);
	let armTimer: ReturnType<typeof setTimeout> | null = null;

	// If we opened on funding for routstr, land on its top-up step instead.
	$effect(() => {
		if (modal?.view === 'fund' && modal.focus === 'routstr' && step === 'ppq-fund') step = 'routstr-topup';
	});

	// Signup mode: nothing configured yet, or the "+" asked for the missing one.
	const signupMode = $derived(!svc?.configured || modal?.view === 'signup');
	const showPpqCard = $derived(!svc?.ppq);
	const showRoutstrCard = $derived(!svc?.routstr);

	const pendingReady = $derived.by(() => {
		if (!modal?.pending || !svc?.active) return false;
		return fundedForTurn(svc, svc.active);
	});

	function close() {
		if (busy || yielding) return;
		app.agentServicesModal = null;
	}
	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && !busy && !yielding) {
			e.stopPropagation();
			if (step !== 'home') step = 'home';
			else close();
		}
	}
	function fail(e: unknown) {
		error = e instanceof Error ? e.message : String(e);
	}
	async function run<T>(label: string, fn: () => Promise<T>): Promise<T | undefined> {
		busy = label;
		error = null;
		try {
			return await fn();
		} catch (e) {
			fail(e);
			return undefined;
		} finally {
			busy = null;
		}
	}
	async function copy(text: string, what: string) {
		try {
			await navigator.clipboard.writeText(text);
			app.pushToast(`${what} copied`, 'success');
		} catch {
			app.pushToast(`could not copy ${what}`, 'error');
		}
	}
	function arm(id: string, fire: () => void) {
		if (armed === id) {
			armed = null;
			if (armTimer) clearTimeout(armTimer);
			fire();
			return;
		}
		armed = id;
		if (armTimer) clearTimeout(armTimer);
		armTimer = setTimeout(() => (armed = null), 4000);
	}
	async function reveal(kind: ProviderKind) {
		const r = await run(`reveal-${kind}`, () => api.revealProviderKey(kind));
		if (r) revealed = { ...revealed, [kind]: r.api_key };
	}
	async function refresh(kind: ProviderKind) {
		await run(`refresh-${kind}`, () => api.refreshProviderBalance(kind));
		await app.refreshProviders();
	}
	async function makeActive(kind: ProviderKind) {
		await app.selectService(kind);
	}
	async function forget(kind: ProviderKind) {
		await run(`forget-${kind}`, () => api.forgetProvider(kind));
		await app.refreshProviders();
		app.pushToast(`${SERVICE_LABEL[kind]} forgotten`, 'info');
		if (!app.providersState?.configured) step = 'home';
	}

	// ---------------------------------------------------------------- ppq
	async function ppqCreate() {
		const r = await run('ppq-setup', () => api.ppqSetup());
		if (r) {
			await app.refreshProviders();
			step = 'ppq-created';
		}
	}
	let adoptKey = $state('');
	let adoptCredit = $state('');
	async function ppqAdopt() {
		const r = await run('ppq-adopt', () => api.ppqAdopt(adoptKey, adoptCredit));
		if (r) {
			await app.refreshProviders();
			adoptKey = '';
			adoptCredit = '';
			app.pushToast('ppq.ai key adopted', 'success');
			step = 'home';
		}
	}

	const AMOUNTS = [500, 2000, 10000];
	let fundAmount = $state(2000);
	let fundOther = $state('');
	let invoice = $state<PpqInvoice | null>(null);
	let invoiceSvg = $state('');
	let invoiceStatus = $state<'waiting' | 'paid' | 'expired'>('waiting');
	let secondsLeft = $state(0);
	let pollTimer: ReturnType<typeof setInterval> | null = null;
	let tickTimer: ReturnType<typeof setInterval> | null = null;

	function stopPolling() {
		if (pollTimer) clearInterval(pollTimer);
		if (tickTimer) clearInterval(tickTimer);
		pollTimer = tickTimer = null;
	}
	async function createInvoice() {
		const sats = fundOther.trim() ? Math.max(100, parseInt(fundOther, 10) || 0) : fundAmount;
		stopPolling();
		invoice = null;
		invoiceSvg = '';
		const inv = await run('ppq-fund', () => api.ppqFund(sats));
		if (!inv) return;
		invoice = inv;
		invoiceStatus = 'waiting';
		try {
			invoiceSvg = await QRCode.toString(inv.bolt11.toUpperCase(), {
				type: 'svg',
				margin: 1,
				errorCorrectionLevel: 'L'
			});
		} catch {
			invoiceSvg = '';
		}
		const tick = () => {
			secondsLeft = Math.max(0, inv.expires_at - Math.floor(Date.now() / 1000));
			if (secondsLeft === 0 && invoiceStatus === 'waiting') {
				invoiceStatus = 'expired';
				stopPolling();
			}
		};
		tick();
		tickTimer = setInterval(tick, 1000);
		pollTimer = setInterval(async () => {
			try {
				const st = await api.ppqFundStatus(inv.invoice_id);
				if (st.paid) {
					invoiceStatus = 'paid';
					stopPolling();
					await app.refreshProviders();
					app.pushToast(`ppq.ai funded — balance ${fmtBalance(st.balance)}`, 'success');
				} else if (st.expired) {
					invoiceStatus = 'expired';
					stopPolling();
				}
			} catch {
				/* transient; keep polling */
			}
		}, 3000);
	}
	$effect(() => () => stopPolling());
	const mmss = $derived(
		`${Math.floor(secondsLeft / 60)}:${String(secondsLeft % 60).padStart(2, '0')}`
	);

	// ------------------------------------------------------------ routstr
	let nodes = $state<RoutstrNodeHealth[]>([]);
	let discovered = $state<{ announced: number; reachable: number } | null>(null);
	let showAllNodes = $state(false);
	let manualUrl = $state('');
	let chosenNode = $state<RoutstrNodeHealth | null>(null);
	let token = $state('');
	const visibleNodes = $derived(showAllNodes ? nodes : nodes.filter((n) => n.reachable));

	async function discover() {
		const r = await run('discover', () => api.routstrDiscover());
		if (r) {
			nodes = r.nodes;
			discovered = { announced: r.announced, reachable: r.reachable };
		}
	}
	async function probeManual() {
		const url = manualUrl.trim();
		if (!url) return;
		const h = await run('probe', () => api.routstrProbe(url.startsWith('http') ? url : `https://${url}`));
		if (h) {
			if (!h.reachable) {
				error = `node not reachable: ${h.error ?? 'no models'}`;
				return;
			}
			pickNode(h);
		}
	}
	function pickNode(n: RoutstrNodeHealth) {
		chosenNode = n;
		error = null;
		step = 'routstr-token';
	}
	async function pasteToken() {
		try {
			token = (await navigator.clipboard.readText()).trim();
		} catch {
			app.pushToast('clipboard not available — paste into the box', 'info');
		}
	}
	/** Client-side peek at a cashu token: amount + mint, before it is sent. */
	const tokenPeek = $derived.by(() => {
		const t = token.trim();
		if (!t.startsWith('cashu')) return null;
		try {
			if (t.startsWith('cashuA')) {
				const json = JSON.parse(atob(t.slice(6).replace(/-/g, '+').replace(/_/g, '/')));
				const mint = json.token?.[0]?.mint as string | undefined;
				const sats = (json.token ?? []).flatMap((x: { proofs?: { amount: number }[] }) =>
					(x.proofs ?? []).map((p) => p.amount)
				);
				return { mint, sats: sats.reduce((a: number, b: number) => a + b, 0) as number, v: 'A' };
			}
			return { mint: undefined, sats: undefined, v: 'B' };
		} catch {
			return null;
		}
	});
	const tokenMintOk = $derived.by(() => {
		const mint = tokenPeek?.mint;
		if (!mint || !chosenNode) return null;
		return chosenNode.mints.some((m) => m.replace(/\/$/, '') === mint.replace(/\/$/, ''));
	});
	async function routstrCreate() {
		if (!chosenNode) return;
		const r = await run('routstr-create', () => api.routstrCreate(chosenNode!.url, token));
		if (r) {
			token = '';
			await app.refreshProviders();
			app.pushToast(`key created on ${chosenNode.name ?? chosenNode.url}`, 'success');
			modelsFor = 'routstr';
			await loadModels('routstr');
			step = 'models';
		}
	}
	let adoptNodeUrl = $state('');
	let adoptNodeKey = $state('');
	async function routstrAdopt() {
		const r = await run('routstr-adopt', () => api.routstrAdopt(adoptNodeUrl, adoptNodeKey));
		if (r) {
			adoptNodeUrl = '';
			adoptNodeKey = '';
			await app.refreshProviders();
			app.pushToast('routstr key adopted', 'success');
			step = 'home';
		}
	}
	async function routstrTopup() {
		const r = await run('routstr-topup', () => api.routstrTopup(token));
		if (r) {
			token = '';
			await app.refreshProviders();
			app.pushToast(`topped up — balance ${fmtBalance(r.balance)}`, 'success');
			step = 'home';
		}
	}
	let refundToken = $state<string | null>(null);
	async function routstrWithdraw() {
		const r = await run('routstr-refund', () => api.routstrRefund());
		if (r) {
			refundToken = r.token;
			await app.refreshProviders();
			if (!r.token) error = 'the node returned no token — check its balance history';
		}
	}

	// ------------------------------------------------------------- models
	let modelsFor = $state<ProviderKind>('ppq');
	let models = $state<ProviderModel[]>([]);
	let modelsNote = $state<string | undefined>(undefined);
	let modelFilter = $state('');
	const filteredModels = $derived(
		modelFilter.trim()
			? models.filter((m) => m.id.toLowerCase().includes(modelFilter.trim().toLowerCase()))
			: models
	);
	async function loadModels(kind: ProviderKind) {
		modelsFor = kind;
		models = [];
		modelsNote = undefined;
		const r = await run(`models-${kind}`, () => api.getProviderModels(kind));
		if (r) {
			models = r.models;
			modelsNote = r.note;
		}
	}
	async function openModels(kind: ProviderKind) {
		step = 'models';
		await loadModels(kind);
	}
	async function chooseModel(m: ProviderModel) {
		const kind = modelsFor;
		const r = await run(`select-${m.id}`, () => api.selectProvider(kind, m.id));
		if (!r) return;
		await app.refreshProviders();
		models = models.map((x) => ({ ...x, selected: x.id === m.id }));
		if (kind === 'routstr' && !m.tool_test) {
			app.pushToast(`testing tool calling on ${m.id}…`, 'pending');
			const t = await run(`test-${m.id}`, () => api.providerSelfTest(kind, m.id));
			if (t) {
				models = models.map((x) => (x.id === m.id ? { ...x, tool_test: t.ok ? 'ok' : 'failed' } : x));
				app.pushToast(
					t.ok ? `${m.id}: tool calling works` : `${m.id}: no tool call returned — ${t.detail}`,
					t.ok ? 'success' : 'error'
				);
				await app.refreshProviders();
			}
		} else {
			app.pushToast(`model: ${m.id}`, 'success');
		}
	}

	// ------------------------------------------------------------- ledger
	let ledgerFor = $state<ProviderKind>('ppq');
	let ledger = $state<Record<string, unknown>[]>([]);
	async function openLedger(kind: ProviderKind) {
		ledgerFor = kind;
		ledger = [];
		step = 'ledger';
		const r = await run(`ledger-${kind}`, () => api.getProviderLedger(kind));
		if (r) ledger = r.rows;
	}
	function fmtAt(v: unknown): string {
		if (typeof v === 'number') return new Date((v < 1e12 ? v * 1000 : v)).toLocaleString();
		if (typeof v === 'string') return new Date(v).toLocaleString();
		return '';
	}

	const title = $derived.by(() => {
		switch (step) {
			case 'ppq-created':
				return 'ppq.ai · account created';
			case 'ppq-fund':
				return 'fund ppq.ai';
			case 'ppq-adopt':
				return 'ppq.ai · existing key';
			case 'routstr-nodes':
				return 'routstr · pick a node';
			case 'routstr-token':
				return `routstr · ${chosenNode?.name ?? 'node'}`;
			case 'routstr-adopt':
				return 'routstr · existing key';
			case 'routstr-topup':
				return 'top up routstr';
			case 'routstr-withdraw':
				return 'withdraw from routstr';
			case 'models':
				return `${SERVICE_LABEL[modelsFor]} · model`;
			case 'ledger':
				return `${SERVICE_LABEL[ledgerFor]} · spend history`;
			default:
				return signupMode ? 'add a service' : 'agent services';
		}
	});
</script>

<svelte:window onkeydown={onKey} />

<div
	class="asm-backdrop"
	class:asm-backdrop--yield={yielding}
	role="presentation"
	onclick={(e) => {
		if (e.target === e.currentTarget) close();
	}}
>
	<div class="asm-modal" role="dialog" aria-modal="true" aria-labelledby="asm-title">
		<header class="asm-header">
			{#if step !== 'home'}
				<button class="asm-back" onclick={() => (step = 'home')} title="Back" aria-label="Back">‹</button>
			{/if}
			<h2 class="asm-title" id="asm-title">{title}</h2>
			<button class="asm-close" onclick={close} title="Close" aria-label="Close">×</button>
		</header>

		<div class="asm-body">
			{#if error}
				<div class="asm-error" role="alert">{error}</div>
			{/if}

			{#if modal?.pending}
				<div class="asm-pending">
					{#if pendingReady}
						Your message is waiting.
						<button class="asm-btn asm-btn--primary" onclick={() => app.resendPendingAgentMessage()}>
							Send it again
						</button>
					{:else if !svc?.configured}
						Your last message was not sent — the assistant has no service to run on
						yet. Set one up below, then send again.
					{:else}
						Your last message was not sent — the active service is below its per-turn
						ceiling. Fund it or switch, then send again.
					{/if}
				</div>
			{/if}

			<!-- ============================================== home -->
			{#if step === 'home'}
				{#if signupMode}
					<p class="asm-lead">
						Give the assistant a model to run on. Both options pay per use — no
						subscription. You can set up both and switch any time.
					</p>
					{#if showPpqCard}
						<section class="asm-card">
							<h3 class="asm-card-title">ppq.ai</h3>
							<p class="asm-card-text">
								One account, 370+ models, web search built in. Fund with Lightning. Balance in USD.
							</p>
							<div class="asm-actions">
								<button class="asm-btn asm-btn--primary" onclick={ppqCreate} disabled={!!busy}>
									{busy === 'ppq-setup' ? 'creating account…' : 'Create account'}
								</button>
								<button class="asm-btn asm-btn--ghost" onclick={() => (step = 'ppq-adopt')} disabled={!!busy}>
									I have a key
								</button>
							</div>
						</section>
					{/if}
					{#if showRoutstrCard}
						<section class="asm-card">
							<h3 class="asm-card-title">routstr</h3>
							<p class="asm-card-text">
								No account. Pick a node on the Nostr network, pay with Cashu ecash. Balance in
								sats, withdrawable any time.
							</p>
							<div class="asm-actions">
								<button
									class="asm-btn asm-btn--primary"
									onclick={() => {
										step = 'routstr-nodes';
										if (!nodes.length) void discover();
									}}
									disabled={!!busy}>Find nodes</button
								>
								<button class="asm-btn asm-btn--ghost" onclick={() => (step = 'routstr-adopt')} disabled={!!busy}>
									I have a key
								</button>
							</div>
						</section>
					{/if}
					{#if svc?.configured}
						<button class="asm-link" onclick={() => (app.agentServicesModal = { view: 'manage' })}>
							‹ back to your services
						</button>
					{/if}
				{:else if svc}
					<div class="asm-running">
						{#if svc.active}
							{@const active = svc.active}
							<div>
								<span class="asm-dim">running on</span>
								{SERVICE_LABEL[active]} ·
								{(active === 'ppq' ? svc.ppq?.model : svc.routstr?.model) ?? 'no model chosen'}
							</div>
						{/if}
						{#if svc.last_turn}
							<div class="asm-dim">
								last turn
								{svc.last_turn.cost != null
									? svc.last_turn.unit === 'usd'
										? fmtUsd(svc.last_turn.cost)
										: fmtSats(svc.last_turn.cost)
									: '—'}
								· {svc.last_turn.tool_calls} tool call{svc.last_turn.tool_calls === 1 ? '' : 's'}
								· {Math.round(svc.last_turn.seconds)} s
							</div>
						{/if}
					</div>

					{#if svc.ppq}
						{@const p = svc.ppq}
						<section class="asm-card" class:asm-card--active={svc.active === 'ppq'} id="asm-ppq">
							<div class="asm-card-head">
								<button
									class="asm-radio"
									class:asm-radio--on={svc.active === 'ppq'}
									onclick={() => makeActive('ppq')}
									disabled={svc.active === 'ppq' || !!busy}
									title="Run the assistant on ppq.ai">{svc.active === 'ppq' ? '●' : '○'}</button
								>
								<h3 class="asm-card-title">ppq.ai</h3>
								<span class="pill {pillFor(p.status)}">{pillLabel(p.status, p.balance)}</span>
								<button
									class="asm-icon"
									onclick={() => refresh('ppq')}
									disabled={!!busy}
									title="Refresh balance"
									aria-label="Refresh ppq.ai balance">{busy === 'refresh-ppq' ? '…' : '⟳'}</button
								>
							</div>
							<dl class="asm-kv">
								<dt>model</dt>
								<dd>
									<button class="asm-pick" onclick={() => openModels('ppq')} disabled={!!busy}>
										{p.model ?? 'choose…'} ▾
									</button>
								</dd>
								<dt>web search</dt>
								<dd>built in</dd>
								<dt>credit id</dt>
								<dd class="asm-mono">
									{p.credit_id}
									<button class="asm-icon" onclick={() => copy(p.credit_id, 'credit id')} title="Copy">⧉</button>
								</dd>
								<dt>api key</dt>
								<dd class="asm-mono">
									{revealed.ppq ?? p.api_key}
									{#if revealed.ppq}
										<button class="asm-icon" onclick={() => copy(revealed.ppq!, 'api key')} title="Copy">⧉</button>
									{:else}
										<button class="asm-icon" onclick={() => reveal('ppq')} title="Reveal (one time)">👁</button>
									{/if}
								</dd>
								{#if p.balance?.total_spent != null}
									<dt>spent</dt>
									<dd>
										{fmtUsd(p.balance.total_spent)} all time
										{#if p.balance.reserved != null}· {fmtUsd(p.balance.reserved)} this period{/if}
									</dd>
								{/if}
								{#if p.balance?.error}
									<dt>last check</dt>
									<dd class="asm-warn">{p.balance.error}</dd>
								{/if}
							</dl>
							<div class="asm-actions">
								<button class="asm-btn asm-btn--primary" onclick={() => (step = 'ppq-fund')} disabled={!!busy}>Fund</button>
								<button class="asm-btn asm-btn--ghost" onclick={() => openLedger('ppq')} disabled={!!busy}>Spend history</button>
								<button
									class="asm-btn asm-btn--danger"
									class:asm-btn--armed={armed === 'forget-ppq'}
									onclick={() => arm('forget-ppq', () => forget('ppq'))}
									disabled={!!busy}
									title="Only the credit id can get this account back"
								>
									{armed === 'forget-ppq' ? 'Tap again to forget' : 'Forget this account'}
								</button>
							</div>
						</section>
					{/if}

					{#if svc.routstr}
						{@const r = svc.routstr}
						<section class="asm-card" class:asm-card--active={svc.active === 'routstr'} id="asm-routstr">
							<div class="asm-card-head">
								<button
									class="asm-radio"
									class:asm-radio--on={svc.active === 'routstr'}
									onclick={() => makeActive('routstr')}
									disabled={svc.active === 'routstr' || !!busy}
									title="Run the assistant on routstr">{svc.active === 'routstr' ? '●' : '○'}</button
								>
								<h3 class="asm-card-title">routstr</h3>
								<span class="pill {pillFor(r.status)}">{pillLabel(r.status, r.balance)}</span>
								<button
									class="asm-icon"
									onclick={() => refresh('routstr')}
									disabled={!!busy}
									title="Refresh balance"
									aria-label="Refresh routstr balance">{busy === 'refresh-routstr' ? '…' : '⟳'}</button
								>
							</div>
							<dl class="asm-kv">
								<dt>node</dt>
								<dd>
									<button
										class="asm-pick"
										onclick={() => {
											step = 'routstr-nodes';
											if (!nodes.length) void discover();
										}}
										disabled={!!busy}
										title={r.node.url}
									>
										{r.node.name ?? r.node.url}
										{#if r.node.version}<span class="asm-dim">v{r.node.version}</span>{/if} ▾
									</button>
								</dd>
								<dt>model</dt>
								<dd>
									<button class="asm-pick" onclick={() => openModels('routstr')} disabled={!!busy}>
										{r.model ?? 'choose…'} ▾
									</button>
									{#if r.model && r.tool_tests[r.model] === 'failed'}
										<span class="pill pill--draft">no tool calls</span>
									{:else if r.model && r.tool_tests[r.model] === 'ok'}
										<span class="pill pill--online">tools ok</span>
									{/if}
								</dd>
								<dt>mints</dt>
								<dd>{r.node.mints.map(mintName).join(', ') || '—'}</dd>
								{#if r.node.npub}
									<dt>operator</dt>
									<dd class="asm-mono">{shortNpub(r.node.npub)}</dd>
								{/if}
								<dt>key</dt>
								<dd class="asm-mono">
									{revealed.routstr ?? r.api_key}
									{#if revealed.routstr}
										<button class="asm-icon" onclick={() => copy(revealed.routstr!, 'key')} title="Copy">⧉</button>
									{:else}
										<button class="asm-icon" onclick={() => reveal('routstr')} title="Reveal (one time)">👁</button>
									{/if}
								</dd>
								{#if r.balance}
									<dt>reserved</dt>
									<dd>
										{fmtSats((r.balance.reserved ?? 0) / 1000)}
										{#if r.balance.total_spent != null}· spent {fmtSats(r.balance.total_spent / 1000)}{/if}
									</dd>
								{/if}
								{#if r.balance?.error}
									<dt>last check</dt>
									<dd class="asm-warn">{r.balance.error}</dd>
								{/if}
							</dl>
							<div class="asm-actions">
								<button
									class="asm-btn asm-btn--primary"
									onclick={() => {
										token = '';
										step = 'routstr-topup';
									}}
									disabled={!!busy}>Top up</button
								>
								<button
									class="asm-btn asm-btn--ghost"
									onclick={() => {
										refundToken = null;
										step = 'routstr-withdraw';
									}}
									disabled={!!busy || !r.balance || r.balance.amount <= 0}>Withdraw</button
								>
								<button class="asm-btn asm-btn--ghost" onclick={() => openLedger('routstr')} disabled={!!busy}>History</button>
								<button
									class="asm-btn asm-btn--danger"
									class:asm-btn--armed={armed === 'forget-routstr'}
									onclick={() => arm('forget-routstr', () => forget('routstr'))}
									disabled={!!busy || (!!r.balance && r.balance.amount > 0)}
									title={r.balance && r.balance.amount > 0
										? 'Withdraw the balance first'
										: 'Remove this key from tendrl'}
								>
									{armed === 'forget-routstr' ? 'Tap again to forget' : 'Forget this key'}
								</button>
							</div>
						</section>
					{/if}

					<div class="asm-ceiling">
						<span class="asm-dim">ceiling per turn</span>
						{#if svc.ppq}
							<label>
								ppq.ai $
								<input
									class="asm-input asm-input--num"
									type="number"
									min="0"
									step="0.05"
									value={svc.ceiling_usd}
									onchange={async (e) => {
										const v = parseFloat(e.currentTarget.value);
										if (!Number.isNaN(v)) {
											await run('ceiling', () => api.setProviderCeiling({ ceiling_usd: v }));
											await app.refreshProviders();
										}
									}}
								/>
							</label>
						{/if}
						{#if svc.routstr}
							<label>
								routstr
								<input
									class="asm-input asm-input--num"
									type="number"
									min="0"
									step="10"
									value={svc.ceiling_sats}
									onchange={async (e) => {
										const v = parseFloat(e.currentTarget.value);
										if (!Number.isNaN(v)) {
											await run('ceiling', () => api.setProviderCeiling({ ceiling_sats: v }));
											await app.refreshProviders();
										}
									}}
								/> sat
							</label>
						{/if}
						<span class="asm-dim">— a turn that starts below it asks first</span>
					</div>

					{#if !(svc.ppq && svc.routstr)}
						<button class="asm-link" onclick={() => (app.agentServicesModal = { view: 'signup' })}>
							+ add a service
						</button>
					{/if}
				{:else}
					<p class="asm-dim">Loading…</p>
				{/if}

				<!-- ======================================= ppq: created -->
			{:else if step === 'ppq-created'}
				{#if svc?.ppq}
					<section class="asm-card asm-card--active">
						<dl class="asm-kv">
							<dt>credit id</dt>
							<dd class="asm-mono">
								{svc.ppq.credit_id}
								<button class="asm-icon" onclick={() => copy(svc!.ppq!.credit_id, 'credit id')} title="Copy">⧉</button>
							</dd>
							<dt>api key</dt>
							<dd class="asm-mono">
								{revealed.ppq ?? svc.ppq.api_key}
								{#if revealed.ppq}
									<button class="asm-icon" onclick={() => copy(revealed.ppq!, 'api key')} title="Copy">⧉</button>
								{:else}
									<button class="asm-icon" onclick={() => reveal('ppq')} title="Reveal">👁</button>
								{/if}
							</dd>
							<dt>balance</dt>
							<dd>{fmtBalance(svc.ppq.balance)}</dd>
						</dl>
						<div class="asm-notice">
							⚠ Save the credit id somewhere safe. It is the only way to recover this account
							if the app data is lost.
						</div>
						<div class="asm-actions">
							<button class="asm-btn asm-btn--primary" onclick={() => (step = 'ppq-fund')}>Fund now</button>
							<button class="asm-btn asm-btn--ghost" onclick={() => (step = 'home')}>Later</button>
						</div>
					</section>
				{/if}

				<!-- ========================================= ppq: fund -->
			{:else if step === 'ppq-fund'}
				{#if !svc?.ppq}
					<p class="asm-dim">No ppq.ai account yet.</p>
					<button class="asm-btn asm-btn--primary" onclick={() => (step = 'home')}>Set one up</button>
				{:else}
					<p class="asm-lead">
						Balance {fmtBalance(svc.ppq.balance)}. Pay a Lightning invoice; the balance updates
						as soon as it settles. Minimum 100 sat.
					</p>
					<div class="asm-chips">
						{#each AMOUNTS as a (a)}
							<button
								class="asm-chip"
								class:asm-chip--on={fundAmount === a && !fundOther.trim()}
								onclick={() => {
									fundAmount = a;
									fundOther = '';
								}}>{a.toLocaleString()} sat</button
							>
						{/each}
						<input
							class="asm-input asm-input--num"
							type="number"
							min="100"
							step="100"
							placeholder="other"
							bind:value={fundOther}
						/>
					</div>
					<div class="asm-actions">
						<button class="asm-btn asm-btn--primary" onclick={createInvoice} disabled={!!busy}>
							{busy === 'ppq-fund' ? 'creating invoice…' : invoice ? 'New invoice' : 'Create invoice'}
						</button>
					</div>
					{#if invoice}
						<section class="asm-card asm-invoice">
							{#if invoiceSvg}
								<div class="asm-qr">{@html invoiceSvg}</div>
							{/if}
							<div class="asm-mono asm-bolt">
								{invoice.bolt11.slice(0, 28)}…
								<button class="asm-icon" onclick={() => copy(invoice!.bolt11, 'invoice')} title="Copy invoice">⧉</button>
							</div>
							<div class="asm-actions">
								<a class="asm-btn asm-btn--ghost" href={`lightning:${invoice.bolt11}`}>Open in wallet</a>
							</div>
							<div class="asm-status">
								{#if invoiceStatus === 'waiting'}
									<span class="dot dot--fetching"></span> waiting for payment · expires {mmss}
								{:else if invoiceStatus === 'paid'}
									<span class="dot dot--online"></span> paid · balance {fmtBalance(svc.ppq.balance)}
								{:else}
									<span class="dot dot--offline"></span> expired — create a new invoice
								{/if}
							</div>
						</section>
					{/if}
				{/if}

				<!-- ======================================== ppq: adopt -->
			{:else if step === 'ppq-adopt'}
				<p class="asm-lead">Paste a ppq.ai key you already have, with its credit id.</p>
				<label class="asm-field">
					api key
					<input class="asm-input" type="password" placeholder="sk-…" bind:value={adoptKey} autocomplete="off" />
				</label>
				<label class="asm-field">
					credit id
					<input class="asm-input" type="text" placeholder="xxxxxxxx-xxxx-…" bind:value={adoptCredit} autocomplete="off" />
				</label>
				<div class="asm-actions">
					<button class="asm-btn asm-btn--primary" onclick={ppqAdopt} disabled={!!busy || !adoptKey || !adoptCredit}>
						{busy === 'ppq-adopt' ? 'checking…' : 'Use this key'}
					</button>
				</div>

				<!-- ==================================== routstr: nodes -->
			{:else if step === 'routstr-nodes'}
				<div class="asm-status">
					{#if busy === 'discover'}
						<span class="dot dot--fetching"></span> finding nodes on the Nostr network…
					{:else if discovered}
						found {discovered.announced} nodes, {discovered.reachable} answering
					{:else}
						<span class="asm-dim">no discovery run yet</span>
					{/if}
					<button class="asm-icon" onclick={discover} disabled={!!busy} title="Discover again" aria-label="Discover again">⟳</button>
					<label class="asm-toggle">
						<input type="checkbox" bind:checked={showAllNodes} /> show all
					</label>
				</div>
				{#if svc?.routstr}
					<p class="asm-dim">
						Switching node creates a new key there. Your balance stays on {svc.routstr.node.name ?? svc.routstr.node.url} — withdraw it first if you want it back.
					</p>
				{/if}
				{#each visibleNodes as n (n.url)}
					<button
						class="asm-node"
						class:asm-node--current={svc?.routstr?.node.url === n.url}
						class:asm-node--dead={!n.reachable}
						onclick={() => pickNode(n)}
						disabled={!n.reachable || !!busy}
					>
						<div class="asm-node-head">
							<span class="asm-node-name">{n.name ?? n.url.replace(/^https:\/\//, '')}</span>
							<span class="asm-dim">{n.reachable ? `${(n.latency_ms / 1000).toFixed(1)} s` : (n.error ?? 'unreachable')}</span>
						</div>
						<div class="asm-node-meta">
							{#if n.reachable}
								{n.model_count} models{#if n.likely_tools} · tool-capable families{/if}{#if n.version} · v{n.version}{/if}
							{/if}
						</div>
						{#if n.mints.length}
							<div class="asm-node-meta">mints: {n.mints.map(mintName).join(', ')}</div>
						{/if}
						<div class="asm-node-meta asm-mono">{n.url.replace(/^https:\/\//, '')}{#if n.npub} · {shortNpub(n.npub)}{/if}</div>
					</button>
				{:else}
					{#if !busy}
						<p class="asm-dim">No answering nodes in the last discovery.</p>
					{/if}
				{/each}
				<div class="asm-field asm-field--row">
					<input class="asm-input" type="text" placeholder="or a node URL by hand" bind:value={manualUrl} />
					<button class="asm-btn asm-btn--ghost" onclick={probeManual} disabled={!!busy || !manualUrl.trim()}>
						{busy === 'probe' ? 'checking…' : 'Check'}
					</button>
				</div>

				<!-- ==================================== routstr: token -->
			{:else if step === 'routstr-token'}
				{#if chosenNode}
					<p class="asm-lead">
						This node accepts ecash from:
					</p>
					<ul class="asm-mints">
						{#each chosenNode.mints as m (m)}<li class="asm-mono">{m}</li>{/each}
					</ul>
					<p class="asm-dim">
						Send a token from a wallet that uses one of these mints (Minibits, Nutstash, cashu.me).
						Cashu is the only way in — routstr nodes take no Lightning directly.
					</p>
					<textarea class="asm-input asm-token" rows="4" placeholder="cashuB…" bind:value={token} spellcheck="false"></textarea>
					<div class="asm-actions">
						<button class="asm-btn asm-btn--ghost" onclick={pasteToken}>Paste from clipboard</button>
					</div>
					{#if tokenPeek}
						<div class="asm-status">
							{#if tokenPeek.sats != null}amount in token {fmtSats(tokenPeek.sats)}{/if}
							{#if tokenMintOk === true}· <span class="asm-ok">mint ok</span>{/if}
							{#if tokenMintOk === false}· <span class="asm-warn">mint not accepted by this node</span>{/if}
							{#if tokenPeek.v === 'B'}<span class="asm-dim">(v4 token — the node will check the mint)</span>{/if}
						</div>
					{:else if token.trim()}
						<div class="asm-status asm-warn">that does not look like a Cashu token</div>
					{/if}
					<div class="asm-actions">
						<button
							class="asm-btn asm-btn--primary"
							onclick={routstrCreate}
							disabled={!!busy || !token.trim().startsWith('cashu') || tokenMintOk === false}
						>
							{busy === 'routstr-create' ? 'creating key…' : 'Create key with this token'}
						</button>
					</div>
				{/if}

				<!-- ==================================== routstr: adopt -->
			{:else if step === 'routstr-adopt'}
				<p class="asm-lead">Paste a routstr key you already have and the node it lives on.</p>
				<label class="asm-field">
					node URL
					<input class="asm-input" type="text" placeholder="https://…" bind:value={adoptNodeUrl} autocomplete="off" />
				</label>
				<label class="asm-field">
					key
					<input class="asm-input" type="password" placeholder="sk-…" bind:value={adoptNodeKey} autocomplete="off" />
				</label>
				<div class="asm-actions">
					<button class="asm-btn asm-btn--primary" onclick={routstrAdopt} disabled={!!busy || !adoptNodeUrl || !adoptNodeKey}>
						{busy === 'routstr-adopt' ? 'checking…' : 'Use this key'}
					</button>
				</div>

				<!-- ==================================== routstr: topup -->
			{:else if step === 'routstr-topup'}
				{#if !svc?.routstr}
					<p class="asm-dim">No routstr key yet.</p>
					<button class="asm-btn asm-btn--primary" onclick={() => (step = 'home')}>Set one up</button>
				{:else}
					<p class="asm-lead">
						Balance {fmtBalance(svc.routstr.balance)} on {svc.routstr.node.name ?? svc.routstr.node.url}.
						Paste a Cashu token from one of its mints:
					</p>
					<ul class="asm-mints">
						{#each svc.routstr.node.mints as m (m)}<li class="asm-mono">{m}</li>{/each}
					</ul>
					<textarea class="asm-input asm-token" rows="4" placeholder="cashuB…" bind:value={token} spellcheck="false"></textarea>
					<div class="asm-actions">
						<button class="asm-btn asm-btn--ghost" onclick={pasteToken}>Paste from clipboard</button>
						<button class="asm-btn asm-btn--primary" onclick={routstrTopup} disabled={!!busy || !token.trim().startsWith('cashu')}>
							{busy === 'routstr-topup' ? 'topping up…' : 'Top up'}
						</button>
					</div>
				{/if}

				<!-- ================================= routstr: withdraw -->
			{:else if step === 'routstr-withdraw'}
				{#if refundToken}
					<div class="asm-notice">
						Paste this into your wallet now — it is not stored anywhere.
					</div>
					<textarea class="asm-input asm-token" rows="5" readonly value={refundToken}></textarea>
					<div class="asm-actions">
						<button class="asm-btn asm-btn--primary" onclick={() => copy(refundToken!, 'token')}>Copy token</button>
						<button class="asm-btn asm-btn--ghost" onclick={() => (step = 'home')}>Done</button>
					</div>
				{:else if svc?.routstr}
					<p class="asm-lead">
						Withdraw {fmtBalance(svc.routstr.balance)} from {svc.routstr.node.name ?? svc.routstr.node.url}
						back into a Cashu token. The key stays; you can top it up again later.
					</p>
					<div class="asm-actions">
						<button
							class="asm-btn asm-btn--danger"
							class:asm-btn--armed={armed === 'withdraw'}
							onclick={() => arm('withdraw', () => routstrWithdraw())}
							disabled={!!busy}
						>
							{busy === 'routstr-refund' ? 'withdrawing…' : armed === 'withdraw' ? 'Tap again to withdraw' : 'Withdraw everything'}
						</button>
					</div>
				{/if}

				<!-- ============================================ models -->
			{:else if step === 'models'}
				{#if modelsNote}
					<div class="asm-notice">ⓘ {modelsNote}</div>
				{/if}
				<input class="asm-input" type="search" placeholder="filter models" bind:value={modelFilter} />
				{#if busy?.startsWith('models-')}
					<p class="asm-dim">loading…</p>
				{/if}
				<div class="asm-models">
					{#each filteredModels as m (m.id)}
						<button
							class="asm-model"
							class:asm-model--on={m.selected}
							onclick={() => chooseModel(m)}
							disabled={!!busy}
							title={m.name ?? m.id}
						>
							<span class="asm-model-dot">{m.selected ? '●' : '○'}</span>
							<span class="asm-model-id">{m.id}</span>
							<span class="asm-model-price">
								{#if modelsFor === 'ppq'}
									{m.input_per_1m != null ? `$${m.input_per_1m}/M` : ''}
								{:else if m.sat_prompt_per_1k != null}
									{m.sat_prompt_per_1k < 0.01 ? m.sat_prompt_per_1k.toFixed(4) : m.sat_prompt_per_1k.toFixed(2)} sat/1k
								{/if}
							</span>
							<span class="asm-model-mark">
								{#if modelsFor === 'routstr'}
									{#if busy === `test-${m.id}`}testing…
									{:else if m.tool_test === 'ok'}<span class="asm-ok">✓ tools ok</span>
									{:else if m.tool_test === 'failed'}<span class="asm-warn">✗ no tool calls</span>
									{:else if m.likely_tools}<span class="asm-dim">· likely</span>
									{:else}<span class="asm-dim">· untested</span>{/if}
								{/if}
							</span>
						</button>
					{:else}
						{#if !busy}<p class="asm-dim">no models match</p>{/if}
					{/each}
				</div>

				<!-- ============================================ ledger -->
			{:else if step === 'ledger'}
				{#if busy?.startsWith('ledger-')}
					<p class="asm-dim">loading…</p>
				{:else if !ledger.length}
					<p class="asm-dim">nothing spent yet</p>
				{:else}
					<table class="asm-table">
						<tbody>
							{#each ledger as row, i (i)}
								<tr>
									<td class="asm-dim">{fmtAt(row.at)}</td>
									<td class="asm-mono">{row.model ?? row.type ?? ''}</td>
									<td class="asm-num">
										{#if ledgerFor === 'ppq'}
											{typeof row.cost === 'number' ? fmtUsd(row.cost) : ''}
										{:else}
											{typeof row.amount === 'number' ? `${row.amount} ${row.unit ?? ''}` : ''}
										{/if}
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				{/if}
			{/if}
		</div>
	</div>
</div>

<style>
	.asm-backdrop {
		position: fixed;
		inset: 0;
		background: var(--scrim);
		z-index: 240;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.asm-backdrop--yield {
		visibility: hidden;
	}
	.asm-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 92vw;
		max-width: 560px;
		max-height: 86dvh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		overflow: hidden;
	}
	@media (max-width: 768px) {
		.asm-backdrop {
			align-items: stretch;
		}
		.asm-modal {
			width: 100vw;
			max-width: none;
			max-height: none;
			height: 100dvh;
			border: none;
			border-radius: 0;
		}
	}
	.asm-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.asm-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-sm);
		color: var(--base7);
		font-weight: 500;
	}
	.asm-back,
	.asm-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 8px;
		min-width: 40px;
		min-height: 40px;
	}
	.asm-back:hover,
	.asm-close:hover {
		color: var(--fg);
	}
	.asm-body {
		padding: 12px 14px 20px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.asm-lead {
		margin: 0;
		color: var(--fg);
		line-height: 1.5;
	}
	.asm-dim {
		color: var(--fg-alt);
	}
	.asm-ok {
		color: var(--state-online);
	}
	.asm-warn {
		color: var(--warning);
	}
	.asm-mono {
		font-family: var(--font-mono);
		word-break: break-all;
	}
	.asm-error {
		border: 1px solid color-mix(in srgb, var(--state-error) 50%, transparent);
		background: color-mix(in srgb, var(--state-error) 10%, transparent);
		color: var(--fg);
		border-radius: var(--r-sm);
		padding: 8px 10px;
	}
	.asm-notice {
		border: 1px solid var(--warn-border);
		background: var(--warn-bg);
		color: var(--warn-fg);
		border-radius: var(--r-sm);
		padding: 8px 10px;
		line-height: 1.45;
	}
	.asm-pending {
		border: 1px solid color-mix(in srgb, var(--id-yours) 40%, transparent);
		background: color-mix(in srgb, var(--id-yours) 10%, transparent);
		border-radius: var(--r-sm);
		padding: 8px 10px;
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
	}
	.asm-running {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.asm-card {
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: 10px 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.asm-card--active {
		border-color: color-mix(in srgb, var(--state-online) 50%, var(--panel-border));
	}
	.asm-card-head {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.asm-card-title {
		margin: 0;
		flex: 1;
		font-size: var(--t-sm);
		font-weight: 500;
		color: var(--base7);
	}
	.asm-card-text {
		margin: 0;
		color: var(--fg-alt);
		line-height: 1.5;
	}
	.asm-radio {
		background: none;
		border: none;
		color: var(--fg-alt);
		font-size: var(--t-sm);
		cursor: pointer;
		padding: 0 4px;
		min-width: 28px;
		min-height: 28px;
	}
	.asm-radio--on {
		color: var(--state-online);
		cursor: default;
	}
	.asm-icon {
		background: none;
		border: none;
		color: var(--fg-alt);
		cursor: pointer;
		padding: 2px 6px;
		font: inherit;
		min-height: 28px;
	}
	.asm-icon:hover {
		color: var(--fg);
	}
	.asm-kv {
		display: grid;
		grid-template-columns: max-content 1fr;
		gap: 4px 12px;
		margin: 0;
	}
	.asm-kv dt {
		color: var(--fg-alt);
	}
	.asm-kv dd {
		margin: 0;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		min-width: 0;
	}
	.asm-pick {
		background: none;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg);
		font: inherit;
		padding: 2px 8px;
		cursor: pointer;
		text-align: left;
	}
	.asm-pick:hover {
		border-color: var(--id-yours);
	}
	.asm-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.asm-btn {
		font: inherit;
		border-radius: var(--r-sm);
		padding: 6px 12px;
		min-height: 36px;
		cursor: pointer;
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		text-decoration: none;
		display: inline-flex;
		align-items: center;
	}
	.asm-btn:disabled {
		opacity: 0.5;
		cursor: default;
	}
	.asm-btn--primary {
		background: color-mix(in srgb, var(--id-yours) 16%, transparent);
		border-color: color-mix(in srgb, var(--id-yours) 40%, transparent);
		color: var(--id-yours);
	}
	.asm-btn--primary:not(:disabled):hover {
		background: color-mix(in srgb, var(--id-yours) 28%, transparent);
	}
	.asm-btn--ghost:not(:disabled):hover {
		border-color: var(--id-yours);
	}
	.asm-btn--danger {
		color: var(--fg-alt);
	}
	.asm-btn--armed {
		background: color-mix(in srgb, var(--state-error) 16%, transparent);
		border-color: var(--state-error);
		color: var(--state-error);
	}
	.asm-link {
		background: none;
		border: none;
		color: var(--id-yours);
		font: inherit;
		cursor: pointer;
		padding: 6px 0;
		text-align: left;
	}
	.asm-ceiling {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
		padding-top: 4px;
	}
	.asm-ceiling label {
		display: inline-flex;
		align-items: center;
		gap: 4px;
	}
	.asm-input {
		font: inherit;
		background: var(--panel-bg, var(--bg));
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg);
		padding: 6px 8px;
		width: 100%;
		box-sizing: border-box;
	}
	.asm-input--num {
		width: 6.5em;
	}
	.asm-token {
		resize: vertical;
		word-break: break-all;
	}
	.asm-field {
		display: flex;
		flex-direction: column;
		gap: 4px;
		color: var(--fg-alt);
	}
	.asm-field--row {
		flex-direction: row;
		align-items: center;
	}
	.asm-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
	}
	.asm-chip {
		font: inherit;
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		border-radius: var(--r-md);
		padding: 4px 10px;
		min-height: 32px;
		cursor: pointer;
	}
	.asm-chip--on {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.asm-invoice {
		align-items: center;
		text-align: center;
	}
	.asm-qr {
		width: min(260px, 70vw);
		background: #fff;
		padding: 8px;
		border-radius: var(--r-sm);
	}
	.asm-qr :global(svg) {
		display: block;
		width: 100%;
		height: auto;
	}
	.asm-bolt {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.asm-status {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		color: var(--fg);
	}
	.asm-toggle {
		margin-left: auto;
		display: inline-flex;
		gap: 4px;
		align-items: center;
		color: var(--fg-alt);
	}
	.asm-mints {
		margin: 0;
		padding-left: 18px;
	}
	.asm-node {
		font: inherit;
		text-align: left;
		width: 100%;
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		border-radius: var(--r-md);
		padding: 8px 10px;
		display: flex;
		flex-direction: column;
		gap: 2px;
		cursor: pointer;
	}
	.asm-node:not(:disabled):hover {
		border-color: var(--id-yours);
	}
	.asm-node--current {
		border-color: color-mix(in srgb, var(--state-online) 50%, var(--panel-border));
	}
	.asm-node--dead {
		opacity: 0.55;
		cursor: default;
	}
	.asm-node-head {
		display: flex;
		justify-content: space-between;
		gap: 8px;
	}
	.asm-node-name {
		color: var(--base7);
	}
	.asm-node-meta {
		color: var(--fg-alt);
		font-size: var(--t-2xs);
	}
	.asm-models {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.asm-model {
		font: inherit;
		text-align: left;
		border: none;
		background: transparent;
		color: var(--fg);
		display: grid;
		grid-template-columns: 1.2em 1fr auto auto;
		gap: 8px;
		align-items: center;
		padding: 6px 4px;
		min-height: 36px;
		cursor: pointer;
		border-radius: var(--r-sm);
	}
	.asm-model:not(:disabled):hover {
		background: color-mix(in srgb, var(--id-yours) 8%, transparent);
	}
	.asm-model--on .asm-model-dot {
		color: var(--state-online);
	}
	.asm-model-id {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.asm-model-price,
	.asm-model-mark {
		color: var(--fg-alt);
		font-size: var(--t-2xs);
		white-space: nowrap;
	}
	.asm-table {
		width: 100%;
		border-collapse: collapse;
	}
	.asm-table td {
		padding: 4px 6px;
		border-bottom: 1px solid var(--panel-border);
		vertical-align: top;
	}
	.asm-num {
		text-align: right;
		white-space: nowrap;
	}
</style>
