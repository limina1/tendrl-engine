<script lang="ts">
	import { onMount } from 'svelte';
	import PublishProgressBuffer from '$lib/wm/renderers/PublishProgressBuffer.svelte';
	import RelaysBuffer from '$lib/wm/renderers/RelaysBuffer.svelte';
	import {
		setProgress,
		mockProgress,
		isLocalRelay,
		type PublishProgressState,
		type PublishEventStatus,
		type RelayResult
	} from '$lib/wm/publish-progress.svelte';
	import type { Buffer } from '$lib/wm/types';

	// The WM renderers take a `buffer` prop (used only for identity); a
	// stub is fine in the artboard — we're driving them via the module
	// store, not the buffer system.
	const ppBuffer: Buffer = { id: 'design:publish', kind: 'publish-progress', label: 'publish' };
	const relaysBuffer: Buffer = { id: 'design:relays', kind: 'relays', label: 'relays' };

	// --- Scenario builders -------------------------------------------------
	// Each transforms the base mock into a publish state that exercises a
	// distinct slice of the UI. The renderer reads the shared module store,
	// so a scenario switch is just `setProgress(builder())`.

	type ScenarioId = 'mixed' | 'in-progress' | 'all-landed' | 'mostly-rejected' | 'empty';

	function setRelayStates(
		ev: PublishEventStatus,
		fn: (i: number, url: string, isLocal: boolean) => [RelayResult, string?]
	): PublishEventStatus {
		return {
			...ev,
			relays: ev.relays.map((r, i) => {
				const [state, message] = fn(i, r.url, r.isLocal);
				return {
					...r,
					state,
					message,
					durationMs: state === 'accepted' ? 120 + i * 40 : undefined
				};
			})
		};
	}

	function allLanded(): PublishProgressState {
		const base = mockProgress();
		return {
			...base,
			completed: true,
			events: base.events.map((ev) => setRelayStates(ev, () => ['accepted']))
		};
	}

	function mostlyRejected(): PublishProgressState {
		const base = mockProgress();
		// Local accepts (the durable-copy guarantee), every external rejects
		// with a representative reason so we can shape the failure state.
		const reasons = [
			'rate-limited: max 1 event per 2 minutes',
			'blocked: pubkey not on allow-list',
			'invalid: created_at too far in the future'
		];
		let ri = 0;
		return {
			...base,
			completed: true,
			events: base.events.map((ev) =>
				setRelayStates(ev, (_i, _url, isLocal) => {
					if (isLocal) return ['accepted'];
					const reason = reasons[ri % reasons.length];
					ri++;
					return ['rejected', reason];
				})
			)
		};
	}

	function inProgress(): PublishProgressState {
		const base = mockProgress();
		// Local landed, externals still in flight — the live-publish moment.
		return {
			...base,
			completed: false,
			events: base.events.map((ev, idx) =>
				setRelayStates(ev, (i, _url, isLocal) => {
					if (isLocal) return ['accepted'];
					// Stagger: earlier events further along than later ones.
					if (idx === 0) return ['accepted'];
					if (i === 1) return ['sending'];
					return ['pending'];
				})
			)
		};
	}

	function buildScenario(id: ScenarioId): PublishProgressState | null {
		switch (id) {
			case 'mixed':
				return mockProgress();
			case 'in-progress':
				return inProgress();
			case 'all-landed':
				return allLanded();
			case 'mostly-rejected':
				return mostlyRejected();
			case 'empty':
				return null;
		}
	}

	const scenarios: Array<{ id: ScenarioId; label: string; sub: string }> = [
		{ id: 'mixed', label: 'Mixed', sub: 'every state at once' },
		{ id: 'in-progress', label: 'Publishing', sub: 'externals in flight' },
		{ id: 'all-landed', label: 'All landed', sub: 'full accept' },
		{ id: 'mostly-rejected', label: 'Mostly rejected', sub: 'local-only survives' },
		{ id: 'empty', label: 'Empty', sub: 'nothing publishing' }
	];

	let active = $state<ScenarioId>('mixed');

	function pick(id: ScenarioId) {
		active = id;
		setProgress(buildScenario(id));
	}

	onMount(() => {
		setProgress(buildScenario(active));
		// Leave the store populated on unmount so navigating away then back
		// to the in-app demo buffer still shows something; the real publish
		// flow overwrites it when wired.
	});
</script>

<svelte:head><title>tendrl · design · publish + relays</title></svelte:head>

