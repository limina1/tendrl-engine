<script lang="ts">
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();

	// Local expansion state, keyed by event index. Reset when the modal's
	// event list identity changes (a new inspector open).
	let expanded = $state(new Set<number>());
	let lastEvents: unknown = $state(null);
	let copiedKey = $state<string | null>(null);

	const data = $derived(app.eventsModal);

	$effect(() => {
		if (data && data.events !== lastEvents) {
			lastEvents = data.events;
			expanded = new Set(); // collapsed by default on open
		}
	});

	const allOpen = $derived(!!data && expanded.size === data.events.length && data.events.length > 0);

	function toggle(i: number) {
		const next = new Set(expanded);
		if (next.has(i)) next.delete(i);
		else next.add(i);
		expanded = next;
	}

	function toggleAll() {
		if (!data) return;
		expanded = allOpen ? new Set() : new Set(data.events.map((_, i) => i));
	}

	function pretty(json: unknown): string {
		try {
			return JSON.stringify(json, null, 2);
		} catch {
			return String(json);
		}
	}

	async function copy(text: string, key: string) {
		try {
			await navigator.clipboard.writeText(text);
			copiedKey = key;
			setTimeout(() => {
				if (copiedKey === key) copiedKey = null;
			}, 1200);
		} catch {
			/* clipboard unavailable */
		}
	}

	function copyAll() {
		if (!data) return;
		// Linked-but-uncached entries have no JSON body — skip them.
		copy(pretty(data.events.filter((e) => e.json != null).map((e) => e.json)), 'all');
	}

	function close() {
		app.eventsModal = null;
	}

	function kindLabel(kind?: number): string {
		if (kind === 30040) return '30040 index';
		if (kind === 30041) return '30041 section';
		if (kind === 30023) return '30023 long-form';
		if (kind === 30818) return '30818 wiki';
		if (kind === 30817) return '30817 wiki';
		return kind != null ? String(kind) : 'event';
	}
</script>

<svelte:window onkeydown={(e) => data && e.key === 'Escape' && close()} />

{#if data}
	<div class="ejm-backdrop" onclick={close} role="presentation">
		<div class="ejm" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
			<header class="ejm-head">
				<span class="ejm-title">{data.title}</span>
				<span class="ejm-count">{data.events.length} event{data.events.length === 1 ? '' : 's'}</span>
				<span class="ejm-sp"></span>
				<button class="ejm-btn" onclick={toggleAll} disabled={data.events.length === 0}>
					{allOpen ? 'Collapse all' : 'Expand all'}
				</button>
				<button class="ejm-btn" onclick={copyAll} disabled={data.events.length === 0}>
					{copiedKey === 'all' ? 'Copied ✓' : 'Copy all'}
				</button>
				<button class="ejm-btn ejm-close" onclick={close}>Close</button>
			</header>

			<div class="ejm-body">
				{#if data.events.length === 0}
					<p class="ejm-empty">No events to inspect.</p>
				{:else}
					{#each data.events as ev, i (i)}
						{@const open = expanded.has(i)}
						<div class="ejm-event" class:ejm-event--open={open}>
							<button class="ejm-event-row" onclick={() => toggle(i)} aria-expanded={open}>
								<span class="ejm-chevron">{open ? '▾' : '▸'}</span>
								<span class="ejm-kind">{kindLabel(ev.kind)}</span>
								<span class="ejm-label">{ev.label}</span>
								{#if ev.banner}
									<span class="ejm-banner ejm-banner--{ev.banner.status}" title={ev.banner.addr}>
										{ev.banner.status === 'forked' ? '⑂' : '⮑'} {ev.banner.text}
									</span>
								{/if}
							</button>
							{#if open}
								<div class="ejm-json-wrap">
									{#if ev.json != null}
										<button
											class="ejm-copy"
											onclick={() => copy(pretty(ev.json), `e${i}`)}
											title="Copy this event's JSON"
										>{copiedKey === `e${i}` ? 'Copied ✓' : 'Copy'}</button>
										<pre class="ejm-json">{pretty(ev.json)}</pre>
									{:else}
										<p class="ejm-missing">
											Original event isn't cached locally — the 30040 references it as
											<code>{ev.banner?.addr ?? 'unknown address'}</code>.
										</p>
									{/if}
								</div>
							{/if}
						</div>
					{/each}
				{/if}
			</div>
		</div>
	</div>
{/if}

<style>
	.ejm-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.55);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 1000;
		padding: 5vh 4vw;
	}
	.ejm {
		background: var(--panel-bg, var(--bg));
		border: 1px solid var(--panel-border, var(--border));
		border-radius: var(--r-md);
		width: min(900px, 100%);
		max-height: 90vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.ejm-head {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
		flex-shrink: 0;
	}
	.ejm-title {
		font-weight: 600;
		font-size: var(--t-sm);
	}
	.ejm-count {
		font-size: var(--t-xs);
		color: var(--base5);
		font-family: var(--font-mono);
	}
	.ejm-sp {
		flex: 1;
	}
	.ejm-btn {
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 4px 10px;
	}
	.ejm-close {
		font-weight: 600;
	}
	.ejm-body {
		overflow-y: auto;
		padding: 6px 0;
		min-height: 0;
	}
	.ejm-empty {
		padding: 24px;
		text-align: center;
		color: var(--base5);
	}
	.ejm-event {
		border-bottom: 1px solid var(--panel-border);
	}
	.ejm-event-row {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		background: transparent;
		border: none;
		text-align: left;
		padding: 8px 14px;
		cursor: pointer;
		color: var(--fg);
	}
	.ejm-event-row:hover {
		background: var(--bg-surface);
	}
	.ejm-chevron {
		color: var(--fg-muted);
		min-width: 12px;
		font-size: var(--t-2xs);
	}
	.ejm-kind {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--id-remote);
		background: rgba(137, 184, 194, 0.12);
		padding: 1px 6px;
		border-radius: var(--r-sm);
		white-space: nowrap;
	}
	.ejm-label {
		font-size: var(--t-sm);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.ejm-banner {
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		padding: 1px 6px;
		border-radius: var(--r-sm);
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		margin-left: auto;
		flex-shrink: 1;
		min-width: 0;
	}
	.ejm-banner--forked {
		color: var(--yellow);
		background: color-mix(in srgb, var(--yellow) 12%, transparent);
	}
	.ejm-banner--linked {
		color: var(--blue);
		background: color-mix(in srgb, var(--blue) 12%, transparent);
	}
	.ejm-missing {
		margin: 0;
		font-size: var(--t-xs);
		color: var(--base5);
		background: var(--bg-surface);
		border: 1px dashed var(--panel-border);
		border-radius: var(--r-sm);
		padding: 10px 12px;
	}
	.ejm-missing code {
		font-family: var(--font-mono);
		word-break: break-all;
	}
	.ejm-json-wrap {
		position: relative;
		padding: 0 14px 12px 34px;
	}
	.ejm-copy {
		position: absolute;
		top: 4px;
		right: 18px;
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 2px 8px;
		z-index: 1;
	}
	.ejm-json {
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		line-height: 1.5;
		white-space: pre-wrap;
		word-break: break-word;
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 10px 12px;
		max-height: 50vh;
		overflow: auto;
	}
</style>
