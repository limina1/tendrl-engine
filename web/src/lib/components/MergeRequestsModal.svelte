<script lang="ts">
	// Merge requests (kind 818) that concern me: addressed to me / targeting
	// my events (Incoming), and the ones I sent (Sent). Status is the owner's
	// newest kind-7 (+ merged, − declined), resolved by the engine. Picking a
	// row loads the engine's review (GET /merge-requests/:id/diff) — the
	// owner's current version against the proposed one; the owner can Merge
	// (signs the next version under the same d + a `+`) or Decline (`−` with
	// an optional reason). Opened by M-x tendrl-merge-requests or the event
	// menu's Merge requests action (filtered to that event).
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { confirmState } from '$lib/network/fetch-events.svelte';
	import { promptText } from '$lib/wm/text-prompt.svelte';
	import ProfileName from './ProfileName.svelte';

	const app = getAppState();
	const targets = $derived(app.mergeRequests?.targets ?? null);

	let tab = $state<'incoming' | 'sent'>('incoming');
	let rows = $state<api.MergeRequestRow[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let picked = $state<api.MergeRequestRow | null>(null);
	let review = $state<api.MergeReview | null>(null);
	let reviewError = $state<string | null>(null);
	let broadcast = $state(false);
	let armed = $state(false);
	let busy = $state(false);
	let armTimer: ReturnType<typeof setTimeout> | undefined;

	const yielding = $derived(busy && confirmState.intent?.type === 'publish_intent');

	async function load() {
		loading = true;
		error = null;
		try {
			rows = (await api.listMergeRequests({ role: 'all', status: 'all' })).requests;
		} catch (e) {
			error = api.errorMessage(e);
		} finally {
			loading = false;
		}
	}
	$effect(() => {
		void load();
	});

	const shown = $derived(
		rows
			.filter((r) =>
				tab === 'incoming' ? r.role === 'owner' || r.role === 'both' : r.role === 'requester' || r.role === 'both'
			)
			.filter((r) => !targets || targets.includes(r.target))
			.sort((a, b) => (a.status === 'open' ? 0 : 1) - (b.status === 'open' ? 0 : 1) || b.created_at - a.created_at)
	);
	const incomingOpen = $derived(
		rows.filter(
			(r) =>
				(r.role === 'owner' || r.role === 'both') &&
				r.status === 'open' &&
				(!targets || targets.includes(r.target))
		).length
	);

	async function pick(r: api.MergeRequestRow) {
		picked = r;
		review = null;
		reviewError = null;
		armed = false;
		try {
			review = await api.reviewMergeRequest(r.id);
		} catch (e) {
			reviewError = api.errorMessage(e);
		}
	}

	const canDecide = $derived(
		!!picked && !!review && picked.status === 'open' && (picked.role === 'owner' || picked.role === 'both')
	);

	async function merge() {
		if (!picked) return;
		if (!armed) {
			armed = true;
			clearTimeout(armTimer);
			armTimer = setTimeout(() => (armed = false), 4000);
			return;
		}
		clearTimeout(armTimer);
		armed = false;
		busy = true;
		try {
			await api.mergeMergeRequest(picked.id, { broadcast });
			app.pushToast(`Merged — your next version is signed${broadcast ? ' and sent' : ' (local)'}`, 'success');
			await load();
			const again = rows.find((r) => r.id === picked?.id);
			if (again) await pick(again);
		} catch (e) {
			app.pushToast(`Merge failed: ${api.errorMessage(e)}`, 'error');
		} finally {
			busy = false;
		}
	}

	async function decline() {
		if (!picked) return;
		const reason = await promptText({
			title: 'Decline this merge request',
			placeholder: 'Reason (optional) — sent with your −',
			confirmLabel: 'Decline'
		});
		if (reason === null) return;
		busy = true;
		try {
			await api.declineMergeRequest(picked.id, { reason: reason.trim() || undefined, broadcast });
			app.pushToast('Declined', 'info');
			await load();
			const again = rows.find((r) => r.id === picked?.id);
			if (again) await pick(again);
		} catch (e) {
			app.pushToast(`Decline failed: ${api.errorMessage(e)}`, 'error');
		} finally {
			busy = false;
		}
	}

	function close() {
		if (busy) return;
		clearTimeout(armTimer);
		app.mergeRequests = null;
	}
	function onKey(e: KeyboardEvent) {
		if (yielding || busy) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			if (picked) picked = null;
			else close();
		}
	}

	const coordLabel = (c: string) => {
		const [k, , ...d] = c.split(':');
		return `${k} · ${d.join(':')}`;
	};
	const when = (ts: number) => new Date(ts * 1000).toLocaleString();
</script>

<svelte:window onkeydown={onKey} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="mr-backdrop"
	class:mr-backdrop--yield={yielding}
	role="presentation"
	onclick={(e) => e.target === e.currentTarget && close()}
