//! NIP-46 bunker client — the engine as a *client* of a remote signer.
//!
//! The user's key lives elsewhere (nsecbunker, Amber, `nak bunker`); this
//! module speaks the protocol from `nip46.rs` over relays and exposes the
//! result as a [`Signer`], so every caller — publish, comments, NIP-42 AUTH
//! — goes through the same `SigningController` chokepoint unchanged.
//!
//! Shape: a [`BunkerTransport`] moves our signed kind-24133 requests to the
//! signer and feeds its replies back; [`BunkerSigner`] owns the transport
//! keypair, the NIP-44 conversation key, and the pending-request table. The
//! relay transport keeps one socket per relay in the URI with a reconnect
//! loop; the tests drive the same signer through an in-memory responder.
//! Decision: `docs/insertions-plan.org` D10.

use crate::nip44::{self, ConversationKey};
use crate::nip46::{self, BunkerUri, Outcome, Request, Response};
use crate::signing::{
    verify_signed_event, EventTemplate, InProcessSigner, SignedEvent, Signer, SignerCapabilities,
    SigningError,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};

/// Per-request wait for the signer's reply. Amber prompts on the phone for
/// each signature until "always allow", so this is a human-scale wait.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// `connect` / `get_public_key` wait: the first pairing needs the user to
/// find the phone and approve.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

/// How a signer reaches its remote: `run` owns the connection(s) for the
/// signer's lifetime, forwarding every event from `outgoing` and pushing
/// every kind-24133 reply into `incoming`. Returning ends the transport.
#[async_trait]
pub trait BunkerTransport: Send + Sync + 'static {
    async fn run(
        self: Box<Self>,
        outgoing: broadcast::Receiver<Value>,
        incoming: mpsc::UnboundedSender<Value>,
        connected: Arc<AtomicBool>,
    );
}

/// What the status endpoint reports about a bunker connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BunkerStatus {
    pub remote_pubkey: String,
    pub relays: Vec<String>,
    pub connected: bool,
    pub user_pubkey: Option<String>,
    /// Set when the signer asked for browser approval and the last request
    /// is waiting on it.
    pub auth_url: Option<String>,
}

struct Inner {
    uri: BunkerUri,
    client_secret: String,
    client_pubkey: String,
    conv: ConversationKey,
    user_pubkey: Mutex<Option<String>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Response>>>,
    auth_url: Mutex<Option<String>>,
    outgoing: broadcast::Sender<Value>,
    connected: Arc<AtomicBool>,
    /// Per-request wait, in milliseconds (atomic so tests can shorten it).
    timeout_ms: std::sync::atomic::AtomicU64,
}

/// A live connection to one remote signer.
#[derive(Clone)]
pub struct BunkerSigner {
    inner: Arc<Inner>,
}

/// A fresh secp256k1 secret (hex) for the transport keypair.
pub fn mint_client_secret() -> String {
    use rand::RngCore;
    loop {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        if secp256k1::SecretKey::from_slice(&bytes).is_ok() {
            return hex::encode(bytes);
        }
    }
}

impl BunkerSigner {
    /// Start the signer over `transport`. `client_secret` is the transport
    /// key — pass the persisted one to resume a pairing, `None` to mint.
    pub fn start(
        uri: BunkerUri,
        client_secret: Option<String>,
        transport: Box<dyn BunkerTransport>,
    ) -> Result<Self, SigningError> {
        let client_secret = client_secret.unwrap_or_else(mint_client_secret);
        let client_pubkey = nip44::pubkey_for_secret(&client_secret)
            .map_err(|e| SigningError::External(e.to_string()))?;
        let conv = nip44::conversation_key(&client_secret, &uri.remote_pubkey)
            .map_err(|e| SigningError::External(e.to_string()))?;
        let (outgoing, rx) = broadcast::channel(64);
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let connected = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(Inner {
            uri,
            client_secret,
            client_pubkey,
            conv,
            user_pubkey: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            auth_url: Mutex::new(None),
            outgoing,
            connected: connected.clone(),
            timeout_ms: std::sync::atomic::AtomicU64::new(REQUEST_TIMEOUT.as_millis() as u64),
        });
        tokio::spawn(transport.run(rx, in_tx, connected));
        tokio::spawn(dispatch_replies(inner.clone(), in_rx));
        Ok(Self { inner })
    }

