<script lang="ts">
	// The generic event composer (M-x tendrl-compose-event, SPC e e): the
	// highlighter's shape — a form, an engine-built preview, one publish
	// call — for ANY kind. The form is rendered from a kind template the
	// engine serves (`GET /event/templates`: music track, web bookmark, …,
	// plus `<data_dir>/event-templates/*.json`), or from nothing for a
	// custom kind. The engine owns everything algorithmic: fixed values
	// the spec mandates, derived `d` / `alt`, type checks, the addressable
	// rule — so this file only collects strings and shows what it is told.
	import * as api from '$lib/api';
	import { getAppState, type EventComposerDraft } from '$lib/state.svelte';
	import { identityCanSign } from '$lib/identity/signer';
	import { confirmState } from '$lib/network/fetch-events.svelte';

	const app = getAppState();
	const canSign = $derived(identityCanSign(app.identityStatus));

	const CUSTOM = '__custom__';

	// Prefill precedence: an existing event handed over by the menu's Edit
	// wins (its kind picks the template; its tags fill the fields); else
	// the open payload's template; else the draft remembered from the last
	// close. Publish clears the memory.
	const opener = app.eventComposer;
	const prefill = opener?.prefill ?? null;
	const draft: EventComposerDraft | null = prefill ? null : app.eventDraft;
	let templates = $state<api.KindTemplate[]>([]);
	let templatesErr = $state<string | null>(null);
	let selected = $state<string>(
		prefill ? '' : (opener?.template ?? (draft ? (draft.template ?? CUSTOM) : ''))
	);
	let customKind = $state(prefill ? String(prefill.kind) : (draft?.kind ?? ''));
	let content = $state(prefill?.content ?? draft?.content ?? '');
	let values = $state<Record<string, string>>({ ...(draft?.values ?? {}) });
	let extra = $state<{ name: string; value: string; rest?: string[] }[]>(
		(draft?.extra ?? []).map((r) => ({ ...r }))
	);
	let posting = $state(false);
	const editing = !!prefill;

	// Lay an existing event's tags over a template: declared fields take
	// their values (multi → one per line), everything else becomes an extra
	// row with its trailing elements kept. The `client` tag is dropped —
	// signing re-stamps it.
	function applyPrefill(t: api.KindTemplate | null) {
		if (!prefill) return;
		const v: Record<string, string> = {};
		const rows: { name: string; value: string; rest?: string[] }[] = [];
		const declared = new Map((t?.tags ?? []).map((f) => [f.name, f]));
		for (const tag of prefill.tags) {
			const [name, value = '', ...rest] = tag;
			if (!name || name === 'client') continue;
			const f = declared.get(name);
			if (f && rest.length === 0) {
				if (f.multi) {
					if (f.fixed.includes(value)) continue;
					v[name] = v[name] ? `${v[name]}\n${value}` : value;
				} else if (!(name in v)) {
					v[name] = value;
				} else {
					rows.push({ name, value });
				}
			} else {
				rows.push({ name, value, rest: rest.length ? rest : undefined });
			}
		}
		values = v;
		extra = rows;
	}

	const template = $derived(
		selected && selected !== CUSTOM ? (templates.find((t) => t.name === selected) ?? null) : null
	);
	const isCustom = $derived(selected === CUSTOM);
	const kindNumber = $derived(
		template ? template.kind : /^\d{1,9}$/.test(customKind.trim()) ? Number(customKind.trim()) : null
	);

	// NIP-01 addressable range: the kind needs a `d`. The identifier is the
	// event's identity, so it is never derived from a title: the user mints
	// a fresh one (engine nanoid, compose's shape) or pastes an existing
	// one to replace that event. Custom addressable kinds get the same
	// field the templates declare.
	const addressable = $derived(kindNumber !== null && kindNumber >= 30000 && kindNumber < 40000);
	const customNeedsD = $derived(isCustom && addressable);
	let minting = $state(false);
	async function mintD() {
		if (minting || posting) return;
		minting = true;
		try {
			values.d = (await api.mintEventD()).d;
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Could not mint an identifier'), 'error');
		} finally {
			minting = false;
		}
	}
	/** A `d` field the user fills by generating or pasting (a transformed
	 *  `d`, e.g. a bookmark URL, is pasted only). */
	function mintable(f: api.EventTagField): boolean {
		return f.name === 'd' && !f.transform;
	}

	// Confirm mode: see the modal-yield rule in feat-ui-patterns — hide
	// while PublishConfirmModal (z 250) decides the broadcast.
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');

	api.listEventTemplates().then(
		(r) => {
			templates = r.templates;
			if (prefill) {
				// An edited event picks the template declared for its kind,
				// else the custom path with the kind filled in.
				const t = templates.find((x) => x.kind === prefill.kind) ?? null;
				selected = t ? t.name : CUSTOM;
				applyPrefill(t);
			} else if (!selected && templates.length) selected = templates[0].name;
			else if (!selected) selected = CUSTOM;
		},
		(e) => {
			templatesErr = api.errorMessage(e, 'templates unavailable');
			if (prefill) {
				selected = CUSTOM;
				applyPrefill(null);
			} else if (!selected) selected = CUSTOM;
		}
	);

	// --- Form → request -----------------------------------------------------
	// Multi fields take one value per line (or comma-separated on one line).
	function splitMulti(raw: string): string[] {
		return raw
			.split(/[\n,]/)
			.map((s) => s.trim())
			.filter(Boolean);
	}

	function buildRequest(): api.EventComposeRequest | null {
		if (!template && kindNumber === null) return null;
		const tags: string[][] = [];
		if (template) {
			for (const f of template.tags) {
				const raw = values[f.name] ?? '';
				if (f.multi) for (const v of splitMulti(raw)) tags.push([f.name, v]);
				else if (raw.trim()) tags.push([f.name, raw.trim()]);
			}
		}
		if (customNeedsD && (values.d ?? '').trim()) tags.push(['d', values.d.trim()]);
		for (const r of extra) {
			if (r.name.trim()) tags.push([r.name.trim(), r.value.trim(), ...(r.rest ?? [])]);
		}
		return {
			template: template ? template.name : null,
			kind: template ? template.kind : kindNumber,
			content,
			tags
		};
	}

	// --- Engine preview: template + problems -------------------------------
	// Debounced, token-guarded; the $effect reads state synchronously and
	// hands a snapshot to the async fetch (effect-async-state memory).
	let problems = $state<api.EventProblem[]>([]);
	let previewed = $state(false);
	let jsonOpen = $state(false);
	let jsonText = $state<string | null>(null);
	let previewErr = $state<string | null>(null);
	let timer: ReturnType<typeof setTimeout> | undefined;
	let token = 0;

	$effect(() => {
		const req = buildRequest();
		const snapshot = req ? JSON.stringify(req) : null;
		clearTimeout(timer);
		timer = setTimeout(async () => {
			const my = ++token;
			if (!snapshot) {
				problems = [];
				previewed = false;
				jsonText = null;
				previewErr = null;
				return;
			}
			try {
				const resp = await api.previewEvent(JSON.parse(snapshot));
				if (my !== token) return;
				problems = resp.problems;
				jsonText = JSON.stringify(resp.event, null, 2);
				previewErr = null;
				previewed = true;
			} catch (e) {
				if (my !== token) return;
				previewErr = api.errorMessage(e, 'preview failed');
				jsonText = null;
				previewed = false;
			}
		}, 300);
		return () => clearTimeout(timer);
	});

	// Inline problems, minus the bare "required" on a field the user hasn't
	// touched: the star and the footer's "N to fix" already say it, and a
	// pristine form shouldn't open red. A typed-but-invalid value is shown.
	const problemsByField = $derived.by(() => {
		const m: Record<string, string[]> = {};
		for (const p of problems) {
			if (p.message === 'required' && !(values[p.field] ?? '').trim()) continue;
			(m[p.field] ??= []).push(p.message);
		}
		return m;
	});

	const canPost = $derived(
		canSign && !posting && previewed && problems.length === 0 && !previewErr
	);

	// --- Lifecycle -----------------------------------------------------------
	function hasAnything(): boolean {
		return (
			content.trim() !== '' ||
			Object.values(values).some((v) => v.trim()) ||
			extra.some((r) => r.name.trim() || r.value.trim())
		);
	}

	function close(rememberDraft = true) {
		app.eventDraft =
			rememberDraft && hasAnything()
				? {
						template: isCustom ? null : selected,
						kind: customKind,
						content,
						values: { ...values },
						extra: extra.map((r) => ({ ...r }))
					}
				: null;
		app.eventComposer = null;
	}

	async function post() {
		if (!canPost) return;
		const req = buildRequest();
		if (!req) return;
		posting = true;
		try {
			const resp = await api.publishEvent(req);
			const { successful, total } = resp.broadcast;
			const label = template ? template.title : `kind ${req.kind}`;
			app.pushToast(
				total === 0
					? `${label} saved locally (no publish relays)`
					: successful === 0
						? `${label} saved locally — 0/${total} relays accepted`
						: `${label} published (${successful}/${total} relays)`,
				successful === 0 && total > 0 ? 'error' : 'success'
			);
			close(false);
		} catch (e) {
			app.pushToast(api.errorMessage(e, 'Publish failed'), 'error', 5000);
		} finally {
			posting = false;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if (posting) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		} else if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
			e.preventDefault();
			post();
		}
	}

	function fieldLabel(f: api.EventTagField): string {
		return f.label || f.name;
	}
	function placeholder(f: api.EventTagField): string {
		switch (f.type) {
			case 'url':
				return 'https://…';
			case 'date':
				return 'YYYY-MM-DD';
			case 'integer':
				return '0';
			case 'language':
				return 'en';
			case 'pubkey':
				return '64-hex pubkey';
			default:
				return f.multi ? 'one per line' : '';
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- One field: label row (name, required star, fixed chips, derived badge),
     the widget for its type, then the engine's problems or the hint. -->
{#snippet field(f: api.EventTagField)}
	{@const errs = problemsByField[f.name] ?? []}
	<div class="evc-field {f.multi ? 'evc-field--wide' : ''}">
		<label class="evc-label" for="evc-f-{f.name}">
			{fieldLabel(f)}{#if f.required}<span class="evc-req" title="required">*</span>{/if}
			{#if f.fixed.length}
				{#each f.fixed as v (v)}<span class="evc-chip">{v}</span>{/each}
			{/if}
			{#if f.auto && !(values[f.name] ?? '').trim()}
				<span class="evc-badge evc-badge--dim">derived</span>
			{/if}
		</label>
		{#if f.multi}
			<textarea
				id="evc-f-{f.name}"
				class="evc-input evc-multi {errs.length ? 'evc-input--bad' : ''}"
				data-entry
				placeholder={placeholder(f)}
				bind:value={values[f.name]}
				disabled={posting}
			></textarea>
		{:else if f.type === 'boolean'}
			<select
				id="evc-f-{f.name}"
				class="evc-input evc-select {errs.length ? 'evc-input--bad' : ''}"
				bind:value={values[f.name]}
				disabled={posting}
			>
				<option value="">—</option>
				<option value="true">true</option>
				<option value="false">false</option>
			</select>
		{:else if mintable(f)}
			<div class="evc-row">
				<input
					id="evc-f-{f.name}"
					class="evc-input evc-mono {errs.length ? 'evc-input--bad' : ''}"
					data-entry
					placeholder="paste an existing identifier, or generate"
					bind:value={values[f.name]}
					disabled={posting}
				/>
				<button class="evc-btn" onclick={mintD} disabled={posting || minting}>
					{minting ? '…' : 'generate'}
				</button>
			</div>
		{:else}
			<input
				id="evc-f-{f.name}"
				class="evc-input {errs.length ? 'evc-input--bad' : ''}"
				data-entry
				inputmode={f.type === 'integer' ? 'numeric' : undefined}
				placeholder={placeholder(f)}
				bind:value={values[f.name]}
				disabled={posting}
			/>
		{/if}
		{#each errs as m (m)}
			<div class="evc-detail evc-detail--bad">{m}</div>
		{/each}
		{#if !errs.length && f.hint}
			<div class="evc-detail">{f.hint}</div>
		{/if}
	</div>
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="evc-backdrop {yielding ? 'evc-backdrop--yield' : ''}"
	onclick={() => {
		if (!posting) close();
	}}
	role="presentation"
>
	<div
		class="evc"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Compose event"
		tabindex="-1"
	>
		<header class="evc__header">
			<h3 class="evc-title">{editing ? 'edit event' : 'compose event'}</h3>
			{#if kindNumber !== null}
				<span class="evc-badge">kind {kindNumber}</span>
			{/if}
			<span class="evc-spacer"></span>
			<button class="evc-close" onclick={() => close()} aria-label="Close">×</button>
		</header>

		<div class="evc__body">
			<section class="evc__section">
				<div class="evc-field">
					<label class="evc-label" for="evc-template">template</label>
					<div class="evc-row">
						<select
							id="evc-template"
							class="evc-input evc-select"
							bind:value={selected}
							disabled={posting}
						>
							{#each templates as t (t.name)}
								<option value={t.name}
									>{t.title} · {t.kind}{t.user_defined ? ' · yours' : ''}</option
								>
							{/each}
							<option value={CUSTOM}>custom kind…</option>
						</select>
						{#if isCustom}
							<input
								class="evc-input evc-kind"
								data-entry
								inputmode="numeric"
								placeholder="kind"
								bind:value={customKind}
								disabled={posting}
							/>
						{/if}
					</div>
					{#if templatesErr}
						<div class="evc-detail evc-detail--bad">{templatesErr}</div>
					{:else if template}
						<div class="evc-detail evc-detail--wrap">
							{template.description ?? ''}{#if template.spec}
								<span class="evc-spec">{' — '}{template.spec}</span>{/if}
						</div>
					{:else if isCustom}
						<div class="evc-detail evc-detail--wrap">
							Any kind, any tags. An addressable kind (30000–39999) needs an identifier —
							generate one or paste an existing one.
						</div>
					{/if}
				</div>
			</section>

			{#if template}
				{@const required = template.tags.filter((f) => f.required)}
				{@const optional = template.tags.filter((f) => !f.required)}
				{#if required.length}
					<section class="evc__section">
						<div class="evc-section-title">required</div>
						{#each required as f (f.name)}
							{@render field(f)}
						{/each}
					</section>
				{/if}
				{#if optional.length}
					<section class="evc__section">
						<div class="evc-section-title">optional</div>
						<div class="evc-grid">
							{#each optional as f (f.name)}
								{@render field(f)}
							{/each}
						</div>
					</section>
				{/if}
			{/if}

			{#if customNeedsD}
				{@const errs = problemsByField.d ?? []}
				<section class="evc__section">
					<div class="evc-field">
						<label class="evc-label" for="evc-f-d">
							identifier<span class="evc-req" title="required">*</span>
						</label>
						<div class="evc-row">
							<input
								id="evc-f-d"
								class="evc-input evc-mono {errs.length ? 'evc-input--bad' : ''}"
								data-entry
								placeholder="paste an existing identifier, or generate"
								bind:value={values.d}
								disabled={posting}
							/>
							<button class="evc-btn" onclick={mintD} disabled={posting || minting}>
								{minting ? '…' : 'generate'}
							</button>
						</div>
						{#each errs as m (m)}
							<div class="evc-detail evc-detail--bad">{m}</div>
						{/each}
						{#if !errs.length}
							<div class="evc-detail">
								kind {kindNumber} is addressable — a pasted identifier replaces that event
							</div>
						{/if}
					</div>
				</section>
			{/if}

			<section class="evc__section">
				<div class="evc-field">
					<label class="evc-label" for="evc-content">
						{template?.content.label || 'content'}{#if template?.content.required}<span
								class="evc-req">*</span>{/if}
					</label>
					<textarea
						id="evc-content"
						class="evc-input evc-content {problemsByField.content ? 'evc-input--bad' : ''}"
						data-entry
						placeholder={template?.content.hint ?? 'Event content'}
						bind:value={content}
						disabled={posting}
					></textarea>
					{#each problemsByField.content ?? [] as m (m)}
						<div class="evc-detail evc-detail--bad">{m}</div>
					{/each}
				</div>
			</section>

			<section class="evc__section">
				<div class="evc-section-title">
					{template ? 'more tags' : 'tags'}
					<button
						class="evc-link"
						onclick={() => (extra = [...extra, { name: '', value: '' }])}
						disabled={posting}>+ add</button
					>
				</div>
				{#each extra as row, i (i)}
					{@const errs = template ? [] : (problemsByField[row.name.trim()] ?? [])}
					<div class="evc-field">
						<div class="evc-row">
							<input
								class="evc-input evc-tagname"
								data-entry
								placeholder="name"
								bind:value={row.name}
								disabled={posting}
							/>
							<input
								class="evc-input evc-tagvalue {errs.length ? 'evc-input--bad' : ''}"
								data-entry
								placeholder="value"
								bind:value={row.value}
								disabled={posting}
							/>
							<button
								class="evc-close"
								aria-label="Remove tag"
								onclick={() => (extra = extra.filter((_, j) => j !== i))}
								disabled={posting}>×</button
							>
						</div>
						{#if row.rest?.length}
							<div class="evc-detail">+ {row.rest.join(' · ')}</div>
						{/if}
						{#each errs as m (m)}
							<div class="evc-detail evc-detail--bad">{m}</div>
						{/each}
					</div>
				{:else}
					<div class="evc-detail">none — tags the template doesn't declare go here</div>
				{/each}
			</section>

			<section class="evc__section">
				<button class="evc-json-toggle" onclick={() => (jsonOpen = !jsonOpen)}>
					{jsonOpen ? '▾' : '▸'} event json
				</button>
				{#if jsonOpen}
					{#if previewErr}
						<div class="evc-detail evc-detail--bad">{previewErr}</div>
					{:else if jsonText}
						<pre class="evc-json">{jsonText}</pre>
					{:else}
						<div class="evc-detail">pick a template or a kind to preview the event</div>
					{/if}
				{/if}
			</section>
		</div>

		<footer class="evc__footer">
			<span class="evc-hint">
				{#if !canSign}
					sign in to publish
				{:else if previewErr}
					{previewErr}
				{:else if problems.length}
					{problems.length} to fix
				{:else if previewed}
					ready · Ctrl-Enter
				{:else}
					fill the form
				{/if}
			</span>
			<span class="evc-spacer"></span>
			<button class="evc-btn" onclick={() => close()} disabled={posting}>Cancel</button>
			<button class="evc-btn evc-btn--post" onclick={post} disabled={!canPost}>
				{posting ? 'Publishing…' : editing ? 'Replace' : 'Publish'}
			</button>
		</footer>
	</div>
</div>

<style>
	/* Chrome twinned with EventViewModal (the event "menu"): same scrim,
	   width, height cap and section rhythm, so the two read as one family.
	   Unlike the menu, the BODY is the scroll container — header and footer
	   stay put so Publish is always one click away on a long form. */
	.evc-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
	}
	/* Stand down while PublishConfirmModal (z 250) is deciding the post —
	   visibility (not display) keeps bound elements alive. */
	.evc-backdrop--yield {
		visibility: hidden;
	}
	.evc {
		width: min(720px, 90vw);
		max-height: 80dvh;
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: var(--radius);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.evc__header,
	.evc__footer {
		flex: 0 0 auto;
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 14px;
	}
	.evc__header {
		border-bottom: 1px solid var(--border);
	}
	.evc__footer {
		border-top: 1px solid var(--border);
	}
	.evc__body {
		flex: 1 1 auto;
		min-height: 0;
		overflow-y: auto;
	}
	.evc__section {
		padding: 12px 14px;
		border-bottom: 1px solid var(--border);
		display: flex;
		flex-direction: column;
		gap: 12px;
	}
	.evc__section:last-child {
		border-bottom: none;
	}
	.evc-section-title {
		display: flex;
		align-items: center;
		gap: 10px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--base5);
	}
	.evc-title {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-sm);
		color: var(--fg);
	}
	.evc-close {
		background: none;
		border: none;
		color: var(--base5);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 0 4px;
	}
	.evc-close:hover:not(:disabled) {
		color: var(--fg);
	}
	/* Optional fields flow into columns on a wide modal, one per row on a
	   phone; a multi-line field always spans the full width. */
	.evc-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 12px 16px;
	}
	.evc-field {
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
	}
	.evc-field--wide {
		grid-column: 1 / -1;
	}
	.evc-label {
		display: flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.evc-req {
		color: var(--id-yours);
	}
	.evc-badge,
	.evc-chip {
		font-size: calc(var(--t-xs) - 2px);
		color: var(--id-yours);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 0 6px;
	}
	.evc-chip {
		color: var(--fg-muted);
		background: var(--bg-surface);
	}
	.evc-badge--dim {
		color: var(--base5);
	}
	.evc-link {
		background: none;
		border: none;
		padding: 0;
		font: inherit;
		letter-spacing: 0;
		text-transform: none;
		color: var(--id-yours);
		cursor: pointer;
	}
	.evc-link:hover:not(:disabled) {
		text-decoration: underline;
	}
	.evc-row {
		display: flex;
		gap: 8px;
		align-items: center;
	}
	.evc-row .evc-input {
		flex: 1;
		min-width: 0;
	}
	.evc-input {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 6px 9px;
	}
	.evc-input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.evc-input--bad {
		border-color: var(--error, #e06c75);
	}
	.evc-select,
	.evc-mono,
	.evc-kind,
	.evc-tagname {
		font-family: var(--font-mono);
	}
	.evc-kind {
		flex: 0 0 6em;
	}
	.evc-tagname {
		flex: 0 0 10em;
	}
	.evc-multi {
		min-height: 52px;
		resize: vertical;
	}
	.evc-content {
		min-height: 120px;
		resize: vertical;
	}
	.evc-detail {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evc-detail--wrap {
		white-space: normal;
	}
	.evc-detail--bad {
		color: var(--error, #e06c75);
		white-space: normal;
	}
	.evc-spec {
		color: var(--fg-muted);
	}
	.evc-json-toggle {
		align-self: flex-start;
		background: none;
		border: none;
		padding: 0;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		cursor: pointer;
	}
	.evc-json-toggle:hover {
		color: var(--fg);
	}
	.evc-json {
		margin: 0;
		padding: 8px 10px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--fg-muted);
		overflow: auto;
		white-space: pre;
	}
	.evc-hint {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.evc-spacer {
		flex: 1;
	}
	.evc-btn {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base6);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 11px;
		cursor: pointer;
		white-space: nowrap;
	}
	.evc-btn:hover:not(:disabled) {
		color: var(--fg);
		border-color: var(--base5);
	}
	.evc-btn--post {
		color: var(--id-yours);
	}
	.evc-btn:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
