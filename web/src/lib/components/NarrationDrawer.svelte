<script lang="ts">
	// Recordings drawer: every kind-32176 narration bound to this
	// publication's nodes, grouped by section in tree order, ranked by the
	// engine (exact version → other version → unpinned). Per recording:
	// play, mirror to my Blossom server, show the transcript (with a
	// sentence diff against the current text when it is another version),
	// and "narrate this myself". Same chrome as HighlightsDrawer.
	import * as api from '$lib/api';
	import * as nar from '$lib/narration.svelte';
	import { getAppState } from '$lib/state.svelte';
	import { getAuthorDisplayName } from '$lib/discussions/authors.svelte';
	import { identityCanSign } from '$lib/identity/signer';

	const app = getAppState();

	let {
		pubkey,
		d_tag,
		pubTitle,
		open,
		onclose,
		onnavigate
	}: {
		pubkey: string;
		d_tag: string;
		pubTitle: string | null;
		open: boolean;
		onclose: () => void;
		/** Scroll / page the reader to a section coordinate. */
		onnavigate: (coord: string) => void;
	} = $props();

	const canSign = $derived(identityCanSign(app.identityStatus));
	const discovery = $derived(nar.discoveryFor(pubkey, d_tag));
	const loading = $derived(nar.isLoading(pubkey, d_tag));
	const nodes = $derived((discovery?.nodes ?? []).filter((n) => !n.is_index));
	const total = $derived(nodes.reduce((a, n) => a + n.recordings.length, 0));
	const servers = $derived(discovery?.servers ?? []);

	let openTranscript = $state<string | null>(null);
	let openSources = $state<string | null>(null);
	let mirroring = $state<Record<string, boolean>>({});
	/** HEAD results per URL: true = served, false = failed, undefined = unchecked. */
	let reach = $state<Record<string, boolean | 'checking'>>({});
	const rootNode = $derived((discovery?.nodes ?? []).find((n) => n.is_index && n.depth === 0) ?? null);

	function host(u: string): string {
		try {
			return new URL(u).host;
		} catch {
			return u;
		}
	}

	/** Open the kind-32176 event itself in the structured event viewer —
	 *  the same inspector highlights and comments get. */
	function inspect(r: api.NarrationRecording) {
		void app.openAddressableInModal({ kind: 32176, pubkey: r.pubkey, d_tag: r.d_tag });
	}

	/** Probe every candidate URL (BUD-01 servers answer HEAD with open CORS). */
	async function checkSources(r: api.NarrationRecording) {
		const all = (r.piece_urls?.length ? r.piece_urls : [r.urls]).flat();
		await Promise.all(
			all.map(async (u) => {
				reach[u] = 'checking';
				try {
					const res = await fetch(u, { method: 'HEAD', mode: 'cors' });
					reach[u] = res.ok;
				} catch {
					reach[u] = false;
				}
			})
		);
	}

	async function pull() {
		await nar.refresh(pubkey, d_tag, 'fetch_always');
	}

	function play(n: api.NarrationNode, r: api.NarrationRecording) {
		nar.playRecording(n.coord, n.title, r, pubTitle);
	}

	function playAllFrom(n: api.NarrationNode) {
		nar.playPublication(pubkey, d_tag, pubTitle, n.coord);
	}

	async function mirror(r: api.NarrationRecording) {
		const firsts = (r.piece_urls?.length ? r.piece_urls : [r.urls]).map((u) => u[0]).filter(Boolean);
		if (firsts.length === 0) return;
		mirroring[r.id] = true;
		try {
			const all = await Promise.all(firsts.map((url) => api.mirrorNarration({ url })));
			const results = all.flatMap((x) => x.results);
			const ok = results.filter((x) => x.ok);
			if (ok.length > 0) app.pushToast(`mirrored ${firsts.length > 1 ? `${firsts.length} blobs ` : ''}to ${[...new Set(ok.map((x) => x.server.replace(/^https?:\/\//, '')))].join(', ')}`, 'success');
			const bad = results.filter((x) => !x.ok);
			if (bad.length > 0) app.pushToast(bad.map((x) => `${x.server}: ${x.error}`).join(' · '), 'error', 6000);
		} catch (e) {
			app.pushToast(e instanceof Error ? e.message : String(e), 'error', 6000);
		} finally {
			mirroring[r.id] = false;
		}
	}

	function narrate(coord?: string) {
		app.narrateModal = { pubkey, d_tag, title: pubTitle, only: coord ? [coord] : undefined };
	}

	async function mirrorTo(r: api.NarrationRecording, server: string) {
		const firsts = (r.piece_urls?.length ? r.piece_urls : [r.urls]).map((u) => u[0]).filter(Boolean);
		if (firsts.length === 0) return;
		mirroring[r.id] = true;
		try {
			const all = await Promise.all(firsts.map((url) => api.mirrorNarration({ url, server })));
			const bad = all.flatMap((x) => x.results).filter((x) => !x.ok);
			if (bad.length === 0) app.pushToast(`mirrored to ${host(server)}`, 'success');
			else app.pushToast(`${host(server)}: ${bad[0].error ?? 'failed'}`, 'error', 6000);
		} catch (e) {
			app.pushToast(e instanceof Error ? e.message : String(e), 'error', 6000);
		} finally {
			mirroring[r.id] = false;
		}
	}

	/** Sentence-level LCS diff: transcript (a) vs current spoken text (b). */
	function sentences(s: string): string[] {
		return s
			.replace(/\s+/g, ' ')
			.split(/(?<=[.!?…])\s+/)
			.map((x) => x.trim())
			.filter(Boolean);
	}
	type Op = { t: 'same' | 'del' | 'add'; s: string };
	function diff(a: string, b: string): Op[] {
		const A = sentences(a);
		const B = sentences(b);
		const n = A.length;
		const m = B.length;
		if (n * m > 400_000) {
			// Too big for a cheap LCS; show as whole-sale replace.
			return [...A.map((s) => ({ t: 'del', s }) as Op), ...B.map((s) => ({ t: 'add', s }) as Op)];
		}
		const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
		for (let i = n - 1; i >= 0; i--)
			for (let j = m - 1; j >= 0; j--) dp[i][j] = A[i] === B[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
		const out: Op[] = [];
		let i = 0;
		let j = 0;
		while (i < n && j < m) {
			if (A[i] === B[j]) {
				out.push({ t: 'same', s: A[i] });
				i++;
				j++;
			} else if (dp[i + 1][j] >= dp[i][j + 1]) out.push({ t: 'del', s: A[i++] });
			else out.push({ t: 'add', s: B[j++] });
		}
		while (i < n) out.push({ t: 'del', s: A[i++] });
		while (j < m) out.push({ t: 'add', s: B[j++] });
		return out;
	}
	function changedCount(ops: Op[]): number {
		return ops.filter((o) => o.t !== 'same').length;
	}
</script>

<aside class="nd-drawer" class:open aria-hidden={!open} aria-label="Recordings drawer">
	<header class="nd-head">
		<h3 class="nd-title">Recordings ({total})</h3>
		<div class="nd-head-actions">
			{#if nodes.some((n) => n.recordings.length > 0)}
				<button class="nd-act" onclick={() => nar.playPublication(pubkey, d_tag, pubTitle)} title="Play every narrated section in order">▶ all</button>
			{/if}
			<button class="nd-act" onclick={pull} disabled={loading} title={app.networkStatus?.mode === 'auto' ? 'Pull recordings from relays' : 'Confirm mode — pulling recordings raises the fetch modal'}>{loading ? '…' : '⟳ relays'}</button>
			{#if canSign}
				<button class="nd-act nd-act--go" onclick={() => narrate()} title="Generate a narration of this publication with ppq.ai TTS">narrate</button>
			{/if}
			<button class="nd-act" onclick={onclose} aria-label="Close">×</button>
		</div>
	</header>
	<div class="nd-body">
		{#if nodes.length === 0}
			<div class="nd-empty">{loading ? 'looking for recordings…' : 'publication not loaded yet'}</div>
		{:else if total === 0}
			<div class="nd-empty">
				No recordings found locally.
				<button class="nd-link" onclick={pull}>Pull from relays</button>{#if canSign} or <button class="nd-link" onclick={() => narrate()}>narrate it</button>{/if}.
			</div>
		{:else}
			{#if rootNode && rootNode.recordings.length > 0}
				<!-- The root 32176s: one per narrator+voice, each a TOC of leaf
				     coordinates. Inspect shows the a-children, A/E/K and blossom hints. -->
				<section class="nd-sec nd-sec--root">
					<div class="nd-sec__title nd-sec__title--static">index narrations</div>
					{#each rootNode.recordings as r (r.id)}
						<div class="nd-rec">
							<span class="nd-play nd-play--ghost" aria-hidden="true">≡</span>
							<div class="nd-rec__main">
								<div class="nd-rec__line">
									<span class="nd-who" title={r.pubkey}>{r.mine ? 'you' : getAuthorDisplayName(r.pubkey)}</span>
									{#if r.voice}<span class="nd-dim">· {r.voice}</span>{/if}
									<span class="nd-dim">· {r.children.length} section{r.children.length === 1 ? '' : 's'}</span>
									{#if r.duration_secs}<span class="nd-dim">· {nar.fmtTime(r.duration_secs)}</span>{/if}
								</div>
								<div class="nd-rec__line">
									<span class="nd-pill nd-pill--{r.version_match}">{nar.matchLabel(r.version_match)}</span>
									{#if r.relays.length === 0}<span class="nd-pill">local</span>{:else}<span class="nd-pill" title={r.relays.join('\n')}>{r.relays.length} relay{r.relays.length === 1 ? '' : 's'}</span>{/if}
								</div>
							</div>
							<div class="nd-rec__acts">
								<button class="nd-act" onclick={() => inspect(r)} title="Open the kind-32176 index event">⋮</button>
							</div>
						</div>
					{/each}
				</section>
			{/if}
			{#each nodes as n (n.coord)}
				<section class="nd-sec" style="--d:{Math.max(0, n.depth - 1)}">
					<div class="nd-sec__head">
						<button class="nd-sec__title" onclick={() => onnavigate(n.coord)} title="Go to this section">{n.title}</button>
						{#if canSign}
							<button class="nd-act nd-act--tiny" onclick={() => narrate(n.coord)} title="Narrate only this section (opens the narrate modal with it preselected)">narrate §</button>
						{/if}
					</div>
					{#if n.recordings.length === 0}
						<div class="nd-none">no recording yet</div>
					{:else}
						{#each n.recordings as r (r.id)}
							{@const active = nar.activeRecording(n.coord)?.id === r.id}
							{@const playing = nar.isPlayingCoord(n.coord) && active}
							<div class="nd-rec" class:nd-rec--active={active}>
								<button class="nd-play" onclick={() => (playing ? nar.pause() : play(n, r))} title={playing ? 'Pause' : 'Play this recording'}>{playing ? '⏸' : '▶'}</button>
								<div class="nd-rec__main">
									<div class="nd-rec__line">
										<span class="nd-who" title={r.pubkey}>{r.mine ? 'you' : getAuthorDisplayName(r.pubkey)}</span>
										{#if r.voice}<span class="nd-dim">· {r.voice}</span>{/if}
										{#if r.duration_secs}<span class="nd-dim">· {nar.fmtTime(r.duration_secs)}</span>{/if}
									</div>
									<div class="nd-rec__line">
										<span class="nd-pill nd-pill--{r.version_match}" title={r.version_match === 'exact' ? 'Recorded from the text you are reading' : r.version_match === 'other_version' ? 'Recorded from another version of this section' : 'Not pinned to a version'}>{nar.matchLabel(r.version_match)}</span>
										{#if r.relays.length === 0}<span class="nd-pill" title="Signed here, not yet on any relay">local</span>{:else}<span class="nd-pill" title={r.relays.join('\n')}>{r.relays.length} relay{r.relays.length === 1 ? '' : 's'}</span>{/if}
										<button class="nd-link nd-link--dim" class:active={openSources === r.id} onclick={() => (openSources = openSources === r.id ? null : r.id)} title="Where the audio can be fetched from">{r.urls.length} source{r.urls.length === 1 ? '' : 's'}</button>
									</div>
								</div>
								<div class="nd-rec__acts">
									{#if r.transcript}
										<button class="nd-act" class:active={openTranscript === r.id} onclick={() => (openTranscript = openTranscript === r.id ? null : r.id)} title="Spoken text">T</button>
									{/if}
									{#if canSign && servers.length > 0}
										<button class="nd-act" onclick={() => mirror(r)} disabled={mirroring[r.id]} title="Copy this blob to my Blossom server(s) (BUD-04 mirror)">{mirroring[r.id] ? '…' : '⇅'}</button>
									{/if}
									{#if !active}
										<button class="nd-act" onclick={() => nar.choose(n.coord, r.id)} title="Use this recording for this section">use</button>
									{/if}
									<button class="nd-act" onclick={() => playAllFrom(n)} title="Play the book from here">▶▶</button>
									<button class="nd-act" onclick={() => inspect(r)} title="Open the kind-32176 event (tags, A/E/K binding, blossom hints, raw JSON)">⋮</button>
								</div>
							</div>
							{#if openSources === r.id}
								<div class="nd-src">
									<div class="nd-src__head">
										<span>{r.pieces.length > 1 ? `${r.pieces.length} blobs` : 'blob'} <code title={r.pieces[0]?.sha256}>{r.pieces[0]?.sha256.slice(0, 12)}…</code>{#if r.size} · {nar.fmtBytes(r.size)}{/if}</span>
										<span class="nd-sp"></span>
										<button class="nd-link" onclick={() => checkSources(r)}>check</button>
									</div>
									{#each r.piece_urls?.length ? r.piece_urls : [r.urls] as urls, pi (pi)}
									{#if r.pieces.length > 1}
										<div class="nd-src__piece">piece {pi + 1}/{r.pieces.length} · <code title={r.pieces[pi]?.sha256}>{r.pieces[pi]?.sha256.slice(0, 10)}…</code>{#if r.pieces[pi]?.size} · {nar.fmtBytes(r.pieces[pi].size)}{/if}</div>
									{/if}
									{#each urls as u (u)}
										<div class="nd-src__row">
											<span class="nd-src__state" title={reach[u] === true ? 'served' : reach[u] === false ? 'not found here' : reach[u] === 'checking' ? 'checking' : 'unchecked'}>{reach[u] === true ? '✓' : reach[u] === false ? '✗' : reach[u] === 'checking' ? '…' : '·'}</span>
											<a class="nd-src__url" href={u} target="_blank" rel="noopener" title={u}>{host(u)}</a>
											{#if r.servers.some((s) => host(s) === host(u))}<span class="nd-dim">hint</span>{:else}<span class="nd-dim">your set</span>{/if}
											{#if canSign && !r.servers.some((s) => host(s) === host(u))}
												<button class="nd-act nd-act--tiny" onclick={() => mirrorTo(r, u.replace(/\/[^/]*$/, ''))} disabled={mirroring[r.id]} title="Mirror the blob to this server">mirror here</button>
											{/if}
										</div>
									{/each}
									{/each}
									{#if r.relays.length > 0}
										<div class="nd-src__row nd-src__row--relays">event on: {r.relays.map((x) => x.replace(/^wss?:\/\//, '')).join(', ')}</div>
									{/if}
								</div>
							{/if}
							{#if openTranscript === r.id && r.transcript}
								{@const ops = r.version_match === 'exact' || !n.spoken ? null : diff(r.transcript, n.spoken)}
								<div class="nd-tr">
									{#if ops}
										{@const changed = changedCount(ops)}
										<div class="nd-tr__head">
											{#if changed === 0}spoken text matches the current text{:else}{changed} sentence{changed === 1 ? '' : 's'} differ — <s>spoken</s> / <ins>current</ins>{/if}
											{#if canSign}· <button class="nd-link" onclick={() => narrate(n.coord)}>narrate the current text</button>{/if}
										</div>
										<p class="nd-tr__body">
											{#each ops as o, i (i)}
												{#if o.t === 'same'}<span>{o.s} </span>{:else if o.t === 'del'}<s>{o.s} </s>{:else}<ins>{o.s} </ins>{/if}
											{/each}
										</p>
									{:else}
										<p class="nd-tr__body">{r.transcript}</p>
									{/if}
								</div>
							{/if}
						{/each}
					{/if}
				</section>
			{/each}
		{/if}
	</div>
</aside>

<style>
	.nd-drawer {
		position: fixed;
		right: 16px;
		bottom: 16px;
		width: 360px;
		max-width: 94vw;
		max-height: 64vh;
		background: var(--bg-panel, var(--bg-surface));
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		display: flex;
		flex-direction: column;
		box-shadow: var(--shadow-md);
		transform: translateX(calc(100% + 32px));
		transition: transform 180ms ease;
		z-index: 200;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
	}
	.nd-drawer.open {
		transform: none;
	}
	.nd-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 8px 10px;
		border-bottom: 1px solid var(--panel-border);
		gap: 6px;
	}
	.nd-title {
		margin: 0;
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--state-online);
		white-space: nowrap;
	}
	.nd-head-actions {
		display: flex;
		align-items: center;
		gap: 4px;
	}
	.nd-act {
		background: none;
		border: 1px solid transparent;
		border-radius: var(--r-sm);
		color: var(--fg-muted);
		cursor: pointer;
		font: inherit;
		padding: 1px 6px;
	}
	.nd-act:hover:not(:disabled),
	.nd-act.active {
		color: var(--fg);
		border-color: var(--panel-border);
	}
	.nd-act:disabled {
		opacity: 0.4;
		cursor: default;
	}
	.nd-act--go {
		color: var(--id-yours);
	}
	.nd-body {
		overflow-y: auto;
		padding: 6px 0;
	}
	.nd-empty,
	.nd-none {
		color: var(--fg-muted);
		padding: 4px 10px;
	}
	.nd-none {
		padding-left: 28px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.nd-link {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: var(--id-yours);
		cursor: pointer;
		text-decoration: underline;
	}
	.nd-sec {
		padding: 4px 10px 4px calc(10px + var(--d, 0) * 10px);
		border-bottom: 1px dashed var(--panel-border);
	}
	.nd-sec:last-child {
		border-bottom: none;
	}
	.nd-sec__title {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: var(--fg);
		cursor: pointer;
		text-align: left;
		width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nd-sec__title:hover {
		text-decoration: underline;
	}
	.nd-sec__head {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.nd-sec__head .nd-sec__title {
		flex: 1 1 auto;
		width: auto;
	}
	.nd-sec__title--static {
		cursor: default;
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 2px);
	}
	.nd-sec__title--static:hover {
		text-decoration: none;
	}
	.nd-sec--root {
		background: var(--bg-surface);
	}
	.nd-act--tiny {
		font-size: calc(var(--t-xs) - 2px);
		padding: 0 4px;
	}
	.nd-play--ghost {
		border-color: transparent;
		color: var(--fg-muted);
		cursor: default;
	}
	.nd-link--dim {
		color: var(--fg-muted);
		text-decoration: none;
		font-size: calc(var(--t-xs) - 1px);
	}
	.nd-link--dim:hover,
	.nd-link--dim.active {
		color: var(--fg);
		text-decoration: underline;
	}
	.nd-src {
		margin: 2px 0 6px 28px;
		padding: 6px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-size: calc(var(--t-xs) - 1px);
	}
	.nd-src__head {
		display: flex;
		gap: 6px;
		align-items: center;
		color: var(--fg-muted);
		margin-bottom: 4px;
	}
	.nd-src__head code {
		font-family: var(--font-mono);
		color: var(--fg);
	}
	.nd-src__row {
		display: flex;
		gap: 6px;
		align-items: center;
		min-width: 0;
	}
	.nd-src__piece {
		color: var(--fg-muted);
		margin-top: 4px;
	}
	.nd-src__piece code {
		font-family: var(--font-mono);
		color: var(--fg);
	}
	.nd-src__row--relays {
		color: var(--fg-muted);
		margin-top: 4px;
		white-space: normal;
	}
	.nd-src__state {
		width: 1em;
		text-align: center;
		color: var(--fg-muted);
	}
	.nd-src__url {
		color: var(--fg);
		text-decoration: none;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nd-src__url:hover {
		text-decoration: underline;
	}
	.nd-rec {
		display: flex;
		align-items: flex-start;
		gap: 6px;
		padding: 3px 0 3px 4px;
		border-left: 2px solid transparent;
	}
	.nd-rec--active {
		border-left-color: var(--state-online, var(--accent));
	}
	.nd-play {
		background: none;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--state-online, var(--accent));
		cursor: pointer;
		font: inherit;
		padding: 0 6px;
		line-height: 1.6;
	}
	.nd-rec__main {
		flex: 1 1 auto;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 1px;
	}
	.nd-rec__line {
		display: flex;
		gap: 4px;
		align-items: center;
		flex-wrap: wrap;
		min-width: 0;
	}
	.nd-who {
		color: var(--fg);
	}
	.nd-dim {
		color: var(--fg-muted);
		font-size: calc(var(--t-xs) - 1px);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nd-pill {
		font-size: calc(var(--t-xs) - 2px);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 0 5px;
		color: var(--fg-muted);
	}
	.nd-pill--exact {
		color: var(--state-online, var(--accent));
		border-color: var(--state-online, var(--accent));
	}
	.nd-pill--other_version {
		color: var(--state-warn, #c90);
		border-color: var(--state-warn, #c90);
	}
	.nd-rec__acts {
		display: flex;
		gap: 2px;
		align-items: center;
	}
	.nd-tr {
		margin: 2px 0 6px 28px;
		padding: 6px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		max-height: 30vh;
		overflow-y: auto;
	}
	.nd-tr__head {
		color: var(--fg-muted);
		margin-bottom: 4px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.nd-tr__body {
		margin: 0;
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		line-height: 1.5;
		color: var(--fg);
		white-space: pre-wrap;
	}
	.nd-tr__body s {
		color: var(--error, #e06c75);
		opacity: 0.8;
	}
	.nd-tr__body ins {
		text-decoration: none;
		color: var(--state-online, var(--accent));
	}
</style>
