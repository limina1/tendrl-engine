//! NIP-46 (Nostr Connect / "bunker") — the pure protocol half: `bunker://`
//! URI parsing, the JSON-RPC request/response shapes, and the kind-24133
//! envelope (NIP-44 encrypted, `p`-tagged to the remote signer). No IO;
//! the relay transport and the `Signer` impl live in `bunker.rs`.
//!
//! Spec: <https://github.com/nostr-protocol/nips/blob/master/46.md>. Only the
//! signer-initiated `bunker://` flow is implemented (the one nsecbunker,
//! Amber and `nak bunker` hand out); requests are NIP-44 v2 only.

use crate::nip44::{self, ConversationKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Nostr Connect request/response event kind.
pub const KIND: u32 = 24133;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Nip46Error {
    #[error("not a bunker:// URI")]
    NotBunkerUri,
    #[error("bunker URI: {0}")]
    Uri(String),
    #[error("nip44: {0}")]
    Crypto(String),
    #[error("malformed response: {0}")]
    Response(String),
}

impl From<nip44::Nip44Error> for Nip46Error {
    fn from(e: nip44::Nip44Error) -> Self {
        Nip46Error::Crypto(e.to_string())
    }
}

/// `bunker://<remote-signer-pubkey>?relay=…&relay=…&secret=…`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BunkerUri {
    /// x-only hex pubkey of the remote signer (NOT necessarily the user's key).
    pub remote_pubkey: String,
    pub relays: Vec<String>,
    /// Optional connect secret the signer minted for this pairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

impl BunkerUri {
    pub fn parse(input: &str) -> Result<Self, Nip46Error> {
        let s = input.trim();
        let rest = s
            .strip_prefix("bunker://")
            .ok_or(Nip46Error::NotBunkerUri)?;
        let (pubkey, query) = match rest.split_once('?') {
            Some((p, q)) => (p, q),
            None => (rest, ""),
        };
        let pubkey = pubkey.trim_end_matches('/').to_ascii_lowercase();
        if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Nip46Error::Uri(
                "remote signer pubkey must be 64 hex characters".into(),
            ));
        }
        let mut relays = Vec::new();
        let mut secret = None;
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let v = percent_decode(v);
            match k {
                "relay" => {
                    let url = crate::relay_url::normalize_relay_url(&v);
                    let url = if url.is_empty() { v.clone() } else { url };
                    if !relays.contains(&url) {
                        relays.push(url);
                    }
                }
                "secret" => secret = Some(v),
                _ => {} // `name`, `url`, `image` and unknown params are informational
            }
        }
        if relays.is_empty() {
            return Err(Nip46Error::Uri("at least one relay= is required".into()));
        }
        Ok(Self {
            remote_pubkey: pubkey,
            relays,
            secret,
        })
    }

    /// The URI with the secret elided — safe for logs and status.
    pub fn redacted(&self) -> String {
        let relays = self
            .relays
            .iter()
            .map(|r| format!("relay={r}"))
            .collect::<Vec<_>>()
            .join("&");
        format!("bunker://{}?{}", self.remote_pubkey, relays)
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                bytes.get(i + 2).and_then(|c| (*c as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One NIP-46 request. `params` are strings per spec (event JSON travels as
/// a string inside the array).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Vec<String>,
}

impl Request {
    pub fn new(method: &str, params: Vec<String>) -> Self {
        Self {
            id: nanoid::nanoid!(12),
            method: method.into(),
            params,
        }
    }

    /// `connect(<remote-signer-pubkey>, [secret], [permissions])`.
    pub fn connect(remote_pubkey: &str, secret: Option<&str>, permissions: Option<&str>) -> Self {
        let mut params = vec![remote_pubkey.to_string()];
        if secret.is_some() || permissions.is_some() {
            params.push(secret.unwrap_or("").to_string());
        }
        if let Some(p) = permissions {
            params.push(p.to_string());
        }
        Self::new("connect", params)
    }

    pub fn get_public_key() -> Self {
        Self::new("get_public_key", vec![])
    }

    pub fn ping() -> Self {
        Self::new("ping", vec![])
    }

    /// `sign_event(<unsigned event JSON>)` — the event carries `kind`,
    /// `content`, `tags`, `created_at`; the signer fills `pubkey`, `id`, `sig`.
    pub fn sign_event(unsigned_event_json: String) -> Self {
        Self::new("sign_event", vec![unsigned_event_json])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What a response means once the `auth_url` convention is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Result(String),
    /// The signer wants the user to approve in a browser first; retry after.
    AuthUrl(String),
    Error(String),
}

impl Response {
    pub fn outcome(self) -> Outcome {
        match (self.result.as_deref(), self.error) {
            (Some("auth_url"), Some(url)) => Outcome::AuthUrl(url),
            (_, Some(err)) if !err.is_empty() => Outcome::Error(err),
            (Some(r), _) => Outcome::Result(r.to_string()),
            (None, _) => Outcome::Error("empty response".into()),
        }
    }
}

/// The unsigned kind-24133 envelope for a request: encrypted to the remote
/// signer, `p`-tagged to it. The caller signs it with the *transport* key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

pub fn request_envelope(
    conv: &ConversationKey,
    remote_pubkey: &str,
    req: &Request,
) -> Result<Envelope, Nip46Error> {
    let plain = serde_json::to_string(req).map_err(|e| Nip46Error::Response(e.to_string()))?;
    Ok(Envelope {
        kind: KIND,
        tags: vec![vec!["p".into(), remote_pubkey.into()]],
        content: nip44::encrypt(conv, &plain)?,
    })
}

/// Decrypt and parse a kind-24133 response event from the remote signer.
pub fn decode_response(conv: &ConversationKey, event: &Value) -> Result<Response, Nip46Error> {
    let content = event
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| Nip46Error::Response("no content".into()))?;
    let plain = nip44::decrypt(conv, content)?;
    serde_json::from_str(&plain).map_err(|e| Nip46Error::Response(e.to_string()))
}

/// REQ filter for the signer's replies to our transport key.
pub fn response_filter(client_pubkey: &str, remote_pubkey: &str, since: u64) -> Value {
    json!({
        "kinds": [KIND],
        "#p": [client_pubkey],
        "authors": [remote_pubkey],
        "since": since,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nip44::{conversation_key, pubkey_for_secret};

    const CLIENT_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const REMOTE_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    #[test]
    fn parses_bunker_uri_with_encoded_relays_and_secret() {
        let pk = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
        let uri = BunkerUri::parse(&format!(
            "bunker://{pk}?relay=wss%3A%2F%2Frelay.example.com&relay=wss://nos.lol/&secret=abc123&name=nak"
        ))
        .unwrap();
        assert_eq!(uri.remote_pubkey, pk);
        assert_eq!(uri.relays, vec!["wss://relay.example.com", "wss://nos.lol"]);
        assert_eq!(uri.secret.as_deref(), Some("abc123"));
        assert!(!uri.redacted().contains("abc123"));
        assert_eq!(
            BunkerUri::parse("nostrconnect://x").unwrap_err(),
            Nip46Error::NotBunkerUri
        );
        assert!(matches!(
            BunkerUri::parse("bunker://nothex?relay=wss://a"),
            Err(Nip46Error::Uri(_))
        ));
        assert!(matches!(
            BunkerUri::parse(&format!("bunker://{pk}")),
            Err(Nip46Error::Uri(_))
        ));
    }

    #[test]
    fn request_shapes_follow_the_spec() {
        let c = Request::connect("ab", Some("s3cret"), None);
        assert_eq!(c.method, "connect");
        assert_eq!(c.params, vec!["ab", "s3cret"]);
        let v: Value = serde_json::to_value(&c).unwrap();
        assert!(v["id"].as_str().unwrap().len() >= 8);
        assert_eq!(Request::get_public_key().params.len(), 0);
        let s = Request::sign_event("{\"kind\":1}".into());
        assert_eq!(s.params, vec!["{\"kind\":1}"]);
    }

    #[test]
    fn envelope_round_trips_between_client_and_remote() {
        let client_pub = pubkey_for_secret(CLIENT_SEC).unwrap();
        let remote_pub = pubkey_for_secret(REMOTE_SEC).unwrap();
        let client_conv = conversation_key(CLIENT_SEC, &remote_pub).unwrap();
        let remote_conv = conversation_key(REMOTE_SEC, &client_pub).unwrap();

        let req = Request::get_public_key();
        let env = request_envelope(&client_conv, &remote_pub, &req).unwrap();
        assert_eq!(env.kind, KIND);
        assert_eq!(env.tags, vec![vec!["p".to_string(), remote_pub.clone()]]);
        // The remote decrypts the request with its own conversation key…
        let plain = nip44::decrypt(&remote_conv, &env.content).unwrap();
        let got: Request = serde_json::from_str(&plain).unwrap();
        assert_eq!(got, req);
        // …and answers; the client decodes the reply event.
        let reply = Response {
            id: req.id.clone(),
            result: Some(client_pub.clone()),
            error: None,
        };
        let ev = json!({ "kind": KIND, "content": nip44::encrypt(&remote_conv, &serde_json::to_string(&reply).unwrap()).unwrap() });
        let decoded = decode_response(&client_conv, &ev).unwrap();
        assert_eq!(decoded.outcome(), Outcome::Result(client_pub));
    }

    #[test]
    fn outcomes() {
        assert_eq!(
            Response {
                id: "1".into(),
                result: Some("auth_url".into()),
                error: Some("https://x".into())
            }
            .outcome(),
            Outcome::AuthUrl("https://x".into())
        );
        assert_eq!(
            Response {
                id: "1".into(),
                result: None,
                error: Some("denied".into())
            }
            .outcome(),
            Outcome::Error("denied".into())
        );
        assert_eq!(
            Response {
                id: "1".into(),
                result: Some("ack".into()),
                error: Some("".into())
            }
            .outcome(),
            Outcome::Result("ack".into())
        );
        let f = response_filter("aa", "bb", 5);
        assert_eq!(f["kinds"][0], KIND);
        assert_eq!(f["#p"][0], "aa");
        assert_eq!(f["authors"][0], "bb");
    }
}