    /// Start with a relay transport built from the URI's relays.
    pub fn start_over_relays(
        uri: BunkerUri,
        client_secret: Option<String>,
    ) -> Result<Self, SigningError> {
        let transport = RelayTransport::new(uri.relays.clone(), &uri.remote_pubkey);
        // The transport needs our pubkey for its REQ filter; it is derived
        // from the same secret, so compute it once here.
        let secret = client_secret.unwrap_or_else(mint_client_secret);
        let client_pubkey =
            nip44::pubkey_for_secret(&secret).map_err(|e| SigningError::External(e.to_string()))?;
        let transport = transport.with_client_pubkey(client_pubkey);
        Self::start(uri, Some(secret), Box::new(transport))
    }

    pub fn uri(&self) -> &BunkerUri {
        &self.inner.uri
    }

    /// The transport secret — persist it with the URI to resume a pairing.
    pub fn client_secret(&self) -> &str {
        &self.inner.client_secret
    }

    pub fn client_pubkey(&self) -> &str {
        &self.inner.client_pubkey
    }

    pub fn user_pubkey(&self) -> Option<String> {
        self.inner.user_pubkey.lock().unwrap().clone()
    }

    #[cfg(test)]
    fn set_timeout(&self, d: Duration) {
        self.inner
            .timeout_ms
            .store(d.as_millis() as u64, Ordering::Relaxed);
    }

    pub fn status(&self) -> BunkerStatus {
        BunkerStatus {
            remote_pubkey: self.inner.uri.remote_pubkey.clone(),
            relays: self.inner.uri.relays.clone(),
            connected: self.inner.connected.load(Ordering::Relaxed),
            user_pubkey: self.user_pubkey(),
            auth_url: self.inner.auth_url.lock().unwrap().clone(),
        }
    }

    /// `connect` (with the URI's secret) then `get_public_key`. Returns the
    /// user's pubkey — the identity this signer signs as.
    pub async fn connect(&self) -> Result<String, SigningError> {
        let req = Request::connect(
            &self.inner.uri.remote_pubkey,
            self.inner.uri.secret.as_deref(),
            None,
        );
        match self.request_with_timeout(req, CONNECT_TIMEOUT).await? {
            Outcome::Result(_) => {}
            Outcome::AuthUrl(url) => return Err(SigningError::AuthRequired(url)),
            Outcome::Error(e) => return Err(SigningError::External(format!("connect: {e}"))),
        }
        let pubkey = match self
            .request_with_timeout(Request::get_public_key(), CONNECT_TIMEOUT)
            .await?
        {
            Outcome::Result(pk) => pk,
            Outcome::AuthUrl(url) => return Err(SigningError::AuthRequired(url)),
            Outcome::Error(e) => {
                return Err(SigningError::External(format!("get_public_key: {e}")))
            }
        };
        if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(SigningError::External(format!(
                "signer returned a malformed pubkey: {pubkey}"
            )));
        }
        *self.inner.user_pubkey.lock().unwrap() = Some(pubkey.clone());
        Ok(pubkey)
    }

    /// Send one request and wait for its reply (the default per-request wait).
    async fn request(&self, req: Request) -> Result<Outcome, SigningError> {
        let wait = Duration::from_millis(self.inner.timeout_ms.load(Ordering::Relaxed));
        self.request_with_timeout(req, wait).await
    }

    async fn request_with_timeout(
        &self,
        req: Request,
        wait: Duration,
    ) -> Result<Outcome, SigningError> {
        let envelope =
            nip46::request_envelope(&self.inner.conv, &self.inner.uri.remote_pubkey, &req)
                .map_err(|e| SigningError::External(e.to_string()))?;
        let template = EventTemplate {
            kind: envelope.kind,
            created_at: now_secs() as i64,
            tags: envelope.tags,
            content: envelope.content,
            pubkey: Some(self.inner.client_pubkey.clone()),
        };
        let transport_signer = InProcessSigner::new(
            self.inner.client_pubkey.clone(),
            self.inner.client_secret.clone(),
        );
        let event = transport_signer.sign(template).await?;

        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .unwrap()
            .insert(req.id.clone(), tx);
        if self.inner.outgoing.send(event).is_err() {
            self.inner.pending.lock().unwrap().remove(&req.id);
            return Err(SigningError::SignerNotConnected);
        }
        // Tests shorten the wait through `set_timeout`; honour it here too.
        let wait = wait.min(Duration::from_millis(
            self.inner.timeout_ms.load(Ordering::Relaxed),
        ));
        let reply = match tokio::time::timeout(wait, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => return Err(SigningError::SignerNotConnected),
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&req.id);
                return Err(SigningError::Timeout);
            }
        };
        let outcome = reply.outcome();
        *self.inner.auth_url.lock().unwrap() = match &outcome {
            Outcome::AuthUrl(url) => Some(url.clone()),
            _ => None,
        };
        Ok(outcome)
    }
}

