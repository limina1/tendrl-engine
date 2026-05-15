// Search configuration — the standing defaults a search runs with when
// the query string itself doesn't pin them: kinds, result limit, author
// scope, time window, the relays the offline fallback queries, and the
// NIP-50 relay-search toggle.
//
// Why this exists: a bare query like `nostr` has no `k:` filter, so the
// engine matches *every* kind and kind-1 noise leaks in; and the author
// scope ("just my stuff") used to be a hidden hardcoded rule. The
// defaults are now explicit and editable (SearchConfigModal, the gear
// button on the search panel) and persist across reloads.
//
// Two consumers inherit the scope:
//   - the in-DB search (`handleSearch`) prepends `applySearchDefaults`
//   - the offline "Search relays" fallback replays the same effective
//     query, so it carries the same scope to the relay REQ
//
// The user always wins: a query that writes its own `k:` / `by:` /
// `since:` / `until:` token keeps it — the matching default is skipped
// for that branch.

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

/** Author scope. 'me' prepends `by:me`; 'pubkey' prepends `by:<pubkey>`;
 *  'anyone' adds no author constraint. */
export type AuthorMode = 'me' | 'anyone' | 'pubkey';

export type SearchConfig = {
	/** Kinds a search scopes to when the query has no explicit k:/kind:. */
	kinds: number[];
	/** Result cap handed to the engine and to relay REQs. */
	limit: number;
	/** Relays the "Search relays" fallback queries (the *selected* set).
	 *  Empty = fall back to the engine's configured `[relay.fetch]` set. */
	relays: string[];
	/** Relay URLs the user typed into the modal. Kept separate from
	 *  `relays` so an added relay toggled *off* still shows (purple)
	 *  instead of vanishing — mirrors `customKinds`. */
	addedRelays: string[];
	/** Non-standard kinds the user added — kept so a custom kind toggled
	 *  *off* still shows (purple-outlined) instead of vanishing. */
	customKinds: number[];
	/** Default author scope. */
	author: { mode: AuthorMode; pubkey: string };
	/** NIP-01 time bounds, unix seconds. null = unbounded. */
	since: number | null;
	until: number | null;
	/** NIP-50 relay-side full-text search. When enabled, the offline
	 *  "Search relays" flow asks NIP-50 relays to match the query's free
	 *  text; the extension knobs ride along in the search string. */
	nip50: { enabled: boolean; language: string; nsfw: boolean; includeSpam: boolean };
};

function defaults(): SearchConfig {
	return {
		kinds: [...DEFAULT_KINDS],
		limit: DEFAULT_LIMIT,
		relays: [],
		addedRelays: [],
		customKinds: [],
		author: { mode: 'me', pubkey: '' },
		since: null,
		until: null,
		nip50: { enabled: false, language: '', nsfw: true, includeSpam: false }
	};
}

/** Live, app-wide search defaults. Read by `handleSearch`; written by
 *  SearchConfigModal. */
export const searchConfig = $state<SearchConfig>(defaults());

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
		const p = JSON.parse(raw) as Partial<SearchConfig>;
		if (Array.isArray(p.kinds)) {
			searchConfig.kinds = [...new Set(p.kinds.filter((k) => Number.isInteger(k) && k >= 0))];
		}
		if (typeof p.limit === 'number' && p.limit > 0) {
			searchConfig.limit = Math.min(MAX_LIMIT, Math.floor(p.limit));
		}
		const relayList = (v: unknown): string[] =>
			Array.isArray(v)
				? v.filter((r): r is string => typeof r === 'string' && /^wss?:\/\//i.test(r))
				: [];
		if (p.relays !== undefined) searchConfig.relays = relayList(p.relays);
		if (p.addedRelays !== undefined) searchConfig.addedRelays = relayList(p.addedRelays);
		if (Array.isArray(p.customKinds)) {
			searchConfig.customKinds = [
				...new Set(p.customKinds.filter((k) => Number.isInteger(k) && k >= 0))
			];
		}
		if (p.author && (p.author.mode === 'me' || p.author.mode === 'anyone' || p.author.mode === 'pubkey')) {
			searchConfig.author = {
				mode: p.author.mode,
				pubkey: typeof p.author.pubkey === 'string' ? p.author.pubkey : ''
			};
		}
		if (typeof p.since === 'number' || p.since === null) searchConfig.since = p.since;
		if (typeof p.until === 'number' || p.until === null) searchConfig.until = p.until;
		if (p.nip50) {
			searchConfig.nip50 = {
				enabled: !!p.nip50.enabled,
				language: typeof p.nip50.language === 'string' ? p.nip50.language : '',
				nsfw: p.nip50.nsfw !== false,
				includeSpam: !!p.nip50.includeSpam
			};
		}
	} catch {
		// Corrupt entry — fall back to defaults silently.
	}
}

