<script lang="ts">
	import Icon from '$lib/Icon.svelte';
	import IdBar from '$lib/design/IdBar.svelte';
	import PanelHeader from '$lib/design/PanelHeader.svelte';
	import PubRow from '$lib/design/PubRow.svelte';

	const typeScale: Array<[string, string]> = [
		['xs', '11'],
		['sm', '12'],
		['base', '13'],
		['md', '14'],
		['lg', '16'],
		['xl', '19'],
		['2xl', '24']
	];

	const baseSwatches: Array<[string, string]> = [
		['bg', '#161821'],
		['bg-alt', '#0f1117'],
		['base0', '#0c0e13'],
		['base1', '#1c1e27'],
		['base2', '#282a36'],
		['base3', '#3d4455'],
		['base4', '#485163'],
		['base5', '#6b7089'],
		['base6', '#9a9ca5'],
		['base7', '#c6c8d1'],
		['base8', '#e3e4e8']
	];

	const roleSwatches: Array<[string, string, string]> = [
		['yours', '#84a0c6', 'your authored content'],
		['imported', '#a093c7', 'read-only reference'],
		['forked', '#b294bb', 'forked, now yours'],
		['diverged', '#e2a478', 'modified upstream'],
		['local', '#e9b189', 'local, not synced'],
		['remote', '#89b8c2', 'fetched from relay'],
		['draft', '#e27878', 'unsigned draft']
	];

	const samplePubs = [
		{
			title: 'Ring Signatures for Anonymous Curation',
			author: 'fiatjaf',
			date: '3/29/26',
			sections: 14,
			status: 'remote' as const,
			kind: 'remote' as const,
			tags: ['crypto']
		},
		{
			title: 'Notes on Markup-Agnostic Citation',
			author: 'me',
			date: '3/28/26',
			sections: 4,
			status: 'draft' as const,
			kind: 'draft' as const,
			tags: ['nostrdown']
		}
	];
</script>

<svelte:head><title>tendrl · design system</title></svelte:head>

