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
}
