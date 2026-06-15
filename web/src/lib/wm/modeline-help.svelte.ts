// Visibility flag for the mode-line help modal — mounted once in
// +layout.svelte, opened from the mode-line's `?` affordance (sits next to its
// W walkthrough chip). Mirrors the search panel's searchHelpUI pattern so the
// two "open a reference" affordances behave identically.

export const modelineHelpUI = $state<{ open: boolean }>({ open: false });

export function openModelineHelp() {
	modelineHelpUI.open = true;
}
export function closeModelineHelp() {
	modelineHelpUI.open = false;
}
