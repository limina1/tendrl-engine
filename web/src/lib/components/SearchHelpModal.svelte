<script lang="ts">
	// Search-syntax cheat sheet. Opened by the `?` button on the search
	// panel (sits right before the gear that opens the knowledge-base
	// settings). Read-only reference — it teaches the query language the
	// parser in src/search.rs implements; it changes no state.
	//
	// Structured as progressive levels: a newcomer reads Level 1 and can
	// search; each level down adds a sharper mode (scope → meaning →
	// precise lookup → power operators), capped by the cross-cutting notes
	// (gotchas + the network-confirm workflow). Levels 1–2 open by
	// default; the rest fold so the modal isn't a wall of syntax.

	import { closeSearchHelp, searchHelpUI } from '$lib/search/search-config.svelte';

	// Which levels are expanded. The first two open; deeper ones fold.
	let open = $state<Record<string, boolean>>({
		l1: true,
		l2: true,
		l3: false,
		l4: false,
		l5: false,
		notes: false
	});

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') closeSearchHelp();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#snippet levelHead(key: string, badge: string, title: string, summary: string)}
	<button
		type="button"
		class="sh-sec-head"
		onclick={() => (open[key] = !open[key])}
		aria-expanded={open[key]}
	>
		<span class="sh-sec-arrow">{open[key] ? '▾' : '▸'}</span>
		<span class="sh-badge">{badge}</span>
		<span class="sh-sec-title">{title}</span>
		<span class="sh-sec-summary">{summary}</span>
	</button>
{/snippet}

