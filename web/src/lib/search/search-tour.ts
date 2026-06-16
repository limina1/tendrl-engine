// Bridge between the static search-exercise tour tips (defined in discovery's
// TIPS registry, which is pure data) and the live search input. The search
// buffer registers a runner on mount — bound to `app.searchFor`, which echoes
// the query into the input box AND executes it — and each exercise tip's
// "Try it" action calls `runSearchExample`. This thin indirection is the seam:
// the registry has no access to the component-scoped app state, so it can't
// call searchFor directly.

let runner: ((query: string) => void) | null = null;

/** The search buffer registers `app.searchFor` here on mount. */
export function registerSearchRunner(fn: (query: string) => void) {
	runner = fn;
}

/** Drop the runner on unmount so a stale closure can't fire into a dead view. */
export function clearSearchRunner(fn: (query: string) => void) {
	if (runner === fn) runner = null;
}

/** Run an exercise's example query through the live search box. No-op if no
 *  search buffer is mounted (the tour can only be reached from one anyway). */
export function runSearchExample(query: string) {
	runner?.(query);
}
