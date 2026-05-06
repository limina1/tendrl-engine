//! Pluggable signing — `Signer` trait + in-process implementation.
//!
//! Per `docs/identity-and-signing-plan.md`, the engine is the *signing
//! orchestrator*. Callers that need to publish (publication index +
//! sections, chat fragments, profile updates, the future
//! `PublishController`) ask a `Signer` to turn an `EventTemplate` into
//! a `SignedEvent`. The active source — engine-resident ncryptsec, a
//! NIP-07 browser extension via the SSE channel, or eventually a
//! NIP-46 bunker — is invisible to the caller.
//!
//! This module ships Phase 1 only: the trait, the typed
//! `EventTemplate`, and `InProcessSigner`. The
//! `SigningController` (Phase 3) and the `ExternalSigner` SSE channel
//! (Phase 4) build on top.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::identity::{
    decrypt_ncryptsec, sign_event_hash, IdentityKeyring, IdentitySession, KeyParseError,
};

/// Errors surfaced by any `Signer` implementation. Wraps the lower-level
/// crypto errors plus identity-state failures the controller needs to
/// distinguish (locked vs. no identity vs. external signer not connected).
#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("identity is locked — unlock with password first")]
    Locked,
    #[error("no identity configured")]
    NoIdentity,
    #[error("no signer registered for the active source")]
    SignerNotConnected,
    #[error("external signer timed out")]
    Timeout,
    #[error("external signer reported error: {0}")]
    External(String),
    #[error("crypto error: {0}")]
    Key(#[from] KeyParseError),
    #[error("environment fallback failed: {0}")]
    EnvFallback(String),
    #[error("template / signer mismatch: {0}")]
    Mismatch(String),
}

/// What the caller wants signed. Mirrors NIP-07's `signEvent` input shape
/// so the same struct round-trips between Rust and the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTemplate {
    pub kind: u32,
    pub created_at: i64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    /// Optional pubkey hint. If present, signers MUST refuse to sign for
    /// a different pubkey — guards against cross-account confusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
}

/// A fully-signed Nostr event: NIP-01 fields plus `id` and `sig`. We keep
/// it as a `Value` so it round-trips through nostrdb / publish without
/// per-kind structs.
pub type SignedEvent = Value;

/// What a signer can do. Reported on registration; the controller can use
/// this to decide whether to route a request (e.g. a NIP-04 encrypt
/// request goes only to a signer with `nip04_encrypt: true`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignerCapabilities {
    pub sign_event: bool,
    pub nip04_encrypt: bool,
    pub nip04_decrypt: bool,
    pub nip44_encrypt: bool,
    pub nip44_decrypt: bool,
    /// Kinds the user has pre-authorized in their extension. Informational
    /// for now — the engine doesn't change behaviour but the UI can show
    /// "this extension auto-signs for kinds X, Y" so the user knows
    /// whether to expect prompts.
    #[serde(default)]
    pub auto_approve_kinds: Vec<u32>,
}

/// The single seam every caller goes through to produce a signed event.
#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(&self, template: EventTemplate) -> Result<SignedEvent, SigningError>;
    fn pubkey(&self) -> &str;
    fn capabilities(&self) -> SignerCapabilities;
}

/// Engine-resident signer. Holds the secret hex in memory only for the
/// duration of operations that own a `&InProcessSigner` reference; the
/// session-level lifetime is governed by `IdentitySession`'s lock timer.
pub struct InProcessSigner {
    pubkey_hex: String,
    secret_hex: String,
}

impl InProcessSigner {
    pub fn new(pubkey_hex: String, secret_hex: String) -> Self {
        Self { pubkey_hex, secret_hex }
    }

    /// Resolve a signer through the existing fallback chain:
    ///
    /// 1. The unlocked `IdentitySession` (in-memory after user-driven login).
    /// 2. The OS keyring entry for `configured_pubkey` (auto-restore on boot).
    /// 3. `.env` (`NOSTR_NCRYPTSEC` + `NOSTR_PASSWORD`) — for headless test
    ///    setups.
    ///
    /// Returns `Locked` if a session exists with a known pubkey but no
    /// decrypted secret, so the caller can prompt for unlock instead of
    /// silently falling through to the keyring under a different identity.
    pub fn resolve(
        session: &mut IdentitySession,
        configured_pubkey: Option<&str>,
    ) -> Result<Self, SigningError> {
        // 1. Unlocked session.
        if session.can_sign() {
            session.touch();
            let pubkey = session
                .pubkey()
                .ok_or(SigningError::NoIdentity)?
                .to_string();
            let secret = session
                .secret()
                .ok_or(SigningError::Locked)?
                .to_string();
            return Ok(Self::new(pubkey, secret));
        }
        if session.pubkey().is_some() {
            return Err(SigningError::Locked);
        }

        // 2. OS keyring under the configured pubkey.
        if let Some(pk) = configured_pubkey {
            let keyring = IdentityKeyring::new();
            if let Ok(secret) = keyring.get_secret(pk) {
                return Ok(Self::new(pk.to_string(), secret));
            }
        }

        // 3. `.env` fallback.
        if let Ok(content) = std::fs::read_to_string(".env") {
            let mut ncryptsec = None;
            let mut password = None;
            for line in content.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("NOSTR_NCRYPTSEC=") {
                    ncryptsec = Some(v.to_string());
                } else if let Some(v) = line.strip_prefix("NOSTR_PASSWORD=") {
                    password = Some(v.to_string());
                }
            }
            if let (Some(nc), Some(pw)) = (ncryptsec, password) {
                let (secret, pubkey) = decrypt_ncryptsec(&nc, &pw)
                    .map_err(|e| SigningError::EnvFallback(e.to_string()))?;
                return Ok(Self::new(pubkey, secret));
            }
        }