{#snippet section(num: string, title: string, sub: string, body: import('svelte').Snippet)}
	<section>
		<div class="sec-head">
			<span class="sec-num">{num}</span>
			<h2 class="sec-title">{title}</h2>
		</div>
		{#if sub}<p class="sec-sub">{sub}</p>{/if}
		{@render body()}
	</section>
{/snippet}

{#snippet typeCard(family: string, label: string, sample: string, note: string)}
	<div class="card">
		<div class="card__eyebrow">{label}</div>
		<div class="card__sample" style:font-family={family}>{sample}</div>
		<div class="card__family">{family}</div>
		<div class="card__note">{note}</div>
	</div>
{/snippet}

{#snippet swatch(name: string, hex: string)}
	<div class="swatch">
		<div class="swatch__chip" style:background={hex}></div>
		<div class="swatch__name">{name}</div>
		<div class="swatch__hex">{hex}</div>
	</div>
{/snippet}

{#snippet roleSwatch(name: string, hex: string, desc: string)}
	<div class="role">
		<div class="role__row">
			<div class="role__bar" style:background={hex}></div>
			<span class="role__name">{name}</span>
		</div>
		<div class="role__hex">{hex}</div>
		<div class="role__desc">{desc}</div>
	</div>
{/snippet}

{#snippet demo(label: string, body: import('svelte').Snippet)}
	<div>
		<div class="demo__label">{label}</div>
		{@render body()}
	</div>
{/snippet}

{#snippet modeCard(label: string, tagline: string, cols: string)}
	{@const colArr = cols.split(' ')}
	<div class="mode">
		<div class="mode__head">
			<span class="mode__label">{label}</span>
			<span class="mode__tagline">{tagline}</span>
		</div>
		<div class="mode__grid" style:grid-template-columns={cols}>
			{#each colArr as _, i}
				<div class="mode__col {i === Math.floor(colArr.length / 2) ? 'mode__col--center' : ''}"></div>
			{/each}
		</div>
	</div>
{/snippet}

<div class="page">
	<div class="page__inner">
		<header class="head">
			<div class="head__eyebrow">tendrl · visual system v0.1</div>
			<h1 class="head__title">A workbench grammar, dense by default.</h1>
			<p class="head__lede">
				Iceberg Dark as the foundation. Power-user density everywhere it earns its keep — the
				workbench, the chat, the search panel — and a single quiet override for prose: serif body,
				generous measure, low chrome.
			</p>
		</header>

		{#snippet typeBody()}
			<div class="grid-3">
				{@render typeCard(
					'ui-sans-serif, system-ui, sans-serif',
					'UI Sans',
					'The unit of composition is the section.',
					'Default. 13/14 in chrome, 16 for headings.'
				)}
				{@render typeCard(
					'ui-monospace, monospace',
					'Mono',
					'kind:30041:abc…d/intro',
					'Addresses, query syntax, status, counts.'
				)}
				{@render typeCard(
					'"Iowan Old Style", Charter, Georgia, serif',
					'Serif',
					'In a network, the smallest reusable thought wants a name.',
					'Read mode only. Generous measure (640px), 18/1.7.'
				)}
			</div>
			<div class="scale">
				{#each typeScale as [k, v] (k)}
					<div class="scale__cell">
						<span class="scale__token">--t-{k}</span>
						<span class="scale__sample" style:font-size={v + 'px'}>Aa</span>
						<span class="scale__px">{v}px</span>
					</div>
				{/each}
			</div>
		{/snippet}
		{@render section('01', 'Type', 'Three families. Sans for UI, mono for instruments and addresses, serif reserved for read mode.', typeBody)}

		{#snippet colorBody()}
			<div class="grid-11">
				{#each baseSwatches as [n, h] (n)}
					{@render swatch(n, h)}
				{/each}
			</div>
			<div class="role-eyebrow">Identity roles — left-edge bars on items</div>
			<div class="grid-7">
				{#each roleSwatches as [n, h, d] (n)}
					{@render roleSwatch(n, h, d)}
				{/each}
			</div>
		{/snippet}
		{@render section('02', 'Color — Iceberg Dark', 'A neutral graphite scale, plus a small palette of typed roles. Accents only ever signal identity or state.', colorBody)}

		{#snippet panelBody()}
			<div class="grid-panel">
				<div class="demo-frame demo-frame--rail">
					<div class="rail">
						<div class="rail__item">
							<button class="rail-btn rail-btn--active"><Icon name="chat" size={13} /></button>
							<div class="rail-label">CHAT</div>
							<div class="rail-key">⌃[</div>
						</div>
					</div>
					<div class="demo-notes">
						<div class="demo-notes__h">collapsed (28px)</div>
						<div>· icon + rotated label</div>
						<div>· keybinding visible</div>
						<div>· active = blue</div>
						<div>· hover = lift</div>
					</div>
				</div>
				<div class="demo-frame demo-frame--col">
					<PanelHeader title="search" subtitle="t:nostr k:30041" onCollapse={() => {}}>
						{#snippet icon()}<Icon name="search" size={11} />{/snippet}
						{#snippet actions()}
							<button class="btn btn--ghost btn--icon" title="filter">
								<Icon name="filter" size={11} />
							</button>
						{/snippet}
					</PanelHeader>
					<div class="demo-notes">
						<div class="demo-notes__h">expanded</div>
						<div>· header is uppercase mono, 11px</div>
						<div>· subtitle is the live state (query, route, …)</div>
						<div>· one collapse affordance, never two</div>
						<div>· active panel: 1px inset blue ring</div>
					</div>
				</div>
			</div>
		{/snippet}
		{@render section('03', 'Panel grammar', 'A panel is either a 28px rail (collapsed) or an open column with a 28px header. Both expose a single collapse affordance — never two.', panelBody)}

		{#snippet primitivesBody()}
			<div class="grid-2">
				{#snippet pubsDemo()}
					<div class="prim-frame">
						{#each samplePubs as p (p.title)}
							<PubRow {p} />
						{/each}
					</div>
				{/snippet}
				{@render demo('Publication row', pubsDemo)}

				{#snippet sectionBlock()}
					<div class="prim-frame prim-frame--row">
						<IdBar kind="imported" />
						<div class="sb">
							<div class="sb__head">
								<span class="pill pill--imported">imported</span>
								<span class="sb__title">§3 — Ring Signatures, Briefly</span>
								<div class="sb__spacer"></div>
								<span class="sb__by">by fiatjaf</span>
							</div>
							<div class="sb__body">
								A ring signature lets a member of a fixed set sign on behalf of the set, without
								revealing which member did so…
							</div>
							<label class="sb__check">
								<input type="checkbox" /> fork to edit
							</label>
						</div>
					</div>
				{/snippet}
				{@render demo('Section block (compose)', sectionBlock)}

				{#snippet searchRow()}
					<div class="prim-frame prim-frame--col">
						<div class="sr__head">
							<span class="pill pill--ghost mono">k:30041</span>
							<span class="sr__title">§7 — On Anonymous Endorsement</span>
							<div class="sb__spacer"></div>
							<span class="pill sr__score">87% ~</span>
						</div>
						<div class="sr__snippet">
							Curation without identity asks a strange question: can a recommendation be valuable…
						</div>
						<div class="sr__meta">
							<span>by pablof7z</span>
							<span class="sr__sep">·</span>
							<span>#curation</span>
						</div>
					</div>
				{/snippet}
				{@render demo('Search result row (semantic)', searchRow)}

				{#snippet chatFrag()}
					<div class="prim-frame prim-frame--row">
						<IdBar kind="yours" />
						<div class="cf">
							<div class="cf__role">user</div>
							<div class="cf__body">How does NKBIP-01 model nested publications?</div>
						</div>
					</div>
				{/snippet}
				{@render demo('Chat fragment', chatFrag)}
			</div>
		{/snippet}
		{@render section('04', 'Primitives', 'The four atoms the workbench is built from.', primitivesBody)}

		{#snippet controlsBody()}
			<div class="controls">
				<button class="btn btn--primary">Compose</button>
				<button class="btn">Sync all</button>
				<button class="btn btn--ghost">Fetch</button>
				<button class="btn btn--ghost btn--icon"><Icon name="refresh" size={12} /></button>
				<span class="pill pill--online"><span class="dot dot--online"></span>online</span>
				<span class="pill pill--local">local</span>
				<span class="pill pill--remote">remote</span>
				<span class="pill pill--draft">draft</span>
				<span class="pill pill--imported">imported</span>
				<span class="pill pill--forked">forked</span>
				<span class="pill pill--diverged">diverged</span>
				<span class="kbd">⌃ [</span>
				<span class="kbd">⌃ ]</span>
				<span class="kbd">⌥ 1</span>
				<span class="kbd">/</span>
			</div>
		{/snippet}
		{@render section('05', 'Controls', 'Buttons, pills, keys.', controlsBody)}

		{#snippet modesBody()}
			<div class="grid-2">
				{@render modeCard('Workbench', 'dense · all panels open · mono · 13px', '28px 1fr 1fr 1fr 28px')}
				{@render modeCard('Read', 'focused · panels as rails · serif · 18px', '28px 1fr 28px')}
			</div>
		{/snippet}
		{@render section('06', 'Two modes, one chrome', 'The panels never go away — but the center adapts. In Workbench (dense) all panels open, mono dominates. In Read (focused) panels collapse to rails, the center switches to serif at 18/1.7, and the chrome fades. Same app; different posture.', modesBody)}

		<footer class="foot">
			<span>tendrl · iceberg dark · v0.1</span>
			<span>see the canvas for applied scenes →</span>
		</footer>
	</div>
</div>

<style>
	.page {
		background: var(--bg);
		color: var(--fg);
		min-height: 100%;
		padding: 32px;
		font-family: var(--font-sans);
	}
	.page__inner {
		max-width: 1100px;
		margin: 0 auto;
		display: flex;
		flex-direction: column;
		gap: 40px;
	}

	.head__eyebrow {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--base5);
		letter-spacing: 0.16em;
		text-transform: uppercase;
	}
	.head__title {
		font-size: 32px;
		margin: 8px 0 6px;
		font-weight: 600;
		color: var(--base8);
	}
	.head__lede {
		color: var(--base6);
		max-width: 640px;
		margin: 0;
		line-height: 1.55;
	}

	.sec-head {
		display: flex;
		align-items: baseline;
		gap: 12px;
		margin-bottom: 4px;
	}
	.sec-num {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--base5);
	}
	.sec-title {
		font-size: 18px;
		font-weight: 600;
		margin: 0;
		color: var(--base8);
	}
	.sec-sub {
		color: var(--base6);
		margin: 0 0 16px;
		max-width: 720px;
		line-height: 1.5;
		font-size: 13px;
	}

	.grid-2 {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 16px;
	}
	.grid-3 {
		display: grid;
		grid-template-columns: 1fr 1fr 1fr;
		gap: 16px;
	}
	.grid-7 {
		display: grid;
		grid-template-columns: repeat(7, 1fr);
		gap: 6px;
	}
	.grid-11 {
		display: grid;
		grid-template-columns: repeat(11, 1fr);
		gap: 6px;
	}
	.grid-panel {
		display: grid;
		grid-template-columns: 320px 1fr;
		gap: 16px;
		align-items: flex-start;
	}

	.card {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 14px;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.card__eyebrow {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		letter-spacing: 0.1em;
		text-transform: uppercase;
	}
	.card__sample {
		font-size: 18px;
		line-height: 1.4;
		color: var(--fg);
	}
	.card__family {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
	}
	.card__note {
		font-size: 12px;
		color: var(--base6);
		line-height: 1.5;
	}

	.scale {
		margin-top: 18px;
		display: grid;
		grid-template-columns: repeat(7, 1fr);
		gap: 12px;
	}
	.scale__cell {
		display: flex;
		flex-direction: column;
		gap: 6px;
		padding: 10px;
		border: 1px solid var(--panel-border);
		border-radius: 3px;
	}
	.scale__token {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
	}
	.scale__px {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base6);
	}

	.role-eyebrow {
		margin-top: 14px;
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--base5);
		letter-spacing: 0.1em;
		text-transform: uppercase;
	}
	.role {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 8px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.role__row {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	.role__bar {
		width: 3px;
		height: 18px;
		border-radius: 1px;
	}
	.role__name {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--fg-alt);
	}
	.role__hex {
		font-size: 10px;
		color: var(--base6);
		font-family: var(--font-mono);
	}
	.role__desc {
		font-size: 11px;
		color: var(--base6);
	}

	.demo-frame {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		overflow: hidden;
		height: 220px;
		display: flex;
	}
	.demo-frame--col {
		flex-direction: column;
	}
	.demo-notes {
		flex: 1;
		padding: 14px;
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 11px;
		font-family: var(--font-mono);
		color: var(--base6);
	}
	.demo-notes__h {
		color: var(--fg-alt);
	}

	.demo__label {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		letter-spacing: 0.1em;
		text-transform: uppercase;
		margin-bottom: 6px;
	}

	.prim-frame {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		background: var(--panel-bg);
	}
	.prim-frame--row {
		display: flex;
	}
	.prim-frame--col {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 10px;
	}

	.sb {
		flex: 1;
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.sb__head {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.sb__title {
		font-size: 13px;
		color: var(--fg);
	}
	.sb__spacer {
		flex: 1;
	}
	.sb__by {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
	}
	.sb__body {
		font-size: 12px;
		color: var(--base6);
		line-height: 1.5;
	}
	.sb__check {
		display: flex;
		gap: 6px;
		align-items: center;
		font-size: 10px;
		font-family: var(--font-mono);
		color: var(--base5);
	}
	.sb__check input {
		accent-color: var(--blue);
	}

	.sr__head {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.sr__title {
		font-size: 13px;
		color: var(--fg);
	}
	.sr__score {
		background: rgba(180, 190, 130, 0.14);
		color: var(--green);
	}
	.sr__snippet {
		font-size: 12px;
		color: var(--base6);
		line-height: 1.5;
	}
	.sr__meta {
		display: flex;
		gap: 4px;
		font-size: 10px;
		font-family: var(--font-mono);
		color: var(--base5);
	}
	.sr__sep {
		color: var(--base3);
	}

	.cf {
		flex: 1;
		padding: 10px;
	}
	.cf__role {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
		text-transform: uppercase;
		letter-spacing: 0.1em;
		margin-bottom: 4px;
	}
	.cf__body {
		font-size: 13px;
		color: var(--fg);
		line-height: 1.5;
	}

	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
		align-items: center;
	}

	.mode {
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.mode__head {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
	}
	.mode__label {
		font-size: 14px;
		color: var(--fg);
		font-weight: 600;
	}
	.mode__tagline {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--base5);
	}
	.mode__grid {
		display: grid;
		gap: 4px;
		height: 80px;
	}
	.mode__col {
		background: var(--bg-alt);
		border: 1px solid var(--panel-border);
		border-radius: 2px;
	}
	.mode__col--center {
		background: var(--base1);
	}

	.foot {
		padding-top: 24px;
		border-top: 1px solid var(--panel-border);
		color: var(--base5);
		font-size: 12px;
		font-family: var(--font-mono);
		display: flex;
		justify-content: space-between;
	}
</style>
