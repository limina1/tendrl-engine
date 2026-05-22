<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import * as api from '$lib/api';
	import { signAndBroadcast, identityCanSign } from '$lib/identity/signer';
	import type { Buffer } from '../types';

	let { buffer: _buffer }: { buffer: Buffer } = $props();

	const app = getAppState();

	type Form = {
		name: string;
		display_name: string;
		about: string;
		picture: string;
		banner: string;
		nip05: string;
		lud16: string;
		website: string;
	};

	const empty: Form = {
		name: '',
		display_name: '',
		about: '',
		picture: '',
		banner: '',
		nip05: '',
		lud16: '',
		website: ''
	};

	let form = $state<Form>({ ...empty });
	let unknownFields = $state<Record<string, unknown>>({});
	let loading = $state(true);
	let saving = $state(false);
	let error = $state<string | null>(null);
	let lastSave = $state<{ pubkey: string; relayCount: number; successCount: number } | null>(null);
	let rawJson = $state('');

	const activePubkey = $derived(
		app.identityStatus?.pubkey ?? app.externalSignerPubkey ?? null
	);
	const canSign = $derived(identityCanSign(app.identityStatus));

	// Fetch the current kind 0 directly from the engine. profile_handler
	// returns the normalized fields it knows about; we also fetch the raw
	// event so unknown fields (lud16, banner, website) survive editing.
	async function load() {
		if (!activePubkey) {
			loading = false;
			return;
		}
		loading = true;
		error = null;
		try {
			// Use a direct kind-0 query so we get the *raw* content blob,
			// not just the normalized subset profile_handler returns.
			const resp = await fetch('/api/v1/query', {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				body: JSON.stringify({
					filters: [{ kinds: [0], authors: [activePubkey], limit: 1 }],
					policy: 'local_only'
				})
			});
			if (!resp.ok) throw new Error(`engine HTTP ${resp.status}`);
			const data = (await resp.json()) as { events?: { content?: string }[] };
			const event = data.events?.[0];
			if (event?.content) {
				rawJson = event.content;
				try {
					const parsed = JSON.parse(event.content) as Record<string, unknown>;
					hydrate(parsed);
				} catch (e) {
					console.warn('[ProfileEdit] kind 0 content not valid JSON', e);
				}
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	function hydrate(parsed: Record<string, unknown>) {
		const known: (keyof Form)[] = [
			'name',
			'display_name',
			'about',
			'picture',
			'banner',
			'nip05',
			'lud16',
			'website'
		];
		const next: Form = { ...empty };
		const unknown: Record<string, unknown> = {};
		for (const [k, v] of Object.entries(parsed)) {
			if ((known as string[]).includes(k) && typeof v === 'string') {
				(next as Record<string, string>)[k] = v;
			} else {
				unknown[k] = v;
			}
		}
		form = next;
		unknownFields = unknown;
	}

	function buildContent(): string {
		// Preserve unknown fields verbatim. Strip empty known fields so
		// we don't write `"name":""` into the metadata blob.
		const out: Record<string, unknown> = { ...unknownFields };
		for (const [k, v] of Object.entries(form)) {
			if (v.trim()) out[k] = v.trim();
		}
		return JSON.stringify(out);
	}

	async function save() {
		if (!canSign || !activePubkey) return;
		saving = true;
		error = null;
		lastSave = null;
		try {
			const template = {
				kind: 0,
				created_at: Math.floor(Date.now() / 1000),
				tags: [] as string[][],
				content: buildContent(),
				pubkey: activePubkey
			};
			const { signed, broadcast } = await signAndBroadcast(template);
			lastSave = {
				pubkey: signed.pubkey,
				relayCount: broadcast.total,
				successCount: broadcast.successful
			};
			rawJson = template.content;
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			saving = false;
		}
	}

	$effect(() => {
		if (activePubkey) load();
	});
</script>

<div class="pe-view">
	<div class="pe-header">
		<span>Edit profile (kind 0)</span>
		{#if app.identityStatus}
			<span class="pe-source-pill">source: {app.identityStatus.source}</span>
		{/if}
	</div>

	{#if !canSign}
		<p class="pe-empty">
			You're not signed in. Open <code>SPC s i</code> to choose an identity source first.
		</p>
	{:else if !activePubkey}
		<p class="pe-empty">No active pubkey resolved yet — try again in a moment.</p>
	{:else if loading}
		<p class="pe-empty">Loading current profile…</p>
	{:else}
		<div class="pe-form">
			<label class="pe-row">
				<span class="pe-label">name</span>
				<input class="pe-input" bind:value={form.name} placeholder="ixcila" />
			</label>
			<label class="pe-row">
				<span class="pe-label">display_name</span>
				<input
					class="pe-input"
					bind:value={form.display_name}
					placeholder="Ixcila"
				/>
			</label>
			<label class="pe-row pe-row--stretch">
				<span class="pe-label">about</span>
				<textarea
					class="pe-input pe-input--textarea"
					bind:value={form.about}
					placeholder="What you do, in a sentence."
					rows="3"
				></textarea>
			</label>
			<label class="pe-row">
				<span class="pe-label">picture</span>
				<input class="pe-input" bind:value={form.picture} placeholder="https://…" />
			</label>
			<label class="pe-row">
				<span class="pe-label">banner</span>
				<input class="pe-input" bind:value={form.banner} placeholder="https://…" />
			</label>
			<label class="pe-row">
				<span class="pe-label">nip05</span>
				<input class="pe-input" bind:value={form.nip05} placeholder="user@domain.tld" />
			</label>
			<label class="pe-row">
				<span class="pe-label">lud16</span>
				<input
					class="pe-input"
					bind:value={form.lud16}
					placeholder="lightning@domain.tld"
				/>
			</label>
			<label class="pe-row">
				<span class="pe-label">website</span>
				<input class="pe-input" bind:value={form.website} placeholder="https://…" />
			</label>

			{#if Object.keys(unknownFields).length > 0}
				<details class="pe-unknown">
					<summary
						>{Object.keys(unknownFields).length} unknown field{Object.keys(unknownFields)
							.length === 1
							? ''
							: 's'}
						preserved</summary
					>
					<pre class="pe-unknown-pre">{JSON.stringify(unknownFields, null, 2)}</pre>
				</details>
			{/if}

			<div class="pe-actions">
				<button class="pe-action pe-action--primary" onclick={save} disabled={saving}>
					{saving ? 'Signing + broadcasting…' : 'Save profile'}
				</button>
				<button class="pe-action" onclick={load} disabled={saving || loading}>
					Reload current
				</button>
			</div>

			{#if error}
				<p class="pe-error">{error}</p>
			{/if}
			{#if lastSave}
				<p class="pe-success">
					Published kind 0 for <code>{lastSave.pubkey.slice(0, 12)}…</code>
					to {lastSave.successCount} / {lastSave.relayCount} relays.
				</p>
			{/if}

			{#if rawJson}
				<details class="pe-raw">
					<summary>Current kind-0 content</summary>
					<pre class="pe-unknown-pre">{rawJson}</pre>
				</details>
			{/if}
		</div>
	{/if}
</div>

<style>
	.pe-view {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 0 0 24px;
	}

	.pe-header {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		padding: 10px 14px;
		font-size: var(--t-xs);
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base6);
		border-bottom: 1px solid var(--panel-border);
	}

	.pe-source-pill {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		font-weight: 400;
		color: var(--base5);
		text-transform: none;
		letter-spacing: 0;
	}

	.pe-empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.pe-empty code {
		font-family: var(--font-mono);
		background: var(--bg-surface);
		padding: 1px 5px;
		border-radius: var(--r-sm);
	}

	.pe-form {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding: 12px 16px;
	}

	.pe-row {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.pe-label {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--base6);
	}
	.pe-input {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 6px 8px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: var(--bg);
		color: var(--fg);
		width: 100%;
		box-sizing: border-box;
	}
	.pe-input--textarea {
		resize: vertical;
		font-family: inherit;
	}

	.pe-unknown {
		font-size: var(--t-xs);
		color: var(--base5);
	}
	.pe-unknown-pre {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		background: var(--bg-surface);
		padding: 8px;
		border-radius: var(--r-sm);
		overflow-x: auto;
		white-space: pre-wrap;
		word-break: break-all;
	}

	.pe-actions {
		display: flex;
		gap: 8px;
		margin-top: 4px;
	}
	.pe-action {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 6px 14px;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: transparent;
		color: var(--fg);
		cursor: pointer;
	}
	.pe-action:hover:not(:disabled) {
		border-color: var(--id-yours);
		color: var(--id-yours);
	}
	.pe-action:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
	.pe-action--primary {
		border-color: var(--state-online);
		color: var(--state-online);
	}

	.pe-error {
		color: var(--id-draft);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		margin: 6px 0 0;
	}
	.pe-success {
		color: var(--state-online);
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		margin: 6px 0 0;
	}
	.pe-success code {
		background: var(--bg-surface);
		padding: 1px 4px;
		border-radius: var(--r-sm);
	}

	.pe-raw {
		font-size: var(--t-xs);
		color: var(--base5);
	}
</style>
