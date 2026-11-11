<script lang="ts">
	// Highlighter colour: a dropdown of the standard highlighter colours, plus
	// a swatch button that pops up an HSV picker (saturation/value square +
	// hue bar) for anything else. Value is lower-case `#rrggbb`.
	import { STANDARD_HIGHLIGHT_COLORS } from '$lib/discussions/colors';

	let {
		value = $bindable('#f7d94c'),
		onchange = undefined,
		disabled = false,
		label = 'colour'
	}: {
		value?: string;
		onchange?: (hex: string) => void;
		disabled?: boolean;
		label?: string;
	} = $props();

	const uid = Math.random().toString(36).slice(2, 8);
	let open = $state(false);
	let h = $state(0);
	let s = $state(0);
	let v = $state(1);

	const standard = $derived(STANDARD_HIGHLIGHT_COLORS.find((c) => c.value === value));

	function hsvToHex(hh: number, ss: number, vv: number): string {
		const f = (n: number) => {
			const k = (n + hh / 60) % 6;
			return vv - vv * ss * Math.max(0, Math.min(k, 4 - k, 1));
		};
		const to = (x: number) => Math.round(x * 255).toString(16).padStart(2, '0');
		return `#${to(f(5))}${to(f(3))}${to(f(1))}`;
	}

	function hexToHsv(hex: string): { h: number; s: number; v: number } {
		const n = (i: number) => (parseInt(hex.slice(i, i + 2), 16) || 0) / 255;
		const [r, g, b] = [n(1), n(3), n(5)];
		const mx = Math.max(r, g, b);
		const d = mx - Math.min(r, g, b);
		let hh = 0;
		if (d) {
			if (mx === r) hh = ((g - b) / d) % 6;
			else if (mx === g) hh = (b - r) / d + 2;
			else hh = (r - g) / d + 4;
			hh *= 60;
			if (hh < 0) hh += 360;
		}
		return { h: hh, s: mx ? d / mx : 0, v: mx };
	}

	function set(hex: string) {
		value = hex;
		onchange?.(hex);
	}

	function toggle() {
		if (!open) ({ h, s, v } = hexToHsv(value));
		open = !open;
	}

	const clamp = (x: number) => Math.max(0, Math.min(1, x));

	function svAt(e: PointerEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		s = clamp((e.clientX - r.left) / r.width);
		v = 1 - clamp((e.clientY - r.top) / r.height);
		set(hsvToHex(h, s, v));
	}
	function hueAt(e: PointerEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		h = clamp((e.clientX - r.left) / r.width) * 359.9;
		set(hsvToHex(h, s, v));
	}
	function drag(fn: (e: PointerEvent) => void) {
		return {
			down: (e: PointerEvent) => {
				(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
				fn(e);
			},
			move: (e: PointerEvent) => {
				if ((e.currentTarget as HTMLElement).hasPointerCapture(e.pointerId)) fn(e);
			}
		};
	}
	const sv = drag(svAt);
	const hue = drag(hueAt);

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && open) {
			e.stopPropagation();
			open = false;
		}
	}
</script>

