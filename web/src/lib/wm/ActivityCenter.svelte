<script lang="ts">
	import type { NetworkStatus } from '$lib/types';

	// The network-activity center list — active fetches (with kill ×) and
	// recent ones (with expand-cause ▸). Extracted from +page's modeline
	// popover so the mobile drawer renders the same list inline: one
	// implementation for both shells, per the mobile "shell split, not
	// component split" rule. Positioning/anchoring stays with the caller;
	// onKill() with no id means "kill all".
	let {
		activity = null,
		onKill
	}: {
		activity?: NetworkStatus | null;
		onKill?: (id?: number) => void;
	} = $props();

	let expanded = $state<Record<number, boolean>>({});

	function shortRelay(url: string): string {
		return url.replace(/^wss?:\/\//, '').replace(/\/+$/, '');
	}
	function actElapsed(startedAt: number): string {
		const s = Math.max(0, Math.floor(Date.now() / 1000) - startedAt);
		return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m${s % 60}s`;
	}
</script>

<div class="act-head">
	<span>network activity</span>
	{#if (activity?.active ?? []).length > 0}
		<button class="act-killall" onclick={() => onKill?.()} title="Kill every in-flight fetch">
			kill all
		</button>
	{/if}
</div>
{#if (activity?.active ?? []).length > 0}
	<div class="act-sect">active</div>
	{#each activity?.active ?? [] as f (f.id)}
		<div class="act-row act-row--live">
			<span class="act-row__reason">{f.reason ?? f.trigger}</span>
			<span class="act-row__relay">{shortRelay(f.relay)}</span>
			<span class="act-row__meta">{actElapsed(f.started_at)}</span>
			<button
				class="act-row__btn act-row__btn--kill"
				onclick={() => onKill?.(f.id)}
				title="Kill this fetch">×</button
			>
		</div>
	{/each}
{/if}
<div class="act-sect">recent</div>
{#if (activity?.recent ?? []).length === 0}
	<div class="act-empty">No relay activity yet</div>
{/if}
{#each (activity?.recent ?? []).slice(0, 14) as r (r.id)}
	<div class="act-row">
		<span
			class="act-row__dot"
			class:act-row__dot--ok={r.success}
			class:act-row__dot--fail={!r.success}
			title={r.success ? 'ok' : r.error ?? 'failed'}
		></span>
		<span class="act-row__reason">{r.reason ?? r.trigger}</span>
		<span class="act-row__relay">{shortRelay(r.relay)}</span>
		<span class="act-row__meta">{r.event_count}ev · {r.duration_ms}ms</span>
		<button
			class="act-row__btn"
			onclick={() => (expanded[r.id] = !expanded[r.id])}
			title="Show cause & query"
		>
			{expanded[r.id] ? '▾' : '▸'}
		</button>
	</div>
	{#if expanded[r.id]}
		<div class="act-detail">
			<div><span class="act-detail__k">cause</span>{r.reason ?? `(none — trigger: ${r.trigger})`}</div>
			<div><span class="act-detail__k">query</span>{r.filter_summary}</div>
			{#if r.error}<div><span class="act-detail__k">error</span>{r.error}</div>{/if}
		</div>
	{/if}
{/each}

<style>
	.act-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 2px 10px 6px;
		color: var(--fg-muted);
		text-transform: lowercase;
		letter-spacing: 0.04em;
	}
	.act-killall {
		appearance: none;
		background: none;
		border: 1px solid color-mix(in srgb, var(--state-error, var(--red)) 45%, var(--panel-border));
		color: var(--state-error, var(--red));
		border-radius: var(--r-sm, 3px);
		font: inherit;
		padding: 1px 7px;
		cursor: pointer;
	}
	.act-killall:hover {
		background: color-mix(in srgb, var(--state-error, var(--red)) 12%, transparent);
	}
	.act-sect {
		padding: 4px 10px 2px;
		color: var(--fg-muted);
		opacity: 0.8;
		font-size: 0.9em;
		text-transform: lowercase;
	}
	.act-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 3px 10px;
		min-width: 0;
	}
	.act-row--live .act-row__reason {
		color: var(--state-online, var(--green));
	}
	.act-row__dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.act-row__dot--ok { background: var(--state-online, var(--green)); opacity: 0.7; }
	.act-row__dot--fail { background: var(--state-error, var(--red)); }
	.act-row__reason {
		color: var(--fg);
		flex-shrink: 0;
		max-width: 40%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.act-row__relay {
		color: var(--fg-muted);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		flex: 1;
		min-width: 0;
	}
	.act-row__meta {
		color: var(--fg-muted);
		font-variant-numeric: tabular-nums;
		flex-shrink: 0;
	}
	.act-row__btn {
		appearance: none;
		background: none;
		border: none;
		color: var(--fg-muted);
		font: inherit;
		padding: 0 3px;
		cursor: pointer;
		border-radius: 3px;
		flex-shrink: 0;
	}
	.act-row__btn:hover {
		background: color-mix(in srgb, var(--fg) 10%, transparent);
		color: var(--fg);
	}
	.act-row__btn--kill {
		color: var(--state-error, var(--red));
		font-size: var(--t-sm);
		line-height: 1;
	}
	.act-detail {
		margin: 0 10px 4px 25px;
		padding: 4px 8px;
		border-left: 2px solid var(--panel-border);
		color: var(--fg-muted);
		word-break: break-word;
	}
	.act-detail__k {
		display: inline-block;
		min-width: 44px;
		color: var(--fg);
		opacity: 0.7;
	}
	.act-empty {
		padding: 4px 10px 6px;
		color: var(--fg-muted);
		font-style: italic;
	}
</style>
