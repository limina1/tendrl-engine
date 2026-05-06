// NIP-11 fetch + cache (browser-side, placeholder).
//
// Per docs/relay-classes-and-info-port.md: this is the lightweight
// equivalent of Amethyst's `Nip11Retriever` + `Nip11CachedRetriever`.
// Eventually moves to the engine (`/api/v1/relay/info?url=…`) so the
// cache is process-wide and survives reload, but the buffer-local
// cache here is sufficient as scaffolding.
//
// Principles applied (port doc §3):
//   - Tolerate sloppy JSON: `supported_nips` accepts ints and strings.
//   - Every field is optional; consumers must presence-check.
//   - URL normalization before cache keying.
//   - 5-second timeout, 256 KB body cap, semaphore of 5 concurrent.

export type Nip11Doc = {
	name?: string;
	description?: string;
	pubkey?: string;
	contact?: string;
	software?: string;
	version?: string;
	icon?: string;
	banner?: string;
	supported_nips?: number[];
	privacy_policy?: string;
	terms_of_service?: string;
	posting_policy?: string;
	limitation?: {
		auth_required?: boolean;
		payment_required?: boolean;
		restricted_writes?: boolean;
		max_message_length?: number;
		max_event_tags?: number;
		max_content_length?: number;
		max_subscriptions?: number;
		max_limit?: number;
		min_pow_difficulty?: number;
		created_at_lower_limit?: number;
		created_at_upper_limit?: number;
	};
	retention?: { kinds?: (number | [number, number])[]; time?: number; count?: number }[];
	relay_countries?: string[];
	language_tags?: string[];
	tags?: string[];
	fees?: {
		admission?: { amount: number; unit: string }[];
		subscription?: { amount: number; unit: string; period?: number }[];
		publication?: { amount: number; unit: string; kinds?: number[] }[];
	};
};

export type Nip11Status =
	| { state: 'pending' }
	| { state: 'loading' }
	| { state: 'loaded'; doc: Nip11Doc }
	| { state: 'failed'; error: string };

const TTL_MS = 60 * 60 * 1000; // 1h, per port doc
const MAX_BODY = 256 * 1024;
const TIMEOUT_MS = 5_000;
const MAX_CONCURRENT = 5;

const cache = new Map<string, { status: Nip11Status; ts: number }>();
const inflight = new Map<string, Promise<Nip11Status>>();
let active = 0;
const queue: (() => void)[] = [];

function acquire(): Promise<void> {
	if (active < MAX_CONCURRENT) {
		active++;
		return Promise.resolve();
	}
	return new Promise<void>((resolve) => queue.push(resolve)).then(() => {
		active++;
	});
}

function release() {
	active--;
	const next = queue.shift();
	if (next) next();
}

export function normalizeRelayUrl(url: string): string {
	return url.trim().toLowerCase().replace(/\/+$/, '');
}

function toHttp(wssUrl: string): string {
	if (wssUrl.startsWith('wss://')) return 'https://' + wssUrl.slice(6);
	if (wssUrl.startsWith('ws://')) return 'http://' + wssUrl.slice(5);
	return wssUrl;
}

// Permissive: NIP-11 in the wild ships supported_nips as a mix of
// ints and stringified ints. Fold to numbers, drop garbage.
function coerceNips(raw: unknown): number[] | undefined {
	if (!Array.isArray(raw)) return undefined;
	const out: number[] = [];
	for (const v of raw) {
		const n = typeof v === 'number' ? v : typeof v === 'string' ? Number(v) : NaN;
		if (Number.isFinite(n)) out.push(n);
	}
	return out;
}

async function fetchOnce(url: string): Promise<Nip11Status> {
	const httpUrl = toHttp(url);
	const ctrl = new AbortController();
	const t = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
	try {
		const resp = await fetch(httpUrl, {
			headers: { Accept: 'application/nostr+json' },
			signal: ctrl.signal
		});
		if (!resp.ok) return { state: 'failed', error: `HTTP ${resp.status}` };
		// Soft body cap — read up to MAX_BODY characters of text.
		const reader = resp.body?.getReader();
		if (!reader) return { state: 'failed', error: 'No response body' };
		const decoder = new TextDecoder();
		let received = '';
		while (received.length < MAX_BODY) {
			const { done, value } = await reader.read();
			if (done) break;
			received += decoder.decode(value, { stream: true });
		}
		const json = JSON.parse(received) as Record<string, unknown>;
		const doc: Nip11Doc = {
			name: typeof json.name === 'string' ? json.name : undefined,
			description: typeof json.description === 'string' ? json.description : undefined,
			pubkey: typeof json.pubkey === 'string' ? json.pubkey : undefined,
			contact: typeof json.contact === 'string' ? json.contact : undefined,
			software: typeof json.software === 'string' ? json.software : undefined,
			version: typeof json.version === 'string' ? json.version : undefined,
			icon: typeof json.icon === 'string' ? json.icon : undefined,
			banner: typeof json.banner === 'string' ? json.banner : undefined,
			supported_nips: coerceNips(json.supported_nips),
			privacy_policy:
				typeof json.privacy_policy === 'string' ? json.privacy_policy : undefined,
			terms_of_service:
				typeof json.terms_of_service === 'string' ? json.terms_of_service : undefined,
			posting_policy:
				typeof json.posting_policy === 'string' ? json.posting_policy : undefined,
			limitation: (json.limitation ?? undefined) as Nip11Doc['limitation'],
			retention: (json.retention ?? undefined) as Nip11Doc['retention'],
			relay_countries: Array.isArray(json.relay_countries)
				? (json.relay_countries as string[])
				: undefined,
			language_tags: Array.isArray(json.language_tags)
				? (json.language_tags as string[])
				: undefined,
			tags: Array.isArray(json.tags) ? (json.tags as string[]) : undefined,
			fees: (json.fees ?? undefined) as Nip11Doc['fees']
		};
		return { state: 'loaded', doc };
	} catch (e) {
		const msg = e instanceof Error ? e.message : String(e);
		return { state: 'failed', error: msg };
	} finally {
		clearTimeout(t);
	}
}

// Returns the current cached status for a relay URL. If absent or
// stale, kicks off a fetch (deduplicated) and returns 'loading'.
// `onUpdate` is called when the fetch resolves.
export function getRelayInfo(
	url: string,
	onUpdate?: (s: Nip11Status) => void
): Nip11Status {
	const key = normalizeRelayUrl(url);
	const cached = cache.get(key);
	const now = Date.now();
	if (cached && now - cached.ts < TTL_MS) return cached.status;

	if (inflight.has(key)) {
		if (onUpdate) inflight.get(key)!.then(onUpdate);
		return { state: 'loading' };
	}

	const promise = acquire().then(async () => {
		try {
			const result = await fetchOnce(url);
			cache.set(key, { status: result, ts: Date.now() });
			return result;
		} finally {
			release();
			inflight.delete(key);
		}
	});
	inflight.set(key, promise);
	if (onUpdate) promise.then(onUpdate);
	return { state: 'loading' };
}
