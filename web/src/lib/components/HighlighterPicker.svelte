<script lang="ts">
	// The highlighter section of a highlight capture: pick a label + colour
	// (a "highlight style") for the 9802. Chips are the pairs found on
	// existing highlights — *Your highlights*, or *Use another's* (an npub's)
	// — via the engine (`GET /api/v1/highlighters`). Picking a chip fills the
	// label and colour; editing either makes a new pair. `plain` = no
	// highlighter tag. Decisions: docs/zettel/idea-highlighters.org.
	import * as api from '$lib/api';
	import type { Highlighter } from '$lib/discussions/highlights';
	import { labelToColor } from '$lib/discussions/colors';
	import ColorPicker from './ColorPicker.svelte';

	let {
		value = $bindable(null),
		disabled = false
	}: {
		value?: Highlighter | null;
		disabled?: boolean;
	} = $props();

	type Tab = 'yours' | 'theirs';
	let tab = $state<Tab>('yours');
	let yours = $state<api.HighlighterPair[]>([]);
	let yoursCount = $state(0);
	let npub = $state('');
	let theirs = $state<api.HighlightersResponse | null>(null);
	let loading = $state(false);
	let loadError = $state('');
	// The label's colour was picked by hand — typing no longer re-derives it.
	let colorTouched = $state(value !== null);
	// Mirrors of `value` the inputs edit.
	// svelte-ignore state_referenced_locally
	let label = $state(value?.label ?? '');
	// svelte-ignore state_referenced_locally
	let color = $state(value?.color ?? labelToColor('x'));

	$effect(() => {
		api
			.listHighlighters()
			.then((r) => {
				yours = r.highlighters;
				yoursCount = r.highlights;
			})
			.catch(() => {
				/* no identity — no "yours" chips */
			});
	});

	const pairs = $derived(tab === 'yours' ? yours : (theirs?.highlighters ?? []));
	const known = $derived([...yours, ...(theirs?.highlighters ?? [])]);

	function emit() {
		const l = label.trim();
		value = l ? { label: l, color } : null;
	}

	function pick(p: { label: string; color: string } | null) {
		label = p?.label ?? '';
		if (p) color = p.color;
		colorTouched = p !== null;
		emit();
	}

	function onLabel(e: Event) {
		label = (e.currentTarget as HTMLInputElement).value;
		const hit = known.find((p) => p.label === label.trim());
		if (hit) {
			color = hit.color;
			colorTouched = true;
		} else if (!colorTouched && label.trim()) {
			color = labelToColor(label);
		}
		emit();
	}

	function onColor(hex: string) {
		color = hex;
		colorTouched = true;
		if (!label.trim()) return;
		emit();
	}

	async function load(policy: 'local_only' | 'fetch_always') {
		const who = npub.trim();
		if (!who || loading) return;
		loading = true;
		loadError = '';
		try {
			theirs = await api.listHighlighters(who, policy);
		} catch (e) {
			theirs = null;
			loadError = api.errorMessage(e, 'Could not read their highlights');
		} finally {
			loading = false;
		}
	}

	const tagLine = $derived(
		label.trim() ? `["highlighter", "${label.trim()}", "${color}"]` : 'plain highlight — no highlighter tag'
	);
</script>

