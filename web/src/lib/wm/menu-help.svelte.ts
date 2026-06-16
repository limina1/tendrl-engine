// Visibility flag for the event-menu help modal — mounted once in
// +layout.svelte, opened from the event menu's `?` affordance (sits next to
// its W walkthrough chip). Mirrors the search / mode-line / composer help
// stores so every "open a reference" affordance behaves identically.

export const menuHelpUI = $state<{ open: boolean }>({ open: false });

export function openMenuHelp() {
	menuHelpUI.open = true;
}
export function closeMenuHelp() {
	menuHelpUI.open = false;
}
