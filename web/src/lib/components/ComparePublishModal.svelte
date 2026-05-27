<script lang="ts">
	import { getAppState } from '$lib/state.svelte';

	const app = getAppState();
	const prompt = $derived(app.republishPrompt);
	const diff = $derived(prompt?.diff ?? null);

	const changedCount = $derived(diff ? diff.matched.filter((m) => m.contentChanged).length : 0);

	function replace() {
		app.confirmRepublish(true);
	}
	function asNew() {
		app.confirmRepublish(false);
	}
	function cancel() {
		app.cancelRepublish();
	}
</script>

<svelte:window onkeydown={(e) => prompt && e.key === 'Escape' && cancel()} />

{#if diff}
	<div class="cpm-backdrop" onclick={cancel} role="presentation">
		<div class="cpm" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" tabindex="-1">
			<header class="cpm-head">
				<span class="cpm-title">Republish “{diff.existingTitle || 'untitled'}”?</span>
				<button class="cpm-x" onclick={cancel}>Close</button>
			</header>

			<p class="cpm-lede">
				A publication of yours with this title already exists. Matching sections (by
				title) can <strong>reuse the existing identifiers</strong> so this <em>replaces</em>
				it instead of creating a new copy.
			</p>

			<div class="cpm-summary">
				<span class="chip chip--same">{diff.matched.length} matched</span>
				{#if changedCount > 0}<span class="chip chip--warn">{changedCount} changed</span>{/if}
				<span class="chip chip--warn">{diff.added.length} added</span>
				<span class="chip chip--warn">{diff.removed.length} removed</span>
			</div>

			<div class="cpm-body">
				{#if diff.matched.length}
					<div class="cpm-group-label">Matched — same identifier reused (replace)</div>
					{#each diff.matched as m (m.t)}
						<div class="cpm-row" class:cpm-row--warn={m.contentChanged} class:cpm-row--same={!m.contentChanged}>
							<span class="cpm-dot"></span>
							<span class="cpm-row-title">{m.title || '[Untitled]'}</span>
							<span class="cpm-row-note">{m.contentChanged ? 'content changed' : 'unchanged'}</span>
						</div>
					{/each}
				{/if}

				{#if diff.added.length}
					<div class="cpm-group-label">Added — only in this draft (fresh identifier)</div>
					{#each diff.added as a (a.t)}
						<div class="cpm-row cpm-row--warn">
							<span class="cpm-dot"></span>
							<span class="cpm-row-title">{a.title || '[Untitled]'}</span>
							<span class="cpm-row-note">new</span>
						</div>
					{/each}
				{/if}

				{#if diff.removed.length}
					<div class="cpm-group-label">Removed — in the published version, not in this draft</div>
					{#each diff.removed as r (r.t)}
						<div class="cpm-row cpm-row--warn">
							<span class="cpm-dot"></span>
							<span class="cpm-row-title">{r.title || '[Untitled]'}</span>
							<span class="cpm-row-note">dropped from index</span>
						</div>
					{/each}
				{/if}
			</div>

			<p class="cpm-deferred">
				Coming soon (see <code>docs/republish-diff.md</code>): per-tag diff and a
				merge step for changed sections. For now, changed sections still
				<strong>replace</strong> by reusing their identifier — your new content wins.
			</p>

			<footer class="cpm-foot">
				<button class="cpm-btn cpm-btn--primary" onclick={replace}>Replace (reuse identifiers)</button>
				<button class="cpm-btn" onclick={asNew}>Publish as new</button>
				<button class="cpm-btn" onclick={cancel}>Cancel</button>
			</footer>
		</div>
	</div>
{/if}

<style>
	.cpm-backdrop {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.55);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 1000;
		padding: 5vh 4vw;
	}
	.cpm {
		background: var(--panel-bg, var(--bg));
		border: 1px solid var(--panel-border, var(--border));
		border-radius: var(--r-md);
		width: min(680px, 100%);
		max-height: 90vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}
	.cpm-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		padding: 12px 16px;
		border-bottom: 1px solid var(--panel-border);
	}
	.cpm-title {
		font-weight: 600;
		font-size: var(--t-md);
	}
	.cpm-x {
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 4px 10px;
	}
	.cpm-lede {
		margin: 0;
		padding: 12px 16px 0;
		font-size: var(--t-sm);
		color: var(--base7);
		line-height: var(--lh-snug);
	}
	.cpm-summary {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		padding: 10px 16px;
	}
	.chip {
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		padding: 2px 8px;
		border-radius: var(--r-sm);
	}
	.chip--same {
		color: var(--state-online);
		background: color-mix(in srgb, var(--state-online) 16%, transparent);
	}
	.chip--warn {
		color: var(--id-draft);
		background: color-mix(in srgb, var(--id-draft) 16%, transparent);
	}
	.cpm-body {
		overflow-y: auto;
		min-height: 0;
		padding: 0 16px;
	}
	.cpm-group-label {
		font-size: var(--t-xs);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--base5);
		margin: 12px 0 4px;
	}
	.cpm-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 0;
		font-size: var(--t-sm);
	}
	.cpm-dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		flex-shrink: 0;
	}
	.cpm-row--same .cpm-dot {
		background: var(--state-online);
	}
	.cpm-row--warn .cpm-dot {
		background: var(--id-draft);
	}
	.cpm-row-title {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.cpm-row-note {
		font-size: var(--t-xs);
		font-family: var(--font-mono);
		color: var(--base5);
	}
	.cpm-deferred {
		margin: 12px 16px 0;
		font-size: var(--t-xs);
		color: var(--base6);
		line-height: var(--lh-snug);
		border-top: 1px dashed var(--panel-border);
		padding-top: 10px;
	}
	.cpm-deferred code {
		font-family: var(--font-mono);
	}
	.cpm-foot {
		display: flex;
		gap: 8px;
		padding: 14px 16px;
		border-top: 1px solid var(--panel-border);
		margin-top: 8px;
	}
	.cpm-btn {
		font-size: var(--t-sm);
		font-family: var(--font-mono);
		padding: 6px 12px;
	}
	.cpm-btn--primary {
		background: rgba(180, 190, 130, 0.16);
		color: var(--state-online);
		border-color: color-mix(in srgb, var(--state-online) 50%, transparent);
		font-weight: 600;
	}
</style>
