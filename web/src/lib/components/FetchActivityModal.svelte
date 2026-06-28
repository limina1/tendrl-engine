<script lang="ts">
	import { getAppState } from '$lib/state.svelte';
	import type { Phase, RequestSummary } from '$lib/types';

	const app = getAppState();

	// The toast whose activity we're showing. Reactively null when the
	// modal closes (state.svelte.ts clears activityModalToastId).
	const toast = $derived(app.activityModalToast);
	const activity = $derived(toast?.activity ?? null);
	const summary: RequestSummary | undefined = $derived(activity?.summary);

	function close() {
		app.closeActivityModal();
	}

	function copy(text: string) {
		navigator.clipboard?.writeText(text).then(
			() => app.pushToast(`Copied`, 'success', 1500),
			() => app.pushToast(`Couldn't copy`, 'error', 2500)
		);
	}

	function relayLabel(phase: Phase): string {
		switch (phase) {
			case 'read':
				return 'Read';
			case 'write':
				return 'Write';
			case 'publish':
				return 'Publish';
			case 'broadcast':
				return 'Broadcast';
			case 'search.default':
				return 'Search · default';
			case 'search.fallback':
				return 'Search · fallback';
			case 'indexer.default':
				return 'Indexer · default';
			case 'indexer.fallback':
				return 'Indexer · fallback';
		}
	}

	function relayStatusLabel(s: NonNullable<typeof activity>['relays'][string]): {
		text: string;
		kind: 'pending' | 'success' | 'error' | 'info';
	} {
		switch (s.kind) {
			case 'connecting':
				return { text: 'connecting…', kind: 'pending' };
			case 'eose':
				return { text: `eose · ${s.event_count} event${s.event_count === 1 ? '' : 's'}`, kind: 'success' };
			case 'error':
				return { text: `error: ${s.msg || '(no detail)'}`, kind: 'error' };
			case 'timeout':
				return { text: 'timeout', kind: 'error' };
			case 'accepted':
				return { text: 'accepted', kind: 'success' };
			case 'rejected':
				return { text: `rejected: ${s.msg || '(no reason)'}`, kind: 'error' };
		}
	}
</script>

