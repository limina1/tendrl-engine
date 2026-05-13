// Minimal NIP-19 `naddr` encoder + helpers.
//
// We avoid pulling in `nostr-tools` (~200 KB, much of it irrelevant)
// for the handful of bytes we actually need: encode `kind:pubkey:dtag`
// → `naddr1...`. Bech32m + the NIP-19 TLV format is ~80 lines.

const CHARSET = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l';
const BECH32M_CONST = 0x2bc830a3;
const BECH32_CONST = 1;

function polymod(values: number[]): number {
	const GEN = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
	let chk = 1;
	for (const v of values) {
		const top = chk >> 25;
		chk = ((chk & 0x1ffffff) << 5) ^ v;
		for (let i = 0; i < 5; i++) if ((top >> i) & 1) chk ^= GEN[i];
	}
	return chk;
}

function hrpExpand(hrp: string): number[] {
	const ret: number[] = [];
	for (let i = 0; i < hrp.length; i++) ret.push(hrp.charCodeAt(i) >> 5);
	ret.push(0);
	for (let i = 0; i < hrp.length; i++) ret.push(hrp.charCodeAt(i) & 31);
	return ret;
}

function createChecksum(hrp: string, data: number[], constant: number): number[] {
	const values = hrpExpand(hrp).concat(data).concat([0, 0, 0, 0, 0, 0]);
	const mod = polymod(values) ^ constant;
	const ret: number[] = [];
	for (let p = 0; p < 6; p++) ret.push((mod >> (5 * (5 - p))) & 31);
	return ret;
}

function bech32Encode(hrp: string, data: number[], constant: number): string {
	const combined = data.concat(createChecksum(hrp, data, constant));
	let ret = `${hrp}1`;
	for (const v of combined) ret += CHARSET[v];
	return ret;
}

// Convert from one bit-group size to another. NIP-19 needs 8→5.
function convertBits(
	data: Uint8Array | number[],
	fromBits: number,
	toBits: number,
	pad: boolean
): number[] {
	let acc = 0;
	let bits = 0;
	const ret: number[] = [];
	const maxv = (1 << toBits) - 1;
	for (const value of data) {
		acc = (acc << fromBits) | value;
		bits += fromBits;
		while (bits >= toBits) {
			bits -= toBits;
			ret.push((acc >> bits) & maxv);
		}
	}
	if (pad && bits > 0) ret.push((acc << (toBits - bits)) & maxv);
	return ret;
}

function hexToBytes(hex: string): Uint8Array {
	const out = new Uint8Array(hex.length / 2);
	for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
	return out;
}

// TLV types per NIP-19.
const TLV_SPECIAL = 0;
const TLV_RELAY = 1;
const TLV_AUTHOR = 2;
const TLV_KIND = 3;

function tlv(type: number, value: Uint8Array): Uint8Array {
	const out = new Uint8Array(value.length + 2);
	out[0] = type;
	out[1] = value.length;
	out.set(value, 2);
	return out;
}

function concatBytes(...arrs: Uint8Array[]): Uint8Array {
	const total = arrs.reduce((n, a) => n + a.length, 0);
	const out = new Uint8Array(total);
	let off = 0;
	for (const a of arrs) {
		out.set(a, off);
		off += a.length;
	}
	return out;
}

/**
 * Encode `kind:pubkey:dtag` (and optional relay hints) into `naddr1...`.
 * pubkey must be 64-char hex.
 */
export function encodeNaddr(opts: {
	kind: number;
	pubkey: string;
	dTag: string;
	relays?: string[];
}): string {
	const dBytes = new TextEncoder().encode(opts.dTag);
	const authorBytes = hexToBytes(opts.pubkey);
	const kindBytes = new Uint8Array(4);
	new DataView(kindBytes.buffer).setUint32(0, opts.kind >>> 0, false);

	const relayTlvs = (opts.relays ?? []).map((r) =>
		tlv(TLV_RELAY, new TextEncoder().encode(r))
	);

	const payload = concatBytes(
		tlv(TLV_SPECIAL, dBytes),
		...relayTlvs,
		tlv(TLV_AUTHOR, authorBytes),
		tlv(TLV_KIND, kindBytes)
	);

	const fiveBit = convertBits(payload, 8, 5, true);
	return bech32Encode('naddr', fiveBit, BECH32M_CONST);
}

/**
 * Encode a 64-char hex pubkey as an `npub1...` bech32 string.
 * Uses the bech32 checksum (NIP-19 spec — npub and nsec are bech32,
 * everything else is bech32m).
 */
export function encodeNpub(hexPubkey: string): string {
	if (hexPubkey.length !== 64) {
		throw new Error(`encodeNpub: expected 64-char hex, got ${hexPubkey.length}`);
	}
	const bytes = hexToBytes(hexPubkey);
	const fiveBit = convertBits(bytes, 8, 5, true);
	return bech32Encode('npub', fiveBit, BECH32_CONST);
}

/**
 * Encode an event id (64-char hex) as an `nevent1...` bech32m string.
 * Only the SPECIAL TLV (the event id) — no relay/author/kind hints.
 */
export function encodeNevent(hexEventId: string): string {
	if (hexEventId.length !== 64) {
		throw new Error(`encodeNevent: expected 64-char hex, got ${hexEventId.length}`);
	}
	const idBytes = hexToBytes(hexEventId);
	const payload = tlv(TLV_SPECIAL, idBytes);
	const fiveBit = convertBits(payload, 8, 5, true);
	return bech32Encode('nevent', fiveBit, BECH32M_CONST);
}

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

/**
 * Convenience: encode the `kind:pubkey:dtag` form of an `a` tag value
 * into its `naddr1...` form. Returns null if input is malformed.
 */
export function naddrFromATag(aTagValue: string, relays?: string[]): string | null {
	const parts = aTagValue.split(':');
	if (parts.length < 3) return null;
	const kind = Number(parts[0]);
	const pubkey = parts[1];
	const dTag = parts.slice(2).join(':');
	if (!Number.isFinite(kind) || pubkey.length !== 64) return null;
	try {
		return encodeNaddr({ kind, pubkey, dTag, relays });
	} catch {
		return null;
	}
}
