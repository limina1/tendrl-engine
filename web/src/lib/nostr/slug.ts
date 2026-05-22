/** Mirror of the engine's `ComposeState::generate_d_tag` (src/tree/state.rs)
 *  — the `T`-tag / d-tag slug. Lowercase, every non-alphanumeric run → '-',
 *  collapse repeats, trim. Used web-side to match sections by `T` when
 *  detecting a republish. Keep in sync with the Rust version. */
export function slugify(title: string): string {
	return title
		.toLowerCase()
		.replace(/[^\p{L}\p{N}]+/gu, '-')
		.replace(/-+/g, '-')
		.replace(/^-|-$/g, '');
}
