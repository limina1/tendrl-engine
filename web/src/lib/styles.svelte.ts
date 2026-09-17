// Writing styles (kind-30078 events tagged `tendrl-style`): a module-level
// cache of coordinate → title so cards can label a `style` tag without each
// fetching the list. Refreshed on demand.
import * as api from '$lib/api';

let styles = $state<api.WritingStyle[]>([]);
let loaded = false;
let inflight: Promise<void> | null = null;

export function stylesList(): api.WritingStyle[] {
	return styles;
}

export function refreshStyles(): Promise<void> {
	if (inflight) return inflight;
	inflight = api
		.listStyles()
		.then((r) => {
			styles = r.styles;
			loaded = true;
		})
		.catch(() => {})
		.finally(() => {
			inflight = null;
		});
	return inflight;
}

/** Title for a style coordinate; kicks a load the first time it is asked. */
export function styleTitle(coord: string): string | null {
	if (!loaded && !inflight) void refreshStyles();
	return styles.find((s) => s.coordinate === coord)?.title ?? null;
}