>
	<div class="mr" role="dialog" aria-modal="true" aria-labelledby="mr-title">
		<header class="mr-head">
			<h2 id="mr-title">Merge requests</h2>
			{#if targets}<span class="mr-dim">for {coordLabel(targets[0])}{targets.length > 1 ? ' and its sections' : ''}</span>{/if}
			<div class="mr-tabs" role="tablist">
				<button role="tab" aria-selected={tab === 'incoming'} class:on={tab === 'incoming'} onclick={() => ((tab = 'incoming'), (picked = null))}
					>Incoming{incomingOpen ? ` (${incomingOpen} open)` : ''}</button
				>
				<button role="tab" aria-selected={tab === 'sent'} class:on={tab === 'sent'} onclick={() => ((tab = 'sent'), (picked = null))}>Sent</button>
			</div>
			<button class="mr-x" onclick={close} aria-label="Close" disabled={busy}>✕</button>
		</header>

		<div class="mr-body">
			{#if !picked}
				{#if loading}
					<div class="mr-dim mr-pad">Loading…</div>
				{:else if error}
					<div class="mr-error mr-pad">{error}</div>
				{:else if shown.length === 0}
					<div class="mr-dim mr-pad">
						{tab === 'incoming'
							? 'No merge requests for your events in your local store.'
							: "You haven't sent any merge requests."}
					</div>
				{:else}
					{#each shown as r (r.id)}
						<button class="mr-row" onclick={() => pick(r)}>
							<span class="mr-status mr-status--{r.status}">{r.status}</span>
							<span class="mr-row-main">
								<span class="mr-row-title">{r.content || 'Merge request'}</span>
								<span class="mr-dim">
									{coordLabel(r.target)} ·
									{#if tab === 'incoming'}from <ProfileName pubkey={r.pubkey} />{:else}to <ProfileName pubkey={r.owner} />{/if}
									· {when(r.created_at)}
								</span>
							</span>
						</button>
					{/each}
				{/if}
			{:else}
				<div class="mr-pad mr-detail">
					<button class="mr-back" onclick={() => (picked = null)}>‹ All requests</button>
					<div class="mr-detail-head">
						<span class="mr-status mr-status--{picked.status}">{picked.status}</span>
						<span class="mr-row-title">{picked.content || 'Merge request'}</span>
					</div>
					<div class="mr-dim">
						{coordLabel(picked.target)} · from <ProfileName pubkey={picked.pubkey} /> · {when(picked.created_at)}
					</div>
					{#if reviewError}
						<div class="mr-error">{reviewError}</div>
					{:else if !review}
						<div class="mr-dim">Loading the diff…</div>
					{:else if picked.status !== 'open'}
						<div class="mr-dim">
							{picked.status === 'merged'
								? 'Merged — the change is in the current version.'
								: 'Declined.'}
						</div>
					{:else}
						{#if review.base.moved_on}
							<div class="mr-warn">
								Your version has moved on since this was proposed — the diff is against your current
								version.
							</div>
						{/if}
						{#if review.note}<div class="mr-dim">{review.note}</div>{/if}
						{#each review.sections as sec, si (si)}
							<div class="mr-sec">
								<div class="mr-sec-head"><b>{sec.title || 'Untitled'}</b> <span class="mr-dim">{sec.format}</span></div>
								<div class="mr-diff">
									{#each sec.hunks as hunk, hi (hi)}
										{#if hi > 0}<div class="mr-gap">⋯</div>{/if}
										{#each hunk as l, li (li)}
											<div class="mr-dl mr-dl--{l.kind}">
												<span class="mr-s">{l.kind === 'added' ? '+' : l.kind === 'removed' ? '−' : ' '}</span><span>{l.line || ' '}</span>
											</div>
										{/each}
									{:else}
										<div class="mr-dim mr-gap">No content change.</div>
									{/each}
								</div>
								{#if sec.tags_added.length || sec.tags_removed.length}
									<div class="mr-tags">
										{#each sec.tags_added as t, ti (ti)}<span class="mr-tag mr-tag--add">+ {JSON.stringify(t)}</span>{/each}
										{#each sec.tags_removed as t, ti (ti)}<span class="mr-tag mr-tag--del">− {JSON.stringify(t)}</span>{/each}
									</div>
								{/if}
							</div>
						{/each}
					{/if}
				</div>
			{/if}
		</div>

		{#if picked && canDecide}
			<footer class="mr-foot">
				<label class="mr-check"><input type="checkbox" bind:checked={broadcast} /> broadcast now</label>
				<span class="mr-dim">Merge signs your next version (same d) and a <code>+</code> reaction; Decline signs a <code>−</code>.</span>
				<span class="mr-sp"></span>
				<button class="mr-btn" onclick={decline} disabled={busy}>Decline…</button>
				<button class="mr-btn mr-btn--primary" class:mr-btn--armed={armed} onclick={merge} disabled={busy}>
					{busy ? 'Signing…' : armed ? 'Click again to merge' : 'Merge'}
				</button>
			</footer>
		{/if}
	</div>
</div>

<style>
	.mr-backdrop {
		position: fixed;
		inset: 0;
		z-index: 230;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 16px;
	}
	.mr-backdrop--yield {
		visibility: hidden;
	}
	.mr {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: min(820px, 100%);
		max-height: 88dvh;
		display: flex;
		flex-direction: column;
	}
	.mr-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 10px;
		padding: 12px 14px;
		border-bottom: 1px solid var(--border);
	}
	.mr-head h2 {
		margin: 0;
		font-size: var(--t-base);
	}
	.mr-tabs {
		display: flex;
		gap: 4px;
	}
	.mr-tabs button {
		font: inherit;
		font-size: var(--t-xs);
		background: none;
		border: 1px solid var(--border);
		color: var(--fg-muted);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.mr-tabs button.on {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.mr-x {
		margin-left: auto;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.mr-body {
		overflow-y: auto;
		flex: 1;
		min-height: 120px;
	}
	.mr-pad {
		padding: 12px 14px;
	}
	.mr-dim {
		color: var(--fg-muted);
		font-size: var(--t-xs);
	}
	.mr-error,
	.mr-warn {
		font-size: var(--t-xs);
		padding: 6px 10px;
		border-radius: var(--r-sm);
		border: 1px solid color-mix(in srgb, var(--orange, #e2a478) 50%, var(--border));
		background: color-mix(in srgb, var(--orange, #e2a478) 8%, transparent);
	}
	.mr-error {
		border-color: color-mix(in srgb, var(--state-error) 50%, var(--border));
	}
	.mr-row {
		display: flex;
		gap: 10px;
		align-items: flex-start;
		width: 100%;
		text-align: left;
		background: none;
		border: none;
		border-bottom: 1px solid var(--border);
		padding: 10px 14px;
		color: var(--fg);
		font: inherit;
		cursor: pointer;
	}
	.mr-row:hover {
		background: color-mix(in srgb, var(--id-yours) 8%, transparent);
	}
	.mr-row-main {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}
	.mr-row-title {
		font-weight: 600;
	}
	.mr-status {
		flex-shrink: 0;
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--border);
		color: var(--fg-muted);
	}
	.mr-status--open {
		border-color: var(--orange, #e2a478);
		color: var(--orange, #e2a478);
	}
	.mr-status--merged {
		border-color: var(--green, #b4be82);
		color: var(--green, #b4be82);
	}
	.mr-detail {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.mr-detail-head {
		display: flex;
		gap: 8px;
		align-items: baseline;
	}
	.mr-back {
		align-self: flex-start;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font: inherit;
		font-size: var(--t-xs);
		padding: 0;
	}
	.mr-sec {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.mr-diff {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 6px 0;
		overflow-x: auto;
		max-height: 320px;
		overflow-y: auto;
	}
	.mr-dl {
		display: flex;
		gap: 8px;
		padding: 0 10px;
		white-space: pre-wrap;
	}
	.mr-s {
		width: 10px;
		flex: none;
		color: var(--fg-muted);
	}
	.mr-dl--added {
		background: color-mix(in srgb, var(--green, #b4be82) 14%, transparent);
	}
	.mr-dl--removed {
		background: color-mix(in srgb, var(--red, #e27878) 14%, transparent);
	}
	.mr-gap {
		color: var(--fg-muted);
		padding: 0 10px;
	}
	.mr-tags {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}
	.mr-tag {
		padding: 1px 6px;
		border-radius: var(--r-sm);
		border: 1px solid var(--border);
	}
	.mr-tag--add {
		border-color: color-mix(in srgb, var(--green, #b4be82) 60%, var(--border));
	}
	.mr-tag--del {
		border-color: color-mix(in srgb, var(--red, #e27878) 60%, var(--border));
	}
	.mr-foot {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		padding: 10px 14px;
		border-top: 1px solid var(--border);
	}
	.mr-sp {
		flex: 1;
	}
	.mr-check {
		display: inline-flex;
		gap: 6px;
		align-items: center;
		font-size: var(--t-sm);
	}
	.mr-btn {
		font: inherit;
		font-size: var(--t-sm);
		padding: 6px 12px;
		min-height: 34px;
		border-radius: var(--r-sm);
		border: 1px solid var(--border);
		background: var(--bg-alt);
		color: var(--fg);
		cursor: pointer;
	}
	.mr-btn--primary {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 16%, transparent);
	}
	.mr-btn--armed {
		border-color: var(--orange, #e2a478);
		background: color-mix(in srgb, var(--orange, #e2a478) 22%, transparent);
	}
	.mr-btn:disabled {
		opacity: 0.45;
	}
</style>