/// Decode replies from the transport and hand each to its waiting request.
async fn dispatch_replies(inner: Arc<Inner>, mut incoming: mpsc::UnboundedReceiver<Value>) {
    // Relays fan the same reply back once each; remember recent ids.
    let mut seen: VecDeque<String> = VecDeque::new();
    let mut seen_set: HashSet<String> = HashSet::new();
    while let Some(event) = incoming.recv().await {
        if let Some(id) = event.get("id").and_then(Value::as_str) {
            if !seen_set.insert(id.to_string()) {
                continue;
            }
            seen.push_back(id.to_string());
            if seen.len() > 256 {
                if let Some(old) = seen.pop_front() {
                    seen_set.remove(&old);
                }
            }
        }
        // Only the remote signer may answer us.
        if event.get("pubkey").and_then(Value::as_str) != Some(inner.uri.remote_pubkey.as_str()) {
            continue;
        }
        let response = match nip46::decode_response(&inner.conv, &event) {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("bunker: undecodable reply: {e}");
                continue;
            }
        };
        if let Some(tx) = inner.pending.lock().unwrap().remove(&response.id) {
            let _ = tx.send(response);
        }
    }
}

#[async_trait]
impl Signer for BunkerSigner {
    async fn sign(&self, template: EventTemplate) -> Result<SignedEvent, SigningError> {
        let user = self.user_pubkey().ok_or(SigningError::SignerNotConnected)?;
        if let Some(want) = &template.pubkey {
            if want != &user {
                return Err(SigningError::Mismatch(format!(
                    "template wants {want} but the bunker signs as {user}"
                )));
            }
        }
        let unsigned = json!({
            "kind": template.kind,
            "created_at": template.created_at,
            "tags": template.tags,
            "content": template.content,
            "pubkey": user,
        });
        let outcome = self
            .request(Request::sign_event(unsigned.to_string()))
            .await?;
        let signed_json = match outcome {
            Outcome::Result(s) => s,
            Outcome::AuthUrl(url) => return Err(SigningError::AuthRequired(url)),
            Outcome::Error(e) => return Err(SigningError::External(format!("sign_event: {e}"))),
        };
        let event: Value = serde_json::from_str(&signed_json)
            .map_err(|e| SigningError::External(format!("sign_event reply: {e}")))?;
        if event.get("pubkey").and_then(Value::as_str) != Some(user.as_str()) {
            return Err(SigningError::Mismatch(
                "bunker signed with a different key than it announced".into(),
            ));
        }
        verify_signed_event(&event).map_err(SigningError::External)?;
        Ok(event)
    }

