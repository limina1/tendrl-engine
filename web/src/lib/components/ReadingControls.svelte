<script lang="ts">
	// The reading-typography knobs (face, size, column width, leading, justify).
	// One component, two presentations: `compact` renders the strip the reader's
	// "Aa" row drops down; the full form is the Settings › Reading block. Both
	// drive the same per-device store in theme/reading.svelte.ts.
	import {
		reading,
		READING_FONTS,
		READING_DEFAULTS,
		MEASURE_MAX,
		MEASURE_MIN,
		SIZE_MAX,
		SIZE_MIN,
		LEADING_MAX,
		LEADING_MIN,
		readingSizePx,
		resetReading,
		setReadingCustomFont,
		setReadingFont,
		setReadingJustify,
		stepReadingLeading,
		stepReadingMeasure,
		stepReadingSize,
		type ReadingFontId
	} from '$lib/theme/reading.svelte';
	import { promptText } from '$lib/wm/text-prompt.svelte';

	let { compact = false }: { compact?: boolean } = $props();

	async function pickCustom() {
		const v = await promptText({
			title: 'Reading font',
			placeholder: 'ETBembo, Charter, Georgia, serif',
			hint: 'A CSS font-family list of fonts installed on this device. Nothing is downloaded — an unavailable name falls through to the next one. Empty resets to Serif.',
			confirmLabel: 'Use font',
			initial: reading.custom
		});
		if (v !== null) setReadingCustomFont(v);
	}

	function chooseFont(id: ReadingFontId) {
		if (id === 'custom') void pickCustom();
		else setReadingFont(id);
	}

	const measureLabel = $derived(reading.measure === 0 ? 'full' : `${reading.measure}ch`);
	const isDefault = $derived(
		reading.font === READING_DEFAULTS.font &&
			reading.size === READING_DEFAULTS.size &&
			reading.measure === READING_DEFAULTS.measure &&
			reading.leading === READING_DEFAULTS.leading &&
			reading.justify === READING_DEFAULTS.justify
	);
</script>

