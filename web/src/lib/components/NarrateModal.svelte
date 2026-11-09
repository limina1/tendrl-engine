<script lang="ts">
	// Narrate a publication: pick a TTS model + voice, see the per-section
	// plan and the cost against the ppq balance, then run. The engine does
	// everything (chunk → ppq TTS → join → Blossom upload → kind-32176 leaf
	// per section → root), streaming progress here. Opened from the reader
	// tools sheet or the recordings drawer via `app.narrateModal`.
	import * as api from '$lib/api';
	import * as nar from '$lib/narration.svelte';
	import { getAppState } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();
	const req = $derived(app.narrateModal);
	const canSign = $derived(identityCanSign(app.identityStatus));

	let models = $state<api.TtsModel[]>([]);
	let ppqConfigured = $state(true);
	let model = $state('');
	/** Every voice the selected model offers (all languages). */
	let allVoices = $state<api.TtsVoice[]>([]);
	let voice = $state('');
	/** ISO 639-1 code, or '' = any language (provider picks). */
	let language = $state('');

	// Languages are whatever the model's voices actually cover; the voice
	// list is that set filtered by the chosen language. Names via the
	// browser's locale data, code as fallback.
	const langNames = typeof Intl !== 'undefined' && 'DisplayNames' in Intl ? new Intl.DisplayNames(undefined, { type: 'language' }) : null;
	function langName(code: string): string {
		try {
			return langNames?.of(code) ?? code;
		} catch {
			return code;
		}
	}
	const languages = $derived.by(() => {
		const counts = new Map<string, number>();
		for (const v of allVoices) {
			const c = (v.language ?? '').trim().toLowerCase();
			if (!c) continue;
			counts.set(c, (counts.get(c) ?? 0) + 1);
		}
		return [...counts.entries()]
			.map(([code, n]) => ({ code, n, name: langName(code) }))
			.sort((a, b) => (a.code === 'en' ? -1 : b.code === 'en' ? 1 : a.name.localeCompare(b.name)));
	});
	const voices = $derived(
		language ? allVoices.filter((v) => (v.language ?? '').trim().toLowerCase() === language) : allVoices
	);
	let servers = $state<string[]>([]);
	let server = $state('');
	let transcript = $state(true);
	let broadcast = $state(true);
	let force = $state(false);
	let estimate = $state<api.NarrationEstimate | null>(null);
	let estimating = $state(false);
	let loadErr = $state<string | null>(null);
	/** Section coordinates ticked for narration (empty set = all). */
	let picked = $state<Record<string, boolean>>({});

	// Hearing a voice before spending on a whole book: ppq's canned clip per
	// voice (free, `preview_url`) and a paid sample of the user's own text.
	let previewAudio: HTMLAudioElement | undefined = $state();
	let previewing = $state<'clip' | 'sample' | null>(null);
	let previewErr = $state<string | null>(null);
	let sampleUrl: string | null = null;
	const selectedVoice = $derived(voices.find((v) => v.id === voice) ?? null);
	const sampleText = $derived.by(() => {
		const first = sectionNodes.find((n) => picked[n.coord] !== false) ?? sectionNodes[0];
		return first?.preview ?? '';
	});
	const sampleCost = $derived.by(() => {
		const m = models.find((x) => x.id === model);
		return m ? Math.max((300 / 1000) * m.price_per_1k, m.min_per_request ?? 0) : 0;
	});

	function stopPreview() {
		previewAudio?.pause();
		previewing = null;
		if (sampleUrl) {
			URL.revokeObjectURL(sampleUrl);
			sampleUrl = null;
		}
	}
	function playUrl(url: string, kind: 'clip' | 'sample') {
		if (!previewAudio) return;
		previewAudio.src = url;
		previewing = kind;
		previewAudio.play().catch((e) => {
			previewErr = e instanceof Error ? e.message : String(e);
			previewing = null;
		});
	}
	function playClip() {
		previewErr = null;
		if (previewing === 'clip') return stopPreview();
		stopPreview();
		if (selectedVoice?.preview_url) playUrl(selectedVoice.preview_url, 'clip');
	}
	async function playSample() {
		previewErr = null;
		if (previewing === 'sample') return stopPreview();
		stopPreview();
		previewing = 'sample';
		try {
			sampleUrl = await api.sampleNarration({ model, voice: voice || undefined, language: language || undefined, text: sampleText });
			playUrl(sampleUrl, 'sample');
			void app.refreshProviders();
		} catch (e) {
			previewErr = e instanceof Error ? e.message : String(e);
			previewing = null;
		}
	}

	let running = $state(false);
	let log = $state<{ kind: 'info' | 'ok' | 'warn' | 'err'; text: string }[]>([]);
	let progress = $state<{ section: number; sections: number; chunk: number; chunks: number } | null>(null);
	let done = $state<Extract<api.NarrationGenEvent, { type: 'done' }> | null>(null);
	let abort: AbortController | null = null;

	// Confirm mode: the broadcast at the end raises a publish_intent modal
	// (z 250). Yield to it like the highlighter does.
	const yielding = $derived(running && confirmState.intent?.type === 'publish_intent');

	const selectedModel = $derived(models.find((m) => m.id === model) ?? null);
	const sectionNodes = $derived((estimate?.nodes ?? []).filter((n) => !n.is_index && n.chars > 0));
	const pickedCoords = $derived(sectionNodes.filter((n) => picked[n.coord] !== false).map((n) => n.coord));
	const pickedChars = $derived(sectionNodes.filter((n) => picked[n.coord] !== false).reduce((a, n) => a + n.chars, 0));
	const pickedChunks = $derived(sectionNodes.filter((n) => picked[n.coord] !== false).reduce((a, n) => a + n.chunks, 0));
	const cost = $derived.by(() => {
		if (!selectedModel) return 0;
		const byChars = (pickedChars / 1000) * selectedModel.price_per_1k;
		const min = selectedModel.min_per_request;
		return min ? Math.max(byChars, min * pickedChunks) : byChars;
	});
	const balance = $derived(estimate?.balance_usd ?? null);
	const underfunded = $derived(balance !== null && cost > balance);
	const canRun = $derived(
		canSign && ppqConfigured && !running && !!selectedModel && pickedCoords.length > 0 && !underfunded && !!server
	);

	function usd(v: number): string {
		return v < 0.01 ? `$${v.toFixed(4)}` : `$${v.toFixed(2)}`;
	}

	async function loadVoices() {
		try {
			const r = await api.getNarrationVoices({ model });
			allVoices = r.voices;
		} catch {
			allVoices = [];
		}
		// A language the new model does not offer falls back to "any".
		if (language && !languages.some((l) => l.code === language)) language = '';
		ensureVoice();
	}

	/** Keep the picked voice inside the filtered list. */
	function ensureVoice() {
		if (voices.some((v) => v.id === voice)) return;
		voice = voices.find((v) => v.id === 'aura-2-thalia-en')?.id ?? voices[0]?.id ?? '';
	}

	function onLanguageChange() {
		stopPreview();
		ensureVoice();
	}

	async function loadEstimate() {
		if (!req || !model) return;
		estimating = true;
		try {
			estimate = await api.estimateNarration({ pubkey: req.pubkey, d_tag: req.d_tag, model });
			if (req.only?.length) {
				const only = new Set(req.only);
				for (const n of estimate.nodes) if (!n.is_index) picked[n.coord] = only.has(n.coord);
			}
			loadErr = null;
		} catch (e) {
			loadErr = e instanceof Error ? e.message : String(e);
		} finally {
			estimating = false;
		}
	}

	$effect(() => {
		// Initial load, once per open.
		void req;
		(async () => {
			try {
				const m = await api.getNarrationModels();
				models = m.models;
				ppqConfigured = m.ppq_configured;
				model = m.default;
				voice = m.default_voice;
			} catch (e) {
				loadErr = e instanceof Error ? e.message : String(e);
			}
			try {
				const s = await api.getNarrationServers();
				servers = s.servers;
				server = s.servers[0] ?? '';
			} catch {
				servers = [];
			}
			await Promise.all([loadVoices(), loadEstimate()]);
		})();
	});

	function onModelChange() {
		void loadVoices();
		void loadEstimate();
	}

	function close() {
		if (running) return;
		stopPreview();
		app.narrateModal = null;
	}

	async function run() {
		if (!req || !canRun) return;
		running = true;
		done = null;
		log = [];
		progress = null;
		abort = new AbortController();
		const only = pickedCoords.length === sectionNodes.length ? undefined : pickedCoords;
		try {
			await api.streamNarration(
				{
					pubkey: req.pubkey,
					d_tag: req.d_tag,
					model,
					voice: voice || undefined,
					language: language || undefined,
					force,
					broadcast,
					transcript,
					server: server || undefined,
					only
				},
				(ev) => {
					switch (ev.type) {
						case 'plan':
							log.push({ kind: 'info', text: `plan: ${ev.total_chars} chars in ${ev.total_chunks} calls → ${ev.server}` });
							break;
						case 'section_start':
							progress = { section: ev.index + 1, sections: ev.total, chunk: 0, chunks: ev.chunks };
							log.push({ kind: 'info', text: `§ ${ev.index + 1}/${ev.total} ${ev.title}` });
							break;
						case 'chunk':
							if (progress) progress = { ...progress, chunk: ev.index + 1, chunks: ev.total };
							break;
						case 'skipped':
							log.push({ kind: 'info', text: `  skipped — ${ev.reason}` });
							break;
						case 'uploaded':
							log.push({
								kind: 'ok',
								text: `  uploaded${ev.pieces > 1 ? ` piece ${ev.piece + 1}/${ev.pieces}` : ''} ${nar.fmtBytes(ev.size)}${ev.duration_secs ? ` · ${nar.fmtTime(ev.duration_secs)}` : ''} → ${(() => { try { return new URL(ev.url).host; } catch { return ev.url; } })()}`
							});
							break;
						case 'signed':
							log.push({ kind: 'ok', text: `  signed ${ev.narration.slice(0, 14)}…` });
							break;
						case 'broadcast':
							log.push({ kind: ev.successful > 0 ? 'ok' : 'warn', text: `broadcast: ${ev.successful}/${ev.total} relay results` });
							break;
						case 'error':
							log.push({ kind: 'err', text: ev.message });
							break;
						case 'done':
							done = ev;
							break;
					}
				},
				abort.signal
			);
		} catch (e) {
			log.push({ kind: 'err', text: e instanceof Error ? e.message : String(e) });
		} finally {
			running = false;
			abort = null;
			progress = null;
			// Refresh what the reader shows.
			void nar.refresh(req.pubkey, req.d_tag, 'local_only');
			// `done` is assigned inside the SSE callback; read it through a
			// typed local so TS does not narrow it to the initial null.
			const d = done as Extract<api.NarrationGenEvent, { type: 'done' }> | null;
			if (d) {
				app.pushToast(
					d.leaves > 0 ? `narrated ${d.leaves} section${d.leaves === 1 ? '' : 's'}${d.spent_usd != null ? ` · ${usd(d.spent_usd)}` : ''}` : 'nothing new narrated',
					d.leaves > 0 ? 'success' : 'info'
				);
				void app.refreshProviders();
			}
		}
	}

	function cancel() {
		abort?.abort();
	}

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && !running) {
			e.stopPropagation();
			close();
		}
	}
