// Global in-app text prompt — the wm-shell replacement for
// window.prompt(). Browser dialogs don't belong in the shell; callers
// await promptText(...) and the single <TextPromptModal /> instance in
// +layout renders whatever is active (same declarative pattern as
// ToastStack — no manual mount()).
//
// Leaf module by design: no imports from wm/registry or components, so
// any buffer/component can use it without creating the build-only TDZ
// import cycles that have bitten wm modules before.

export interface TextPromptOptions {
	title: string;
	placeholder?: string;
	/** One muted line of guidance under the title. */
	hint?: string;
	/** Confirm-button label; defaults to "Add". */
	confirmLabel?: string;
	/** Pre-filled input value (e.g. rename flows). */
	initial?: string;
}

interface ActivePrompt {
	title: string;
	placeholder: string;
	hint: string | null;
	confirmLabel: string;
	value: string;
	resolve: (v: string | null) => void;
}

export const textPrompt = $state<{ active: ActivePrompt | null }>({ active: null });

/** Open the prompt and resolve with the trimmed input, or null on
 *  cancel/empty. One prompt at a time: opening over a live one cancels
 *  the earlier caller (resolves it null). */
export function promptText(opts: TextPromptOptions): Promise<string | null> {
	textPrompt.active?.resolve(null);
	return new Promise((resolve) => {
		textPrompt.active = {
			title: opts.title,
			placeholder: opts.placeholder ?? '',
			hint: opts.hint ?? null,
			confirmLabel: opts.confirmLabel ?? 'Add',
			value: opts.initial ?? '',
			resolve
		};
	});
}

export function resolveTextPrompt(ok: boolean) {
	const active = textPrompt.active;
	if (!active) return;
	const v = active.value.trim();
	active.resolve(ok && v ? v : null);
	textPrompt.active = null;
}