<!-- Escape closes the HSV panel without closing the capture card. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="cp" onkeydown={onKey} role="group" aria-label="Highlight colour">
	<div class="cp-top">
	<label class="cp-lab" for="cp-std-{uid}">{label}</label>
	<select
		id="cp-std-{uid}"
		class="cp-select"
		value={standard ? value : 'custom'}
		{disabled}
		onchange={(e) => {
			const v = (e.currentTarget as HTMLSelectElement).value;
			if (v !== 'custom') set(v);
		}}
	>
		{#each STANDARD_HIGHLIGHT_COLORS as c (c.value)}
			<option value={c.value}>{c.name}</option>
		{/each}
		{#if !standard}<option value="custom">Custom {value}</option>{/if}
	</select>
	<button
		type="button"
		class="cp-swatch"
		{disabled}
		onclick={toggle}
		aria-label="Custom colour"
		aria-expanded={open}
		title="Custom colour"
	>
		<span style="background: {value}"></span>
	</button>
	</div>
	{#if open}
		<div class="cp-pop" role="dialog" aria-label="Custom colour">
			<div
				class="cp-sv"
				style="background: linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, {hsvToHex(h, 1, 1)})"
				onpointerdown={sv.down}
				onpointermove={sv.move}
				role="slider"
				aria-label="Saturation and brightness"
				aria-valuetext="saturation {Math.round(s * 100)}%, brightness {Math.round(v * 100)}%"
				aria-valuenow={Math.round(s * 100)}
				tabindex="-1"
			>
				<span class="cp-knob" style="left: {s * 100}%; top: {(1 - v) * 100}%"></span>
			</div>
			<div
				class="cp-hue"
				onpointerdown={hue.down}
				onpointermove={hue.move}
				role="slider"
				aria-label="Hue"
				aria-valuemin={0}
				aria-valuemax={360}
				aria-valuenow={Math.round(h)}
				tabindex="-1"
			>
				<span class="cp-hue-knob" style="left: {(h / 360) * 100}%"></span>
			</div>
			<div class="cp-row">
				<span class="cp-chip" style="background: {value}"></span>
				<span class="cp-hex">{value}</span>
				<button type="button" class="cp-done" onclick={() => (open = false)}>Done</button>
			</div>
		</div>
	{/if}
</div>

<style>
	/* The HSV panel opens in flow under the controls, not as a floating
	   layer — the capture card scrolls, and a floating panel would clip. */
	.cp {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 6px;
	}
	.cp-top {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.cp-lab {
		font-family: var(--font-mono);
		font-size: calc(var(--t-xs) - 2px);
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--base5);
	}
	.cp-select {
		font-family: var(--font-sans);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--bg-surface);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 4px;
	}
	.cp-select:focus {
		outline: none;
		border-color: var(--id-yours);
	}
	.cp-swatch {
		width: 28px;
		height: 24px;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 0;
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		background: var(--bg-surface);
		cursor: pointer;
	}
	.cp-swatch:hover:not(:disabled) {
		border-color: var(--base5);
	}
	.cp-swatch span {
		width: 16px;
		height: 16px;
		border-radius: 2px;
		box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.25);
	}
	.cp-pop {
		width: 220px;
		box-sizing: border-box;
		padding: 10px;
		display: flex;
		flex-direction: column;
		gap: 8px;
		background: var(--bg);
		border: 1px solid var(--panel-border-strong, var(--panel-border));
		border-radius: var(--r-md, 5px);
		box-shadow: var(--shadow-lg, 0 8px 30px rgba(0, 0, 0, 0.4));
	}
	.cp-sv {
		position: relative;
		height: 130px;
		border-radius: var(--r-sm);
		cursor: crosshair;
		touch-action: none;
	}
	.cp-knob {
		position: absolute;
		width: 12px;
		height: 12px;
		margin: -6px 0 0 -6px;
		border-radius: 50%;
		border: 2px solid #fff;
		box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6);
		pointer-events: none;
	}
	.cp-hue {
		position: relative;
		height: 12px;
		border-radius: 6px;
		cursor: ew-resize;
		touch-action: none;
		background: linear-gradient(to right, #f00, #ff0, #0f0, #0ff, #00f, #f0f, #f00);
	}
	.cp-hue-knob {
		position: absolute;
		top: -2px;
		width: 6px;
		height: 16px;
		margin-left: -3px;
		border-radius: 2px;
		background: #fff;
		box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6);
		pointer-events: none;
	}
	.cp-row {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.cp-chip {
		width: 18px;
		height: 18px;
		border-radius: 3px;
	}
	.cp-hex {
		flex: 1;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg-muted);
	}
	.cp-done {
		font: inherit;
		font-size: var(--t-xs);
		padding: 3px 10px;
		color: var(--fg);
		background: var(--base2);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		cursor: pointer;
	}
</style>
