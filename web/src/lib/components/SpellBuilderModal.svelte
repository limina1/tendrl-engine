<script lang="ts">
	// Structured spell builder (the 🪄 affordance): build a kind-777 spell
	// from form controls instead of query syntax — including pipelines
	// (composed spells: ordered stages with map/join combinators), which
	// have no syntax form at all. The form serializes to a search string +
	// override fields; the engine compiles it (POST /api/v1/spell/compose)
	// and the preview shows the resulting clause line, so the builder
	// teaches the syntax as a side effect.
	import * as api from '$lib/api';
	import { getAppState } from '$lib/state.svelte';
	import SpellClauseBlock from './SpellClauseBlock.svelte';

	let { onclose }: { onclose: () => void } = $props();

	const app = getAppState();

	const KIND_CHIPS: [number, string][] = [
		[1, 'notes'],
		[1111, 'comments'],
		[30023, 'articles'],
		[30040, 'publications'],
		[30041, 'sections'],
		[30818, 'wikis'],
		[9802, 'highlights'],
		[777, 'spells']
	];
	const WINDOWS = ['any', '1h', '6h', '1d', '7d', '30d', '1y'];

	let pipelineMode = $state(false);
	let countMode = $state(false);
	let kinds = $state<number[]>([]);
	let customKind = $state('');
	let authorMe = $state(false);
	let authorContacts = $state(false);
	let authorsText = $state('');
	let tagRows = $state<{ name: string; value: string }[]>([]);
	let searchText = $state('');
	let timeWindow = $state('any');
	let limitText = $state('');
	let relaysText = $state('');
	let stageRows = $state<{ id: string; combinator: 'map' | 'join' }[]>([
		{ id: '', combinator: 'map' }
	]);
	let paramRows = $state<{ name: string; prompt: string }[]>([]);
	// `in` chaining: apply this spell to a previous spell's results.
	let inputId = $state('');
	let inputRelays = $state(''); // "find the spell on these relays"
	let projRoots = $state(false); // $in.tag.E + $in.tag.e:root → ids
	let projRefs = $state(false); // $in.tag.e → ids
	let projIds = $state(false); // $in.ids → ids
	let projAuthors = $state(false); // $in.pubkeys → authors

	const HEX64 = /^[0-9a-f]{64}$/i;
	// Loose client gate — the engine does the real decode. An nevent's
	// relay hints unpack server-side into "find the spell on these relays".
	const NIP19_REF = /^(nostr:)?(nevent1|note1)[023456789acdefghjklmnpqrstuvwxyz]+$/i;
	function isSpellRef(s: string): boolean {
		return HEX64.test(s) || NIP19_REF.test(s);
	}

	let name = $state('');
	let description = $state('');
	let topics = $state('');

	let preview = $state<api.SpellComposeResponse | null>(null);
	let composeError = $state<string | null>(null);
	let saving = $state(false);

	function toggleKind(k: number) {
		kinds = kinds.includes(k) ? kinds.filter((x) => x !== k) : [...kinds, k];
	}
	function addCustomKind() {
		const k = parseInt(customKind.trim(), 10);
		if (!Number.isNaN(k) && !kinds.includes(k)) kinds = [...kinds, k];
		customKind = '';
	}

	function composeRequest(): Parameters<typeof api.composeSpell>[0] {
		const shared = {
			name: name.trim() || undefined,
			description: description.trim() || undefined,
			topics: topics.split(',').map((t) => t.trim()).filter(Boolean)
		};
		if (pipelineMode) {
			return {
				...shared,
				stages: stageRows
					.filter((r) => r.id.trim())
					.map((r, i) => ({
						spell_id: r.id.trim(),
						combinator: i === 0 ? undefined : r.combinator
					})),
				params: paramRows
					.filter((p) => p.name.trim())
					.map((p) => ({ name: p.name.trim(), prompt: p.prompt.trim() || undefined }))
			};
		}
		const parts: string[] = [];
		for (const k of kinds) parts.push(`k:${k}`);
		for (const row of tagRows) {
			const tagName = row.name.trim();
			const value = row.value.trim();
			if (!tagName || !value) continue;
			parts.push(/\s/.test(value) ? `${tagName}:"${value}"` : `${tagName}:${value}`);
		}
		if (searchText.trim()) parts.push(`"${searchText.trim()}"`);
		const limit = parseInt(limitText.trim(), 10);
		if (!Number.isNaN(limit) && limit > 0) parts.push(`limit:${limit}`);
		for (const r of relaysText.split(',').map((s) => s.trim()).filter(Boolean)) {
			parts.push(`relay:${r}`);
		}
		const authors: string[] = [];
		if (authorMe) authors.push('$me');
		if (authorContacts) authors.push('$contacts');
		for (const line of authorsText.split(/\s+/).map((s) => s.trim()).filter(Boolean)) {
			authors.push(line);
		}
		const chained = isSpellRef(inputId.trim());
		const ids: string[] = [];
		if (chained) {
			if (projRoots) ids.push('$in.tag.E', '$in.tag.e:root');
			if (projRefs) ids.push('$in.tag.e');
			if (projIds) ids.push('$in.ids');
			if (projAuthors) authors.push('$in.pubkeys');
		}
		return {
			...shared,
			query: parts.join(' '),
			authors,
			input: chained ? inputId.trim() : undefined,
			input_relays: chained
				? inputRelays.split(',').map((s) => s.trim()).filter(Boolean)
				: undefined,
			ids: ids.length ? ids : undefined,
			since: timeWindow === 'any' ? undefined : timeWindow,
			cmd: countMode ? 'COUNT' : undefined
		};
	}

	// An untouched form isn't an error state — skip the compile until the
	// request actually filters something (or has a stage).
	function requestIsEmpty(req: Parameters<typeof api.composeSpell>[0]): boolean {
		if (req.stages !== undefined) return req.stages.length === 0;
		if (req.input) {
			// A chain with nothing pulled from it isn't previewable yet.
			return !(req.ids?.length || req.authors?.some((a) => a.startsWith('$in.')));
		}
		return !req.query?.trim() && !(req.authors?.length) && !req.since && !req.cmd;
	}

	$effect(() => {
		void pipelineMode; void countMode; void kinds; void authorMe; void authorContacts;
		void authorsText; void searchText; void timeWindow; void limitText; void relaysText;
		void name; void description; void topics;
		void inputId; void inputRelays; void projRoots; void projRefs; void projIds; void projAuthors;
		void tagRows.map((r) => r.name + r.value).join();
		void stageRows.map((r) => r.id + r.combinator).join();
		void paramRows.map((p) => p.name + p.prompt).join();
		const t = setTimeout(() => {
			const chainId = inputId.trim();
			if (!pipelineMode && chainId && !isSpellRef(chainId)) {
				preview = null;
				composeError = 'enter a 64-hex event id, note1…, or nevent1…';
				return;
			}
			const req = composeRequest();
			if (requestIsEmpty(req)) {
				preview = null;
				composeError = null;
				return;
			}
			api.composeSpell(req)
				.then((r) => { preview = r; composeError = null; })
				.catch((e) => { preview = null; composeError = api.errorMessage(e); });
		}, 250);
		return () => clearTimeout(t);
	});

	async function save(broadcast: boolean) {
		if (!preview || saving) return;
		saving = true;
		try {
			const composed = await api.composeSpell(composeRequest());
			const { signed_event } = await api.signTemplate({ template: composed.template });
			await api.ingestEvent(signed_event);
			if (broadcast) {
				const res = await api.broadcastEvent({ event: signed_event });
				app.pushToast(
					`Spell saved — accepted by ${res.successful}/${res.total} relays`,
					res.successful > 0 ? 'success' : 'error'
				);
			} else {
				app.pushToast('Spell saved locally — broadcast when ready', 'success');
			}
			onclose();
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Save failed'), 'error');
		} finally {
			saving = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') onclose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="sb-backdrop" onclick={onclose} role="presentation">
	<div
		class="sb-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		tabindex="-1"
	>
		<header class="sb-header">
			<h3 class="sb-title">🪄 Spell builder</h3>
			<label class="sb-mode">
				<input type="checkbox" bind:checked={pipelineMode} />
				pipeline (composed spell)
			</label>
			<button class="sb-close" onclick={onclose} aria-label="Close">×</button>
		</header>

		<div class="sb-body">
			<label class="sb-field">
				<span>Name</span>
				<input bind:value={name} placeholder="What this spell is called" />
			</label>
			<label class="sb-field">
				<span>Description</span>
				<input bind:value={description} placeholder="What it collects (event content)" />
			</label>
			<label class="sb-field">
				<span>Topics — label the spell itself, not a filter</span>
				<input
					bind:value={topics}
					placeholder="comma,separated (to match #hashtags in events, add a tag condition below)"
				/>
			</label>

			{#if pipelineMode}
				<div class="sb-section">
					<div class="sb-section-head">
						<span>Stages — run in order; each stage's $in.* reads the previous result</span>
						<button
							class="sb-add"
							onclick={() => (stageRows = [...stageRows, { id: '', combinator: 'map' }])}
						>+ stage</button>
					</div>
					{#each stageRows as row, i (i)}
						<div class="sb-row">
							<span class="sb-row-label">{i + 1}.</span>
							<input class="sb-stage-id" bind:value={row.id} placeholder="spell event id or nevent" />
							{#if i === 0}
								<span class="sb-row-note">source</span>
							{:else}
								<select bind:value={row.combinator}>
									<option value="map">map — replace with referents</option>
									<option value="join">join — fetch alongside</option>
								</select>
							{/if}
							{#if stageRows.length > 1}
								<button
									class="sb-remove"
									onclick={() => (stageRows = stageRows.filter((_, j) => j !== i))}
									aria-label="Remove stage"
								>×</button>
							{/if}
						</div>
					{/each}
					<div class="sb-section-head">
						<span>Declared parameters (prompted when the pipeline runs)</span>
						<button
							class="sb-add"
							onclick={() => (paramRows = [...paramRows, { name: '', prompt: '' }])}
						>+ param</button>
					</div>
					{#each paramRows as p, i (i)}
						<div class="sb-row">
							<input class="sb-param-name" bind:value={p.name} placeholder="name" />
							<input bind:value={p.prompt} placeholder="prompt shown at run time" />
							<button
								class="sb-remove"
								onclick={() => (paramRows = paramRows.filter((_, j) => j !== i))}
								aria-label="Remove parameter"
							>×</button>
						</div>
					{/each}
				</div>
			{:else}
				<div class="sb-section">
					<div class="sb-section-head">
						<span>Apply to a spell — chain: run it first, this spell reads its results</span>
					</div>
					<input
						class="sb-stage-id"
						bind:value={inputId}
						placeholder="input spell — event id or nevent (relay hints unpack), optional"
					/>
					{#if inputId.trim()}
						<input
							bind:value={inputRelays}
							placeholder="find the spell on these relays — comma,separated (optional)"
						/>
					{/if}
					{#if inputId.trim()}
						<div class="sb-chips">
							<button
								class="sb-chip"
								class:sb-chip--on={projRoots}
								onclick={() => (projRoots = !projRoots)}
							>root references <code>$in.tag.E e:root</code></button>
							<button
								class="sb-chip"
								class:sb-chip--on={projRefs}
								onclick={() => (projRefs = !projRefs)}
							>referenced events <code>$in.tag.e</code></button>
							<button
								class="sb-chip"
								class:sb-chip--on={projIds}
								onclick={() => (projIds = !projIds)}
							>the events themselves <code>$in.ids</code></button>
							<button
								class="sb-chip"
								class:sb-chip--on={projAuthors}
								onclick={() => (projAuthors = !projAuthors)}
							>their authors <code>$in.pubkeys</code></button>
						</div>
					{/if}

					<div class="sb-section-head"><span>Kinds</span></div>
					<div class="sb-chips">
						{#each KIND_CHIPS as [k, label] (k)}
							<button
								class="sb-chip"
								class:sb-chip--on={kinds.includes(k)}
								onclick={() => toggleKind(k)}
							>{label} <code>k:{k}</code></button>
						{/each}
						<input
							class="sb-kind-custom"
							bind:value={customKind}
							placeholder="k:…"
							onkeydown={(e) => { if (e.key === 'Enter') addCustomKind(); }}
							onblur={addCustomKind}
						/>
					</div>

					<div class="sb-section-head"><span>Authors</span></div>
					<div class="sb-row">
						<label class="sb-check"><input type="checkbox" bind:checked={authorMe} /> me ($me)</label>
						<label class="sb-check"><input type="checkbox" bind:checked={authorContacts} /> my contacts ($contacts)</label>
					</div>
					<input bind:value={authorsText} placeholder="extra pubkeys (64-hex, space-separated)" />

					<div class="sb-section-head">
						<span>Tag conditions</span>
						<button
							class="sb-add"
							onclick={() => (tagRows = [...tagRows, { name: 't', value: '' }])}
						>+ tag</button>
					</div>
					{#each tagRows as row, i (i)}
						<div class="sb-row">
							<input class="sb-param-name" bind:value={row.name} placeholder="tag (t, d, e…)" />
							<input bind:value={row.value} placeholder="value" />
							<button
								class="sb-remove"
								onclick={() => (tagRows = tagRows.filter((_, j) => j !== i))}
								aria-label="Remove tag condition"
							>×</button>
						</div>
					{/each}

					<div class="sb-grid">
						<label class="sb-field">
							<span>Text (NIP-50)</span>
							<input bind:value={searchText} placeholder="relay-side search" />
						</label>
						<label class="sb-field">
							<span>Window</span>
							<select bind:value={timeWindow}>
								{#each WINDOWS as w (w)}<option value={w}>{w === 'any' ? 'any time' : `last ${w}`}</option>{/each}
							</select>
						</label>
						<label class="sb-field">
							<span>Limit</span>
							<input bind:value={limitText} placeholder="50" />
						</label>
						<label class="sb-field">
							<span>Relays</span>
							<input bind:value={relaysText} placeholder="nos.lol, thecitadel.nostr1.com" />
						</label>
					</div>
					<label class="sb-check">
						<input type="checkbox" bind:checked={countMode} /> COUNT — return a count, not the events
					</label>
				</div>
			{/if}

			<div class="sb-preview">
				<span class="sb-preview-head">Preview</span>
				{#if composeError}
					<p class="sb-error">{composeError}</p>
				{:else if preview}
					<SpellClauseBlock clauses={preview.clauses} />
					{#each preview.stages ?? [] as st, si (st.spell_id + si)}
						<div class="sb-stage-preview">
							<span class="sb-row-note">
								stage {si + 1}{st.combinator ? ` · ${st.combinator}` : ''}:
								{st.name ?? st.spell_id.slice(0, 12) + '…'}
							</span>
							{#if st.error}
								<span class="sb-error">{st.error}</span>
							{:else}
								<SpellClauseBlock clauses={st.clauses} />
							{/if}
						</div>
					{/each}
					{#each preview.warnings as w (w)}
						<p class="sb-warning">⚠ {w}</p>
					{/each}
				{:else}
					<p class="sb-muted">
						{pipelineMode
							? 'Add a stage (a spell event id) to preview the pipeline'
							: inputId.trim()
								? 'Pick what to pull from the input spell’s results — roots, referenced events, or authors'
								: 'Pick kinds, authors, tags, or a time window to preview the spell'}
					</p>
				{/if}
			</div>
		</div>

		<footer class="sb-footer">
			<button class="sb-save" disabled={!preview || saving} onclick={() => save(false)}>
				{saving ? 'Saving…' : 'Save locally'}
			</button>
			<button class="sb-save sb-save--broadcast" disabled={!preview || saving} onclick={() => save(true)}>
				Save + broadcast
			</button>
		</footer>
	</div>
</div>

<style>
	.sb-backdrop {
		position: fixed;
		inset: 0;
		background: color-mix(in srgb, black 55%, transparent);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 60;
	}
	.sb-modal {
		width: min(640px, 94vw);
		max-height: 88vh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		display: flex;
		flex-direction: column;
	}
	.sb-header {
		display: flex;
		align-items: center;
		gap: 12px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--border);
	}
	.sb-title { margin: 0; font-size: var(--t-sm); }
	.sb-mode {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 5px;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sb-close {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-size: var(--t-md);
		cursor: pointer;
	}
	.sb-body {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 12px 14px;
	}
	.sb-field {
		display: flex;
		flex-direction: column;
		gap: 2px;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sb-body input,
	.sb-body select {
		background: var(--bg-alt, transparent);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg);
		font-size: var(--t-xs);
		padding: 4px 8px;
	}
	.sb-section {
		display: flex;
		flex-direction: column;
		gap: 6px;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 8px 10px;
	}
	.sb-section-head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}
	.sb-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
		align-items: center;
	}
	.sb-chip {
		background: transparent;
		border: 1px solid var(--border);
		border-radius: var(--radius);
		color: var(--fg-muted);
		font-size: var(--t-2xs);
		padding: 2px 8px;
		cursor: pointer;
	}
	.sb-chip--on {
		border-color: var(--accent);
		color: var(--accent);
		background: color-mix(in srgb, var(--accent) 10%, transparent);
	}
	.sb-chip code { font-size: var(--t-3xs); opacity: 0.7; }
	.sb-kind-custom { width: 64px; }
	.sb-row {
		display: flex;
		gap: 6px;
		align-items: center;
	}
	.sb-row input { flex: 1; }
	.sb-row-label { font-size: var(--t-xs); color: var(--fg-muted); }
	.sb-row-note {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		color: var(--fg-muted);
	}
	.sb-stage-id { font-family: var(--font-mono); }
	.sb-param-name { max-width: 130px; }
	.sb-check {
		display: flex;
		align-items: center;
		gap: 5px;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sb-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 8px;
	}
	.sb-add,
	.sb-remove {
		background: none;
		border: none;
		color: var(--accent);
		font-size: var(--t-2xs);
		cursor: pointer;
	}
	.sb-preview {
		border: 1px solid var(--border);
		border-radius: var(--radius);
		padding: 8px 10px;
	}
	.sb-preview-head {
		font-size: var(--t-2xs);
		color: var(--fg-muted);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}
	.sb-stage-preview {
		border-left: 2px solid var(--border);
		padding-left: 8px;
		margin-top: 4px;
	}
	.sb-warning {
		margin: 4px 0 0;
		font-size: var(--t-2xs);
		color: var(--warning, #c90);
	}
	.sb-error {
		margin: 4px 0 0;
		font-size: var(--t-xs);
		color: var(--error, #c55);
	}
	.sb-muted {
		margin: 4px 0 0;
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.sb-footer {
		display: flex;
		gap: 8px;
		justify-content: flex-end;
		padding: 10px 14px;
		border-top: 1px solid var(--border);
	}
	.sb-save {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 4px 14px;
		background: transparent;
		border: 1px solid var(--accent);
		border-radius: var(--radius);
		color: var(--accent);
		cursor: pointer;
	}
	.sb-save--broadcast {
		background: var(--accent);
		color: var(--bg);
	}
	.sb-save:disabled {
		opacity: 0.5;
		cursor: default;
	}
</style>
