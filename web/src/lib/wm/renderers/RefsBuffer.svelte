<script lang="ts">
	import type { Buffer } from '../types';

	let { buffer }: { buffer: Buffer } = $props();
</script>

<div class="refs">
	<div class="refs__header">
		<span>References — {buffer.kicker ?? 'no source'}</span>
	</div>
	<div class="empty">
		<p>Citation/reference rendering — to be implemented.</p>
		<p class="hint">A refs buffer would show citations resolved against the focused document, with click-to-jump and click-to-add-to-compose.</p>
	</div>
</div>

<style>
	.refs { display: flex; flex-direction: column; height: 100%; min-height: 0; }
	.refs__header {
		padding: 8px 12px;
		font-size: var(--t-xs);
		font-weight: 600;
		color: var(--base6);
		text-transform: uppercase;
		letter-spacing: 0.05em;
		border-bottom: 1px solid var(--panel-border);
	}
	.empty {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 6px;
		padding: 24px;
		text-align: center;
		color: var(--base5);
		font-size: var(--t-sm);
	}
	.hint { max-width: 36ch; font-size: var(--t-xs); color: var(--base5); }
</style>
