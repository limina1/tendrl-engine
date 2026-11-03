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

	// Prefill: the open payload's template wins; otherwise the draft
	// remembered from the last close. Publish clears the memory.
	const draft: EventComposerDraft | null = app.eventDraft;
	const opener = app.eventComposer;
	let templates = $state<api.KindTemplate[]>([]);
	let templatesErr = $state<string | null>(null);
	let selected = $state<string>(
		opener?.template ?? (draft ? (draft.template ?? CUSTOM) : '')
	);
	let customKind = $state(draft?.kind ?? '');
	let content = $state(draft?.content ?? '');
	let values = $state<Record<string, string>>({ ...(draft?.values ?? {}) });
	let extra = $state<{ name: string; value: string }[]>(
		(draft?.extra ?? []).map((r) => ({ ...r }))
	);
	let posting = $state(false);

	const template = $derived(
		selected && selected !== CUSTOM ? (templates.find((t) => t.name === selected) ?? null) : null
	);
	const isCustom = $derived(selected === CUSTOM);
	const kindNumber = $derived(
		template ? template.kind : /^\d{1,9}$/.test(customKind.trim()) ? Number(customKind.trim()) : null
	);

	// Confirm mode: see the modal-yield rule in feat-ui-patterns — hide
	// while PublishConfirmModal (z 250) decides the broadcast.
	const yielding = $derived(posting && confirmState.intent?.type === 'publish_intent');

	api.listEventTemplates().then(
		(r) => {
			templates = r.templates;
			// A fresh open with nothing remembered lands on the first template.
			if (!selected && templates.length) selected = templates[0].name;
			else if (!selected) selected = CUSTOM;
		},
		(e) => {
			templatesErr = api.errorMessage(e, 'templates unavailable');
			if (!selected) selected = CUSTOM;
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
		for (const r of extra) {
			if (r.name.trim()) tags.push([r.name.trim(), r.value.trim()]);
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

	const problemsByField = $derived.by(() => {
		const m: Record<string, string[]> = {};
		for (const p of problems) (m[p.field] ??= []).push(p.message);
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

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div
	class="evc-backdrop {yielding ? 'evc-backdrop--yield' : ''}"
	onclick={() => {
		if (!posting) close();
	}}
	role="presentation"
>
	<div
		class="evc-modal"
		onclick={(e) => e.stopPropagation()}
		role="dialog"
		aria-modal="true"
		aria-label="Compose event"
		tabindex="-1"
	>
		<header class="evc-header">
			<h3 class="evc-title">compose event</h3>
			<button class="evc-close" onclick={() => close()} aria-label="Close">×</button>
		</header>

		<label class="evc-label" for="evc-template">
			template
			{#if kindNumber !== null}
				<span class="evc-badge">kind {kindNumber}</span>
			{/if}
		</label>
		<div class="evc-row">
			<select id="evc-template" class="evc-input evc-select" bind:value={selected} disabled={posting}>
				{#each templates as t (t.name)}
					<option value={t.name}>{t.title} · {t.kind}{t.user_defined ? ' · yours' : ''}</option>
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
					<span class="evc-spec"> — {template.spec}</span>{/if}
			</div>
		{:else if isCustom}
			<div class="evc-detail evc-detail--wrap">
				Any kind, any tags. An addressable kind (30000–39999) gets a <code>d</code> slugged
				from a <code>title</code> tag unless you add one.
			</div>
		{/if}

		{#if template}
			{#each template.tags as f (f.name)}
				{@const errs = problemsByField[f.name] ?? []}
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
			{/each}
		{/if}

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

		<div class="evc-label">
			{template ? 'more tags' : 'tags'}
			<button
				class="evc-link"
				onclick={() => (extra = [...extra, { name: '', value: '' }])}
				disabled={posting}>+ add</button
			>
		</div>
		{#each extra as row, i (i)}
			{@const errs = template ? [] : (problemsByField[row.name.trim()] ?? [])}
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
			{#each errs as m (m)}
				<div class="evc-detail evc-detail--bad">{m}</div>
			{/each}
		{/each}
		{#if isCustom}
			{#each problemsByField.d ?? [] as m (m)}
				{#if !extra.some((r) => r.name.trim() === 'd')}
					<div class="evc-detail evc-detail--bad">d: {m}</div>
				{/if}
			{/each}
		{/if}

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

		<footer class="evc-foot">
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
				{posting ? 'Publishing…' : 'Publish'}
			</button>
		</footer>
	</div>
</div>

<style>
	.evc-backdrop {
		position: fixed;
		inset: 0;
		z-index: 300;
		background: rgba(0, 0, 0, 0.45);
		display: flex;
		align-items: flex-start;
		justify-content: center;
		padding-top: 8dvh;
	}
	.evc-backdrop--yield {
		visibility: hidden;
	}
	.evc-modal {
		width: min(560px, 92vw);
		max-height: 86dvh;
		overflow-y: auto;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-sm, 3px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
		padding: 12px;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.evc-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
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
	.evc-label {
		display: flex;
		align-items: center;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
		margin-top: 6px;
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
		color: var(--id-yours);
		cursor: pointer;
	}
	.evc-link:hover:not(:disabled) {
		text-decoration: underline;
	}
	.evc-row {
		display: flex;
		gap: 6px;
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
		padding: 5px 8px;
	}
	.evc-input:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.evc-input--bad {
		border-color: var(--error, #e06c75);
	}
	.evc-select {
		font-family: var(--font-mono);
	}
	.evc-kind {
		flex: 0 0 6em;
		font-family: var(--font-mono);
	}
	.evc-tagname {
		flex: 0 0 9em;
		font-family: var(--font-mono);
	}
	.evc-multi {
		min-height: 44px;
		resize: vertical;
	}
	.evc-content {
		min-height: 88px;
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
		margin-top: 6px;
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
		padding: 6px 8px;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--fg-muted);
		max-height: 220px;
		overflow: auto;
		white-space: pre;
	}
	.evc-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 8px;
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
		padding: 3px 10px;
		cursor: pointer;
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
