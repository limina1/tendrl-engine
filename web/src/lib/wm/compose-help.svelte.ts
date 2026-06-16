// Visibility flag for the composer help modal — mounted once in
// +layout.svelte, opened from the composer mode-bar's `?` affordance (sits
// next to its W walkthrough chip). Mirrors the search panel's searchHelpUI and
// the mode-line's modelineHelpUI so every "open a reference" affordance behaves
// identically.

export const composeHelpUI = $state<{ open: boolean }>({ open: false });

export function openComposeHelp() {
	composeHelpUI.open = true;
}
export function closeComposeHelp() {
	composeHelpUI.open = false;
}
