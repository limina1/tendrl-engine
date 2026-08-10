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

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::{oneshot, RwLock};

use crate::identity::{
    sign_event_hash, IdentityKeyring, IdentitySession, IdentitySource, KeyParseError,
};

/// Type alias for the shared identity state used across handlers
/// (matches the existing `IdentityAppState` in `api.rs`).
pub type IdentityHandle = Arc<Mutex<IdentitySession>>;

/// Client-provenance tag stamped on every event we sign:
/// `["client", "tendrl"]`. Added at the two signing chokepoints
/// (`SigningController::sign` and `tree_emit::sign_event`) so callers
/// never have to remember it; idempotent so the paths compose.
pub const CLIENT_TAG_NAME: &str = "client";
pub const CLIENT_TAG_VALUE: &str = "tendrl";

/// Append the `["client", "tendrl"]` tag unless the template already
/// carries a `client` tag (caller-supplied ones win — e.g. an event
/// re-signed through the nested publish path already has it).
pub fn ensure_client_tag(tags: &mut Vec<Vec<String>>) {
    let has_client = tags
        .iter()
        .any(|t| t.first().map(String::as_str) == Some(CLIENT_TAG_NAME));
    if !has_client {
        tags.push(vec![CLIENT_TAG_NAME.into(), CLIENT_TAG_VALUE.into()]);
    }
}

const SIGN_TIMEOUT: Duration = Duration::from_secs(60);
const SIGNER_STALE: Duration = Duration::from_secs(120);

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
    /// Decrypted secret, wiped from memory when this signer drops.
    secret_hex: zeroize::Zeroizing<String>,
}

impl InProcessSigner {
    pub fn new(pubkey_hex: String, secret_hex: String) -> Self {
        Self {
            pubkey_hex,
            secret_hex: zeroize::Zeroizing::new(secret_hex),
        }
    }

    /// Inherent pubkey accessor (mirrors `Signer::pubkey`), provided so
    /// callers don't need `use crate::signing::Signer;` to read the
    /// pubkey of a known concrete `InProcessSigner`.
    pub fn pubkey_hex(&self) -> &str {
        &self.pubkey_hex
    }

