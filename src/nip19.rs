//! NIP-19 bech32 decoders for nevent / naddr / nprofile.
//!
//! `decode_npub`/`decode_nsec` already live in `identity.rs`; this module
//! covers the TLV-payload variants and exposes a unified `decode()` that
//! returns a tagged enum suitable for serialisation back to the web layer
//! over the HTTP API.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("Invalid bech32 encoding: {0}")]
    Bech32(String),
    #[error("Unknown HRP: {0}")]
    UnknownHrp(String),
    #[error("Truncated TLV record")]
    TruncatedTlv,
    #[error("Invalid length for TLV type {tlv_type:#04x}: expected {expected}, got {actual}")]
    InvalidLength {
        tlv_type: u8,
        expected: usize,
        actual: usize,
    },
    #[error("Missing required TLV type {0:#04x}")]
    MissingRequiredTlv(u8),
    #[error("Invalid UTF-8 in d-tag")]
    InvalidUtf8,
}

/// A successfully decoded NIP-19 identifier.
///
/// JSON shape (tagged by `kind`):
/// - `{"kind":"npub","pubkey":...}`
/// - `{"kind":"note","event_id":...}`
/// - `{"kind":"nprofile","pubkey":..., "relays":[...]}`
/// - `{"kind":"nevent","event_id":..., "relays":[...], "author":?, "kind_int":?}`
/// - `{"kind":"naddr","kind_int":..., "pubkey":..., "d_tag":..., "relays":[...]}`
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Decoded {
    Npub {
        pubkey: String,
    },
    /// `note1…` — a plain 32-byte event id (no TLV, like `npub`).
    Note {
        event_id: String,
    },
    Nprofile {
        pubkey: String,
        relays: Vec<String>,
    },
    Nevent {
        event_id: String,
        relays: Vec<String>,
        author: Option<String>,
        kind_int: Option<u32>,
    },
    Naddr {
        kind_int: u32,
        pubkey: String,
        d_tag: String,
        relays: Vec<String>,
    },
}

/// Strip an optional `nostr:` URI prefix. Idempotent on plain bech32 input.
pub fn strip_nostr_prefix(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix("nostr:").unwrap_or(s)
}

pub fn decode(input: &str) -> Result<Decoded, DecodeError> {
    let input = strip_nostr_prefix(input);
    let (hrp, data) = bech32::decode(input).map_err(|e| DecodeError::Bech32(e.to_string()))?;

    let hrp_str = hrp.as_str().to_lowercase();
    match hrp_str.as_str() {
        "npub" => {
            if data.len() != 32 {
                return Err(DecodeError::InvalidLength {
                    tlv_type: 0,
                    expected: 32,
                    actual: data.len(),
                });
            }
            Ok(Decoded::Npub {
                pubkey: hex::encode(&data),
            })
        }
        "note" => {
            if data.len() != 32 {
                return Err(DecodeError::InvalidLength {
                    tlv_type: 0,
                    expected: 32,
                    actual: data.len(),
                });
            }
            Ok(Decoded::Note {
                event_id: hex::encode(&data),
            })
        }
        "nprofile" => decode_nprofile_payload(&data),
        "nevent" => decode_nevent_payload(&data),
        "naddr" => decode_naddr_payload(&data),
        _ => Err(DecodeError::UnknownHrp(hrp_str)),
    }
}

fn parse_tlv(data: &[u8]) -> Result<Vec<(u8, &[u8])>, DecodeError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if i + 2 > data.len() {
            return Err(DecodeError::TruncatedTlv);
        }
        let t = data[i];
        let l = data[i + 1] as usize;
        i += 2;
        if i + l > data.len() {
            return Err(DecodeError::TruncatedTlv);
        }
        out.push((t, &data[i..i + l]));
        i += l;
    }
    Ok(out)
}

fn require_len(tlv_type: u8, v: &[u8], expected: usize) -> Result<(), DecodeError> {
    if v.len() != expected {
        return Err(DecodeError::InvalidLength {
            tlv_type,
            expected,
            actual: v.len(),
        });
    }
    Ok(())
}

