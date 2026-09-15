// Unsent text the user has typed into a buffer, keyed by where it was typed.
//
// Buffer renderers unmount on every buffer switch (BufferRenderer's
// `{#key}{#if}` dispatch), so a half-written comment, chat message or issue
// body evaporated the moment the user looked at another buffer. This is the
// save-place for that text: a component reads its draft on mount, writes it
// back in its `$effect` teardown, and clears it once the text is actually
// sent. Session-only, in memory — nothing here is ever persisted or published.
//
// Leaf module on purpose (no imports): buffer components must not pull in the
// wm registry (build-only TDZ cycles), so this sits beside `tours.ts`.
//
// Structured view state (mode, cursor, selection) belongs in
// `store.bufferState`, not here — this is strictly for typed text.

const drafts = new Map<string, string>();

export const entryDrafts = {
	/** The saved text for `key`, or '' when there is none. */
	get(key: string): string {
		return drafts.get(key) ?? '';
	},
	/** Remember `value` for `key`; an empty/whitespace value forgets it. */
	set(key: string, value: string): void {
		if (value.trim()) drafts.set(key, value);
		else drafts.delete(key);
	},
	/** Forget the draft — the text was sent (or deliberately discarded). */
	clear(key: string): void {
		drafts.delete(key);
	}
};
