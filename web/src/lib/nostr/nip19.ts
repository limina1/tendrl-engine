// NIP-19 client-side string helpers.
//
// Bech32 NIP-19 *encoding* (npub / nevent / naddr) and decoding both live in
// the Rust engine — see `src/nip19.rs` and `POST /api/v1/encode` / `/decode`.
// The frontend no longer ships its own bech32 implementation; it calls the
// engine via `api.encode(...)`. What remains here are two trivial,
// non-algorithmic string helpers used to sanitize input before building search
// queries (they derive nothing from events, so they stay frontend-side).

/** Strip an optional `nostr:` URI prefix from a NIP-19 identifier. */
export function stripNostrPrefix(s: string): string {
	const t = s.trim();
	return t.startsWith('nostr:') ? t.slice(6) : t;
}

/** RFC 4648 lowercase hex, exactly 64 chars. */
const HEX64_RE = /^[0-9a-f]{64}$/;
export function isHex64(s: string): boolean {
	return HEX64_RE.test(s.toLowerCase());
}