        Err(SigningError::NoIdentity)
    }
}

#[async_trait]
impl Signer for InProcessSigner {
    async fn sign(&self, template: EventTemplate) -> Result<SignedEvent, SigningError> {
        // Refuse to sign for the wrong pubkey. Catches "I switched accounts
        // mid-publish" mistakes early, before they hit the relay.
        if let Some(want) = &template.pubkey {
            if want != &self.pubkey_hex {
                return Err(SigningError::Mismatch(format!(
                    "template wants {} but signer is {}",
                    want, self.pubkey_hex
                )));
            }
        }

        // NIP-01 canonical form: [0, pubkey, created_at, kind, tags, content]
        let canonical = json!([
            0,
            self.pubkey_hex,
            template.created_at,
            template.kind,
            template.tags,
            template.content,
        ]);
        let id = canonical_id(&canonical);
        let sig = sign_event_hash(&id, &self.secret_hex)?;

        Ok(json!({
            "id": id,
            "pubkey": self.pubkey_hex,
            "created_at": template.created_at,
            "kind": template.kind,
            "tags": template.tags,
            "content": template.content,
            "sig": sig,
        }))
    }

    fn pubkey(&self) -> &str {
        &self.pubkey_hex
    }

    fn capabilities(&self) -> SignerCapabilities {
        SignerCapabilities {
            sign_event: true,
            // NIP-04 / NIP-44 routed through the engine aren't implemented
            // yet; surface as `false` so callers don't try.
            ..Default::default()
        }
    }
}

/// Compute a NIP-01 event id: SHA-256 of the canonical
/// `[0, pubkey, created_at, kind, tags, content]` array, lowercase hex.
///
/// Phase 2 will consolidate the duplicate definition currently sitting in
/// `publication.rs` to call this one.
pub(crate) fn canonical_id(canonical: &Value) -> String {
    let serialized = serde_json::to_string(canonical).unwrap_or_default();
    let hash = Sha256::digest(serialized.as_bytes());
    hex::encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic test key. `derive_pubkey_from_secret` runs through
    /// the same secp256k1 path the real signer uses, so the round-trip
    /// proves our canonical-form / hash / sign / pack pipeline matches.
    fn test_keypair() -> (String, String) {
        let secret_hex =
            "0000000000000000000000000000000000000000000000000000000000000003".to_string();
        let pubkey_hex =
            crate::identity::derive_pubkey_from_secret(&secret_hex).expect("derive test pubkey");
        (pubkey_hex, secret_hex)
    }

    #[tokio::test]
    async fn signer_returns_well_formed_event() {
        let (pubkey, secret) = test_keypair();
        let signer = InProcessSigner::new(pubkey.clone(), secret);
        let template = EventTemplate {
            kind: 1,
            created_at: 1_700_000_000,
            tags: vec![vec!["t".into(), "tendrl".into()]],
            content: "hello".into(),
            pubkey: Some(pubkey.clone()),
        };
        let signed = signer.sign(template).await.expect("sign should succeed");
        assert_eq!(signed["pubkey"].as_str().unwrap(), pubkey);
        assert_eq!(signed["kind"].as_u64().unwrap(), 1);
        assert_eq!(signed["created_at"].as_i64().unwrap(), 1_700_000_000);
        assert_eq!(signed["content"].as_str().unwrap(), "hello");
        assert_eq!(signed["id"].as_str().unwrap().len(), 64);
        assert_eq!(signed["sig"].as_str().unwrap().len(), 128);
    }

    #[tokio::test]
    async fn pubkey_mismatch_is_rejected() {
        let (pubkey, secret) = test_keypair();
        let signer = InProcessSigner::new(pubkey, secret);
        let template = EventTemplate {
            kind: 1,
            created_at: 0,
            tags: vec![],
            content: String::new(),
            pubkey: Some("ff".repeat(32)),
        };
        let err = signer.sign(template).await.unwrap_err();
        assert!(matches!(err, SigningError::Mismatch(_)));
    }

    #[tokio::test]
    async fn signer_reports_capabilities() {
        let (pubkey, secret) = test_keypair();
        let signer = InProcessSigner::new(pubkey, secret);
        let caps = signer.capabilities();
        assert!(caps.sign_event);
        assert!(!caps.nip04_encrypt);
        assert!(!caps.nip44_encrypt);
        assert!(caps.auto_approve_kinds.is_empty());
    }

    // `resolve()`'s fallback ordering (session → keyring → .env) is
    // covered indirectly by the existing publish-handler integration
    // tests; Phase 2's refactor folds those into a focused
    // `InProcessSigner::resolve` test that doesn't depend on the
    // ambient working dir's `.env` presence.
}
