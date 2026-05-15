// Search configuration — the kinds and limit a search runs with when
// the query string itself doesn't pin them.
//
// Why this exists: a bare query like `nostr` has no `k:` filter, so the
// engine matches *every* kind and kind-1 noise leaks in. tendrl is built
// around publications, so searches scope to the four publication kinds
// by default. The defaults are editable (SearchConfigModal, opened by
// the gear button on the search panel) and persist across reloads.
//
// Two consumers inherit the scope:
//   - the in-DB search (`handleSearch`) prepends `applyKindScope`
//   - the offline "Search relays" fallback replays the same effective
//     query, so it carries the same kinds to the relay REQ
//
// The user always wins: a query with an explicit `k:`/`kind:` token is
// left untouched — `applyKindScope` is a no-op for it.

import { browser } from '$app/environment';

const STORAGE_KEY = 'tendrl.searchConfig.v1';

/** Event kinds the UI can name. Order = display order in the form.
 *  `note` cites the spec so the modal can show provenance. */
export const KNOWN_KINDS: { kind: number; label: string; note: string }[] = [
	{ kind: 30040, label: 'Publication index', note: 'NKBIP-01' },
	{ kind: 30041, label: 'Publication section', note: 'NKBIP-01' },
	{ kind: 30023, label: 'Long-form article', note: 'NIP-23' },
	{ kind: 30818, label: 'Wiki article', note: 'NKBIP-02' },
	{ kind: 1, label: 'Short note', note: 'NIP-01' },
	{ kind: 1111, label: 'Comment', note: 'NIP-22' },
	{ kind: 9802, label: 'Highlight', note: 'NIP-84' },
	{ kind: 0, label: 'Profile metadata', note: 'NIP-01' }
];

/** The publication kinds tendrl is built around — the out-of-box scope. */
export const DEFAULT_KINDS = [30040, 30041, 30818, 30023];
const DEFAULT_LIMIT = 100;
const MAX_LIMIT = 1000;

export type SearchConfig = {
	/** Kinds a search scopes to when the query has no explicit k:/kind:. */
	kinds: number[];
	/** Result cap handed to the engine and to relay REQs. */
	limit: number;
	/** Relays the "Search relays" fallback queries. Empty = fall back to
	 *  the engine's configured `[relay.fetch]` set (the modal still shows
	 *  those as the pickable options). */
	relays: string[];
	/** Non-standard kinds the user added in the config modal. Kept
	 *  separate from `kinds` so a custom kind toggled *off* still shows
	 *  (as a purple-outlined pill) instead of vanishing from the board. */
	customKinds: number[];
};

/** Live, app-wide search defaults. Read by `handleSearch` and the
 *  interpretation strip; written by SearchConfigModal. */
export const searchConfig = $state<SearchConfig>({
	kinds: [...DEFAULT_KINDS],
	limit: DEFAULT_LIMIT,
	relays: [],
	customKinds: []
});

/** Visibility flag for the config modal — mounted once in +layout.svelte,
 *  opened from the search panel's gear button. */
export const searchConfigUI = $state<{ open: boolean }>({ open: false });

export function openSearchConfig() {
	searchConfigUI.open = true;
}
export function closeSearchConfig() {
	searchConfigUI.open = false;
}

export function loadSearchConfig() {
	if (!browser) return;
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return;
		const parsed = JSON.parse(raw) as Partial<SearchConfig>;
		if (Array.isArray(parsed.kinds)) {
			const kinds = [...new Set(parsed.kinds.filter((k) => Number.isInteger(k) && k >= 0))];
			searchConfig.kinds = kinds;
		}
		if (typeof parsed.limit === 'number' && parsed.limit > 0) {
			searchConfig.limit = Math.min(MAX_LIMIT, Math.floor(parsed.limit));
		}
		if (Array.isArray(parsed.relays)) {
			searchConfig.relays = parsed.relays.filter(
				(r): r is string => typeof r === 'string' && /^wss?:\/\//i.test(r)
			);
		}
		if (Array.isArray(parsed.customKinds)) {
			searchConfig.customKinds = [
				...new Set(parsed.customKinds.filter((k) => Number.isInteger(k) && k >= 0))
			];
		}
	} catch {
		// Corrupt entry — fall back to defaults silently.
	}
}

export function saveSearchConfig() {
	if (!browser) return;
	try {
		localStorage.setItem(
			STORAGE_KEY,
			JSON.stringify({
				kinds: searchConfig.kinds,
				limit: searchConfig.limit,
				relays: searchConfig.relays,
				customKinds: searchConfig.customKinds
			})
		);
	} catch {
		// Storage full / disabled — defaults still apply in-memory.
	}
}

export function resetSearchConfig() {
	searchConfig.kinds = [...DEFAULT_KINDS];
	searchConfig.limit = DEFAULT_LIMIT;
	searchConfig.relays = [];
	searchConfig.customKinds = [];
	saveSearchConfig();
}

export function kindLabel(kind: number): string {
	return KNOWN_KINDS.find((k) => k.kind === kind)?.label ?? `kind ${kind}`;
}

/** Match `k:30041` / `kind:1` anywhere a token can start. */
const KIND_TOKEN = /(?:^|\s)k(?:ind)?:\d+/i;

/** True when the query already pins kinds itself. */
export function queryHasExplicitKind(query: string): boolean {
	return KIND_TOKEN.test(query);
}

/**
 * Scope a raw query to the configured default kinds. Splits on `|` so
 * each OR-branch of a compound query is scoped independently; a branch
 * that already names kinds is left alone (the user's intent wins).
 */
export function applyKindScope(query: string): string {
	if (searchConfig.kinds.length === 0) return query;
	const kindTokens = searchConfig.kinds.map((k) => `k:${k}`).join(' ');
	return query
		.split('|')
		.map((branch) => {
			const b = branch.trim();
			if (!b) return b;
			if (queryHasExplicitKind(b)) return b;
			return `${kindTokens} ${b}`;
		})
		.join(' | ');
}