    fn pubkey(&self) -> &str {
        // The trait wants a borrow; the user pubkey is behind a lock. Callers
        // in this codebase pass the pubkey explicitly (see SigningController),
        // so this mirrors the controller's own stub.
        ""
    }

    fn capabilities(&self) -> SignerCapabilities {
        SignerCapabilities {
            sign_event: true,
            ..Default::default()
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Relay transport
// ---------------------------------------------------------------------------

/// One socket per relay, each subscribed to the signer's replies to us;
/// every outgoing request goes to all of them. Reconnects with backoff.
pub struct RelayTransport {
    relays: Vec<String>,
    remote_pubkey: String,
    client_pubkey: String,
}

impl RelayTransport {
    pub fn new(relays: Vec<String>, remote_pubkey: &str) -> Self {
        Self {
            relays,
            remote_pubkey: remote_pubkey.to_string(),
            client_pubkey: String::new(),
        }
    }

    pub fn with_client_pubkey(mut self, client_pubkey: String) -> Self {
        self.client_pubkey = client_pubkey;
        self
    }
}

#[async_trait]
impl BunkerTransport for RelayTransport {
    async fn run(
        self: Box<Self>,
        outgoing: broadcast::Receiver<Value>,
        incoming: mpsc::UnboundedSender<Value>,
        connected: Arc<AtomicBool>,
    ) {
        let live = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for url in self.relays.clone() {
            let rx = outgoing.resubscribe();
            let incoming = incoming.clone();
            let connected = connected.clone();
            let live = live.clone();
            let remote = self.remote_pubkey.clone();
            let client = self.client_pubkey.clone();
            tasks.push(tokio::spawn(relay_loop(
                url, remote, client, rx, incoming, connected, live,
            )));
        }
        for t in tasks {
            let _ = t.await;
        }
    }
}

async fn relay_loop(
    url: String,
    remote_pubkey: String,
    client_pubkey: String,
    mut outgoing: broadcast::Receiver<Value>,
    incoming: mpsc::UnboundedSender<Value>,
    connected: Arc<AtomicBool>,
    live: Arc<std::sync::atomic::AtomicUsize>,
) {
    use crate::relay::{Frame, RelayAuth, RelayConn};
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut conn = match RelayConn::connect(&url, RelayAuth::none()).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("bunker: {url}: connect failed: {e}; retry in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(30));
                continue;
            }
        };
        backoff = Duration::from_secs(1);
        let sub_id = format!("nip46-{}", &client_pubkey[..8.min(client_pubkey.len())]);
        let filter = nip46::response_filter(
            &client_pubkey,
            &remote_pubkey,
            now_secs().saturating_sub(60),
        );
        if conn.send(json!(["REQ", sub_id, filter])).await.is_err() {
            continue;
        }
        live.fetch_add(1, Ordering::Relaxed);
        connected.store(true, Ordering::Relaxed);
        tracing::info!("bunker: subscribed to {url}");
        let mut alive = true;
        while alive {
            tokio::select! {
                out = outgoing.recv() => match out {
                    Ok(event) => {
                        if conn.send(json!(["EVENT", event])).await.is_err() {
                            alive = false;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        conn.close().await;
                        if live.fetch_sub(1, Ordering::Relaxed) == 1 {
                            connected.store(false, Ordering::Relaxed);
                        }
                        return; // signer dropped
                    }
                },
                frame = conn.next(Duration::from_secs(60)) => match frame {
                    Frame::Msg(crate::nip42::RelayMessage::Event { event, .. }) => {
                        let _ = incoming.send(event);
                    }
                    Frame::Msg(_) | Frame::Idle => {}
                    Frame::Ended | Frame::Error(_) => alive = false,
                },
            }
        }
        if live.fetch_sub(1, Ordering::Relaxed) == 1 {
            connected.store(false, Ordering::Relaxed);
        }
        tracing::debug!("bunker: {url}: socket ended; reconnecting");
    }
}

// ---------------------------------------------------------------------------
// Tests: the whole exchange against an in-memory remote signer
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nip44::pubkey_for_secret;

    const REMOTE_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000002";
    /// The key the bunker signs *as* — deliberately not the remote signer's
    /// own key, to prove the client never assumes they are the same.
    const USER_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000003";
    const SECRET: &str = "s3cret";

    /// Decrypts requests as the remote signer and answers like a bunker.
    struct FakeBunker {
        auth_first: bool,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl BunkerTransport for FakeBunker {
        async fn run(
            self: Box<Self>,
            mut outgoing: broadcast::Receiver<Value>,
            incoming: mpsc::UnboundedSender<Value>,
            connected: Arc<AtomicBool>,
        ) {
            connected.store(true, Ordering::Relaxed);
            let remote_pub = pubkey_for_secret(REMOTE_SEC).unwrap();
            let user_pub = pubkey_for_secret(USER_SEC).unwrap();
            let user_signer = InProcessSigner::new(user_pub.clone(), USER_SEC.into());
            let remote_signer = InProcessSigner::new(remote_pub.clone(), REMOTE_SEC.into());
            let mut authed = !self.auth_first;
            while let Ok(event) = outgoing.recv().await {
                assert_eq!(event["kind"], nip46::KIND);
                let client_pub = event["pubkey"].as_str().unwrap().to_string();
                let conv = nip44::conversation_key(REMOTE_SEC, &client_pub).unwrap();
                let plain = nip44::decrypt(&conv, event["content"].as_str().unwrap()).unwrap();
                let req: Request = serde_json::from_str(&plain).unwrap();
                self.seen.lock().unwrap().push(req.method.clone());
                let (result, error) = match req.method.as_str() {
                    _ if !authed => {
                        authed = true;
                        (
                            Some("auth_url".to_string()),
                            Some("https://bunker.example/approve".to_string()),
                        )
                    }
                    "connect" => {
                        if req.params.get(1).map(String::as_str) == Some(SECRET) {
                            (Some("ack".into()), None)
                        } else {
                            (None, Some("bad secret".into()))
                        }
                    }
                    "get_public_key" => (Some(user_pub.clone()), None),
                    "sign_event" => {
                        let unsigned: Value = serde_json::from_str(&req.params[0]).unwrap();
                        let t = EventTemplate {
                            kind: unsigned["kind"].as_u64().unwrap() as u32,
                            created_at: unsigned["created_at"].as_i64().unwrap(),
                            tags: serde_json::from_value(unsigned["tags"].clone()).unwrap(),
                            content: unsigned["content"].as_str().unwrap().into(),
                            pubkey: None,
                        };
                        let signed = user_signer.sign(t).await.unwrap();
                        (Some(signed.to_string()), None)
                    }
                    other => (None, Some(format!("unknown method {other}"))),
                };
                let reply = Response {
                    id: req.id,
                    result,
                    error,
                };
                let content =
                    nip44::encrypt(&conv, &serde_json::to_string(&reply).unwrap()).unwrap();
                let ev = remote_signer
                    .sign(EventTemplate {
                        kind: nip46::KIND,
                        created_at: now_secs() as i64,
                        tags: vec![vec!["p".into(), client_pub]],
                        content,
                        pubkey: None,
                    })
                    .await
                    .unwrap();
                // Relays would echo it twice; prove dedupe holds.
                let _ = incoming.send(ev.clone());
                let _ = incoming.send(ev);
            }
        }
    }

    fn uri() -> BunkerUri {
        BunkerUri {
            remote_pubkey: pubkey_for_secret(REMOTE_SEC).unwrap(),
            relays: vec!["wss://unused.example".into()],
            secret: Some(SECRET.into()),
        }
    }

    #[tokio::test]
    async fn connect_get_public_key_and_sign_through_a_bunker() {
        let seen = Arc::new(Mutex::new(vec![]));
        let signer = BunkerSigner::start(
            uri(),
            None,
            Box::new(FakeBunker {
                auth_first: false,
                seen: seen.clone(),
            }),
        )
        .unwrap();
        let user = signer.connect().await.unwrap();
        assert_eq!(user, pubkey_for_secret(USER_SEC).unwrap());
        assert_ne!(
            user,
            signer.uri().remote_pubkey,
            "user key ≠ remote signer key"
        );
        assert_eq!(signer.user_pubkey().as_deref(), Some(user.as_str()));

        let signed = signer
            .sign(EventTemplate {
                kind: 1,
                created_at: 1_700_000_000,
                tags: vec![vec!["client".into(), "tendrl".into()]],
                content: "signed remotely".into(),
                pubkey: Some(user.clone()),
            })
            .await
            .unwrap();
        assert_eq!(signed["pubkey"], user);
        assert_eq!(signed["content"], "signed remotely");
        assert!(verify_signed_event(&signed).is_ok());
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["connect", "get_public_key", "sign_event"]
        );
        let st = signer.status();
        assert!(st.connected && st.auth_url.is_none());

        // A template for someone else is refused before it reaches the bunker.
        let err = signer
            .sign(EventTemplate {
                kind: 1,
                created_at: 1,
                tags: vec![],
                content: String::new(),
                pubkey: Some("ab".repeat(32)),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, SigningError::Mismatch(_)));
    }

    #[tokio::test]
    async fn auth_url_is_surfaced_then_the_retry_succeeds() {
        let signer = BunkerSigner::start(
            uri(),
            Some(mint_client_secret()),
            Box::new(FakeBunker {
                auth_first: true,
                seen: Arc::new(Mutex::new(vec![])),
            }),
        )
        .unwrap();
        match signer.connect().await {
            Err(SigningError::AuthRequired(url)) => {
                assert_eq!(url, "https://bunker.example/approve")
            }
            other => panic!("expected AuthRequired, got {other:?}"),
        }
        assert_eq!(
            signer.status().auth_url.as_deref(),
            Some("https://bunker.example/approve")
        );
        let user = signer.connect().await.unwrap();
        assert_eq!(user, pubkey_for_secret(USER_SEC).unwrap());
        assert!(signer.status().auth_url.is_none());
    }

    #[tokio::test]
    async fn wrong_secret_is_an_error_and_a_dead_transport_times_out() {
        let mut u = uri();
        u.secret = Some("wrong".into());
        let signer = BunkerSigner::start(
            u,
            None,
            Box::new(FakeBunker {
                auth_first: false,
                seen: Arc::new(Mutex::new(vec![])),
            }),
        )
        .unwrap();
        assert!(matches!(
            signer.connect().await,
            Err(SigningError::External(_))
        ));

        struct Silent;
        #[async_trait]
        impl BunkerTransport for Silent {
            async fn run(
                self: Box<Self>,
                mut o: broadcast::Receiver<Value>,
                _i: mpsc::UnboundedSender<Value>,
                _c: Arc<AtomicBool>,
            ) {
                while o.recv().await.is_ok() {}
            }
        }
        let signer = BunkerSigner::start(uri(), None, Box::new(Silent)).unwrap();
        signer.set_timeout(Duration::from_millis(200));
        assert!(matches!(signer.connect().await, Err(SigningError::Timeout)));
        // Signing before connect: no user pubkey yet.
        assert!(matches!(
            signer
                .sign(EventTemplate {
                    kind: 1,
                    created_at: 1,
                    tags: vec![],
                    content: String::new(),
                    pubkey: None
                })
                .await,
            Err(SigningError::SignerNotConnected)
        ));
    }

    #[test]
    fn minted_secrets_are_valid_and_distinct() {
        let a = mint_client_secret();
        let b = mint_client_secret();
        assert_ne!(a, b);
        assert!(pubkey_for_secret(&a).is_ok());
    }
}
