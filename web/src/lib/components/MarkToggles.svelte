<script lang="ts">
	// The marks a highlighter draws — fill / underline / bold / italic,
	// combinable, at least one. Each toggle previews itself in the ink.
	import { HIGHLIGHT_MARKS, type HighlightMark } from '$lib/discussions/highlights';
	import { markSampleStyle } from '$lib/discussions/colors';

	let {
		value = $bindable<HighlightMark[]>(['fill']),
		color,
		disabled = false,
		onchange = undefined
	}: {
		value?: HighlightMark[];
		color: string;
		disabled?: boolean;
		onchange?: (marks: HighlightMark[]) => void;
	} = $props();

	function toggle(m: HighlightMark) {
		const has = value.includes(m);
		if (has && value.length === 1) return; // keep at least one mark
		const next = HIGHLIGHT_MARKS.filter((k) => (k === m ? !has : value.includes(k)));
		value = next;
		onchange?.(next);
	}
</script>

<div class="mt" role="group" aria-label="Highlight marks">
	{#each HIGHLIGHT_MARKS as m (m)}
		<button
			type="button"
			class="mt-btn"
			class:on={value.includes(m)}
			aria-pressed={value.includes(m)}
			{disabled}
			onclick={() => toggle(m)}
			title={m}
		>
			<span style={markSampleStyle({ color, marks: [m] })}>{m}</span>
		</button>
	{/each}
</div>

<style>
	.mt {
		display: flex;
		gap: 3px;
		flex-wrap: wrap;
	}
	.mt-btn {
		font: inherit;
		font-size: calc(var(--t-xs) - 1px);
		padding: 2px 6px;
		color: var(--fg);
		background: transparent;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		cursor: pointer;
		opacity: 0.55;
	}
	.mt-btn.on {
		opacity: 1;
		border-color: var(--base5);
		background: var(--base1, var(--bg-surface));
	}
	.mt-btn span {
		padding: 0 2px;
		border-radius: 2px;
	}
</style>