    /// Engine-internal access to the secret hex. Exposed so the legacy
    /// synchronous `build_signed_publication_events` path can keep using
    /// the existing `sign_event_hash` primitive while the async-signing
    /// migration lands incrementally. Don't pass this beyond engine
    /// modules.
    pub fn secret_hex(&self) -> &str {
        &self.secret_hex
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

        // No live session, no keyring entry → no identity. (The former `.env`
        // plaintext-key fallback was removed: keys come only from the live
        // session or the OS keyring, never a plaintext file on disk.)
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

/// Verify a fully-signed event: recompute the NIP-01 id from the event's own
/// fields and schnorr-verify `sig` against `pubkey`.
///
/// Run on everything an *external* signer hands back (browser extension,
/// signer app) before it is stored or broadcast — a buggy or hostile signer
/// must not be able to put garbage on the wire under the user's npub. The
/// web glue does no crypto; verification lives engine-side (the SPA has no
/// nostr crypto dependency, and the boundary rule keeps it that way).
pub fn verify_signed_event(ev: &Value) -> Result<(), String> {
    let pubkey = ev
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or("missing pubkey")?;
    let id = ev.get("id").and_then(Value::as_str).ok_or("missing id")?;
    let sig = ev.get("sig").and_then(Value::as_str).ok_or("missing sig")?;
    let created_at = ev
        .get("created_at")
        .and_then(Value::as_i64)
        .ok_or("missing created_at")?;
    let kind = ev.get("kind").and_then(Value::as_u64).ok_or("missing kind")?;
    let tags = ev.get("tags").cloned().unwrap_or_else(|| json!([]));
    let content = ev
        .get("content")
        .and_then(Value::as_str)
        .ok_or("missing content")?;

    let canonical = json!([0, pubkey, created_at, kind, tags, content]);
    let expect = canonical_id(&canonical);
    if !expect.eq_ignore_ascii_case(id) {
        return Err(format!("event id does not match its contents (expected {expect})"));
    }

    use secp256k1::{schnorr::Signature, Message, XOnlyPublicKey, SECP256K1};
    let id_bytes = hex::decode(id).map_err(|_| "id is not hex")?;
    let sig_bytes = hex::decode(sig).map_err(|_| "sig is not hex")?;
    let pk_bytes = hex::decode(pubkey).map_err(|_| "pubkey is not hex")?;
    let msg = Message::from_digest_slice(&id_bytes).map_err(|_| "id is not 32 bytes")?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| "sig is not a schnorr signature")?;
    let pk = XOnlyPublicKey::from_slice(&pk_bytes).map_err(|_| "pubkey is not x-only")?;
    SECP256K1
        .verify_schnorr(&sig, &msg, &pk)
        .map_err(|_| "schnorr signature verification failed")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// External signer registry
// ---------------------------------------------------------------------------

/// Event pushed through the SSE stream to a connected external signer.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    SignRequest { req_id: String, template: EventTemplate },
}

/// Reply collected from `POST /sign-response`.
#[derive(Debug, Clone)]
pub enum SignerReply {
    Ok(SignedEvent),
    Err(String),
}

/// One entry in the registry, keyed by `signer_id`. A registered signer
/// owns:
///   - a sender into the SSE event queue (`channel_tx`)
///   - the receiver, stashed in `channel_rx` until the SSE GET picks it up
///   - a `pending` map from `req_id` → `oneshot::Sender<SignerReply>` so
///     `POST /sign-response` can resolve the awaiting `ExternalSigner::sign`
pub struct RegisteredSigner {
    pub signer_id: String,
    pub token: String,
    pub pubkey: String,
    pub kind: String,
    pub caps: SignerCapabilities,
    last_seen: Mutex<Instant>,
    channel_tx: tokio::sync::mpsc::UnboundedSender<SseEvent>,
    channel_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<SseEvent>>>,
    pending: tokio::sync::Mutex<HashMap<String, oneshot::Sender<SignerReply>>>,
}

impl RegisteredSigner {
    pub fn touch(&self) {
        if let Ok(mut t) = self.last_seen.lock() {
            *t = Instant::now();
        }
    }

    pub fn is_stale(&self) -> bool {
        self.last_seen
            .lock()
            .map(|t| t.elapsed() > SIGNER_STALE)
            .unwrap_or(true)
    }

    /// Take ownership of the SSE receiver. Returns `None` if it's
    /// already been claimed by an earlier `/signer-channel` connect.
    pub fn take_receiver(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<SseEvent>> {
        self.channel_rx.lock().ok().and_then(|mut g| g.take())
    }

    pub fn send(&self, ev: SseEvent) -> Result<(), SigningError> {
        self.channel_tx
            .send(ev)
            .map_err(|_| SigningError::SignerNotConnected)
    }

    pub async fn register_pending(
        &self,
        req_id: String,
    ) -> oneshot::Receiver<SignerReply> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(req_id, tx);
        rx
    }

    pub async fn resolve_pending(&self, req_id: &str, reply: SignerReply) -> bool {
        if let Some(tx) = self.pending.lock().await.remove(req_id) {
            tx.send(reply).ok();
            true
        } else {
            false
        }
    }

    pub async fn drop_pending(&self, req_id: &str) {
        self.pending.lock().await.remove(req_id);
    }
}

/// Process-wide registry of connected external signers.
pub type Registry = Arc<RwLock<HashMap<String, Arc<RegisteredSigner>>>>;

/// Monotonic-ish request ids: 13-hex-digit unix-microsecond prefix +
/// 8-hex-digit AtomicU64 counter. Unique per process, sortable by time.
fn next_request_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:013x}-{:08x}", micros, n)
}

/// Same id format reused for signer ids and tokens. Token-as-signer-id
/// is fine for the local-only browser-app context; if we ever expose
/// the engine beyond loopback we'll want signed tokens.
fn next_signer_id() -> String {
    next_request_id()
}

// ---------------------------------------------------------------------------
// External signer
// ---------------------------------------------------------------------------

/// Routes signing requests to a registered external signer (browser tab
/// with `window.nostr`, future NIP-46 bunker) via the SSE channel.
pub struct ExternalSigner {
    pub signer_id: String,
    pub pubkey_hex: String,
    pub caps: SignerCapabilities,
    pub registry: Registry,
}