{#snippet row(token: string, desc: string)}
	<div class="sh-row">
		<code class="sh-token">{token}</code>
		<span class="sh-desc">{desc}</span>
	</div>
{/snippet}

{#if searchHelpUI.open}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="sh-backdrop" onclick={closeSearchHelp} role="presentation">
		<div
			class="sh-modal"
			onclick={(e) => e.stopPropagation()}
			role="dialog"
			aria-modal="true"
			tabindex="-1"
		>
			<header class="sh-header">
				<h3 class="sh-title">Search syntax</h3>
				<button class="sh-close" onclick={closeSearchHelp} aria-label="Close">×</button>
			</header>

			<p class="sh-blurb">
				A query is a space-separated list of <em>filters</em>. With no filter
				you do a plain text match; add filters to narrow by meaning, kind,
				author, or a specific event. Each level below adds a sharper mode —
				start at the top, reach for the rest when you need them.
			</p>

			<div class="sh-scroll">
				<!-- Level 1 — just type ----------------------------------------- -->
				<section class="sh-sec">
					{@render levelHead('l1', '1', 'Just type', 'plain text match')}
					{#if open.l1}
						<div class="sh-sec-body">
							<p class="sh-lead">
								Type words to match them literally in event content. This is
								the default — no syntax required.
							</p>
							{@render row('nostr relay', 'events containing both words')}
							{@render row('"exact phrase"', 'match the phrase verbatim, in order')}
							<p class="sh-hint">
								Results are scoped to your knowledge-base defaults (the kinds
								shown by the <code>scope</code> strip). The gear edits them.
							</p>
						</div>
					{/if}
				</section>

				<!-- Level 2 — scope: kinds & authors ---------------------------- -->
				<section class="sh-sec">
					{@render levelHead('l2', '2', 'Scope it', 'kinds · authors · time')}
					{#if open.l2}
						<div class="sh-sec-body">
							<div class="sh-group">Kinds</div>
							{@render row('k:30040', 'only this event kind (repeat for several)')}
							<p class="sh-hint">
								Common kinds: <code>30040</code> publication index,
								<code>30041</code> section, <code>30023</code> long-form,
								<code>30818</code> wiki, <code>1</code> note, <code>0</code>
								profile.
							</p>

							<div class="sh-group">Authors</div>
							{@render row('by:me', 'events you signed')}
							{@render row('by:name:alice', 'author whose profile name contains “alice”')}
							{@render row('by:npub1…', 'a specific author by npub or 64-hex key')}

							<div class="sh-group">Time</div>
							{@render row('since:<ts>', 'on/after a unix timestamp')}
							{@render row('until:<ts>', 'on/before a unix timestamp')}
						</div>
					{/if}
				</section>

				<!-- Level 3 — semantic ------------------------------------------ -->
				<section class="sh-sec">
					{@render levelHead('l3', '3', 'Search by meaning', 'semantic · ~:')}
					{#if open.l3}
						<div class="sh-sec-body">
							<p class="sh-lead">
								<code>~:</code> runs a <strong>semantic</strong> search: instead
								of matching the exact words, it ranks events by how close their
								<em>meaning</em> is to your phrase, using the local embedding
								index. Good for “find things about X” when you don't know the
								wording.
							</p>
							{@render row('~:decentralized identity', 'nearest 10 events by meaning')}
							{@render row('~:"key management":5', 'cap to the 5 closest (quote multi-word)')}
							<p class="sh-hint">
								Needs the embedding index enabled (<em>Embedding</em> section of
								the knowledge-base settings). Text and semantic are different
								modes — pick by intent: exact wording vs. concept.
							</p>
						</div>
					{/if}
				</section>

				<!-- Level 4 — precise lookups ----------------------------------- -->
				<section class="sh-sec">
					{@render levelHead('l4', '4', 'Jump to a specific thing', 'ids · entities · people')}
					{#if open.l4}
						<div class="sh-sec-body">
							<p class="sh-lead">
								When you already hold an identifier, paste it — these resolve to
								an exact lookup and ignore the default scope.
							</p>

							<div class="sh-group">Events</div>
							{@render row('id:<64-hex>', 'one event by raw hex id')}
							{@render row('note1…', 'one event (NIP-19 note id)')}
							{@render row('nevent1…', 'one event + relay/author hints')}
							{@render row('naddr1…', 'a replaceable event by kind:pubkey:d-tag')}

							<div class="sh-group">People</div>
							{@render row('npub1…', 'resolve a profile by public key')}
							{@render row('nprofile1…', 'a profile + relay hints')}
							<p class="sh-hint">
								A leading <code>nostr:</code> is fine — <code>nostr:naddr1…</code>
								works the same.
							</p>
						</div>
					{/if}
				</section>

				<!-- Level 5 — power operators ----------------------------------- -->
				<section class="sh-sec">
					{@render levelHead('l5', '5', 'Power operators', 'tags · counts · OR')}
					{#if open.l5}
						<div class="sh-sec-body">
							<div class="sh-group">Tags</div>
							{@render row('has:NAME', 'events carrying any NAME tag')}
							{@render row('NAME:value', 'events with a NAME tag equal to value')}
							<p class="sh-hint">
								Bare key — <strong>no</strong> <code>#</code>. <code>title:…</code>,
								<code>t:…</code>, <code>a:…</code> all work this way.
							</p>

							<div class="sh-group">Aggregate</div>
							{@render row('count:NAME', 'histogram of distinct values for tag NAME')}

							<div class="sh-group">Combine</div>
							{@render row('a | b', 'OR — events matching either branch, each scoped on its own')}
						</div>
					{/if}
				</section>

				<!-- Cross-cutting notes ----------------------------------------- -->
				<section class="sh-sec">
					{@render levelHead('notes', '!', 'Gotchas & fetching', 'redundancies · network mode')}
					{#if open.notes}
						<div class="sh-sec-body">
							<div class="sh-group">Easy to confuse</div>
							<ul class="sh-list">
								<li>
									<code>by:</code> filters the <em>publishing pubkey</em>;
									<code>author:</code> is a <em>tag</em> filter (events with an
									<code>["author", …]</code> tag) — not the same thing.
								</li>
								<li>
									<code>NAME:value</code> is a tag filter; <code>#NAME:value</code>
									is <em>not</em> — the <code>#</code> form parses as literal text.
								</li>
								<li>
									A bare word after <code>by:</code> (<code>by:alice</code>)
									resolves as a name partial, same as <code>by:name:alice</code>.
								</li>
							</ul>

							<div class="sh-group">Local vs. relays</div>
							<p class="sh-lead">
								Searches read your local store first. Reaching out to relays is
								governed by the network mode (mode-line, top right):
							</p>
							<ul class="sh-list">
								<li>
									<strong>Auto</strong> — relay fetches run automatically.
								</li>
								<li>
									<strong>Confirm</strong> — each relay fetch asks first; approve
									the intent and the same query replays against relays.
								</li>
							</ul>
							<p class="sh-hint">
								So a query can return more after you confirm a relay fetch — the
								local pass and the relay pass use the identical filters.
							</p>
						</div>
					{/if}
				</section>
			</div>

			<footer class="sh-footer">
				<span class="sh-foot-hint">Esc to close</span>
				<span class="sh-spacer"></span>
				<button class="sh-action sh-action--primary" onclick={closeSearchHelp}>Got it</button>
			</footer>
		</div>
	</div>
{/if}

<style>
	.sh-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.55);
		z-index: 250;
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.sh-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 90vw;
		max-width: 540px;
		max-height: 88vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.sh-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
	}
	.sh-title {
		margin: 0;
		font-size: var(--t-sm);
		color: var(--affordance-help);
	}
	.sh-close {
		background: transparent;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
	}
	.sh-close:hover {
		color: var(--fg);
	}
	.sh-blurb {
		margin: 0;
		padding: 8px 14px;
		color: var(--base5);
		line-height: 1.5;
		border-bottom: 1px solid var(--panel-border);
	}
	.sh-hint code,
	.sh-list code,
	.sh-lead code {
		background: transparent;
		color: var(--id-yours);
	}

	.sh-scroll {
		overflow-y: auto;
	}

	/* Collapsible level */
	.sh-sec {
		border-bottom: 1px solid var(--panel-border);
	}
	.sh-sec-head {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 8px 14px;
		background: transparent;
		border: none;
		cursor: pointer;
		font: inherit;
		text-align: left;
		color: var(--id-yours);
	}
	.sh-sec-head:hover {
		background: var(--bg-surface);
	}
	.sh-sec-arrow {
		color: var(--base5);
		width: 10px;
		flex-shrink: 0;
	}
	.sh-badge {
		flex-shrink: 0;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 16px;
		height: 16px;
		border-radius: 3px;
		border: 1px solid var(--id-yours);
		color: var(--id-yours);
		font-size: calc(var(--t-xs) - 1px);
	}
	.sh-sec-title {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 1px);
	}
	.sh-sec-summary {
		margin-left: auto;
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.sh-sec-body {
		padding: 4px 14px 12px;
	}

	.sh-lead {
		margin: 4px 0 8px;
		color: var(--fg);
		line-height: 1.5;
	}
	.sh-group {
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--id-yours);
		margin: 10px 0 5px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.sh-group:first-child {
		margin-top: 2px;
	}

	/* token → description rows */
	.sh-row {
		display: flex;
		gap: 10px;
		align-items: baseline;
		padding: 2px 0;
	}
	.sh-token {
		flex-shrink: 0;
		min-width: 130px;
		color: var(--state-online);
		background: transparent;
	}
	.sh-desc {
		color: var(--base6);
		line-height: 1.45;
	}

	.sh-list {
		margin: 4px 0 0;
		padding-left: 18px;
		color: var(--base6);
		line-height: 1.55;
	}
	.sh-list li {
		margin: 3px 0;
	}
	.sh-list code,
	.sh-row .sh-token {
		font-family: var(--font-mono);
	}

	.sh-hint {
		margin: 8px 0 0;
		color: var(--base5);
		line-height: 1.5;
	}

	.sh-footer {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 10px 14px;
		border-top: 1px solid var(--panel-border);
	}
	.sh-foot-hint {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.sh-spacer {
		flex: 1;
	}
	.sh-action {
		font: inherit;
		padding: 5px 14px;
		border-radius: var(--r-sm);
		border: 1px solid var(--panel-border);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.sh-action--primary {
		border-color: var(--state-online);
		color: var(--state-online);
	}
	.sh-action--primary:hover {
		background: color-mix(in srgb, var(--state-online) 18%, transparent);
	}
</style>
