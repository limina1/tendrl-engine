// NIP-55 (Android signer app / Amber) client for tendrl's external-signer
// protocol — the sibling of the NIP-07 client in `signer.ts`, fulfilling the
// same engine SSE channel through the Tauri host's in-app `nip55` plugin
// instead of `window.nostr`.
//
// Division of labor:
//   - Kotlin plugin: ContentResolver-first / intent-fallback transport,
//     npub→hex normalization. Reached via `window.__TAURI__.core.invoke`
//     (withGlobalTauri — no npm dependency).
//   - This module: register → use → SSE → fulfil → teardown, plus the NIP-01
//     id precomputation the signer contract wants (`id` set, `sig: ""`).
//   - Engine: signature verification (`verify_signed_event`) and the
//     pubkey-mismatch guard — no crypto validation happens in JS.

import * as api from '$lib/api';
import type { EventTemplate, SignedEvent } from './signer';

/** localStorage persistence: which signer app + pubkey to silently
 *  re-register on boot. Public data only — never key material. */
const PERSIST_KEY = 'tendrl.nip55';

/** The kinds tendrl writes; requested for auto-approval once, at connect
 *  time, so batch publishes sign silently via ContentResolver. The user can
 *  decline any of them in the signer app — everything still works, with
 *  prompts. */
export const NIP55_WRITE_KINDS = [0, 30040, 30041, 30023, 30817, 30818, 9802, 777, 30777, 1111];

export interface Nip55SignerApp {
	name: string;
	packageName: string;
}

export interface Nip55Persisted {
	packageName: string;
	pubkey: string;
}

interface TauriGlobal {
	core: {
		invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
	};
}

declare global {
	interface Window {
		__TAURI__?: TauriGlobal;
	}
}

/** True only inside the Tauri host (the Android app) — the NIP-55 login
 *  surfaces render nowhere else, the way armada's AndroidSignerOptions
 *  returns null off Capacitor Android. */
export function detectNip55(): boolean {
	return typeof window !== 'undefined' && typeof window.__TAURI__ !== 'undefined';
}

async function invokePlugin<T>(command: string, args?: Record<string, unknown>): Promise<T> {
	if (!detectNip55()) {
		throw new Error('NIP-55 signer apps are only reachable inside the Android app');
	}
	return (await window.__TAURI__!.core.invoke(`plugin:nip55|${command}`, args)) as T;
}

/** Enumerate installed NIP-55 signer apps (Amber, …). Empty off-device or
 *  when none are installed. */
export async function getSignerApps(): Promise<Nip55SignerApp[]> {
	const res = await invokePlugin<{ apps?: Nip55SignerApp[] }>('get_installed_signer_apps');
	return res.apps ?? [];
}

export function persistedNip55(): Nip55Persisted | null {
	if (typeof localStorage === 'undefined') return null;
	try {
		const raw = localStorage.getItem(PERSIST_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as Nip55Persisted;
		if (!parsed.packageName || !parsed.pubkey) return null;
		return parsed;
	} catch {
		return null;
	}
}

export function clearPersistedNip55(): void {
	try {
		localStorage.removeItem(PERSIST_KEY);
	} catch {
		/* storage unavailable */
	}
}

/** NIP-01 event id: SHA-256 of the canonical
 *  `[0, pubkey, created_at, kind, tags, content]` serialization. WebCrypto —
 *  no nostr library in the SPA (validation is engine-side; this exists only
 *  because the NIP-55 signer contract wants the id precomputed). */
export async function getEventHash(pubkey: string, template: EventTemplate): Promise<string> {
	const payload = JSON.stringify([
		0,
		pubkey,
		template.created_at,
		template.kind,
		template.tags,
		template.content
	]);
	const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(payload));
	return Array.from(new Uint8Array(digest))
		.map((b) => b.toString(16).padStart(2, '0'))
		.join('');
}

/** Ask the signer app for its pubkey (consent intent on first contact),
 *  requesting auto-approval for tendrl's write kinds in the same prompt.
 *  Used standalone by the watch-only upgrade flow to compare identities
 *  BEFORE displacing the watched npub; pass the result to
 *  `registerNip55Signer` as `prefetchedPubkey` so it doesn't re-prompt. */