fn decode_nprofile_payload(data: &[u8]) -> Result<Decoded, DecodeError> {
    let mut pubkey = None;
    let mut relays = Vec::new();
    for (t, v) in parse_tlv(data)? {
        match t {
            0x00 => {
                require_len(0, v, 32)?;
                pubkey = Some(hex::encode(v));
            }
            0x01 => relays.push(String::from_utf8_lossy(v).into_owned()),
            _ => {}
        }
    }
    Ok(Decoded::Nprofile {
        pubkey: pubkey.ok_or(DecodeError::MissingRequiredTlv(0))?,
        relays,
    })
}

fn decode_nevent_payload(data: &[u8]) -> Result<Decoded, DecodeError> {
    let mut event_id = None;
    let mut relays = Vec::new();
    let mut author = None;
    let mut kind_int = None;
    for (t, v) in parse_tlv(data)? {
        match t {
            0x00 => {
                require_len(0, v, 32)?;
                event_id = Some(hex::encode(v));
            }
            0x01 => relays.push(String::from_utf8_lossy(v).into_owned()),
            0x02 => {
                require_len(2, v, 32)?;
                author = Some(hex::encode(v));
            }
            0x03 => {
                require_len(3, v, 4)?;
                kind_int = Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
            }
            _ => {}
        }
    }
    Ok(Decoded::Nevent {
        event_id: event_id.ok_or(DecodeError::MissingRequiredTlv(0))?,
        relays,
        author,
        kind_int,
    })
}

fn decode_naddr_payload(data: &[u8]) -> Result<Decoded, DecodeError> {
    let mut d_tag = None;
    let mut relays = Vec::new();
    let mut pubkey = None;
    let mut kind_int = None;
    for (t, v) in parse_tlv(data)? {
        match t {
            0x00 => {
                d_tag = Some(
                    std::str::from_utf8(v)
                        .map_err(|_| DecodeError::InvalidUtf8)?
                        .to_string(),
                );
            }
            0x01 => relays.push(String::from_utf8_lossy(v).into_owned()),
            0x02 => {
                require_len(2, v, 32)?;
                pubkey = Some(hex::encode(v));
            }
            0x03 => {
                require_len(3, v, 4)?;
                kind_int = Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]]));
            }
            _ => {}
        }
    }
    Ok(Decoded::Naddr {
        d_tag: d_tag.ok_or(DecodeError::MissingRequiredTlv(0))?,
        relays,
        pubkey: pubkey.ok_or(DecodeError::MissingRequiredTlv(2))?,
        kind_int: kind_int.ok_or(DecodeError::MissingRequiredTlv(3))?,
    })
}

// ============================================================================
// Encoding (the inverse of `decode`)
// ============================================================================
//
// NIP-19 specifies *bech32* (checksum constant 1) for every entity — npub /
// nevent / naddr alike — NOT bech32m. The `bech32::encode::<Bech32>` path below
// therefore round-trips exactly with `decode` above (see the round-trip tests).

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("Invalid hex for {what}: {source}")]
    Hex {
        what: &'static str,
        source: hex::FromHexError,
    },
    #[error("Expected a 32-byte {what}, got {actual} bytes")]
    BadLength { what: &'static str, actual: usize },
    #[error("Bech32 encode failed: {0}")]
    Bech32(String),
    #[error("Malformed `a`-tag coordinate (want kind:pubkey:d_tag): {0:?}")]
    BadCoordinate(String),
}