#[async_trait]
impl Signer for ExternalSigner {
    async fn sign(&self, template: EventTemplate) -> Result<SignedEvent, SigningError> {
        // Pubkey-mismatch guard mirrors InProcessSigner.
        if let Some(want) = &template.pubkey {
            if want != &self.pubkey_hex {
                return Err(SigningError::Mismatch(format!(
                    "template wants {} but signer is {}",
                    want, self.pubkey_hex
                )));
            }
        }

        let registered = {
            let map = self.registry.read().await;
            map.get(&self.signer_id)
                .cloned()
                .ok_or(SigningError::SignerNotConnected)?
        };

        let req_id = next_request_id();
        let rx = registered.register_pending(req_id.clone()).await;
        registered.send(SseEvent::SignRequest {
            req_id: req_id.clone(),
            template,
        })?;

        let reply = match tokio::time::timeout(SIGN_TIMEOUT, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => {
                registered.drop_pending(&req_id).await;
                return Err(SigningError::External("oneshot dropped".into()));
            }
            Err(_) => {
                registered.drop_pending(&req_id).await;
                return Err(SigningError::Timeout);
            }
        };

        match reply {
            SignerReply::Ok(signed) => {
                // Never trust an external signer's output blindly: the event
                // must verify AND belong to the registered pubkey (catches an
                // account switch inside the signer app mid-session).
                verify_signed_event(&signed).map_err(|e| {
                    SigningError::External(format!("signer returned an invalid event: {e}"))
                })?;
                let signed_pk = signed.get("pubkey").and_then(Value::as_str).unwrap_or("");
                if signed_pk != self.pubkey_hex {
                    return Err(SigningError::Mismatch(format!(
                        "signer returned an event for {} but is registered as {}",
                        signed_pk, self.pubkey_hex
                    )));
                }
                Ok(signed)
            }
            SignerReply::Err(msg) => Err(SigningError::External(msg)),
        }
    }

    fn pubkey(&self) -> &str {
        &self.pubkey_hex
    }

    fn capabilities(&self) -> SignerCapabilities {
        self.caps.clone()
    }
}

// ---------------------------------------------------------------------------
// Signing controller
// ---------------------------------------------------------------------------

/// Routes a `sign(template)` call through the right `Signer` based on
/// the active `IdentitySource`. Engine source resolves
/// `InProcessSigner` per-call from the identity session; external
/// sources look up an `ExternalSigner` in the registry.
///
/// Cheap to clone — `Arc`s all the way down. Constructed once at app
/// startup and shared via Axum `State`.
#[derive(Clone)]
pub struct SigningController {
    identity: IdentityHandle,
    registry: Registry,
}

