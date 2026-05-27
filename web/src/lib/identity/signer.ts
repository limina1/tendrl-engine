// NIP-07 client for tendrl's external-signer protocol.
//
// Architecture (see docs/identity-and-signing-plan.md):
//   - The engine owns the SigningController + registry.
//   - When the active source is `nip07`, the engine emits SignRequest
//     events through `GET /api/v1/identity/signer-channel` (SSE).
//   - This module connects to that channel, fulfils requests via
//     `window.nostr.signEvent`, and POSTs results back to
//     `/api/v1/identity/sign-response`.
//   - `signAndPublish` is the convenience used by ComposeView's
//     Publish button: builds templates, asks the engine to sign each
//     (which round-trips through the SSE channel for nip07 users or
//     resolves in-process for engine users), then submits the pre-
//     signed events to the publish endpoint.

import * as api from '$lib/api';
import type { SignTemplateResponse } from '$lib/api';
import type { IdentityStatus } from '$lib/types';

/**
 * True when the active identity can produce signatures: either the
 * in-process engine key is unlocked, OR an external signer (NIP-07 /
 * NIP-46) is connected. `state === 'unlocked'` describes only the
 * engine key, so checking it alone wrongly excludes signer logins —
 * which is why the compose Publish button vanished under a NIP-07 login.
 */
export function identityCanSign(status: IdentityStatus | null | undefined): boolean {
	if (!status) return false;
	return (
		status.state === 'unlocked' || status.source === 'nip07' || status.source === 'nip46'
	);
}

declare global {
	interface Window {
		nostr?: NostrSigner;
	}
}

export interface NostrSigner {
	getPublicKey(): Promise<string>;
	signEvent(template: EventTemplate): Promise<SignedEvent>;
	getRelays?(): Promise<Record<string, { read: boolean; write: boolean }>>;
	nip04?: {
		encrypt(pubkey: string, plaintext: string): Promise<string>;
		decrypt(pubkey: string, ciphertext: string): Promise<string>;
	};
	nip44?: {
		encrypt(pubkey: string, plaintext: string): Promise<string>;
		decrypt(pubkey: string, ciphertext: string): Promise<string>;
	};
}

export interface EventTemplate {
	kind: number;
	created_at: number;
	tags: string[][];
	content: string;
	pubkey?: string;
}

export interface SignedEvent {
	id: string;
	pubkey: string;
	kind: number;
	created_at: number;
	tags: string[][];
	content: string;
	sig: string;
}

/** Whether a NIP-07 signer is reachable on `window.nostr`. */
export function detectNip07(): boolean {
	return typeof window !== 'undefined' && typeof window.nostr !== 'undefined';
}

/**
 * Register `window.nostr` as an external signer with the engine, then
 * open the SSE channel and start fulfilling sign requests.
 *
 * Returns a teardown function that closes the EventSource and clears
 * the engine-side registration. Calling teardown reverses everything.
 *
 * Errors propagate from any step (extension refusal, registration
 * POST failure, EventSource open failure) — callers should fall back
 * to `engine` source on failure.
 */
export async function registerNip07Signer(): Promise<() => void> {
	if (!detectNip07()) {
		throw new Error('No window.nostr signer detected');
	}
	const signer = window.nostr!;
	const pubkey = await signer.getPublicKey();

	const reg = await api.registerSigner({
		kind: 'nip07',
		pubkey,
		capabilities: {
			sign_event: true,
			nip04_encrypt: !!signer.nip04?.encrypt,
			nip04_decrypt: !!signer.nip04?.decrypt,
			nip44_encrypt: !!signer.nip44?.encrypt,
			nip44_decrypt: !!signer.nip44?.decrypt,
			auto_approve_kinds: []
		}
	});

	// Switch the engine's active source to point at this registration.
	await api.useIdentitySource({ source: 'nip07', signer_id: reg.signer_id });

	const url = `/api/v1/identity/signer-channel?token=${encodeURIComponent(reg.token)}`;
	const es = new EventSource(url);

	es.onmessage = async (msg) => {
		try {
			const data = JSON.parse(msg.data) as { type: string; req_id: string; template: EventTemplate };
			if (data.type !== 'sign_request') return;
			try {
				const signed = await signer.signEvent(data.template);
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
			console.warn('[NIP-07 signer] failed to handle SSE message', e);
		}
	};

	es.onerror = (e) => {
		console.warn('[NIP-07 signer] SSE connection error', e);
	};

	return () => {
		es.close();
		// Best-effort source revert; engine drops the registration on
		// channel close anyway via the stale-sweep path.
		api.useIdentitySource({ source: 'engine' }).catch(() => {});
	};
}

/**
 * Sign + submit. Builds an array of EventTemplates from the caller,
 * asks the engine to sign each (which round-trips through the SSE
 * channel for nip07 sources or resolves in-process for engine
 * sources), then hands the batch to the publish endpoint.
 *
 * The batch-up-front pattern is the mitigation for the 12-prompts
 * problem: a publication with 12 sections triggers 12 sign-event
 * prompts but the user clicks them in one burst rather than spread
 * across the publish duration.
 */
export async function signAndPublish(
	templates: EventTemplate[]
): Promise<SignedEvent[]> {
	const signed: SignedEvent[] = [];
	for (const template of templates) {
		const resp = (await api.signTemplate({ template })) as SignTemplateResponse;
		signed.push(resp.signed_event as SignedEvent);
	}
	return signed;
}

/**
 * Sign one template through the active source, then broadcast the
 * resulting event via the engine's `/api/v1/broadcast` endpoint.
 * Used by the profile-edit flow and any other "publish a single
 * non-publication event" surface.
 */
export async function signAndBroadcast(
	template: EventTemplate,
	relays?: string[]
): Promise<{ signed: SignedEvent; broadcast: import('$lib/api').BroadcastResponse }> {
	const resp = (await api.signTemplate({ template })) as SignTemplateResponse;
	const signed = resp.signed_event as SignedEvent;
	const broadcast = await api.broadcastEvent({ event: signed, relays });
	return { signed, broadcast };
}
