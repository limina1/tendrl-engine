//! NIP-42 relay authentication — pure protocol helpers (`nips/42.md`).
//!
//! No IO here. The relay client (`relay.rs`) drives the handshake; this
//! module owns the three things that are pure functions of the wire:
//!
//! - [`auth_template`]: the ephemeral kind-22242 event a client signs in
//!   answer to a relay's challenge. It proves control of a pubkey to
//!   *one* relay for *one* connection — never stored, never rebroadcast,
//!   never ingested (relays MUST NOT serve it either).
//! - [`RelayMessage`]: typed parse of the inbound frames the client
//!   loops on (`AUTH`, `CLOSED`, `OK`, `EVENT`, `EOSE`, `NOTICE`).
//! - [`Refusal`]: the NIP-01 machine-readable prefixes NIP-42 defines
//!   for `OK`/`CLOSED` reasons — `auth-required:` (authenticate, then
//!   retry) and `restricted:` (authenticated but not permitted — terminal).
//!
//! Policy (which relays tendrl will authenticate to, and as whom) lives
//! in `relay::RelayAuth` / `signing::AuthSigner`; the design is in
//! `docs/nip42-relay-auth-plan.org`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::signing::EventTemplate;

/// The ephemeral authentication event kind.
pub const KIND_AUTH: u32 = 22242;

/// Build the kind-22242 template for a `(relay, challenge)` pair.
///
/// `relay_url` should be the URL as connected (normalized) — relays
/// check at least the domain. `pubkey` is the optional signer hint
/// (`None` when the active source resolves it, e.g. the controller).
pub fn auth_template(
    relay_url: &str,
    challenge: &str,
    pubkey: Option<String>,
    now: i64,
) -> EventTemplate {
    EventTemplate {
        kind: KIND_AUTH,
        created_at: now,
        tags: vec![
            vec!["relay".to_string(), relay_url.to_string()],
            vec!["challenge".to_string(), challenge.to_string()],
        ],
        content: String::new(),
        pubkey,
    }
}

/// Inbound relay → client frames the client loop cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayMessage {
    /// `["AUTH", <challenge>]` — the relay offers (or demands) auth.
    Auth { challenge: String },
    /// `["CLOSED", <sub_id>, <reason>]` — subscription refused/ended.
    Closed { sub_id: String, reason: String },
    /// `["OK", <event_id>, <accepted>, <message>]` — reply to `EVENT`
    /// and to a client `AUTH`.
    Ok {
        id: String,
        accepted: bool,
        message: String,
    },
    /// `["EVENT", <sub_id>, <event>]`.
    Event { sub_id: String, event: Value },
    /// `["EOSE", <sub_id>]`.
    Eose { sub_id: String },
    /// `["NOTICE", <message>]`.
    Notice { message: String },
    /// Anything else (unknown verb, malformed array of a known verb).
    Other,
}

impl RelayMessage {
    /// Parse one text frame. Returns `None` only when the text is not a
    /// JSON array with a string verb — a malformed *known* verb parses
    /// as [`RelayMessage::Other`] so the loop keeps going.
    pub fn parse(text: &str) -> Option<Self> {
        let msg: Vec<Value> = serde_json::from_str(text).ok()?;
        let verb = msg.first()?.as_str()?;
        let s = |i: usize| msg.get(i).and_then(Value::as_str).map(str::to_string);
        Some(match verb {
            "AUTH" => match s(1) {
                Some(challenge) => Self::Auth { challenge },
                None => Self::Other,
            },
            "CLOSED" => match s(1) {
                Some(sub_id) => Self::Closed {
                    sub_id,
                    reason: s(2).unwrap_or_default(),
                },
                None => Self::Other,
            },
            "OK" => match s(1) {
                Some(id) => Self::Ok {
                    id,
                    accepted: msg.get(2).and_then(Value::as_bool).unwrap_or(false),
                    message: s(3).unwrap_or_default(),
                },
                None => Self::Other,
            },
            "EVENT" => match (s(1), msg.get(2)) {
                (Some(sub_id), Some(event)) => Self::Event {
                    sub_id,
                    event: event.clone(),
                },
                _ => Self::Other,
            },
            "EOSE" => match s(1) {
                Some(sub_id) => Self::Eose { sub_id },
                None => Self::Other,
            },
            "NOTICE" => Self::Notice {
                message: s(1).unwrap_or_default(),
            },
            _ => Self::Other,
        })
    }
}