/// Decode a 32-byte hex field, erroring with a field-named message on bad
/// input. Used for pubkeys and event ids.
fn hex32(s: &str, what: &'static str) -> Result<Vec<u8>, EncodeError> {
    let bytes = hex::decode(s).map_err(|source| EncodeError::Hex { what, source })?;
    if bytes.len() != 32 {
        return Err(EncodeError::BadLength {
            what,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

/// Append one NIP-19 TLV record. Lengths are a single byte, so relay URLs and
/// d-tags must be < 256 bytes — the same bound the decoder assumes.
fn push_tlv(out: &mut Vec<u8>, t: u8, v: &[u8]) {
    out.push(t);
    out.push(v.len() as u8);
    out.extend_from_slice(v);
}

fn bech32_encode(hrp: &str, data: &[u8]) -> Result<String, EncodeError> {
    use bech32::{Bech32, Hrp};
    let hrp = Hrp::parse(hrp).map_err(|e| EncodeError::Bech32(e.to_string()))?;
    bech32::encode::<Bech32>(hrp, data).map_err(|e| EncodeError::Bech32(e.to_string()))
}

/// Encode a 32-byte hex pubkey as `npub1…` (plain bech32, no TLV).
pub fn encode_npub(pubkey_hex: &str) -> Result<String, EncodeError> {
    bech32_encode("npub", &hex32(pubkey_hex, "pubkey")?)
}

/// Encode a 32-byte hex event id as `note1…` (plain bech32, no TLV).
pub fn encode_note(event_id_hex: &str) -> Result<String, EncodeError> {
    bech32_encode("note", &hex32(event_id_hex, "event id")?)
}

/// Encode an `nprofile1…`: the type-0 pubkey plus optional relay hints.
/// Mirrors `decode_nprofile_payload`.
pub fn encode_nprofile(pubkey_hex: &str, relays: &[String]) -> Result<String, EncodeError> {
    let mut tlv = Vec::new();
    push_tlv(&mut tlv, 0x00, &hex32(pubkey_hex, "pubkey")?);
    for r in relays {
        push_tlv(&mut tlv, 0x01, r.as_bytes());
    }
    bech32_encode("nprofile", &tlv)
}

/// Encode an `nevent1…`: the type-0 event id plus optional relay/author/kind
/// hints. Mirrors `decode_nevent_payload`.
pub fn encode_nevent(
    event_id_hex: &str,
    relays: &[String],
    author: Option<&str>,
    kind: Option<u32>,
) -> Result<String, EncodeError> {
    let mut tlv = Vec::new();
    push_tlv(&mut tlv, 0x00, &hex32(event_id_hex, "event id")?);
    for r in relays {
        push_tlv(&mut tlv, 0x01, r.as_bytes());
    }
    if let Some(a) = author {
        push_tlv(&mut tlv, 0x02, &hex32(a, "author")?);
    }
    if let Some(k) = kind {
        push_tlv(&mut tlv, 0x03, &k.to_be_bytes());
    }
    bech32_encode("nevent", &tlv)
}

/// Encode an `naddr1…` for a `kind:pubkey:d_tag` coordinate plus optional relay
/// hints. Mirrors `decode_naddr_payload` (type 0 = d-tag, 2 = author, 3 = kind).
pub fn encode_naddr(
    kind: u32,
    pubkey_hex: &str,
    d_tag: &str,
    relays: &[String],
) -> Result<String, EncodeError> {
    let author = hex32(pubkey_hex, "pubkey")?;
    let mut tlv = Vec::new();
    push_tlv(&mut tlv, 0x00, d_tag.as_bytes());
    for r in relays {
        push_tlv(&mut tlv, 0x01, r.as_bytes());
    }
    push_tlv(&mut tlv, 0x02, &author);
    push_tlv(&mut tlv, 0x03, &kind.to_be_bytes());
    bech32_encode("naddr", &tlv)
}

/// Convenience: encode a raw `a`-tag coordinate string (`kind:pubkey:d_tag`,
/// the d-tag may itself contain colons) into its `naddr1…` form.
pub fn naddr_from_a_tag(a_tag: &str, relays: &[String]) -> Result<String, EncodeError> {
    let mut parts = a_tag.splitn(3, ':');
    let kind = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or_else(|| EncodeError::BadCoordinate(a_tag.to_string()))?;
    let pubkey = parts
        .next()
        .ok_or_else(|| EncodeError::BadCoordinate(a_tag.to_string()))?;
    let d_tag = parts.next().unwrap_or("");
    encode_naddr(kind, pubkey, d_tag, relays)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_idempotent() {
        assert_eq!(strip_nostr_prefix("nostr:npub1abc"), "npub1abc");
        assert_eq!(strip_nostr_prefix("npub1abc"), "npub1abc");
        assert_eq!(strip_nostr_prefix("  nostr:npub1abc  "), "npub1abc");
    }

    // Known NIP-19 test vector from the spec:
    //   npub1sn0wdenkukak0d9dfczzeacvhkrgz92ak56egt7vdgzn8pv2wfqqhrjdv9
    //   pubkey = 84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240
    #[test]
    fn decode_known_npub() {
        let res = decode("npub1sn0wdenkukak0d9dfczzeacvhkrgz92ak56egt7vdgzn8pv2wfqqhrjdv9")
            .expect("decode npub");
        match res {
            Decoded::Npub { pubkey } => {
                assert_eq!(
                    pubkey,
                    "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240"
                );
            }
            other => panic!("expected Npub, got {:?}", other),
        }
    }

    #[test]
    fn nostr_prefix_decodes() {
        let res = decode("nostr:npub1sn0wdenkukak0d9dfczzeacvhkrgz92ak56egt7vdgzn8pv2wfqqhrjdv9");
        assert!(res.is_ok());
    }

    #[test]
    fn unknown_hrp_rejected() {
        let err = decode("foo1qpzry9x8gf2tvdw0s3jn54khce6mua7l").unwrap_err();
        match err {
            DecodeError::Bech32(_) | DecodeError::UnknownHrp(_) => {}
            other => panic!("expected Bech32/UnknownHrp, got {:?}", other),
        }
    }

    #[test]
    fn decode_note_round_trips() {
        use bech32::{Bech32, Hrp};
        let id = "ae3a6f7ce2971e43cfeeda2a41f30206d205cc16542a5cd9e127cefb01d409a4";
        let bytes = hex::decode(id).unwrap();
        let note = bech32::encode::<Bech32>(Hrp::parse("note").unwrap(), &bytes).unwrap();
        match decode(&note).expect("decode note") {
            Decoded::Note { event_id } => assert_eq!(event_id, id),
            other => panic!("expected Note, got {:?}", other),
        }
    }

    fn encode(hrp: &str, payload: &[u8]) -> String {
        use bech32::{Bech32, Hrp};
        bech32::encode::<Bech32>(Hrp::parse(hrp).unwrap(), payload).unwrap()
    }

    #[test]
    fn decode_nevent_with_all_tlvs() {
        // A realistic nevent: event-id + relay + author + kind TLVs,
        // exactly as Damus / Amethyst / njump emit them.
        let id = "ae3a6f7ce2971e43cfeeda2a41f30206d205cc16542a5cd9e127cefb01d409a4";
        let author = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
        let relay = b"wss://relay.damus.io";
        let mut tlv = vec![0x00, 32];
        tlv.extend(hex::decode(id).unwrap());
        tlv.extend([0x01, relay.len() as u8]);
        tlv.extend_from_slice(relay);
        tlv.extend([0x02, 32]);
        tlv.extend(hex::decode(author).unwrap());
        tlv.extend([0x03, 4]);
        tlv.extend(1u32.to_be_bytes());

        match decode(&encode("nevent", &tlv)).expect("decode nevent") {
            Decoded::Nevent {
                event_id,
                relays,
                author: a,
                kind_int,
            } => {
                assert_eq!(event_id, id);
                assert_eq!(relays, vec!["wss://relay.damus.io".to_string()]);
                assert_eq!(a.as_deref(), Some(author));
                assert_eq!(kind_int, Some(1));
            }
            other => panic!("expected Nevent, got {:?}", other),
        }
    }

    #[test]
    fn decode_nevent_id_only() {
        // The minimal nevent — just the type-0 event id, no hints.
        let id = "ae3a6f7ce2971e43cfeeda2a41f30206d205cc16542a5cd9e127cefb01d409a4";
        let mut tlv = vec![0x00, 32];
        tlv.extend(hex::decode(id).unwrap());
        match decode(&encode("nevent", &tlv)).expect("decode nevent") {
            Decoded::Nevent { event_id, .. } => assert_eq!(event_id, id),
            other => panic!("expected Nevent, got {:?}", other),
        }
    }

    #[test]
    fn decode_naddr_round_trips() {
        let pubkey = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
        let d_tag = b"my-publication";
        let mut tlv = vec![0x00, d_tag.len() as u8];
        tlv.extend_from_slice(d_tag);
        tlv.extend([0x02, 32]);
        tlv.extend(hex::decode(pubkey).unwrap());
        tlv.extend([0x03, 4]);
        tlv.extend(30040u32.to_be_bytes());

        match decode(&encode("naddr", &tlv)).expect("decode naddr") {
            Decoded::Naddr {
                kind_int,
                pubkey: pk,
                d_tag: d,
                ..
            } => {
                assert_eq!(kind_int, 30040);
                assert_eq!(pk, pubkey);
                assert_eq!(d, "my-publication");
            }
            other => panic!("expected Naddr, got {:?}", other),
        }
    }

    #[test]
    fn tlv_parser_round_trip() {
        // Build a minimal nevent-style payload by hand:
        // type=0, len=32, 32 zero bytes; type=3, len=4, big-endian u32=30041.
        let mut data = vec![0x00u8, 32];
        data.extend(std::iter::repeat(0u8).take(32));
        data.extend([0x03u8, 4]);
        data.extend(30041u32.to_be_bytes());

        let tlvs = parse_tlv(&data).expect("parse_tlv");
        assert_eq!(tlvs.len(), 2);
        assert_eq!(tlvs[0].0, 0x00);
        assert_eq!(tlvs[0].1.len(), 32);
        assert_eq!(tlvs[1].0, 0x03);
        assert_eq!(tlvs[1].1, &30041u32.to_be_bytes());
    }

    #[test]
    fn tlv_truncated_record_errors() {
        // type=0, len=32, but only 4 bytes follow.
        let data = vec![0x00u8, 32, 1, 2, 3, 4];
        let err = parse_tlv(&data).unwrap_err();
        matches!(err, DecodeError::TruncatedTlv);
    }

    // === Encoding: the core invariant is decode(encode(x)) == x. ===

    const PUBKEY: &str = "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240";
    const EVENT_ID: &str = "ae3a6f7ce2971e43cfeeda2a41f30206d205cc16542a5cd9e127cefb01d409a4";

    #[test]
    fn encode_npub_round_trips() {
        let npub = encode_npub(PUBKEY).expect("encode npub");
        assert!(npub.starts_with("npub1"));
        match decode(&npub).expect("decode npub") {
            Decoded::Npub { pubkey } => assert_eq!(pubkey, PUBKEY),
            other => panic!("expected Npub, got {:?}", other),
        }
    }

    #[test]
    fn encode_npub_matches_known_vector() {
        // The spec vector from `decode_known_npub`, in the other direction.
        assert_eq!(
            encode_npub(PUBKEY).unwrap(),
            "npub1sn0wdenkukak0d9dfczzeacvhkrgz92ak56egt7vdgzn8pv2wfqqhrjdv9"
        );
    }

    #[test]
    fn encode_note_round_trips() {
        match decode(&encode_note(EVENT_ID).expect("encode note")).expect("decode note") {
            Decoded::Note { event_id } => assert_eq!(event_id, EVENT_ID),
            other => panic!("expected Note, got {:?}", other),
        }
    }

    #[test]
    fn encode_nprofile_round_trips() {
        let relays = vec!["wss://relay.damus.io".to_string()];
        let nprofile = encode_nprofile(PUBKEY, &relays).expect("encode nprofile");
        assert!(nprofile.starts_with("nprofile1"));
        match decode(&nprofile).expect("decode nprofile") {
            Decoded::Nprofile { pubkey, relays: r } => {
                assert_eq!(pubkey, PUBKEY);
                assert_eq!(r, relays);
            }
            other => panic!("expected Nprofile, got {:?}", other),
        }
    }

    #[test]
    fn encode_nevent_id_only_round_trips() {
        let nevent = encode_nevent(EVENT_ID, &[], None, None).expect("encode nevent");
        match decode(&nevent).expect("decode nevent") {
            Decoded::Nevent {
                event_id,
                relays,
                author,
                kind_int,
            } => {
                assert_eq!(event_id, EVENT_ID);
                assert!(relays.is_empty());
                assert_eq!(author, None);
                assert_eq!(kind_int, None);
            }
            other => panic!("expected Nevent, got {:?}", other),
        }
    }

    #[test]
    fn encode_nevent_with_all_hints_round_trips() {
        let relays = vec!["wss://relay.damus.io".to_string()];
        let nevent =
            encode_nevent(EVENT_ID, &relays, Some(PUBKEY), Some(1)).expect("encode nevent");
        match decode(&nevent).expect("decode nevent") {
            Decoded::Nevent {
                event_id,
                relays: r,
                author,
                kind_int,
            } => {
                assert_eq!(event_id, EVENT_ID);
                assert_eq!(r, relays);
                assert_eq!(author.as_deref(), Some(PUBKEY));
                assert_eq!(kind_int, Some(1));
            }
            other => panic!("expected Nevent, got {:?}", other),
        }
    }

    #[test]
    fn encode_naddr_round_trips() {
        let naddr = encode_naddr(30040, PUBKEY, "my-publication", &[]).expect("encode naddr");
        assert!(naddr.starts_with("naddr1"));
        match decode(&naddr).expect("decode naddr") {
            Decoded::Naddr {
                kind_int,
                pubkey,
                d_tag,
                relays,
            } => {
                assert_eq!(kind_int, 30040);
                assert_eq!(pubkey, PUBKEY);
                assert_eq!(d_tag, "my-publication");
                assert!(relays.is_empty());
            }
            other => panic!("expected Naddr, got {:?}", other),
        }
    }

    #[test]
    fn naddr_from_a_tag_handles_colons_in_dtag() {
        // d-tags can contain colons; only the first two `:` split the coord.
        let a_tag = format!("30041:{PUBKEY}:chapter:1:intro");
        match decode(&naddr_from_a_tag(&a_tag, &[]).expect("encode")).expect("decode") {
            Decoded::Naddr {
                kind_int,
                pubkey,
                d_tag,
                ..
            } => {
                assert_eq!(kind_int, 30041);
                assert_eq!(pubkey, PUBKEY);
                assert_eq!(d_tag, "chapter:1:intro");
            }
            other => panic!("expected Naddr, got {:?}", other),
        }
    }

    #[test]
    fn naddr_from_a_tag_empty_dtag() {
        let a_tag = format!("30040:{PUBKEY}:");
        match decode(&naddr_from_a_tag(&a_tag, &[]).expect("encode")).expect("decode") {
            Decoded::Naddr { d_tag, .. } => assert_eq!(d_tag, ""),
            other => panic!("expected Naddr, got {:?}", other),
        }
    }

    #[test]
    fn encode_rejects_bad_hex() {
        assert!(matches!(
            encode_npub("not-hex"),
            Err(EncodeError::Hex { .. })
        ));
        assert!(matches!(
            encode_npub("abcd"),
            Err(EncodeError::BadLength { actual: 2, .. })
        ));
    }

    #[test]
    fn naddr_from_a_tag_rejects_malformed() {
        assert!(matches!(
            naddr_from_a_tag("not-a-coordinate", &[]),
            Err(EncodeError::BadCoordinate(_))
        ));
    }
}