export function saveSearchConfig() {
	if (!browser) return;
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify($state.snapshot(searchConfig)));
	} catch {
		// Storage full / disabled — defaults still apply in-memory.
	}
}

export function resetSearchConfig() {
	Object.assign(searchConfig, defaults());
	saveSearchConfig();
}

export function kindLabel(kind: number): string {
	return KNOWN_KINDS.find((k) => k.kind === kind)?.label ?? `kind ${kind}`;
}

// ---- Query scoping ---------------------------------------------------

const KIND_TOKEN = /(?:^|\s)k(?:ind)?:\d+/i;
const AUTHOR_TOKEN = /(?:^|\s)by:/i;
const SINCE_TOKEN = /(?:^|\s)since:\d+/i;
const UNTIL_TOKEN = /(?:^|\s)until:\d+/i;
const SEMANTIC_TOKEN = /(?:^|\s)~:/;

/** True when the query already pins kinds itself. */
export function queryHasExplicitKind(query: string): boolean {
	return KIND_TOKEN.test(query);
}

/**
 * Prepend the configured default scope to a raw query. Splits on `|` so
 * each OR-branch is scoped independently; a branch that already names a
 * given dimension (kind / author / time) keeps its own value.
 *
 * `scopeAuthor=false` suppresses the author default (used by replays
 * that explicitly opted out of "scope to me"). `hasIdentity=false`
 * suppresses `by:me` specifically — the engine rejects `by:me` with no
 * configured pubkey, so an identity-less session searches everyone.
 */
export function applySearchDefaults(
	query: string,
	opts: { scopeAuthor?: boolean; hasIdentity?: boolean } = {}
): string {
	const scopeAuthor = opts.scopeAuthor ?? true;
	const hasIdentity = opts.hasIdentity ?? true;
	const kindTokens = searchConfig.kinds.map((k) => `k:${k}`).join(' ');

	return query
		.split('|')
		.map((branch) => {
			const b = branch.trim();
			if (!b) return b;
			const prefix: string[] = [];

			if (searchConfig.kinds.length > 0 && !KIND_TOKEN.test(b)) {
				prefix.push(kindTokens);
			}
			// Semantic branches are usually cross-author — don't force a
			// `by:` onto them.
			if (
				scopeAuthor &&
				searchConfig.author.mode !== 'anyone' &&
				!AUTHOR_TOKEN.test(b) &&
				!SEMANTIC_TOKEN.test(b)
			) {
				if (searchConfig.author.mode === 'me') {
					if (hasIdentity) prefix.push('by:me');
				} else if (searchConfig.author.pubkey) {
					prefix.push(`by:${searchConfig.author.pubkey}`);
				}
			}
			if (searchConfig.since != null && !SINCE_TOKEN.test(b)) {
				prefix.push(`since:${searchConfig.since}`);
			}
			if (searchConfig.until != null && !UNTIL_TOKEN.test(b)) {
				prefix.push(`until:${searchConfig.until}`);
			}

			return prefix.length ? `${prefix.join(' ')} ${b}` : b;
		})
		.join(' | ');
}

/**
 * Build the NIP-50 `search` string for a query: its free-text keywords
 * (operator tokens stripped) plus the configured extension knobs.
 * Returns null when there's nothing to search for.
 */
export function nip50SearchString(query: string): string | null {
	const keywords = query
		.split(/\s+/)
		.filter((t) => t && t !== '|' && !/^[a-zA-Z_]+:/.test(t) && !t.startsWith('~:'))
		.join(' ')
		.trim();
	const parts: string[] = [];
	if (keywords) parts.push(keywords);
	// NIP-50 extension key:value pairs — relays ignore unsupported ones.
	if (searchConfig.nip50.language) parts.push(`language:${searchConfig.nip50.language}`);
	if (!searchConfig.nip50.nsfw) parts.push('nsfw:false');
	if (searchConfig.nip50.includeSpam) parts.push('include:spam');
	const out = parts.join(' ').trim();
	return out.length > 0 ? out : null;
}
