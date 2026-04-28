<script lang="ts">
	type SlotKind = 'rail' | 'open' | 'hidden';
	type Frame = {
		caption: string;
		layout: 'write' | 'read';
		left: { kind: SlotKind; label: string };
		center: { label: string; kicker?: string };
		right: { kind: SlotKind; label: string };
		modeline: string;
	};

	const sceneA: Frame[] = [
		{
			caption: '1. In write layout. Composer in center, chat open left, refs open right.',
			layout: 'write',
			left: { kind: 'open', label: 'chat' },
			center: { label: 'composer', kicker: 'untitled draft' },
			right: { kind: 'open', label: 'refs' },
			modeline: 'L:write  composer * (untitled draft)  online'
		},
		{
			caption: '2. Switch to read. The write layout is frozen — its windows + buffers stored as-is.',
			layout: 'read',
			left: { kind: 'rail', label: 'chat' },
			center: { label: 'reader', kicker: 'last reader instance' },
			right: { kind: 'open', label: 'search' },
			modeline: 'L:read  reader  online'
		},
		{
			caption: '3. Back to write. Composer instance is restored exactly where it was.',
			layout: 'write',
			left: { kind: 'open', label: 'chat' },
			center: { label: 'composer', kicker: 'untitled draft (preserved)' },
			right: { kind: 'open', label: 'refs' },
			modeline: 'L:write  composer * (untitled draft)  online'
		}
	];

	const sceneB: Frame[] = [
		{
			caption: '1. In write layout. Composer in center, chat open left, refs open right.',
			layout: 'write',
			left: { kind: 'open', label: 'chat' },
			center: { label: 'composer', kicker: 'untitled draft' },
			right: { kind: 'open', label: 'refs' },
			modeline: 'L:write  composer * (untitled draft)  online'
		},
		{
			caption: '2. Switch to read. Only window shape changes — center buffer is global, stays put.',
			layout: 'read',
			left: { kind: 'rail', label: 'chat' },
			center: { label: 'composer', kicker: 'untitled draft (stays)' },
			right: { kind: 'open', label: 'search' },
			modeline: 'L:read  composer * (untitled draft)  online'
		},
		{
			caption: '3. Back to write. Same buffer in center — it never moved.',
			layout: 'write',
			left: { kind: 'open', label: 'chat' },
			center: { label: 'composer', kicker: 'untitled draft' },
			right: { kind: 'open', label: 'refs' },
			modeline: 'L:write  composer * (untitled draft)  online'
		}
	];

	let activeA = $state(0);
	let activeB = $state(0);
</script>

<svelte:head><title>tendrl · layout scoping</title></svelte:head>