/// Classification of an `OK false` / `CLOSED` reason by its NIP-01
/// machine-readable prefix, restricted to the two NIP-42 defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// `auth-required:` — no AUTH performed yet; authenticate and retry.
    AuthRequired,
    /// `restricted:` — authenticated, but this key is not allowed.
    /// Terminal: retrying with the same key cannot help.
    Restricted,
    /// Any other reason (`invalid:`, `blocked:`, `rate-limited:`, free text).
    Other,
}

impl Refusal {
    pub fn classify(reason: &str) -> Self {
        let r = reason.trim_start();
        if has_prefix(r, "auth-required:") {
            Self::AuthRequired
        } else if has_prefix(r, "restricted:") {
            Self::Restricted
        } else {
            Self::Other
        }
    }
}

fn has_prefix(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_template_has_kind_and_both_tags() {
        let t = auth_template("wss://relay.example", "abc123", None, 1_700_000_000);
        assert_eq!(t.kind, KIND_AUTH);
        assert_eq!(t.created_at, 1_700_000_000);
        assert_eq!(t.content, "");
        assert_eq!(t.tags.len(), 2);
        assert_eq!(t.tags[0], vec!["relay", "wss://relay.example"]);
        assert_eq!(t.tags[1], vec!["challenge", "abc123"]);
        assert!(t.pubkey.is_none());
    }

    #[test]
    fn parses_auth_challenge() {
        let m = RelayMessage::parse(r#"["AUTH","chal-1"]"#).unwrap();
        assert_eq!(
            m,
            RelayMessage::Auth {
                challenge: "chal-1".into()
            }
        );
    }

    #[test]
    fn parses_closed_ok_event_eose_notice() {
        assert_eq!(
            RelayMessage::parse(r#"["CLOSED","sub","auth-required: nope"]"#).unwrap(),
            RelayMessage::Closed {
                sub_id: "sub".into(),
                reason: "auth-required: nope".into()
            }
        );
        assert_eq!(
            RelayMessage::parse(r#"["OK","id1",false,"restricted: not a member"]"#).unwrap(),
            RelayMessage::Ok {
                id: "id1".into(),
                accepted: false,
                message: "restricted: not a member".into()
            }
        );
        assert_eq!(
            RelayMessage::parse(r#"["OK","id2",true]"#).unwrap(),
            RelayMessage::Ok {
                id: "id2".into(),
                accepted: true,
                message: String::new()
            }
        );
        assert_eq!(
            RelayMessage::parse(r#"["EVENT","sub",{"id":"x"}]"#).unwrap(),
            RelayMessage::Event {
                sub_id: "sub".into(),
                event: serde_json::json!({"id":"x"})
            }
        );
        assert_eq!(
            RelayMessage::parse(r#"["EOSE","sub"]"#).unwrap(),
            RelayMessage::Eose {
                sub_id: "sub".into()
            }
        );
        assert_eq!(
            RelayMessage::parse(r#"["NOTICE","hi"]"#).unwrap(),
            RelayMessage::Notice {
                message: "hi".into()
            }
        );
    }

    #[test]
    fn malformed_and_unknown_frames() {
        assert!(RelayMessage::parse("not json").is_none());
        assert!(RelayMessage::parse(r#"{"a":1}"#).is_none());
        assert!(RelayMessage::parse("[]").is_none());
        assert_eq!(RelayMessage::parse(r#"["COUNT","s",{}]"#).unwrap(), RelayMessage::Other);
        // Known verb, missing payload → Other, not None.
        assert_eq!(RelayMessage::parse(r#"["AUTH"]"#).unwrap(), RelayMessage::Other);
        assert_eq!(RelayMessage::parse(r#"["EVENT","sub"]"#).unwrap(), RelayMessage::Other);
    }

    #[test]
    fn classifies_prefixes() {
        assert_eq!(
            Refusal::classify("auth-required: we only serve members"),
            Refusal::AuthRequired
        );
        assert_eq!(Refusal::classify("AUTH-REQUIRED: x"), Refusal::AuthRequired);
        assert_eq!(Refusal::classify("restricted: not you"), Refusal::Restricted);
        assert_eq!(Refusal::classify("invalid: bad sig"), Refusal::Other);
        assert_eq!(Refusal::classify(""), Refusal::Other);
        // Prefix must be the whole word — `auth-required` without the
        // colon is free text.
        assert_eq!(Refusal::classify("auth-required soon"), Refusal::Other);
    }
}