{#if toast && activity}
	<!-- svelte-ignore a11y_click_events_have_key_events -->
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="fam-backdrop" onclick={close} role="presentation">
		<div class="fam-modal" onclick={(e) => e.stopPropagation()} role="dialog" tabindex="-1">
			<div class="fam-header">
				<span class="fam-title">
					{activity.mode === 'publish' ? 'Publish activity' : 'Fetch activity'}
				</span>
				<button class="fam-close" onclick={close} aria-label="Close">×</button>
			</div>

			<div class="fam-body">
				<!-- DSL sentence -->
				{#if summary?.dsl}
					<section class="fam-section">
						<div class="fam-section-head">
							<span class="fam-label">Query</span>
							<button class="fam-copy" onclick={() => copy(summary.dsl)} title="Copy full query">
								copy
							</button>
						</div>
						<code class="fam-dsl">{summary.dsl}</code>
					</section>
				{/if}

				<!-- Filters block -->
				{#if summary?.filters?.length}
					<section class="fam-section">
						<div class="fam-section-head">
							<span class="fam-label">Filters</span>
						</div>
						{#each summary.filters as f, i}
							<div class="fam-filter">
								<span class="fam-filter-idx">#{i + 1}</span>
								<div class="fam-filter-clauses">
									{#if f.kinds?.length}
										<span class="fam-clause">k:{f.kinds.join(',')}</span>
									{/if}
									{#if f.authors?.length}
										<span class="fam-clause" title={f.authors.join(', ')}>
											by:{f.authors.length === 1
												? `${f.authors[0].slice(0, 12)}…`
												: `${f.authors.length} authors`}
										</span>
									{/if}
									{#if f.ids?.length}
										<span class="fam-clause">ids:{f.ids.length}</span>
									{/if}
									{#if f.since != null}
										<span class="fam-clause">since:{f.since}</span>
									{/if}
									{#if f.until != null}
										<span class="fam-clause">until:{f.until}</span>
									{/if}
									{#if f.limit != null}
										<span class="fam-clause">limit:{f.limit}</span>
									{/if}
									{#if f.search}
										<span class="fam-clause">~:"{f.search}"</span>
									{/if}
									{#if f.tags}
										{#each Object.entries(f.tags) as [tag, vals]}
											<span class="fam-clause">{tag}:{vals.join(',')}</span>
										{/each}
									{/if}
								</div>
							</div>
						{/each}
					</section>
				{/if}

				<!-- Composition -->
				{#if summary?.composition?.phases?.length}
					<section class="fam-section">
						<div class="fam-section-head">
							<span class="fam-label">Composition</span>
						</div>
						{#each summary.composition.phases as stage, i}
							<div class="fam-stage">
								<div class="fam-stage-head">
									<span class="fam-stage-num">{i + 1}.</span>
									<span class="fam-stage-label">{stage.label}</span>
									{#if stage.start_delay_ms > 0}
										<span class="fam-stage-delay">Δ{stage.start_delay_ms}ms</span>
									{/if}
								</div>
								{#each stage.members as [phase, relays]}
									<div class="fam-phase">
										<div class="fam-phase-label">{relayLabel(phase)}</div>
										<div class="fam-relay-list">
											{#each relays as url}
												{@const sr = activity.relays[url]}
												{@const lbl = sr
													? relayStatusLabel(sr)
													: { text: 'pending', kind: 'info' as const }}
												<div class="fam-relay">
													<span class="fam-relay-dot fam-relay-dot--{lbl.kind}"></span>
													<button
														class="fam-relay-url"
														onclick={() => copy(url)}
														title="Copy URL"
													>{url}</button>
													<span class="fam-relay-status">{lbl.text}</span>
												</div>
											{/each}
										</div>
									</div>
								{/each}
							</div>
						{/each}
					</section>
				{/if}

				<!-- Operation ID footer (debugging aid) -->
				<section class="fam-footer">
					<span class="fam-label">op</span>
					<code class="fam-op-id">{activity.operation_id}</code>
				</section>
			</div>
		</div>
	</div>
{/if}

<style>
	.fam-backdrop {
		position: fixed;
		inset: 0 0 var(--modeline-h, 0) 0;
		z-index: 100;
		background: var(--scrim);
		display: flex;
		align-items: center;
		justify-content: center;
	}
	.fam-modal {
		background: var(--bg);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-md);
		width: 90vw;
		max-width: 640px;
		max-height: 80vh;
		display: flex;
		flex-direction: column;
		font-family: var(--font-sans);
		color: var(--fg);
	}
	.fam-header {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 10px 14px;
		border-bottom: 1px solid var(--panel-border);
		font-weight: 600;
		font-size: var(--t-xs);
	}
	.fam-close {
		appearance: none;
		background: none;
		border: none;
		color: var(--muted);
		font-size: var(--t-md);
		cursor: pointer;
		padding: 2px 6px;
		border-radius: 3px;
	}
	.fam-close:hover {
		background: color-mix(in srgb, var(--fg) 10%, transparent);
		color: var(--fg);
	}
	.fam-body {
		flex: 1;
		overflow: auto;
		padding: 14px;
		display: flex;
		flex-direction: column;
		gap: 16px;
		font-size: var(--t-xs);
	}
	.fam-section {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.fam-section-head {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
	}
	.fam-label {
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.04em;
		color: var(--muted);
	}
	.fam-copy {
		appearance: none;
		background: none;
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		color: var(--muted);
		font-size: var(--t-3xs);
		padding: 2px 6px;
		cursor: pointer;
	}
	.fam-copy:hover {
		color: var(--fg);
		background: color-mix(in srgb, var(--fg) 8%, transparent);
	}
	.fam-dsl {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		background: var(--panel-bg);
		border: 1px solid var(--panel-border);
		border-radius: 3px;
		padding: 6px 10px;
		display: block;
		overflow-x: auto;
	}
	.fam-filter {
		display: flex;
		gap: 8px;
		align-items: baseline;
		padding: 4px 0;
	}
	.fam-filter-idx {
		color: var(--muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		min-width: 18px;
	}
	.fam-filter-clauses {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 8px;
	}
	.fam-clause {
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
	}
	.fam-stage {
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 6px 0;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 50%, transparent);
	}
	.fam-stage:first-child {
		border-top: none;
		padding-top: 0;
	}
	.fam-stage-head {
		display: flex;
		gap: 6px;
		align-items: baseline;
	}
	.fam-stage-num {
		color: var(--muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
	}
	.fam-stage-label {
		font-weight: 500;
	}
	.fam-stage-delay {
		color: var(--muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
	}
	.fam-phase {
		padding-left: 22px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.fam-phase-label {
		font-size: var(--t-3xs);
		color: var(--muted);
		margin-top: 4px;
	}
	.fam-relay-list {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.fam-relay {
		display: grid;
		grid-template-columns: 10px 1fr auto;
		gap: 8px;
		align-items: center;
		padding: 2px 0;
	}
	.fam-relay-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--muted);
	}
	.fam-relay-dot--pending {
		background: var(--id-forked);
		animation: relay-pulse 1.1s ease-in-out infinite;
	}
	.fam-relay-dot--success {
		background: var(--state-online, var(--green));
	}
	.fam-relay-dot--error {
		background: var(--state-error, var(--red));
	}
	.fam-relay-dot--info {
		background: var(--muted);
	}
	.fam-relay-url {
		appearance: none;
		background: none;
		border: none;
		color: var(--fg);
		font-family: var(--font-mono);
		font-size: var(--t-2xs);
		text-align: left;
		padding: 0;
		cursor: pointer;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.fam-relay-url:hover {
		text-decoration: underline;
	}
	.fam-relay-status {
		color: var(--muted);
		font-size: var(--t-3xs);
		font-family: var(--font-mono);
	}
	.fam-footer {
		margin-top: auto;
		display: flex;
		gap: 8px;
		align-items: baseline;
		padding-top: 8px;
		border-top: 1px solid color-mix(in srgb, var(--panel-border) 50%, transparent);
	}
	.fam-op-id {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--muted);
	}
	@keyframes relay-pulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.45;
		}
	}
</style>