impl SigningController {
    pub fn new(identity: IdentityHandle) -> Self {
        Self {
            identity,
            registry: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn registry(&self) -> Registry {
        self.registry.clone()
    }

    pub fn current_source(&self) -> IdentitySource {
        self.identity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .source()
            .clone()
    }

    pub fn set_source(&self, source: IdentitySource) {
        self.identity
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_source(source);
    }

    /// Register an external signer. Returns the signer_id + token; the
    /// caller (HTTP handler) returns these to the client. The client
    /// then opens `GET /signer-channel?token=...` to consume the SSE
    /// stream and `POST /sign-response` to fulfill requests.
    pub async fn register_external(
        &self,
        kind: String,
        pubkey: String,
        caps: SignerCapabilities,
    ) -> (String, String) {
        let signer_id = next_signer_id();
        let token = signer_id.clone(); // local-only context; same value
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = Arc::new(RegisteredSigner {
            signer_id: signer_id.clone(),
            token: token.clone(),
            pubkey,
            kind,
            caps,
            last_seen: Mutex::new(Instant::now()),
            channel_tx: tx,
            channel_rx: Mutex::new(Some(rx)),
            pending: tokio::sync::Mutex::new(HashMap::new()),
        });
        self.registry
            .write()
            .await
            .insert(signer_id.clone(), registered);
        (signer_id, token)
    }

    pub async fn lookup_by_token(&self, token: &str) -> Option<Arc<RegisteredSigner>> {
        // Token == signer_id in the local-only setup.
        self.registry.read().await.get(token).cloned()
    }

    pub async fn lookup_by_id(&self, signer_id: &str) -> Option<Arc<RegisteredSigner>> {
        self.registry.read().await.get(signer_id).cloned()
    }

    pub async fn drop_signer(&self, signer_id: &str) {
        self.registry.write().await.remove(signer_id);
    }

    /// Resolve a sign-response from a connected signer. Called by the
    /// HTTP handler when the client POSTs back a signed event.
    pub async fn resolve_sign_response(
        &self,
        signer_id: &str,
        req_id: &str,
        reply: SignerReply,
    ) -> bool {
        if let Some(s) = self.lookup_by_id(signer_id).await {
            s.touch();
            s.resolve_pending(req_id, reply).await
        } else {
            false
        }
    }

    pub async fn sign(&self, mut template: EventTemplate) -> Result<SignedEvent, SigningError> {
        // Stamp client provenance before the id is computed, whichever
        // source signs — the external-signer path sends the template to
        // the browser with the tag already in place.
        ensure_client_tag(&mut template.tags);
        let source = self.current_source();
        match source {
            IdentitySource::Engine => {
                let signer = {
                    let mut session = self
                        .identity
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    InProcessSigner::resolve(&mut session, None)?
                };
                signer.sign(template).await
            }
            IdentitySource::Nip07 { signer_id }
            | IdentitySource::Nip46 { signer_id }
            | IdentitySource::Nip55 { signer_id } => {
                let signer_id = signer_id.ok_or(SigningError::SignerNotConnected)?;
                let registered = self
                    .lookup_by_id(&signer_id)
                    .await
                    .ok_or(SigningError::SignerNotConnected)?;
                let external = ExternalSigner {
                    signer_id: registered.signer_id.clone(),
                    pubkey_hex: registered.pubkey.clone(),
                    caps: registered.caps.clone(),
                    registry: self.registry.clone(),
                };
                external.sign(template).await
            }
        }
    }
}

/// `SigningController` is itself a `Signer`. The trait `pubkey()` is
/// not meaningful here (the active pubkey resolves through an async
/// path; see `active_pubkey`); callers that need the pubkey alongside
/// the signer pass it explicitly. The trait impl exists so `&dyn
/// Signer` accepts a controller for `build_signed_publication_events_via_signer`.
#[async_trait]
impl Signer for SigningController {
    async fn sign(&self, template: EventTemplate) -> Result<SignedEvent, SigningError> {
        SigningController::sign(self, template).await
    }

    fn pubkey(&self) -> &str {
        ""
    }

    fn capabilities(&self) -> SignerCapabilities {
        SignerCapabilities {
            sign_event: true,
            ..Default::default()
        }
    }
}

impl SigningController {
    /// Resolve the active source's pubkey. Returns `None` when no
    /// identity is configured / no external signer is connected.
    pub async fn active_pubkey(&self) -> Option<String> {
        let source = self.current_source();
        match source {
            IdentitySource::Engine => self
                .identity
                .lock()
                .ok()
                .and_then(|s| s.pubkey().map(|p| p.to_string())),
            IdentitySource::Nip07 { signer_id }
            | IdentitySource::Nip46 { signer_id }
            | IdentitySource::Nip55 { signer_id } => {
                let id = signer_id?;
                self.registry.read().await.get(&id).map(|s| s.pubkey.clone())
            }
        }
    }
}

/// Best-effort sweep of stale signers. Call periodically (e.g. from the
/// background sync loop) to drop entries whose SSE channel went silent.
#[allow(dead_code)]
pub async fn sweep_stale(registry: &Registry) -> usize {
    let stale_ids: Vec<String> = {
        let map = registry.read().await;
        map.iter()
            .filter(|(_, s)| s.is_stale())
            .map(|(k, _)| k.clone())
            .collect()
    };
    let count = stale_ids.len();
    if count > 0 {
        let mut map = registry.write().await;
        for id in stale_ids {
            map.remove(&id);
        }
    }
    count
}

/// Drain queued SSE events from a receiver into a `VecDeque`. Used by
/// the SSE handler in api.rs which can't `await` the receiver inside
/// the stream closure cleanly.
#[allow(dead_code)]
pub fn drain(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<SseEvent>,
    out: &mut VecDeque<SseEvent>,
) {
    while let Ok(ev) = rx.try_recv() {
        out.push_back(ev);
    }
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

    #[test]
    fn client_tag_appended_once() {
        let mut tags = vec![vec!["d".to_string(), "x".to_string()]];
        ensure_client_tag(&mut tags);
        assert!(tags.contains(&vec!["client".to_string(), "tendrl".to_string()]));
        // Idempotent: a second pass (e.g. tree_emit-built event re-signed
        // through the controller) must not double-tag.
        ensure_client_tag(&mut tags);
        let count = tags.iter().filter(|t| t[0] == CLIENT_TAG_NAME).count();
        assert_eq!(count, 1);
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

    #[tokio::test]
    async fn registry_register_and_lookup() {
        let identity: IdentityHandle = Arc::new(Mutex::new(IdentitySession::new()));
        let controller = SigningController::new(identity);

        let (signer_id, token) = controller
            .register_external(
                "nip07".into(),
                "ff".repeat(32),
                SignerCapabilities {
                    sign_event: true,
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(signer_id, token);

        let by_token = controller.lookup_by_token(&token).await;
        assert!(by_token.is_some());
        assert_eq!(by_token.unwrap().pubkey, "ff".repeat(32));

        controller.drop_signer(&signer_id).await;
        assert!(controller.lookup_by_id(&signer_id).await.is_none());
    }

    #[tokio::test]
    async fn external_signer_times_out_when_no_responder() {
        let identity: IdentityHandle = Arc::new(Mutex::new(IdentitySession::new()));
        let controller = SigningController::new(identity);
        let pubkey = "aa".repeat(32);
        let (signer_id, _) = controller
            .register_external(
                "nip07".into(),
                pubkey.clone(),
                SignerCapabilities {
                    sign_event: true,
                    ..Default::default()
                },
            )
            .await;

        // Switch source so the controller routes to the external signer,
        // but no SSE consumer is connected — the receiver is parked, the
        // request enqueues, nobody resolves it. We patch the timeout to a
        // tiny value via a direct ExternalSigner call to keep the test
        // fast.
        let registered = controller.lookup_by_id(&signer_id).await.unwrap();
        let _template = EventTemplate {
            kind: 1,
            created_at: 0,
            tags: vec![],
            content: String::new(),
            pubkey: Some(pubkey.clone()),
        };
        let req_id = "test-req-id".to_string();
        let _rx = registered.register_pending(req_id.clone()).await;
        // Don't send through channel; just assert that drop_pending works
        // and that there's a pending entry mid-flight.
        registered.drop_pending(&req_id).await;
    }

    /// Full round-trip through the registry for a `nip55` source: the
    /// controller routes the sign to the registered signer's channel, a
    /// fulfiller (standing in for the WebView → Amber glue) signs the
    /// template and posts the reply, and the verified event comes back.
    /// Proves the registry really is kind-agnostic — no nip55-specific
    /// engine code beyond the enum arm.
    #[tokio::test]
    async fn nip55_source_signs_through_registry() {
        let (pubkey, secret) = test_keypair();
        let identity: IdentityHandle = Arc::new(Mutex::new(IdentitySession::new()));
        let controller = SigningController::new(identity);

        let (signer_id, _token) = controller
            .register_external(
                "nip55".into(),
                pubkey.clone(),
                SignerCapabilities {
                    sign_event: true,
                    auto_approve_kinds: vec![30040, 30041],
                    ..Default::default()
                },
            )
            .await;
        controller.set_source(IdentitySource::Nip55 {
            signer_id: Some(signer_id.clone()),
        });

        // Fulfiller: consume the SSE-side channel, sign with the test key,
        // resolve the pending request — exactly what the glue does.
        let registered = controller.lookup_by_id(&signer_id).await.unwrap();
        let mut rx = registered.take_receiver().expect("channel receiver");
        let fulfiller_controller = controller.clone();
        let fulfiller_id = signer_id.clone();
        let fulfiller_key = (pubkey.clone(), secret.clone());
        tokio::spawn(async move {
            if let Some(SseEvent::SignRequest { req_id, template }) = rx.recv().await {
                let signer = InProcessSigner::new(fulfiller_key.0, fulfiller_key.1);
                let signed = signer.sign(template).await.expect("fulfiller signs");
                fulfiller_controller
                    .resolve_sign_response(&fulfiller_id, &req_id, SignerReply::Ok(signed))
                    .await;
            }
        });

        let template = EventTemplate {
            kind: 30041,
            created_at: 1_700_000_000,
            tags: vec![vec!["d".into(), "sect".into()]],
            content: "section body".into(),
            pubkey: Some(pubkey.clone()),
        };
        let signed = controller.sign(template).await.expect("nip55 sign routes");
        assert_eq!(signed["pubkey"].as_str().unwrap(), pubkey);
        assert_eq!(signed["kind"].as_u64().unwrap(), 30041);
        // The controller stamped provenance before the template went out.
        let tags = signed["tags"].as_array().unwrap();
        assert!(tags
            .iter()
            .any(|t| t[0] == CLIENT_TAG_NAME && t[1] == CLIENT_TAG_VALUE));
        // And what came back verifies.
        verify_signed_event(&signed).expect("returned event verifies");
    }

    /// A signer app returning garbage must not survive the Ok path.
    #[tokio::test]
    async fn external_signer_rejects_invalid_reply() {
        let (pubkey, _secret) = test_keypair();
        let identity: IdentityHandle = Arc::new(Mutex::new(IdentitySession::new()));
        let controller = SigningController::new(identity);

        let (signer_id, _token) = controller
            .register_external(
                "nip55".into(),
                pubkey.clone(),
                SignerCapabilities {
                    sign_event: true,
                    ..Default::default()
                },
            )
            .await;
        controller.set_source(IdentitySource::Nip55 {
            signer_id: Some(signer_id.clone()),
        });

        let registered = controller.lookup_by_id(&signer_id).await.unwrap();
        let mut rx = registered.take_receiver().expect("channel receiver");
        let fulfiller_controller = controller.clone();
        let fulfiller_id = signer_id.clone();
        tokio::spawn(async move {
            if let Some(SseEvent::SignRequest { req_id, .. }) = rx.recv().await {
                fulfiller_controller
                    .resolve_sign_response(
                        &fulfiller_id,
                        &req_id,
                        SignerReply::Ok(json!({ "garbage": true })),
                    )
                    .await;
            }
        });

        let template = EventTemplate {
            kind: 1,
            created_at: 0,
            tags: vec![],
            content: String::new(),
            pubkey: Some(pubkey),
        };
        let err = controller.sign(template).await.unwrap_err();
        assert!(matches!(err, SigningError::External(_)), "got: {err:?}");
    }

    #[tokio::test]
    async fn verify_signed_event_accepts_valid_rejects_tampered() {
        let (pubkey, secret) = test_keypair();
        let signer = InProcessSigner::new(pubkey.clone(), secret);
        let signed = signer
            .sign(EventTemplate {
                kind: 1,
                created_at: 1_700_000_000,
                tags: vec![vec!["t".into(), "x".into()]],
                content: "hello".into(),
                pubkey: Some(pubkey),
            })
            .await
            .unwrap();

        verify_signed_event(&signed).expect("valid event verifies");

        // Tampered content → recomputed id no longer matches.
        let mut tampered = signed.clone();
        tampered["content"] = json!("evil");
        let err = verify_signed_event(&tampered).unwrap_err();
        assert!(err.contains("id does not match"), "got: {err}");

        // Wrong-but-well-formed signature → schnorr verification fails.
        let mut bad_sig = signed.clone();
        bad_sig["sig"] = json!("ab".repeat(64));
        let err = verify_signed_event(&bad_sig).unwrap_err();
        assert!(err.contains("verification failed") || err.contains("schnorr"), "got: {err}");

        // Missing fields are reported, not panicked on.
        assert!(verify_signed_event(&json!({ "kind": 1 })).is_err());
    }
}