<div class="page">
	<header class="page__head">
		<div class="eyebrow">design · open question</div>
		<h1 class="title">Layout scoping: do layouts own their buffers, or just their geometry?</h1>
		<p class="lede">
			When you switch from <code>write</code> to <code>read</code> and back to <code>write</code>,
			what's in the center? Two models below — click the layout tabs in each scene to step through.
			The difference shows up in frame&nbsp;3.
		</p>
	</header>

	<div class="scenes">
		<section class="scene">
			<div class="scene__head">
				<span class="badge badge--a">A</span>
				<div>
					<div class="scene__name">Layout-scoped</div>
					<div class="scene__sub">Emacs perspectives, Doom layouts. Each layout is a frozen window+buffer state.</div>
				</div>
			</div>

			{#each sceneA as frame, i (i)}
				<div class="frame">
					<div class="frame__caption">{frame.caption}</div>
					{@render shell(frame, i === activeA, () => (activeA = i))}
				</div>
			{/each}
		</section>

		<section class="scene">
			<div class="scene__head">
				<span class="badge badge--b">B</span>
				<div>
					<div class="scene__name">Geometry-only</div>
					<div class="scene__sub">Layout names a window shape. Buffers are global; switching layout only resizes/hides windows.</div>
				</div>
			</div>

			{#each sceneB as frame, i (i)}
				<div class="frame">
					<div class="frame__caption">{frame.caption}</div>
					{@render shell(frame, i === activeB, () => (activeB = i))}
				</div>
			{/each}
		</section>
	</div>

	<footer class="page__foot">
		<h2 class="foot__title">Trade-off</h2>
		<div class="foot__cols">
			<div>
				<div class="foot__h">A · Layout-scoped</div>
				<ul>
					<li>Matches Emacs perspectives + Doom layouts directly. Forward-compatible with the eventual Tendrl+Emacs port.</li>
					<li>"Layouts" become real workspaces — read mode and write mode genuinely have different center buffers.</li>
					<li>More state to persist (per-layout window tree + buffer assignments + scroll positions).</li>
					<li>Mental model: like switching tmux sessions or i3 workspaces. Things you weren't using stay where you left them.</li>
				</ul>
			</div>
			<div>
				<div class="foot__h">B · Geometry-only</div>
				<ul>
					<li>Less state. Layouts are presets for the window shape.</li>
					<li>"Read" and "write" feel like view modes on the same content, not separate workspaces.</li>
					<li>Doesn't match Emacs window-config behavior — closer to CSS Grid templates.</li>
					<li>Mental model: the buffer is the document; the layout is just the window arrangement.</li>
				</ul>
			</div>
		</div>
		<p class="foot__rec">
			<strong>My read:</strong> A is the right call given the Emacs-portability constraint. B is appealing for
			simplicity but loses the "different mode = different working set" semantic that read/write/triage actually want.
			B also breaks down once you have multi-instance buffers (which composer search reader all are) — when you
			switch layouts, which instance does the center keep?
		</p>
	</footer>
</div>

{#snippet shell(f: Frame, active: boolean, onclick: () => void)}
	<div class="shell {active ? 'shell--active' : ''}">
		<div class="shell__header">
			<div class="shell__brand">tendrl</div>
			<div class="shell__layouts">
				<button class="lt {f.layout === 'write' ? 'lt--on' : ''}" onclick={onclick}>write</button>
				<button class="lt {f.layout === 'read' ? 'lt--on' : ''}" onclick={onclick}>read</button>
				<button class="lt" disabled>triage</button>
				<button class="lt" disabled>zen</button>
			</div>
			<div class="shell__mx">M-x</div>
		</div>

		<div class="shell__body">
			{#if f.left.kind === 'open'}
				<div class="win win--side">
					<div class="win__head">{f.left.label}</div>
					<div class="win__body win__body--side"></div>
				</div>
			{:else if f.left.kind === 'rail'}
				<div class="rail">
					<span class="rail__tab">{f.left.label}</span>
				</div>
			{/if}

			<div class="win win--center">
				<div class="win__head win__head--center">
					<span>{f.center.label}</span>
					{#if f.center.kicker}
						<span class="win__kicker">· {f.center.kicker}</span>
					{/if}
				</div>
				<div class="win__body win__body--center">
					<div class="ghost-line ghost-line--lg"></div>
					<div class="ghost-line"></div>
					<div class="ghost-line"></div>
					<div class="ghost-line ghost-line--short"></div>
				</div>
			</div>

			{#if f.right.kind === 'open'}
				<div class="win win--side">
					<div class="win__head">{f.right.label}</div>
					<div class="win__body win__body--side"></div>
				</div>
			{:else if f.right.kind === 'rail'}
				<div class="rail">
					<span class="rail__tab">{f.right.label}</span>
				</div>
			{/if}
		</div>

		<div class="shell__modeline">{f.modeline}</div>
	</div>
{/snippet}

<style>
	.page {
		min-height: 100dvh;
		background: var(--bg-alt);
		color: var(--fg);
		font-family: var(--font-sans);
		padding: var(--s-8) var(--s-6);
		max-width: 1400px;
		margin: 0 auto;
	}

	.page__head { margin-bottom: var(--s-8); }
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
		max-width: 70ch;
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

	.scenes {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: var(--s-6);
		margin-bottom: var(--s-10);
	}
	@media (max-width: 1100px) {
		.scenes { grid-template-columns: 1fr; }
	}

	.scene {
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: var(--s-5);
		display: flex;
		flex-direction: column;
		gap: var(--s-4);
	}
	.scene__head {
		display: flex;
		gap: var(--s-3);
		align-items: flex-start;
		padding-bottom: var(--s-3);
		border-bottom: 1px solid var(--panel-border);
	}
	.scene__name { font-size: var(--t-md); font-weight: 600; }
	.scene__sub { font-size: var(--t-sm); color: var(--base6); margin-top: 2px; line-height: var(--lh-snug); }

	.badge {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 22px;
		height: 22px;
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 600;
		flex-shrink: 0;
	}
	.badge--a { background: color-mix(in srgb, var(--id-yours) 25%, transparent); color: var(--id-yours); }
	.badge--b { background: color-mix(in srgb, var(--id-imported) 25%, transparent); color: var(--id-imported); }

	.frame {
		display: flex;
		flex-direction: column;
		gap: var(--s-2);
	}
	.frame__caption {
		font-size: var(--t-sm);
		color: var(--base6);
		line-height: var(--lh-snug);
		font-family: var(--font-mono);
	}

	.shell {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		overflow: hidden;
		display: flex;
		flex-direction: column;
		transition: border-color 120ms;
	}
	.shell--active {
		border-color: var(--base4);
	}

	.shell__header {
		display: flex;
		align-items: center;
		gap: var(--s-3);
		padding: 0 var(--s-3);
		height: 28px;
		background: var(--panel-header-bg);
		border-bottom: 1px solid var(--panel-border);
	}
	.shell__brand {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
	}
	.shell__layouts {
		display: flex;
		gap: 2px;
	}
	.lt {
		font-family: var(--font-mono);
		font-size: 10px;
		padding: 2px 8px;
		background: transparent;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--base5);
		cursor: pointer;
	}
	.lt:hover:not(:disabled) { color: var(--fg); }
	.lt--on {
		background: var(--base2);
		color: var(--fg);
		border-color: var(--base3);
	}
	.lt:disabled { opacity: 0.4; cursor: not-allowed; }
	.shell__mx {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		padding: 2px 8px;
		border: 1px solid var(--base3);
		border-radius: var(--r-sm);
	}

	.shell__body {
		display: flex;
		min-height: 180px;
	}
	.win {
		display: flex;
		flex-direction: column;
		border-right: 1px solid var(--panel-border);
	}
	.win:last-child { border-right: none; }
	.win--side { width: 110px; flex-shrink: 0; }
	.win--center { flex: 1; }

	.win__head {
		font-family: var(--font-mono);
		font-size: 10px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		padding: 4px var(--s-3);
		background: var(--panel-bg-soft);
		border-bottom: 1px solid var(--panel-border);
	}
	.win__head--center {
		display: flex;
		align-items: baseline;
		gap: var(--s-2);
		color: var(--base7);
	}
	.win__kicker { color: var(--base5); text-transform: none; letter-spacing: 0; }

	.win__body {
		flex: 1;
		padding: var(--s-3);
	}
	.win__body--side {
		background:
			repeating-linear-gradient(
				180deg,
				var(--base1) 0,
				var(--base1) 6px,
				transparent 6px,
				transparent 14px
			);
		opacity: 0.6;
	}
	.win__body--center {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: var(--s-4);
	}

	.ghost-line {
		height: 8px;
		background: var(--base2);
		border-radius: 2px;
		width: 100%;
	}
	.ghost-line--lg { height: 12px; width: 60%; background: var(--base3); }
	.ghost-line--short { width: 35%; }

	.rail {
		width: var(--rail-w);
		flex-shrink: 0;
		background: var(--panel-rail-bg);
		border-right: 1px solid var(--panel-border);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: var(--s-3);
	}
	.rail__tab {
		writing-mode: vertical-rl;
		transform: rotate(180deg);
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base6);
		letter-spacing: 0.05em;
		text-transform: uppercase;
	}

	.shell__modeline {
		height: 22px;
		background: var(--panel-bg-soft);
		border-top: 1px solid var(--panel-border);
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base6);
		display: flex;
		align-items: center;
		padding: 0 var(--s-3);
		letter-spacing: 0.02em;
	}

	.page__foot {
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		padding: var(--s-6);
	}
	.foot__title {
		font-size: var(--t-lg);
		font-weight: 600;
		margin: 0 0 var(--s-4);
	}
	.foot__cols {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: var(--s-6);
		margin-bottom: var(--s-5);
	}
	@media (max-width: 900px) {
		.foot__cols { grid-template-columns: 1fr; }
	}
	.foot__h {
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--base7);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		margin-bottom: var(--s-2);
	}
	.foot__cols ul {
		margin: 0;
		padding-left: var(--s-4);
		font-size: var(--t-sm);
		color: var(--base6);
		line-height: var(--lh-snug);
	}
	.foot__cols li { margin-bottom: var(--s-2); }
	.foot__rec {
		font-size: var(--t-md);
		color: var(--fg-alt);
		padding-top: var(--s-4);
		border-top: 1px solid var(--panel-border);
		margin: 0;
		line-height: var(--lh-snug);
	}
	.foot__rec strong { color: var(--id-yours); }
</style>
