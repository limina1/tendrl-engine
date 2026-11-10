<script lang="ts">
	// The review a Sign opens for an edit started from an event menu
	// (app.composeEdit): per edited section / article, what will be signed —
	// the route (republish in place, or fork + merge request), the diff, the
	// kind-818 as it will be signed, and where it goes. Everything shown comes
	// from the engine's preview (POST /api/v1/section-edit/preview), the same
	// planning the publish runs. A two-step armed button signs.
	//
	// Modal-yield rule (feat-ui-patterns): while a publish is in flight and a
	// Confirm-mode intent is pending, this hides itself and ignores Escape /
	// backdrop clicks.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();

	type Row = {
		req: api.SectionEditRequest;
		preview: api.SectionEditPreview | null;
		error: string | null;
		message: string;
		/** Destination urls switched off. */
		off: Set<string>;
		showEvents: boolean;
	};

	let rows = $state<Row[]>([]);
	let loading = $state(true);
	let broadcast = $state(false);
	let armed = $state(false);
	let posting = $state(false);
	let armTimer: ReturnType<typeof setTimeout> | undefined;

	const reqs = $derived(app.editReview);
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');

	$effect(() => {
		const list = reqs;
		if (!list) return;
		rows = list.map((req) => ({
			req,
			preview: null,
			error: null,
			message: '',
			off: new Set<string>(),
			showEvents: false
		}));
		loading = true;
		armed = false;
		void Promise.all(
			list.map((req, i) =>
				api
					.sectionEditPreview(req)
					.then((p) => {
						rows[i].preview = p;
					})
					.catch((e) => {
						rows[i].error = api.errorMessage(e);
					})
			)
		).finally(() => (loading = false));
	});

	const ready = $derived(rows.filter((r) => r.preview && !r.preview.unchanged));
	const forks = $derived(ready.filter((r) => r.preview!.route === 'fork_and_request'));
	const undeliverable = $derived(
		forks.some((r) => {
			const d = r.preview!.destinations;
			if (!d) return false;
			return !d.destinations.some(
				(x) =>
					!r.off.has(x.url) && (x.reasons.includes('inbox') || x.reasons.includes('found_on'))
			);
		})
	);
	const eventCount = $derived(
		ready.reduce(
			(n, r) =>
				n +
				1 +
				(r.preview!.index ? 1 : 0) +
				(r.preview!.route === 'fork_and_request' ? 1 : 0),
			0
		)
	);

	function close() {
		if (posting) return;
		clearTimeout(armTimer);
		app.editReview = null;
	}

	const ROUTE_LABEL: Record<api.EditRoute, string> = {
		republish: 'Republish · same d',
		fork_into_index: 'Fork into your publication',
		fork_and_request: 'Fork + merge request'
	};
	const REASON: Record<string, string> = {
		inbox: 'their inbox',
		found_on: 'found here',
		hint: 'relay hint',
		mine: 'your relays'
	};

	function short(id: string) {
		return id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-4)}` : id;
	}

	function toggleDest(r: Row, url: string) {
		const next = new Set(r.off);
		if (next.has(url)) next.delete(url);
		else next.add(url);
		r.off = next;
		armed = false;
	}

	async function sign() {
		if (!armed) {
			armed = true;
			clearTimeout(armTimer);
			armTimer = setTimeout(() => (armed = false), 4000);
			return;
		}
		clearTimeout(armTimer);
		armed = false;
		posting = true;
		let done = 0;
		const failures: string[] = [];
		for (const r of ready) {
			const p = r.preview!;
			const dests = p.destinations?.destinations ?? [];
			const relays =
				p.route === 'fork_and_request' && r.off.size > 0
					? dests.filter((d) => !r.off.has(d.url)).map((d) => d.url)
					: undefined;
			try {
				await api.sectionEditPublish({
					...r.req,
					message: r.message.trim() || undefined,
					broadcast,
					relays,
					mode_confirm: broadcast
				});
				done++;
			} catch (e) {
				failures.push(`${p.section_coordinate}: ${api.errorMessage(e)}`);
			}
		}
		posting = false;
		if (failures.length === 0) {
			const forked = forks.length;
			app.pushToast(
				forked > 0
					? `Signed ${done} edit${done === 1 ? '' : 's'} — ${forked} merge request${forked === 1 ? '' : 's'} ${broadcast ? 'sent' : 'saved locally'}`
					: `Republished ${done} edit${done === 1 ? '' : 's'}${broadcast ? '' : ' locally'}`,
				'success'
			);
			app.editReview = null;
			// The edit is signed: start a fresh draft rather than leave the
			// placed publication in the composer under a plain Sign.
			app.handleCompose();
		} else {
			app.pushToast(`${failures.length} edit(s) failed: ${failures[0]}`, 'error');
		}
	}

	function onKey(e: KeyboardEvent) {
		if (!reqs || yielding || posting) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		}
	}
</script>

<svelte:window onkeydown={onKey} />

{#if reqs}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div
		class="er-backdrop"
		class:er-backdrop--yield={yielding}
		role="presentation"
		onclick={(e) => e.target === e.currentTarget && close()}
	>
		<div class="er" role="dialog" aria-modal="true" aria-labelledby="er-title">
			<header class="er-head">
				<h2 id="er-title">
					{forks.length > 0 ? 'Fork + request merge' : 'Republish'}
				</h2>
				<span class="er-sub">
					{loading
						? 'Planning…'
						: `${eventCount} event${eventCount === 1 ? '' : 's'} to sign`}
				</span>
				<button class="er-x" onclick={close} aria-label="Close" disabled={posting}>✕</button>
			</header>

			<div class="er-body">
				{#each rows as r, i (i)}
					{@const p = r.preview}
					<section class="er-row">
						{#if r.error}
							<div class="er-error">{r.req.title || r.req.section}: {r.error}</div>
						{:else if !p}
							<div class="er-dim">Planning {r.req.title || 'the edit'}…</div>
						{:else}
							<div class="er-row-head">
								<span class="er-pill" class:er-pill--fork={p.route !== 'republish'}
									>{ROUTE_LABEL[p.route]}</span
								>
								<span class="er-title">{r.req.title || p.section_coordinate}</span>
								<span class="er-dim">kind {p.kind} · {p.format}</span>
							</div>
							{#if p.unchanged}
								<div class="er-dim">Unchanged — nothing to sign for this one.</div>
							{/if}
							{#if p.moved_on}
								<div class="er-warn">
									A newer version of this is in your store than the one you edited — the
									diff is against the version you opened.
								</div>
							{/if}
							{#if p.overwrites}
								<div class="er-warn">
									You already have an event at this coordinate ({short(p.overwrites)}); your
									copy replaces it (NIP-54 forks keep the d).
								</div>
							{/if}
							{#if p.title_changed}
								<div class="er-dim">Title changed → “{r.req.title}”.</div>
							{/if}
							<div class="er-diff" role="region" aria-label="Changes">
								{#each p.hunks as hunk, hi (hi)}
									{#if hi > 0}<div class="er-gap">⋯</div>{/if}
									{#each hunk as l, li (li)}
										<div class="er-dl er-dl--{l.kind}">
											<span class="er-s"
												>{l.kind === 'added' ? '+' : l.kind === 'removed' ? '−' : ' '}</span
											><span>{l.line || ' '}</span>
										</div>
									{/each}
								{/each}
							</div>
							{#if p.route === 'fork_and_request'}
								<label class="er-label" for="er-msg-{i}">Message to the author (the 818's content)</label>
								<input
									id="er-msg-{i}"
									class="er-input"
									placeholder="Default: a one-line summary of the change"
									bind:value={r.message}
								/>
								{#if p.destinations}
									<div class="er-label">Send the merge request to</div>
									<div class="er-dests">
										{#each p.destinations.destinations as d (d.url)}
											<button
												class="er-dest"
												class:on={!r.off.has(d.url)}
												aria-pressed={!r.off.has(d.url)}
												onclick={() => toggleDest(r, d.url)}
											>
												<span class="er-mono">{d.url.replace(/^wss?:\/\//, '')}</span>
												<span class="er-dim">{d.reasons.map((x) => REASON[x] ?? x).join(' · ')}</span>
											</button>
										{:else}
											<span class="er-dim">No relays known for the author yet.</span>
										{/each}
									</div>
									{#if p.destinations.note}<div class="er-dim">{p.destinations.note}</div>{/if}
								{/if}
							{/if}
							<button class="er-link" onclick={() => (r.showEvents = !r.showEvents)} aria-expanded={r.showEvents}>
								{r.showEvents ? '▾' : '▸'} events as they will be signed
							</button>
							{#if r.showEvents}
								<pre class="er-json">{JSON.stringify(p.section, null, 2)}</pre>
								{#if p.index}<pre class="er-json">{JSON.stringify(p.index, null, 2)}</pre>{/if}
								{#if p.merge_request}<pre class="er-json">{JSON.stringify(p.merge_request, null, 2)}</pre>{/if}
							{/if}
						{/if}
					</section>
				{/each}
			</div>

			<footer class="er-foot">
				<label class="er-check">
					<input type="checkbox" bind:checked={broadcast} />
					broadcast now
				</label>
				<span class="er-dim">
					{broadcast
						? forks.length > 0
							? 'Signs, stores and sends to the relays above'
							: 'Signs, stores and sends to your publish relays'
						: 'Signs and keeps it local — broadcast later'}
				</span>
				<span class="er-sp"></span>
				{#if undeliverable && broadcast}
					<span class="er-warn-inline">Pick the author's inbox or a found-on relay</span>
				{/if}
				<button class="er-btn" onclick={close} disabled={posting}>Back to editing</button>
				<button
					class="er-btn er-btn--primary"
					class:er-btn--armed={armed}
					disabled={loading || posting || ready.length === 0 || (broadcast && undeliverable)}
					onclick={sign}
				>
					{posting
						? 'Signing…'
						: armed
							? `Click again to sign ${eventCount}`
							: forks.length > 0
								? 'Publish fork + request merge'
								: 'Republish'}
				</button>
			</footer>
		</div>
	</div>
{/if}

<style>
	.er-backdrop {
		position: fixed;
		inset: 0;
		z-index: 240;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 16px;
	}
	.er-backdrop--yield {
		visibility: hidden;
	}
	.er {
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		width: min(820px, 100%);
		max-height: 88dvh;
		display: flex;
		flex-direction: column;
	}
	.er-head {
		display: flex;
		align-items: baseline;
		gap: 10px;
		padding: 12px 14px;
		border-bottom: 1px solid var(--border);
	}
	.er-head h2 {
		margin: 0;
		font-size: var(--t-base);
		font-weight: 600;
	}
	.er-sub,
	.er-dim {
		color: var(--fg-muted);
		font-size: var(--t-xs);
	}
	.er-x {
		margin-left: auto;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
	}
	.er-body {
		overflow-y: auto;
		padding: 4px 14px;
	}
	.er-row {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 12px 0;
		border-bottom: 1px solid var(--border);
	}
	.er-row:last-child {
		border-bottom: none;
	}
	.er-row-head {
		display: flex;
		gap: 8px;
		align-items: baseline;
		flex-wrap: wrap;
	}
	.er-title {
		font-weight: 600;
	}
	.er-pill {
		font-size: var(--t-2xs);
		padding: 1px 8px;
		border-radius: 999px;
		border: 1px solid var(--id-yours);
		color: var(--id-yours);
	}
	.er-pill--fork {
		border-color: var(--id-forked, #b294bb);
		color: var(--id-forked, #b294bb);
	}
	.er-warn,
	.er-error {
		font-size: var(--t-xs);
		padding: 6px 10px;
		border-radius: var(--r-sm);
		border: 1px solid color-mix(in srgb, var(--orange, #e2a478) 50%, var(--border));
		background: color-mix(in srgb, var(--orange, #e2a478) 8%, transparent);
	}
	.er-error {
		border-color: color-mix(in srgb, var(--state-error) 50%, var(--border));
		background: color-mix(in srgb, var(--state-error) 8%, transparent);
	}
	.er-warn-inline {
		color: var(--orange, #e2a478);
		font-size: var(--t-xs);
	}
	.er-diff {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 6px 0;
		overflow-x: auto;
		max-height: 260px;
		overflow-y: auto;
	}
	.er-dl {
		display: flex;
		gap: 8px;
		padding: 0 10px;
		white-space: pre-wrap;
	}
	.er-s {
		width: 10px;
		flex: none;
		color: var(--fg-muted);
	}
	.er-dl--added {
		background: color-mix(in srgb, var(--green, #b4be82) 14%, transparent);
	}
	.er-dl--removed {
		background: color-mix(in srgb, var(--red, #e27878) 14%, transparent);
	}
	.er-gap {
		color: var(--fg-muted);
		padding: 0 10px;
	}
	.er-label {
		font-size: var(--t-2xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--fg-muted);
	}
	.er-input {
		font: inherit;
		font-size: var(--t-sm);
		color: var(--fg);
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 6px 8px;
	}
	.er-dests {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.er-dest {
		display: inline-flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 2px;
		font: inherit;
		font-size: var(--t-xs);
		padding: 6px 10px;
		min-height: 40px;
		border-radius: var(--r-sm);
		border: 1px solid var(--border);
		background: var(--bg-alt);
		color: var(--fg-muted);
		cursor: pointer;
		text-align: left;
	}
	.er-dest.on {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.er-mono {
		font-family: var(--font-mono);
	}
	.er-link {
		align-self: flex-start;
		background: none;
		border: none;
		color: var(--fg-muted);
		cursor: pointer;
		font: inherit;
		font-size: var(--t-xs);
		padding: 0;
	}
	.er-json {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		background: var(--bg-alt);
		border: 1px solid var(--border);
		border-radius: var(--r-sm);
		padding: 8px;
		overflow-x: auto;
		max-height: 240px;
	}
	.er-foot {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		padding: 10px 14px;
		border-top: 1px solid var(--border);
	}
	.er-sp {
		flex: 1;
	}
	.er-check {
		display: inline-flex;
		gap: 6px;
		align-items: center;
		font-size: var(--t-sm);
	}
	.er-btn {
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
	.er-btn--primary {
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 16%, transparent);
	}
	.er-btn--armed {
		border-color: var(--orange, #e2a478);
		background: color-mix(in srgb, var(--orange, #e2a478) 22%, transparent);
	}
	.er-btn:disabled {
		opacity: 0.45;
		cursor: default;
	}
</style>