<div class="hp" data-tour="highlighter-picker">
	<div class="hp-head">
		<span class="hp-lab">highlighter</span>
		<div class="hp-tabs" role="tablist" aria-label="Highlighter source">
			<button
				type="button"
				role="tab"
				aria-selected={tab === 'yours'}
				class:on={tab === 'yours'}
				onclick={() => (tab = 'yours')}>Your highlights</button
			>
			<button
				type="button"
				role="tab"
				aria-selected={tab === 'theirs'}
				class:on={tab === 'theirs'}
				onclick={() => (tab = 'theirs')}>Use another's</button
			>
		</div>
	</div>

	{#if tab === 'theirs'}
		<form
			class="hp-npub"
			onsubmit={(e) => {
				e.preventDefault();
				void load('local_only');
			}}
		>
			<input
				class="hp-field hp-mono"
				data-entry
				placeholder="npub1… or hex pubkey"
				aria-label="Their npub"
				bind:value={npub}
				{disabled}
			/>
			<button type="submit" class="hp-btn" disabled={disabled || loading || !npub.trim()}>
				{loading ? '…' : 'Load'}
			</button>
		</form>
		{#if loadError}<div class="hp-note hp-err">{loadError}</div>{/if}
	{/if}

	{#if tab === 'yours' || theirs}
		<div class="hp-note">
			{#if tab === 'yours'}
				{yours.length ? `found on your ${yoursCount} highlights` : 'no labelled highlights yet — type a label'}
			{:else if theirs}
				{#if theirs.highlighters.length}
					found on {theirs.highlights} of their highlights — pick one to use their style
				{:else}
					none of their highlights are stored here
					<button type="button" class="hp-link" onclick={() => load('fetch_always')} disabled={loading}
						>fetch from relays</button
					>
				{/if}
			{/if}
		</div>
		<div class="hp-chips">
			<button
				type="button"
				class="hp-chip hp-chip--plain"
				class:on={!label.trim()}
				onclick={() => pick(null)}
				{disabled}>plain</button
			>
			{#each pairs as p (p.label)}
				<button
					type="button"
					class="hp-chip"
					class:on={label.trim() === p.label && color === p.color}
					title="{p.label} · {p.color} · on {p.count} highlight{p.count === 1 ? '' : 's'}"
					onclick={() => pick(p)}
					{disabled}
				>
					<span class="hp-dot" style="background: {p.color}"></span>
					<span>{p.label}</span>
					<span class="hp-n">{p.count}</span>
				</button>
			{/each}
		</div>
	{/if}

	<div class="hp-edit">
		<input
			class="hp-field hp-label"
			data-entry
			placeholder="new label, or pick one"
			aria-label="Highlighter label"
			maxlength="64"
			value={label}
			oninput={onLabel}
			{disabled}
		/>
		<ColorPicker value={color} onchange={onColor} {disabled} />
	</div>
	<div class="hp-tag">{tagLine}</div>
</div>

<style>
	.hp {
		display: flex;
		flex-direction: column;
		gap: 6px;
		border-top: 1px solid var(--panel-border);
		padding-top: 6px;
	}
	.hp-head {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.hp-lab {
		flex: 1;
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
	}
	.hp-tabs {
		display: flex;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		overflow: hidden;
	}
	.hp-tabs button {
		font: inherit;
		font-size: calc(var(--t-xs) - 1px);
		padding: 3px 8px;
		border: 0;
		background: transparent;
		color: var(--base5);
		cursor: pointer;
	}
	.hp-tabs button + button {
		border-left: 1px solid var(--panel-border);
	}
	.hp-tabs button.on {
		background: var(--base2);
		color: var(--fg);
	}
	.hp-npub {
		display: flex;
		gap: 6px;
	}
	.hp-field {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 4px 8px;
		min-width: 0;
	}
	.hp-field:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.hp-mono {
		flex: 1;
		font-family: var(--font-mono);
	}
	.hp-btn,
	.hp-link {
		font: inherit;
		font-size: var(--t-xs);
		color: var(--fg-muted);
		background: var(--base2);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 10px;
		cursor: pointer;
	}
	.hp-link {
		background: none;
		border: 0;
		padding: 0;
		color: var(--id-yours);
		text-decoration: underline;
	}
	.hp-note {
		font-size: calc(var(--t-xs) - 1px);
		color: var(--base5);
	}
	.hp-err {
		color: var(--state-error);
	}
	.hp-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}
	.hp-chip {
		font: inherit;
		font-size: calc(var(--t-xs) - 1px);
		display: inline-flex;
		align-items: center;
		gap: 5px;
		padding: 2px 7px;
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
	.hp-chip:hover:not(:disabled) {
		border-color: var(--base5);
	}
	.hp-chip.on {
		border-color: var(--fg);
	}
	.hp-chip--plain {
		color: var(--base5);
	}
	.hp-dot {
		width: 9px;
		height: 9px;
		border-radius: 2px;
	}
	.hp-n {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 3px);
		color: var(--base5);
	}
	.hp-edit {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		flex-wrap: wrap;
	}
	.hp-label {
		flex: 1;
		min-width: 110px;
	}
	.hp-tag {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		color: var(--base5);
		background: var(--panel-rail-bg, var(--bg-alt));
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 6px;
		overflow-wrap: anywhere;
	}
</style>
