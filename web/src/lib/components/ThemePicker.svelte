<script lang="ts">
	// Custom theme picker with live hover/keyboard preview. A native <select>
	// can't do this — its option list is OS-rendered and exposes no hover
	// events — so this is a small button + popup list. Previewing just toggles
	// <html data-theme> (the same mechanism setTheme uses), so the whole UI
	// re-skins live; we revert to the committed theme on leave/close/unmount and
	// only persist on click. Theme ids come from the fixed THEMES registry, so
	// the value handed to setAttribute is always from a trusted allowlist.
	import { THEMES, themeFamilies, applyThemeAttribute } from '$lib/themes';

	let {
		current,
		oncommit,
		livePreview = false
	}: { current: string; oncommit: (id: string) => void; livePreview?: boolean } = $props();

	let open = $state(false);
	// Set to `current` each time the menu opens (see openMenu); not seeded from
	// the prop here, which would only capture its initial value.
	let highlighted = $state('');

	const currentLabel = $derived.by(() => {
		const t = THEMES.find((x) => x.id === current);
		return t ? `${t.familyLabel} ${t.mode === 'dark' ? 'Dark' : 'Light'}` : current;
	});
	// Flat id order for arrow-key navigation (registry order).
	const order = $derived(THEMES.map((t) => t.id));

	function preview(id: string) {
		highlighted = id;
		if (livePreview) applyThemeAttribute(id); // live preview — does not persist
	}
	function revert() {
		if (livePreview) applyThemeAttribute(current); // back to the committed theme
	}
	function openMenu() {
		highlighted = current;
		open = true;
	}
	function closeMenu() {
		open = false;
		revert();
	}
	function commit(id: string) {
		oncommit(id); // persists + updates `current`; preview already matches
		open = false;
	}

	function onKeydown(e: KeyboardEvent) {
		if (!open) {
			if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowDown') {
				e.preventDefault();
				openMenu();
			}
			return;
		}
		const i = order.indexOf(highlighted);
		switch (e.key) {
			case 'ArrowDown':
				e.preventDefault();
				preview(order[Math.min(order.length - 1, i + 1)]);
				break;
			case 'ArrowUp':
				e.preventDefault();
				preview(order[Math.max(0, i - 1)]);
				break;
			case 'Home':
				e.preventDefault();
				preview(order[0]);
				break;
			case 'End':
				e.preventDefault();
				preview(order[order.length - 1]);
				break;
			case 'Enter':
			case ' ':
				e.preventDefault();
				commit(highlighted);
				break;
			case 'Escape':
				e.preventDefault();
				closeMenu();
				break;
		}
	}

	// If the component is torn down mid-preview (e.g. the Settings buffer closes
	// while a menu is open), don't leave a dangling preview applied.
	$effect(() => {
		return () => {
			if (open && livePreview) applyThemeAttribute(current);
		};
	});
</script>

<div class="theme-picker">
	<button
		type="button"
		class="theme-trigger"
		aria-haspopup="listbox"
		aria-expanded={open}
		onclick={() => (open ? closeMenu() : openMenu())}
		onkeydown={onKeydown}
	>
		<span class="theme-trigger__label">{currentLabel}</span>
		<span class="theme-caret" aria-hidden="true">▾</span>
	</button>

	{#if open}
		<!-- Transparent catcher so an outside click closes + reverts. -->
		<div
			class="theme-backdrop"
			role="presentation"
			onclick={closeMenu}
			oncontextmenu={closeMenu}
		></div>
		<div class="theme-menu" role="listbox" tabindex="-1" aria-label="Theme" onpointerleave={revert}>
			{#each themeFamilies() as fam (fam.family)}
				<div class="theme-group">{fam.label}</div>
				{#each fam.variants as v (v.id)}
					<button
						type="button"
						class="theme-opt"
						class:is-highlighted={highlighted === v.id}
						class:is-current={current === v.id}
						role="option"
						aria-selected={current === v.id}
						onpointerenter={() => preview(v.id)}
						onfocus={() => preview(v.id)}
						onclick={() => commit(v.id)}
					>
						{v.mode === 'dark' ? 'Dark' : 'Light'}
						{#if current === v.id}<span class="theme-tick" aria-hidden="true">●</span>{/if}
					</button>
				{/each}
			{/each}
		</div>
	{/if}
</div>

<style>
	.theme-picker {
		position: relative;
		display: inline-block;
	}

	.theme-trigger {
		display: inline-flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		min-width: 11rem;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: var(--panel-bg-soft);
		border: 1px solid var(--panel-border);
		border-radius: var(--r-sm);
		padding: 3px 8px;
		cursor: pointer;
	}
	.theme-trigger:hover {
		border-color: var(--panel-border-strong);
	}
	.theme-trigger:focus-visible {
		outline: 1px solid var(--id-yours);
		outline-offset: 1px;
	}
	.theme-caret {
		font-size: var(--t-3xs);
		color: var(--base6);
	}

	.theme-backdrop {
		position: fixed;
		inset: 0;
		z-index: 60;
	}

	.theme-menu {
		position: absolute;
		top: calc(100% + 4px);
		left: 0;
		z-index: 61;
		min-width: 12rem;
		max-height: 50vh;
		overflow-y: auto;
		padding: 4px;
		background: var(--panel-bg);
		border: 1px solid var(--panel-border-strong);
		border-radius: var(--r-md);
		box-shadow: var(--shadow-md);
	}

	.theme-group {
		font-size: var(--t-3xs);
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--base5);
		padding: 6px 8px 2px;
	}

	.theme-opt {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		width: 100%;
		text-align: left;
		font-family: var(--font-mono);
		font-size: var(--t-xs);
		color: var(--fg);
		background: transparent;
		border: none;
		border-radius: var(--r-sm);
		padding: 4px 8px;
		cursor: pointer;
	}
	.theme-opt.is-highlighted {
		background: var(--panel-bg-soft);
	}
	.theme-opt.is-current {
		color: var(--id-yours);
	}
	.theme-tick {
		font-size: var(--t-3xs);
		color: var(--id-yours);
	}
</style>