export async function fetchSignerPubkey(packageName: string): Promise<string> {
	const permissions = JSON.stringify(
		NIP55_WRITE_KINDS.map((kind) => ({ type: 'sign_event', kind }))
	);
	const res = await invokePlugin<{ pubkey: string }>('get_public_key', {
		packageName,
		permissions
	});
	if (!res.pubkey) throw new Error('signer app returned no public key');
	return res.pubkey;
}

/**
 * Connect a NIP-55 signer app as the active signing source and start
 * fulfilling the engine's sign requests.
 *
 * `prefetchedPubkey`: pass the persisted pubkey on boot re-attach to skip
 * the `get_public_key` intent entirely — no signer-app prompt on every app
 * start (armada's seeded-pubkey pattern). First-time connects omit it, which
 * fires the consent intent with the write-kinds permission request.
 *
 * Returns a teardown closure (close the EventSource, revert the source) —
 * the same contract as `registerNip07Signer`, so callers treat both signer
 * families uniformly.
 */
export async function registerNip55Signer(
	packageName: string,
	prefetchedPubkey?: string
): Promise<() => void> {
	const pubkey = prefetchedPubkey ?? (await fetchSignerPubkey(packageName));

	// NIP-55's ContentResolver contract passes the logged-in account as an
	// npub (the spec's projection example is `listOf(event, "", npub)`).
	// Passing hex made the signer treat every silent query as
	// unauthorized → intent fallback → one prompt per event, which is
	// exactly the batch experience this path exists to avoid. The engine
	// owns NIP-19; fall back to hex only if the encode call itself fails.
	const currentUser = await api
		.encode({ kind: 'npub', pubkey })
		.catch(() => pubkey);

	const reg = await api.registerSigner({
		kind: 'nip55',
		pubkey,
		capabilities: {
			sign_event: true,
			nip04_encrypt: false,
			nip04_decrypt: false,
			nip44_encrypt: false,
			nip44_decrypt: false,
			auto_approve_kinds: NIP55_WRITE_KINDS
		}
	});

	await api.useIdentitySource({ source: 'nip55', signer_id: reg.signer_id, pubkey });

	const url = `/api/v1/identity/signer-channel?token=${encodeURIComponent(reg.token)}`;
	const es = new EventSource(url);

	es.onmessage = async (msg) => {
		try {
			const data = JSON.parse(msg.data) as {
				type: string;
				req_id: string;
				template: EventTemplate;
			};
			if (data.type !== 'sign_request') return;
			try {
				// The registered pubkey is authoritative: the engine's mismatch
				// guard already refused templates for anyone else.
				const template = { ...data.template, pubkey };
				const id = await getEventHash(pubkey!, template);
				const eventJson = JSON.stringify({ ...template, id, sig: '' });
				const res = await invokePlugin<{ event?: string; signature?: string }>('sign_event', {
					packageName,
					eventJson,
					id,
					currentUser
				});
				let signed: SignedEvent;
				if (res.event) {
					signed = JSON.parse(res.event) as SignedEvent;
				} else if (res.signature) {
					signed = { ...template, id, sig: res.signature } as SignedEvent;
				} else {
					throw new Error('signer app returned neither event nor signature');
				}
				await api.postSignResponse({
					signer_id: reg.signer_id,
					req_id: data.req_id,
					signed_event: signed
				});
			} catch (err) {
				await api.postSignResponse({
					signer_id: reg.signer_id,
					req_id: data.req_id,
					error: err instanceof Error ? err.message : String(err)
				});
			}
		} catch (e) {
			console.warn('[NIP-55 signer] failed to handle SSE message', e);
		}
	};

	es.onerror = (e) => {
		console.warn('[NIP-55 signer] SSE connection error', e);
	};

	try {
		localStorage.setItem(PERSIST_KEY, JSON.stringify({ packageName, pubkey }));
	} catch {
		/* storage unavailable — boot re-attach just won't be silent */
	}

	return () => {
		es.close();
		// Best-effort source revert; engine drops the registration on
		// channel close anyway via the stale-sweep path.
		api.useIdentitySource({ source: 'engine' }).catch(() => {});
	};
}