<div class="rc" class:rc--compact={compact}>
	<div class="rc-group" title="Reading face. Custom takes any font installed on this device.">
		{#if !compact}<span class="rc-label">Font</span>{/if}
		<div class="rc-faces">
			{#each READING_FONTS as f (f.id)}
				<button
					class="rc-face rc-face--{f.id}"
					class:on={reading.font === f.id}
					onclick={() => chooseFont(f.id)}
					title={f.id === 'custom'
						? reading.custom
							? `Custom: ${reading.custom} — click to change`
							: 'Type a font family installed on this device'
						: `Read in ${f.label.toLowerCase()}`}
				>{f.id === 'custom' && reading.font === 'custom' && reading.custom
						? reading.custom.split(',')[0].replace(/["']/g, '')
						: f.label}{f.id === 'custom' ? '…' : ''}</button>
			{/each}
		</div>
	</div>

	<div class="rc-group" title="Body text size for reading, independent of the app-wide text size.">
		{#if !compact}<span class="rc-label">Size</span>{/if}
		<span class="rc-step">
			{#if compact}<span class="rc-step__tag">Aa</span>{/if}
			<button
				onclick={() => stepReadingSize(-1)}
				disabled={reading.size <= SIZE_MIN}
				aria-label="Smaller reading text">−</button>
			<span class="rc-step__val">{readingSizePx()}px</span>
			<button
				onclick={() => stepReadingSize(1)}
				disabled={reading.size >= SIZE_MAX}
				aria-label="Larger reading text">+</button>
		</span>
	</div>

	<div
		class="rc-group"
		title="Line length. A measured column (~60–75 characters) is the single biggest readability win for unstyled text; one step past the widest setting is full width."
	>
		{#if !compact}<span class="rc-label">Width</span>{/if}
		<span class="rc-step">
			{#if compact}<span class="rc-step__tag">col</span>{/if}
			<button
				onclick={() => stepReadingMeasure(-1)}
				disabled={reading.measure !== 0 && reading.measure <= MEASURE_MIN}
				aria-label="Narrower column">−</button>
			<span class="rc-step__val">{measureLabel}</span>
			<button
				onclick={() => stepReadingMeasure(1)}
				disabled={reading.measure === 0}
				aria-label="Wider column">+</button>
		</span>
	</div>

	<div class="rc-group" title="Line spacing (leading).">
		{#if !compact}<span class="rc-label">Leading</span>{/if}
		<span class="rc-step">
			{#if compact}<span class="rc-step__tag">lh</span>{/if}
			<button
				onclick={() => stepReadingLeading(-1)}
				disabled={reading.leading <= LEADING_MIN}
				aria-label="Tighter lines">−</button>
			<span class="rc-step__val">{reading.leading.toFixed(2)}</span>
			<button
				onclick={() => stepReadingLeading(1)}
				disabled={reading.leading >= LEADING_MAX}
				aria-label="Looser lines">+</button>
		</span>
	</div>

	<div
		class="rc-group"
		title="Justify both edges and hyphenate. Off (ragged right) is easier to read on narrow columns."
	>
		{#if !compact}<span class="rc-label">Justify</span>{/if}
		<button
			class="rc-toggle"
			class:on={reading.justify}
			onclick={() => setReadingJustify(!reading.justify)}
		>{compact ? (reading.justify ? 'justified' : 'ragged') : reading.justify ? 'on' : 'off'}</button>
	</div>

	<button class="rc-reset" onclick={resetReading} disabled={isDefault} title="Back to the shipped reading defaults"
		>reset</button>
</div>

<style>
	.rc {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px 14px;
	}
	.rc--compact {
		gap: 4px 8px;
	}
	.rc-group {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.rc-label {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--fg-muted);
		min-width: 6ch;
	}

	.rc-faces {
		display: flex;
		gap: 2px;
	}
	.rc-face {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 3px);
		color: var(--fg-muted);
		font-size: var(--t-2xs);
		padding: 2px 8px;
		cursor: pointer;
		max-width: 16ch;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	/* Each chip previews the face it selects — the fastest way to choose. */
	.rc-face--serif { font-family: var(--font-serif); }
	.rc-face--sans { font-family: var(--font-sans); }
	.rc-face--mono { font-family: var(--font-mono); }
	.rc-face--custom { font-family: var(--font-sans); font-style: italic; }
	.rc-face:hover {
		border-color: var(--id-yours);
		color: var(--fg);
	}
	.rc-face.on {
		border-color: var(--id-yours);
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}

	.rc-step {
		display: inline-flex;
		align-items: center;
		gap: 2px;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 3px);
		padding: 0 2px;
	}
	.rc-step__tag {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg-muted);
		padding: 0 3px;
	}
	.rc-step button {
		background: none;
		border: none;
		color: var(--id-yours);
		font-size: var(--t-xs);
		line-height: 1;
		padding: 3px 6px;
		cursor: pointer;
	}
	.rc-step button:hover:not(:disabled) {
		background: color-mix(in srgb, var(--id-yours) 14%, transparent);
	}
	.rc-step button:disabled {
		color: var(--fg-muted);
		opacity: 0.45;
		cursor: default;
	}
	.rc-step__val {
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		color: var(--fg);
		min-width: 5ch;
		text-align: center;
	}

	.rc-toggle {
		background: none;
		border: 1px solid var(--border);
		border-radius: var(--r-sm, 3px);
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		padding: 3px 8px;
		cursor: pointer;
	}
	.rc-toggle.on {
		border-color: var(--id-yours);
		color: var(--id-yours);
		background: color-mix(in srgb, var(--id-yours) 12%, transparent);
	}

	.rc-reset {
		background: none;
		border: none;
		color: var(--fg-muted);
		font-family: var(--font-mono);
		font-size: var(--t-3xs);
		text-decoration: underline dotted;
		cursor: pointer;
		padding: 2px 4px;
	}
	.rc-reset:hover:not(:disabled) { color: var(--id-yours); }
	.rc-reset:disabled { opacity: 0.4; cursor: default; text-decoration: none; }
</style>