</script>

<svelte:window onkeydown={onKey} />

{#if req}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<div class="nm-backdrop {yielding ? 'nm-backdrop--yield' : ''}" onclick={close} role="presentation">
		<div class="nm-modal" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-label="Narrate publication" tabindex="-1">
			<header class="nm-header">
				<h3 class="nm-title">narrate · {req.title ?? req.d_tag}</h3>
				<button class="nm-close" onclick={close} aria-label="Close" disabled={running}>×</button>
			</header>

			{#if !ppqConfigured}
				<div class="nm-detail nm-detail--bad">
					No ppq.ai account. Narration spends from it —
					<button class="nm-link" onclick={() => (app.agentServicesModal = { view: 'signup' })}>set one up</button> first.
				</div>
			{/if}
			{#if !canSign}
				<div class="nm-detail nm-detail--bad">Sign in to narrate — the Blossom upload token and the recording events are signed as you.</div>
			{/if}
			{#if loadErr}
				<div class="nm-detail nm-detail--bad">{loadErr}</div>
			{/if}

			<div class="nm-grid">
				<label class="nm-label" for="nm-model">model</label>
				<select id="nm-model" class="nm-input" bind:value={model} onchange={onModelChange} disabled={running}>
					{#each models as m (m.id)}
						<option value={m.id}>{m.id} · {usd(m.price_per_1k)}/1k chars · ≤{m.char_cap}</option>
					{/each}
				</select>

				<label class="nm-label" for="nm-lang">language</label>
				<select id="nm-lang" class="nm-input" bind:value={language} onchange={onLanguageChange} disabled={running}>
					<option value="">any — {allVoices.length} voice{allVoices.length === 1 ? '' : 's'}</option>
					{#each languages as l (l.code)}
						<option value={l.code}>{l.name} ({l.code}) — {l.n} voice{l.n === 1 ? '' : 's'}</option>
					{/each}
				</select>

				<label class="nm-label" for="nm-voice">voice</label>
				<div class="nm-voice">
					<select id="nm-voice" class="nm-input" bind:value={voice} onchange={stopPreview} disabled={running}>
						<option value="">provider default</option>
						{#each voices as v (v.id)}
							<option value={v.id}>{v.name ?? v.id}{!language && v.language ? ` · ${v.language}` : ''}{v.gender ? ` · ${v.gender}` : ''}{v.description ? ` — ${v.description.slice(0, 60)}` : ''}</option>
						{/each}
					</select>
					<button
						class="nm-btn nm-btn--icon"
						onclick={playClip}
						disabled={running || !selectedVoice?.preview_url}
						title={selectedVoice?.preview_url ? "Play the provider's sample clip for this voice (free)" : 'No sample clip for this voice'}
						aria-label="Play voice sample"
					>{previewing === 'clip' ? '⏹' : '▶'}</button>
					<button
						class="nm-btn nm-btn--icon"
						onclick={playSample}
						disabled={running || !ppqConfigured || !sampleText}
						title={sampleText ? `Hear this voice read the opening of "${(sectionNodes.find((n) => picked[n.coord] !== false) ?? sectionNodes[0])?.title ?? 'the first section'}" (≈ ${usd(sampleCost)}, nothing is uploaded or published)` : 'Load a section first'}
					>{previewing === 'sample' ? '⏹' : 'hear my text'}</button>
				</div>
				{#if selectedVoice?.description}
					<span></span>
					<div class="nm-detail">{selectedVoice.description}</div>
				{/if}
				{#if previewErr}
					<span></span>
					<div class="nm-detail nm-detail--bad">{previewErr}</div>
				{/if}
				<audio bind:this={previewAudio} onended={() => (previewing = null)} onerror={() => { if (previewing) { previewErr = 'could not play the sample'; previewing = null; } }}></audio>

				<label class="nm-label" for="nm-server">upload to</label>
				<select id="nm-server" class="nm-input" bind:value={server} disabled={running}>
					{#each servers as s (s)}
						<option value={s}>{s}</option>
					{/each}
					{#if servers.length === 0}
						<option value="">no Blossom server — add one in Settings › Narration</option>
					{/if}
				</select>
			</div>

			<div class="nm-opts">
				<label><input type="checkbox" bind:checked={transcript} disabled={running} /> transcript in content</label>
				<label><input type="checkbox" bind:checked={broadcast} disabled={running} /> broadcast when done</label>
				<label><input type="checkbox" bind:checked={force} disabled={running} /> re-narrate already recorded</label>
			</div>

			<div class="nm-plan">
				<div class="nm-plan__head">
					<span>sections</span>
					<span class="nm-sp"></span>
					<button class="nm-link" onclick={() => { for (const n of sectionNodes) picked[n.coord] = true; }} disabled={running}>all</button>
					<button class="nm-link" onclick={() => { for (const n of sectionNodes) picked[n.coord] = false; }} disabled={running}>none</button>
				</div>
				{#if estimating && !estimate}
					<div class="nm-detail">loading the publication…</div>
				{:else}
					<ul class="nm-list">
						{#each estimate?.nodes ?? [] as n (n.coord)}
							{#if n.is_index}
								<li class="nm-row nm-row--index" style="--d:{n.depth}">{n.title}</li>
							{:else}
								<li class="nm-row" style="--d:{n.depth}">
									<label>
										<input type="checkbox" checked={picked[n.coord] !== false && n.chars > 0} disabled={running || n.chars === 0} onchange={(e) => (picked[n.coord] = (e.target as HTMLInputElement).checked)} />
										<span class="nm-row__title" title={n.preview ?? ''}>{n.title}</span>
									</label>
									<span class="nm-row__meta" class:nm-row__meta--split={(n.est_pieces ?? 0) > 1}
										title={(n.est_pieces ?? 0) > 1 ? `About ${nar.fmtBytes(n.est_bytes ?? 0)} of audio — over the ${nar.fmtBytes(estimate?.piece_cap_bytes ?? 0)} per-blob cap, so it uploads as ${n.est_pieces} blobs (NKBIP-07 pieces) and plays back to back` : ''}
									>{n.chars === 0 ? 'no text' : `${n.chars} chars · ${n.chunks} call${n.chunks === 1 ? '' : 's'} · ~${nar.fmtBytes(n.est_bytes ?? 0)}${(n.est_pieces ?? 0) > 1 ? ` · ${n.est_pieces} blobs` : ''}`}</span>
								</li>
							{/if}
						{/each}
					</ul>
				{/if}
			</div>

			<div class="nm-cost" class:nm-cost--bad={underfunded}>
				<span>{pickedCoords.length} section{pickedCoords.length === 1 ? '' : 's'} · {pickedChars} chars · {pickedChunks} calls · ~{nar.fmtBytes(pickedChars * 400)}</span>
				<span class="nm-sp"></span>
				<span>≈ {usd(cost)}{balance !== null ? ` of ${usd(balance)} ppq balance` : ''}</span>
				{#if underfunded}
					<button class="nm-link" onclick={() => (app.agentServicesModal = { view: 'fund', focus: 'ppq' })}>fund</button>
				{/if}
			</div>

			{#if running || log.length > 0}
				<div class="nm-log">
					{#if progress}
						<div class="nm-progress">
							§ {progress.section}/{progress.sections}
							{#if progress.chunks > 0}· call {progress.chunk}/{progress.chunks}{/if}
							<span class="nm-bar"><span class="nm-bar__fill" style="width:{progress.chunks ? (progress.chunk / progress.chunks) * 100 : 0}%"></span></span>
						</div>
					{/if}
					{#each log as l, i (i)}
						<div class="nm-line nm-line--{l.kind}">{l.text}</div>
					{/each}
					{#if done}
						<div class="nm-line nm-line--ok">
							done · {done.leaves} new recording{done.leaves === 1 ? '' : 's'}{done.root ? ' · root index signed' : ''}{done.spent_usd != null ? ` · spent ${usd(done.spent_usd)}` : ''}{done.balance_usd != null ? ` · balance ${usd(done.balance_usd)}` : ''}
						</div>
					{/if}
				</div>
			{/if}

			<footer class="nm-foot">
				<span class="nm-hint">
					{#if running}running — closing the tab cancels after the current section{:else}each section becomes one Blossom blob + one kind-32176 event, bound to this exact version{/if}
				</span>
				<span class="nm-sp"></span>
				{#if running}
					<button class="nm-btn" onclick={cancel}>Cancel</button>
				{:else}
					<button class="nm-btn" onclick={close}>Close</button>
					<button class="nm-btn nm-btn--go" onclick={run} disabled={!canRun}>Narrate</button>
				{/if}
			</footer>
		</div>
	</div>
{/if}

<style>
	.nm-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 8dvh;
	}
	.nm-backdrop--yield {
		visibility: hidden;
	}
	.nm-modal {
		width: min(640px, 94vw);
		max-height: 86dvh;
		overflow-y: auto;
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
	}
	.nm-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.nm-title {
		margin: 0;
		font-size: var(--t-sm);
		font-weight: normal;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nm-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: 1.2em;
		cursor: pointer;
	}
	.nm-grid {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 6px 10px;
		align-items: center;
	}
	.nm-label {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.nm-input {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 8px;
		min-width: 0;
		width: 100%;
	}
	.nm-voice {
		display: flex;
		gap: 4px;
		align-items: center;
		min-width: 0;
	}
	.nm-voice .nm-input {
		flex: 1 1 auto;
	}
	.nm-btn--icon {
		white-space: nowrap;
		padding: 3px 7px;
	}
	.nm-opts {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 14px;
		color: var(--fg-muted);
	}
	.nm-opts label {
		display: inline-flex;
		align-items: center;
		gap: 4px;
	}
	.nm-plan {
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		max-height: 32dvh;
		overflow-y: auto;
	}
	.nm-plan__head {
		display: flex;
		gap: 8px;
		padding: 4px 8px;
		border-bottom: 1px solid var(--panel-border);
		color: var(--base5);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		font-size: calc(var(--t-xs) - 2px);
		position: sticky;
		top: 0;
		background: var(--bg);
	}
	.nm-list {
		list-style: none;
		margin: 0;
		padding: 2px 0;
	}
	.nm-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 2px 8px 2px calc(8px + var(--d, 0) * 12px);
	}
	.nm-row label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		flex: 1 1 auto;
	}
	.nm-row--index {
		color: var(--fg-muted);
		font-style: italic;
	}
	.nm-row__title {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.nm-row__meta {
		color: var(--base5);
		white-space: nowrap;
		font-size: calc(var(--t-xs) - 2px);
	}
	.nm-row__meta--split {
		color: var(--state-warn, #c90);
	}
	.nm-cost {
		display: flex;
		gap: 8px;
		align-items: center;
		padding: 4px 8px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		color: var(--fg-muted);
	}
	.nm-cost--bad {
		color: var(--error, #e06c75);
		border-color: var(--error, #e06c75);
	}
	.nm-sp {
		flex: 1;
	}
	.nm-log {
		max-height: 24dvh;
		overflow-y: auto;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 6px 8px;
		font-size: calc(var(--t-xs) - 1px);
	}
	.nm-progress {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 4px;
	}
	.nm-bar {
		flex: 1;
		height: 4px;
		background: var(--panel-border);
		border-radius: 2px;
		overflow: hidden;
	}
	.nm-bar__fill {
		display: block;
		height: 100%;
		background: var(--state-online, var(--accent));
		transition: width 160ms ease;
	}
	.nm-line {
		white-space: pre-wrap;
		color: var(--fg-muted);
	}
	.nm-line--ok {
		color: var(--state-online, var(--fg));
	}
	.nm-line--warn {
		color: var(--state-warn, #c90);
	}
	.nm-line--err {
		color: var(--error, #e06c75);
	}
	.nm-detail {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.nm-detail--bad {
		color: var(--error, #e06c75);
	}
	.nm-link {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		color: var(--id-yours);
		cursor: pointer;
		text-decoration: underline;
	}
	.nm-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 4px;
	}
	.nm-hint {
		color: var(--base5);
		font-size: calc(var(--t-xs) - 1px);
	}
	.nm-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.nm-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.nm-btn--go {
		color: var(--id-yours);
	}
	.nm-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