<div class="page">
	<header class="page__head">
		<nav class="crumbs">
			<a href="/design">design</a>
			<span class="sep">/</span>
			<a href="/design/layouts">layouts</a>
			<span class="sep">/</span>
			<a href="/design/graph">graph</a>
			<span class="sep">/</span>
			<span class="crumbs__here">publish</span>
		</nav>
		<div class="eyebrow">design · publish destinations</div>
		<h1 class="title">Where did my events land — and which relays rejected them, and why?</h1>
		<p class="lede">
			Two surfaces for the publish path. The <strong>progress buffer</strong> shows
			every <code>event × relay</code> cell with accept/reject/timeout state and the
			relay's verbatim reason — fed here by mock scenarios. The
			<strong>relays buffer</strong> is the configuration menu: the publish/fetch/general
			sets with live NIP-11 detail. The engine already returns
			<code>broadcast_results</code> per relay; the wiring gap is feeding it into
			the progress store instead of the console.
		</p>
	</header>

	<section class="board-section">
		<div class="board-section__head">
			<div>
				<div class="board-section__name">Publish progress</div>
				<div class="board-section__sub">
					Mock data — switch scenarios to shape each state. Expand a row for per-relay
					reasons; the local relay carries the durable-copy guarantee.
				</div>
			</div>
			<div class="scenario-bar">
				{#each scenarios as s (s.id)}
					<button
						class="scenario {active === s.id ? 'scenario--on' : ''}"
						onclick={() => pick(s.id)}
						title={s.sub}
					>
						<span class="scenario__label">{s.label}</span>
						<span class="scenario__sub">{s.sub}</span>
					</button>
				{/each}
			</div>
		</div>
		<div class="board board--tall">
			<PublishProgressBuffer buffer={ppBuffer} />
		</div>
	</section>

	<section class="board-section">
		<div class="board-section__head">
			<div>
				<div class="board-section__name">Relay configuration</div>
				<div class="board-section__sub">
					Live — reads <code>/api/v1/relays</code> + NIP-11 from the running engine.
					Read/write/auth toggles and “add relay” are not yet persisted (the
					<code>/api/v1/config/update</code> endpoint exists, unwired).
				</div>
			</div>
		</div>
		<div class="board board--tall">
			<RelaysBuffer buffer={relaysBuffer} />
		</div>
	</section>
</div>

<style>
	.page {
		min-height: 100dvh;
		background: var(--bg-alt);
		color: var(--fg);
		font-family: var(--font-sans);
		padding: var(--s-8) var(--s-6);
		max-width: 1100px;
		margin: 0 auto;
	}

	.page__head {
		margin-bottom: var(--s-8);
	}

	.crumbs {
		display: flex;
		gap: var(--s-2);
		align-items: center;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		margin-bottom: var(--s-3);
	}
	.crumbs a {
		color: var(--base6);
		text-decoration: none;
	}
	.crumbs a:hover {
		color: var(--cyan);
	}
	.crumbs .sep {
		color: var(--base4);
	}
	.crumbs__here {
		color: var(--base8);
	}

	.eyebrow {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base5);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		margin-bottom: var(--s-2);
	}
	.title {
		font-size: var(--t-2xl);
		font-weight: 600;
		margin: 0 0 var(--s-3);
		line-height: var(--lh-tight);
	}
	.lede {
		font-size: var(--t-md);
		color: var(--base7);
		max-width: 74ch;
		margin: 0;
		line-height: var(--lh-snug);
	}
	.lede code {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		background: var(--base1);
		padding: 1px 5px;
		border-radius: var(--r-sm);
		color: var(--cyan);
	}
	.lede strong {
		color: var(--base8);
		font-weight: 600;
	}

	.board-section {
		margin-bottom: var(--s-10);
	}
	.board-section__head {
		display: flex;
		justify-content: space-between;
		align-items: flex-start;
		gap: var(--s-4);
		margin-bottom: var(--s-4);
		flex-wrap: wrap;
	}
	.board-section__name {
		font-size: var(--t-lg);
		font-weight: 600;
	}
	.board-section__sub {
		font-size: var(--t-sm);
		color: var(--base6);
		margin-top: 2px;
		line-height: var(--lh-snug);
		max-width: 64ch;
	}
	.board-section__sub code {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--base1);
		padding: 1px 4px;
		border-radius: var(--r-sm);
		color: var(--base7);
	}

	/* A buffer-sized frame so the renderers sit in something resembling a
	   real pane. The renderers bring their own scoped styles. */
	.board {
		background: var(--panel-bg, var(--bg));
		border: 1px solid var(--panel-border, var(--border));
		border-radius: var(--r-md);
		overflow: auto;
	}
	.board--tall {
		height: 560px;
	}

	.scenario-bar {
		display: flex;
		gap: var(--s-2);
		flex-wrap: wrap;
	}
	.scenario {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 1px;
		padding: var(--s-2) var(--s-3);
		background: var(--base1);
		border: 1px solid var(--panel-border, var(--border));
		border-radius: var(--r-sm);
		cursor: pointer;
		color: var(--base6);
		font-family: var(--font-sans);
		text-align: left;
		transition: border-color 0.1s, color 0.1s;
	}
	.scenario:hover {
		color: var(--base8);
		border-color: var(--base4);
	}
	.scenario--on {
		color: var(--base8);
		border-color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
	}
	.scenario__label {
		font-size: var(--t-sm);
		font-weight: 600;
	}
	.scenario__sub {
		font-size: var(--t-xs);
		color: var(--base5);
	}
</style>
