//! WebSocket relay client
//!
//! Fetches events from Nostr relays and ingests them into nostrdb;
//! publishes signed events and collects the relay's `OK` verdicts.
//!
//! One socket per operation. Every socket runs the same small NIP-42
//! state machine ([`RelayConn`]): a relay may challenge with `AUTH` at
//! any point, and may refuse a REQ (`CLOSED`) or an EVENT (`OK false`)
//! with the `auth-required:` prefix. Whether tendrl answers is decided
//! by [`RelayAuth`] — the user's per-relay opt-in set plus the narrow
//! kind-22242 signer — and never by the relay: a relay outside the set
//! has its challenge ignored and its refusal reported as
//! [`EngineError::RelayAuthRequired`] / [`PublishResult::refusal`], with
//! no prompt anywhere. Design: `docs/nip42-relay-auth-plan.org`.

use crate::error::{EngineError, Result};
use crate::nip42::{Refusal, RelayMessage};
use crate::signing::AuthSigner;
use futures::{SinkExt, StreamExt};
use nostrdb::{IngestMetadata, Ndb};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// Default relays to fetch from (focused on publication-supporting
/// relays). Public relays only — a `ws://localhost` dev relay used to be
/// seeded here, which put a guaranteed-refused connection into every
/// fresh install's working sets (and every operation on mobile, where
/// nothing listens on loopback). Add a local relay explicitly when you
/// run one.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.noswhere.com",
    "wss://relay.damus.io",
    "wss://theforest.nostr1.com",
    "wss://thecitadel.nostr1.com",
    "wss://mercury-relay.imwald.eu/relay",
];

/// Defaults that seed as read-only: included in the read/general seed,
/// excluded from the publish seed. (thecitadel is read+write and so is
/// absent here; theforest and mercury are read sources only.)
pub const DEFAULT_READ_ONLY: &[&str] = &[
    "wss://theforest.nostr1.com",
    "wss://mercury-relay.imwald.eu/relay",
];

/// Default search-kind relays (see `RelaySets::search_kinds`):
/// fetches of these kinds also go to the listed relay, alongside the
/// read set. Mercury serves the publication/wiki kinds tendrl
/// resolves most — 30040 (index), 30041 (section), 30818 (wiki).
pub const DEFAULT_RESOLVE_CLAIMS: &[(&str, &[u64])] =
    &[("wss://mercury-relay.imwald.eu/relay", &[30040, 30041, 30818])];

/// Bound on TCP + TLS + WebSocket-upgrade establishment. The per-fetch
/// timeouts below only start ticking once the socket is up — without
/// this cap, a blackholed connect (wifi↔cellular handoff, unreachable
/// IPv6, Doze-frozen radio — all routine on Android) blocks for the OS
/// TCP timeout, i.e. minutes, and anything awaiting the fetch hangs
/// with it. A healthy relay completes the handshake well under 5s.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Whole-fetch bound (restarted after an AUTH round-trip re-sends the REQ).
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
/// Silence bound between frames on a fetch.
const FETCH_IDLE: Duration = Duration::from_secs(5);
/// Silence bound between frames on a publish. Some relays never send
/// `OK`; silence after the sends is (still) read as acceptance.
const PUBLISH_IDLE: Duration = Duration::from_secs(5);
/// How long to wait for the relay's `OK` to our `AUTH` event.
const AUTH_OK_TIMEOUT: Duration = Duration::from_secs(10);

/// Default profile / relay-list relays (the former "indexers"). Seeded
/// on first boot as search relays narrowed to
/// `relay_store::INDEXER_KINDS`, so a fresh install can find profiles
/// without manual configuration. Well-known
/// purpose-built profile relays:
///   - purplepag.es: pubkey-indexed profile + relay-list mirror
///   - user.kindpag.es: kind-10002 / 10003 / 10006 / 10007 indexer
/// Users can remove them on the relays screen; removals persist to
/// relays.json so they won't be re-seeded.
pub const DEFAULT_INDEXERS: &[&str] = &[
    "wss://purplepag.es",
    "wss://user.kindpag.es",
];

/// Default search relays — NIP-50-capable. Empty by default since
/// search is opt-in and the user is more likely to have a specific
/// search relay preference than a profile-discovery one.
pub const DEFAULT_SEARCH: &[&str] = &[];

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ---------------------------------------------------------------------------
// Local mode gate
// ---------------------------------------------------------------------------
//
// `NetworkMode::Local` promises that the engine never opens a relay socket.
// The network layer already refuses fetch/publish operations in that mode;
// this is the floor under it: every relay socket goes through
// `connect_bounded`, so a gate here holds even for a path that forgot to ask.
// The one deliberate way through is a [`RelayPermit`] — the user picked
// these relays for this operation (the broadcast modal) — and the NIP-46
// signer transport ([`RelayConn::connect_signer`]), which carries sign
// requests, never the user's events.

static LOCAL_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PERMITS: std::sync::Mutex<Vec<(String, usize)>> = std::sync::Mutex::new(Vec::new());

/// The message a refused relay connection carries.
pub const LOCAL_MODE_REFUSAL: &str = "local mode — relays are off";

/// Turn the local-mode gate on or off (follows `NetworkMode::Local`).
pub fn set_local_only(on: bool) {
    LOCAL_ONLY.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn local_only() -> bool {
    LOCAL_ONLY.load(std::sync::atomic::Ordering::Relaxed)
}

fn permit_key(url: &str) -> String {
    let n = crate::relay_url::normalize_relay_url(url);
    if n.is_empty() {
        url.to_string()
    } else {
        n
    }
}

/// True when a connection to `url` may open now.
pub fn relay_allowed(url: &str) -> bool {
    if !local_only() {
        return true;
    }
    let key = permit_key(url);
    PERMITS.lock().unwrap().iter().any(|(u, n)| *n > 0 && *u == key)
}

/// Lets the listed relays through the local-mode gate while it lives.
/// Counted, so overlapping permits for one relay compose.
#[must_use = "the permit lasts only while it is held"]
pub struct RelayPermit {
    urls: Vec<String>,
}

pub fn permit(urls: &[String]) -> RelayPermit {
    let urls: Vec<String> = urls.iter().map(|u| permit_key(u)).collect();
    let mut p = PERMITS.lock().unwrap();
    for u in &urls {
        match p.iter_mut().find(|(k, _)| k == u) {
            Some((_, n)) => *n += 1,
            None => p.push((u.clone(), 1)),
        }
    }
    RelayPermit { urls }
}

impl Drop for RelayPermit {
    fn drop(&mut self) {
        let mut p = PERMITS.lock().unwrap();
        for u in &self.urls {
            if let Some((_, n)) = p.iter_mut().find(|(k, _)| k == u) {
                *n = n.saturating_sub(1);
            }
        }
        p.retain(|(_, n)| *n > 0);
    }
}

/// `connect_async` bounded by [`CONNECT_TIMEOUT`]; error is a plain
/// string so both the fetch path (`EngineError`) and the publish path
/// (`String`) can wrap it. Refused in local mode unless permitted.
async fn connect_bounded(relay_url: &str) -> std::result::Result<WsStream, String> {
    if !relay_allowed(relay_url) {
        return Err(LOCAL_MODE_REFUSAL.to_string());
    }
    connect_bounded_ungated(relay_url).await
}

async fn connect_bounded_ungated(relay_url: &str) -> std::result::Result<WsStream, String> {
    match timeout(CONNECT_TIMEOUT, connect_async(relay_url)).await {
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(e)) => Err(format!("Failed to connect: {}", e)),
        Err(_) => Err(format!(
            "Connect timed out after {}s",
            CONNECT_TIMEOUT.as_secs()
        )),
    }
}

// ---------------------------------------------------------------------------
// NIP-42 policy + per-connection state
// ---------------------------------------------------------------------------

/// What the relay client may authenticate with, and to whom. Built per
/// operation by `Engine::relay_auth()`; [`RelayAuth::none`] is the
/// anonymous client (tests, and callers with no engine at hand).
#[derive(Clone, Default, Debug)]
pub struct RelayAuth {
    /// The narrow kind-22242 signer (the user's active source). `None`
    /// = nothing loaded; every challenge is ignored.
    pub signer: Option<AuthSigner>,
    /// Normalized URLs of relays the user opted into authenticating to.
    pub allowed: HashSet<String>,
}

impl RelayAuth {
    pub fn none() -> Self {
        Self::default()
    }

    /// True when tendrl will answer this relay's challenge.
    pub fn permits(&self, normalized_url: &str) -> bool {
        self.signer.is_some() && self.allowed.contains(normalized_url)
    }
}

/// Why a connection ended up not authenticated after trying.
#[derive(Debug, Clone)]
enum AuthFailure {
    /// Signer unavailable / locked / refused (no event was sent).
    Sign(String),
    /// Relay answered our AUTH with `OK false`.
    Rejected(String),
    /// No `OK` for our AUTH within [`AUTH_OK_TIMEOUT`] or the socket died.
    NoReply(String),
}

impl AuthFailure {
    fn message(&self) -> String {
        match self {
            AuthFailure::Sign(m) => format!("auth signing failed: {m}"),
            AuthFailure::Rejected(m) => format!("auth rejected by relay: {m}"),
            AuthFailure::NoReply(m) => format!("auth unanswered: {m}"),
        }
    }
}

/// One frame-read outcome.
pub(crate) enum Frame {
    Msg(RelayMessage),
    Idle,
    Ended,
    Error(String),
}

/// A single relay socket with the NIP-42 state riding on it.
pub(crate) struct RelayConn {
    ws: WsStream,
    /// URL as given by the caller (for logs / results).
    url: String,
    /// Normalized URL — the `relay` tag of the AUTH event, the ingest
    /// provenance tag, and the key into [`RelayAuth::allowed`].
    tag_url: String,
    auth: RelayAuth,
    /// Latest challenge from the relay (valid for the connection).
    challenge: Option<String>,
    /// An `AUTH` we sent was answered `OK true`.
    authed: bool,
    /// We already tried once (success or not) — never loop on a relay
    /// that keeps saying `auth-required`.
    auth_attempted: bool,
    /// Frames that arrived while we were waiting for the AUTH `OK`;
    /// drained before the socket is read again.
    pending: VecDeque<RelayMessage>,
}

impl RelayConn {
    pub(crate) async fn connect(url: &str, auth: RelayAuth) -> std::result::Result<Self, String> {
        let ws = connect_bounded(url).await?;
        Ok(Self::wrap(ws, url, auth))
    }

    /// A NIP-46 signer's transport. Exempt from the local-mode gate: it
    /// carries sign requests to the user's own bunker, never their events
    /// to an audience.
    pub(crate) async fn connect_signer(url: &str, auth: RelayAuth) -> std::result::Result<Self, String> {
        let ws = connect_bounded_ungated(url).await?;
        Ok(Self::wrap(ws, url, auth))
    }

    fn wrap(ws: WsStream, url: &str, auth: RelayAuth) -> Self {
        let normalized = crate::relay_url::normalize_relay_url(url);
        let tag_url = if normalized.is_empty() {
            url.to_string()
        } else {
            normalized
        };
        Self {
            ws,
            url: url.to_string(),
            tag_url,
            auth,
            challenge: None,
            authed: false,
            auth_attempted: false,
            pending: VecDeque::new(),
        }
    }

    fn permits(&self) -> bool {
        self.auth.permits(&self.tag_url)
    }

    pub(crate) async fn send(&mut self, msg: Value) -> std::result::Result<(), String> {
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| format!("Failed to send: {e}"))
    }

    /// Next parsed frame: anything queued during an AUTH wait first, then
    /// the socket (skipping non-text and unparsable frames), bounded by
    /// `idle`.
    pub(crate) async fn next(&mut self, idle: Duration) -> Frame {
        if let Some(m) = self.pending.pop_front() {
            return Frame::Msg(m);
        }
        self.next_from_socket(idle).await
    }

    /// Read the socket only — never the pending queue. The AUTH wait uses
    /// this so the frames it parks in `pending` are not handed straight
    /// back to it (which would spin without ever reading the relay's OK).
    async fn next_from_socket(&mut self, idle: Duration) -> Frame {
        loop {
            match timeout(idle, self.ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => match RelayMessage::parse(&text) {
                    Some(m) => return Frame::Msg(m),
                    None => continue,
                },
                Ok(Some(Ok(Message::Close(_)))) => return Frame::Ended,
                Ok(Some(Ok(_))) => continue, // binary / ping / pong
                Ok(Some(Err(e))) => return Frame::Error(e.to_string()),
                Ok(None) => return Frame::Ended,
                Err(_) => return Frame::Idle,
            }
        }
    }

    /// Record a challenge. Returns true when policy says to answer it now.
    fn on_challenge(&mut self, challenge: String) -> bool {
        self.challenge = Some(challenge);
        self.permits() && !self.authed && !self.auth_attempted
    }

    /// Sign the stored challenge, send `AUTH`, and wait for the relay's
    /// `OK`. Frames other than that `OK` are queued for the caller. The
    /// caller checks `permits()` / `challenge` first; this only runs the
    /// round-trip.
    async fn authenticate(&mut self) -> std::result::Result<(), AuthFailure> {
        self.auth_attempted = true;
        let challenge = self
            .challenge
            .clone()
            .ok_or_else(|| AuthFailure::Sign("no challenge from relay".into()))?;
        let signer = self
            .auth
            .signer
            .clone()
            .ok_or_else(|| AuthFailure::Sign("no signer".into()))?;
        let event = signer
            .sign_auth(&self.tag_url, &challenge)
            .await
            .map_err(|e| AuthFailure::Sign(e.to_string()))?;
        let auth_id = event
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.send(json!(["AUTH", event]))
            .await
            .map_err(AuthFailure::NoReply)?;

        let start = Instant::now();
        while start.elapsed() < AUTH_OK_TIMEOUT {
            match self
                .next_from_socket(AUTH_OK_TIMEOUT.saturating_sub(start.elapsed()))
                .await
            {
                Frame::Msg(RelayMessage::Ok {
                    id,
                    accepted,
                    message,
                }) if id == auth_id => {
                    if accepted {
                        self.authed = true;
                        info!("Authenticated to {} (NIP-42)", self.url);
                        return Ok(());
                    }
                    return Err(AuthFailure::Rejected(message));
                }
                Frame::Msg(RelayMessage::Auth { challenge }) => {
                    // A fresh challenge supersedes; keep it for a caller
                    // that wants to retry, but this attempt stands.
                    self.challenge = Some(challenge);
                }
                Frame::Msg(m) => self.pending.push_back(m),
                Frame::Idle => break,
                Frame::Ended => return Err(AuthFailure::NoReply("connection closed".into())),
                Frame::Error(e) => return Err(AuthFailure::NoReply(e)),
            }
        }
        Err(AuthFailure::NoReply(format!(
            "no OK within {}s",
            AUTH_OK_TIMEOUT.as_secs()
        )))
    }

    pub(crate) async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

// ---------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------

/// What a fetch returned, plus whether the socket authenticated.
#[derive(Debug, Default)]
pub struct FetchOutcome {
    pub events: Vec<Value>,
    pub authenticated: bool,
}

static SUB_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_sub_id() -> String {
    // Unique per REQ. With one REQ per socket this is only hygiene, but a
    // shared/pooled connection (the planned persistent layer) would collide
    // subscriptions — and CLOSE the wrong one — on a fixed id. It also
    // matters *within* a socket now: the post-AUTH re-REQ uses a fresh id
    // so a late CLOSED for the refused one can't be mistaken for it.
    format!(
        "tendrl-fetch-{}",
        SUB_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Fetch events using NIP-01 filters from a relay.
///
/// Answers a NIP-42 challenge when `auth` permits it for this relay;
/// otherwise an `auth-required` CLOSED ends the fetch with
/// [`EngineError::RelayAuthRequired`] (a per-relay status, not a fault).
pub async fn fetch_with_filters(
    ndb: &Ndb,
    relay_url: &str,
    filters: &[Value],
    auth: &RelayAuth,
) -> Result<FetchOutcome> {
    debug!("Fetching events from {} with {} filters", relay_url, filters.len());

    let mut conn = RelayConn::connect(relay_url, auth.clone())
        .await
        .map_err(|e| EngineError::Relay(format!("{}: {}", relay_url, e)))?;

    let req_for = |sub_id: &str| {
        let mut req = vec![json!("REQ"), json!(sub_id)];
        req.extend(filters.iter().cloned());
        Value::Array(req)
    };

    let mut sub_id = next_sub_id();
    conn.send(req_for(&sub_id))
        .await
        .map_err(|e| EngineError::Relay(format!("Failed to send REQ: {}", e)))?;

    let mut fetched_events = Vec::new();
    let started = Instant::now();
    let mut start = Instant::now();
    // Set when the relay CLOSED our REQ with auth-required before we had
    // a challenge to answer; the next AUTH frame completes the dance.
    let mut resend_after_auth = false;
    // The post-AUTH re-REQ happened; a second auth-required CLOSED after
    // that is the relay's final word.
    let mut resent = false;

    let auth_required = |msg: String| EngineError::RelayAuthRequired {
        relay: relay_url.to_string(),
        msg,
    };

    'outer: while start.elapsed() < FETCH_TIMEOUT {
        match conn.next(FETCH_IDLE).await {
            Frame::Msg(RelayMessage::Event { sub_id: sid, event }) => {
                if sid != sub_id {
                    continue;
                }
                // Ingest into nostrdb for caching, tagging the source
                // relay (normalized) so `note.relays()` records where the
                // event was seen without trailing-slash / case duplicates.
                let text = json!(["EVENT", sid, event]).to_string();
                if let Err(e) = ndb.process_event_with(
                    &text,
                    IngestMetadata::new().client(false).relay(&conn.tag_url),
                ) {
                    debug!("Failed to ingest event: {}", e);
                }
                fetched_events.push(event);
            }
            Frame::Msg(RelayMessage::Eose { sub_id: sid }) => {
                if sid == sub_id {
                    debug!("EOSE received after {} events", fetched_events.len());
                    break;
                }
            }
            Frame::Msg(RelayMessage::Auth { challenge }) => {
                if conn.on_challenge(challenge) {
                    if let Err(f) = conn.authenticate().await {
                        conn.close().await;
                        return Err(auth_required(f.message()));
                    }
                    if resend_after_auth {
                        resend_after_auth = false;
                        resent = true;
                        sub_id = next_sub_id();
                        conn.send(req_for(&sub_id))
                            .await
                            .map_err(|e| EngineError::Relay(format!("Failed to re-send REQ: {}", e)))?;
                        start = Instant::now();
                    }
                }
            }
            Frame::Msg(RelayMessage::Closed { sub_id: sid, reason }) => {
                if sid != sub_id {
                    continue;
                }
                match Refusal::classify(&reason) {
                    Refusal::AuthRequired => {
                        if !conn.permits() {
                            conn.close().await;
                            return Err(auth_required(reason));
                        }
                        if resent || (conn.auth_attempted && !conn.authed) {
                            // Re-REQ'd after AUTH and still refused, or the
                            // AUTH itself failed — don't loop; report the
                            // relay's words.
                            conn.close().await;
                            return Err(auth_required(reason));
                        }
                        if conn.authed {
                            // The CLOSED is for the pre-AUTH REQ (the relay
                            // challenged and refused in one breath, and we
                            // answered the challenge first). Re-REQ.
                            resent = true;
                            sub_id = next_sub_id();
                            conn.send(req_for(&sub_id)).await.map_err(|e| {
                                EngineError::Relay(format!("Failed to re-send REQ: {}", e))
                            })?;
                            start = Instant::now();
                        } else if conn.challenge.is_some() {
                            if let Err(f) = conn.authenticate().await {
                                conn.close().await;
                                return Err(auth_required(f.message()));
                            }
                            resent = true;
                            sub_id = next_sub_id();
                            conn.send(req_for(&sub_id)).await.map_err(|e| {
                                EngineError::Relay(format!("Failed to re-send REQ: {}", e))
                            })?;
                            start = Instant::now();
                        } else {
                            // Spec allows CLOSED before AUTH; wait for the
                            // challenge (idle timeout bounds the wait).
                            resend_after_auth = true;
                        }
                    }
                    Refusal::Restricted | Refusal::Other => {
                        conn.close().await;
                        return Err(EngineError::Relay(format!("{}: {}", relay_url, reason)));
                    }
                }
            }
            Frame::Msg(RelayMessage::Notice { message }) => {
                warn!("Relay notice from {}: {}", relay_url, message);
            }
            Frame::Msg(_) => {}
            Frame::Idle => {
                debug!("Timeout waiting for events from {}", relay_url);
                break 'outer;
            }
            Frame::Ended => break,
            Frame::Error(e) => {
                warn!("WebSocket error from {}: {}", relay_url, e);
                break;
            }
        }
    }

    // Close subscription
    let _ = conn.send(json!(["CLOSE", sub_id])).await;
    let authenticated = conn.authed;
    conn.close().await;

    info!(
        "Fetched {} events from {} in {:?}{} for {}",
        fetched_events.len(),
        relay_url,
        started.elapsed(),
        if authenticated { " (authenticated)" } else { "" },
        crate::network::describe_filters_for_log(filters)
    );

    Ok(FetchOutcome {
        events: fetched_events,
        authenticated,
    })
}

// ---------------------------------------------------------------------------
// Publish
// ---------------------------------------------------------------------------

/// Result of publishing an event to a relay
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublishResult {
    pub relay_url: String,
    pub success: bool,
    pub message: Option<String>,
    /// The id of the event this result is for. Lets callers record relay
    /// provenance per (event, relay) without relying on result ordering.
    pub event_id: String,
    /// NIP-42 classification of a rejection: `auth_required` when the
    /// relay wanted auth and tendrl did not (or could not) authenticate,
    /// `restricted` when it authenticated but the key is not allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    /// The socket that carried this result had authenticated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub authenticated: bool,
    /// Sent, but the relay never answered with `OK` (some relays never do;
    /// a stalled one doesn't). Kept `success` for single-event callers, as
    /// before; batch publishers read the events back and settle it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
}

impl PublishResult {
    fn unconfirmed(relay_url: &str, event_id: String, authenticated: bool) -> Self {
        Self {
            relay_url: relay_url.to_string(),
            success: true,
            message: Some(UNCONFIRMED_MSG.to_string()),
            event_id,
            refusal: None,
            authenticated,
            unconfirmed: true,
        }
    }

    fn ok(relay_url: &str, event_id: String, message: Option<String>, authenticated: bool) -> Self {
        Self {
            relay_url: relay_url.to_string(),
            success: true,
            message,
            event_id,
            refusal: None,
            authenticated,
            unconfirmed: false,
        }
    }

    fn failed(
        relay_url: &str,
        event_id: String,
        message: String,
        refusal: Option<Refusal>,
        authenticated: bool,
    ) -> Self {
        Self {
            relay_url: relay_url.to_string(),
            success: false,
            message: Some(message),
            event_id,
            refusal,
            authenticated,
            unconfirmed: false,
        }
    }
}

fn event_id_of(event: &Value) -> String {
    event
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default()
}

/// Publish a batch of signed events to ONE relay over ONE socket, and
/// return a result per event (same order as `events`).
///
/// One socket per relay per batch is what makes NIP-42 bearable: the
/// AUTH round-trip (one signature — one extension/Amber prompt on a
/// first encounter) happens at most once for the whole batch instead of
/// once per event.
pub async fn publish_batch(relay_url: &str, events: &[String], auth: &RelayAuth) -> Vec<PublishResult> {
    publish_batch_with(relay_url, events, auth, &|_| {}).await
}

/// What a running batch reports, for progress.
#[derive(Debug, Clone, Copy)]
pub enum PublishEvent<'a> {
    /// One event finished (accepted, refused, or unconfirmed).
    Result(&'a PublishResult),
    /// The pace changed: `gap` between EVENTs; `rate_limited` = slowed by
    /// a `rate-limited:` refusal (else sped back up after a clean streak).
    Pace { gap: Duration, rate_limited: bool },
}

/// The pace learned per relay this session (normalized URL → gap between
/// EVENTs), so the next batch to a relay that rate-limited starts at the
/// pace it tolerated instead of rediscovering the limit.
fn learned_gaps() -> &'static std::sync::Mutex<HashMap<String, Duration>> {
    static GAPS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Duration>>> = std::sync::OnceLock::new();
    GAPS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

/// The pace a batch to `relay_url` starts at.
pub fn learned_gap(relay_url: &str) -> Duration {
    let key = crate::relay_url::normalize_relay_url(relay_url);
    learned_gaps()
        .lock()
        .ok()
        .and_then(|m| m.get(&key).copied())
        .unwrap_or(PUBLISH_GAP)
}

fn remember_gap(relay_url: &str, gap: Duration) {
    let key = crate::relay_url::normalize_relay_url(relay_url);
    if let Ok(mut m) = learned_gaps().lock() {
        m.insert(key, gap);
    }
}

/// Events per second at `gap`, for labels.
pub fn rate_label(gap: Duration) -> String {
    let per_s = 1.0 / gap.as_secs_f64().max(0.001);
    if per_s >= 1.0 {
        format!("{per_s:.0}/s")
    } else {
        format!("1 per {:.0}s", gap.as_secs_f64())
    }
}

/// Most events awaiting an `OK` at once on one socket. A relay that
/// rate-limits per pubkey (Buzz: 10 events/s) silently drops a 400-event
/// burst; a window keeps the relay's queue short and the pace honest.
const PUBLISH_WINDOW: usize = 10;
/// Starting (and fastest) gap between two EVENT frames — 10 per second.
const PUBLISH_GAP: Duration = Duration::from_millis(100);
/// Slowest pace a rate-limited relay is backed off to.
const PUBLISH_MAX_GAP: Duration = Duration::from_secs(10);
/// Accepted events in a row before the pace speeds back up (×4/3).
const PUBLISH_SPEEDUP_STREAK: u32 = 20;
/// A batch gives up only after this long with no event accepted (scaled
/// up for a slow pace) — not after a fixed total, so a long, rate-limited
/// broadcast that keeps making progress runs to the end.
const PUBLISH_STALL: Duration = Duration::from_secs(120);
/// A socket the relay drops mid-batch is reopened (and re-AUTHed) this
/// many times; the unanswered events are re-sent on the new socket.
const PUBLISH_RECONNECTS: usize = 3;
/// `rate-limited:` refusals of one event are retried this many times (the
/// whole batch slows down on each, so retries get progressively gentler).
const PUBLISH_RATE_RETRIES: u32 = 8;
/// Message on an event the relay never answered (see `PublishResult::unconfirmed`).
pub const UNCONFIRMED_MSG: &str = "no OK from the relay — unconfirmed";

/// [`publish_batch`] reporting each finished event and every pace change.
///
/// Expects rate limits. Windowed (≤ [`PUBLISH_WINDOW`] in flight, fewer
/// at a slow pace) and paced: it starts at the gap this relay tolerated
/// last time ([`learned_gap`]); every `rate-limited:` refusal doubles the
/// gap (up to [`PUBLISH_MAX_GAP`]) and requeues the event; a clean streak
/// speeds back up. It stops only after [`PUBLISH_STALL`] without an
/// accepted event. A dropped socket is reopened and the unanswered events
/// re-sent ([`PUBLISH_RECONNECTS`]); NIP-42 AUTH is answered once per
/// socket. A relay that never sends `OK` is detected after its first
/// silent window and the rest are sent without waiting; events it never
/// answered come back `success` + [`PublishResult::unconfirmed`], for a
/// read-back to settle.
pub async fn publish_batch_with(
    relay_url: &str,
    events: &[String],
    auth: &RelayAuth,
    on_event: &(dyn Fn(PublishEvent<'_>) + Send + Sync),
) -> Vec<PublishResult> {
    // Parse up front; a malformed event fails on its own, the rest go.
    let mut results: Vec<Option<PublishResult>> = Vec::with_capacity(events.len());
    let mut live: HashMap<usize, (String, Value)> = HashMap::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, json_text) in events.iter().enumerate() {
        match serde_json::from_str::<Value>(json_text) {
            Ok(v) => {
                live.insert(i, (event_id_of(&v), v));
                queue.push_back(i);
                results.push(None);
            }
            Err(e) => {
                let r = PublishResult::failed(relay_url, String::new(), format!("Invalid event JSON: {e}"), None, false);
                on_event(PublishEvent::Result(&r));
                results.push(Some(r));
            }
        }
    }

    let finish = |results: &mut Vec<Option<PublishResult>>, idx: usize, r: PublishResult| {
        on_event(PublishEvent::Result(&r));
        results[idx] = Some(r);
    };

    // Pace: the gap this relay tolerated last time; adapted as we go.
    let mut gap = learned_gap(relay_url);
    let mut streak = 0u32;
    if gap > PUBLISH_GAP {
        on_event(PublishEvent::Pace { gap, rate_limited: true });
    }
    // Progress clock: the batch stalls only after a long stretch with no
    // accepted event (longer when the pace itself is slow).
    let mut last_progress = Instant::now();
    let stall_after = |gap: Duration| PUBLISH_STALL.max(gap * 12);
    let hard_cap = Duration::from_secs(3 * 60 * 60);
    let start = Instant::now();
    let mut reconnects = 0usize;
    let mut rate_tries: HashMap<usize, u32> = HashMap::new();
    // The relay has answered at least one EVENT with `OK` — silence from it
    // is then a stall, not a relay that never acknowledges.
    let mut ever_ok = false;
    let mut fire_and_forget = false;
    let mut last_error: Option<String> = None;
    let mut authed_any = false;

    'conn: while !queue.is_empty() {
        if last_progress.elapsed() >= stall_after(gap) || start.elapsed() >= hard_cap {
            last_error = Some(format!(
                "stalled — nothing accepted for {}s (pace {})",
                last_progress.elapsed().as_secs(),
                rate_label(gap)
            ));
            break;
        }
        let mut conn = match RelayConn::connect(relay_url, auth.clone()).await {
            Ok(c) => c,
            Err(e) => {
                reconnects += 1;
                if reconnects > PUBLISH_RECONNECTS {
                    warn!("Failed to publish to {}: {}", relay_url, e);
                    last_error = Some(e);
                    break;
                }
                tokio::time::sleep(Duration::from_secs(reconnects as u64)).await;
                continue;
            }
        };

        // id → index, events sent on this socket and not yet answered.
        let mut inflight: HashMap<String, usize> = HashMap::new();
        // Refused with auth-required before the relay sent its challenge.
        let mut awaiting_auth: Vec<usize> = Vec::new();
        // Re-sent after AUTH; a second auth-required for one is final.
        let mut resent: HashSet<usize> = HashSet::new();
        let mut last_send: Option<Instant> = None;
        let mut last_heard = Instant::now();
        let mut pause_until = Instant::now();
        let mut dropped: Option<String> = None;

        loop {
            // At a slow pace keep ~1 s of events in flight, never fewer than one.
            let window = if fire_and_forget {
                usize::MAX
            } else {
                PUBLISH_WINDOW.min(((1000 / gap.as_millis().max(1)) as usize).max(1))
            };
            // Fill the window, one EVENT per gap.
            while inflight.len() + awaiting_auth.len() < window
                && Instant::now() >= pause_until
                && last_send.map_or(true, |t| t.elapsed() >= gap)
            {
                let Some(idx) = queue.pop_front() else { break };
                let (id, v) = &live[&idx];
                if let Err(e) = conn.send(json!(["EVENT", v])).await {
                    queue.push_front(idx);
                    dropped = Some(e);
                    break;
                }
                if inflight.is_empty() {
                    last_heard = Instant::now();
                }
                inflight.insert(id.clone(), idx);
                last_send = Some(Instant::now());
            }
            if dropped.is_some() {
                break;
            }
            if queue.is_empty() && inflight.is_empty() && awaiting_auth.is_empty() {
                break;
            }
            if last_progress.elapsed() >= stall_after(gap) || start.elapsed() >= hard_cap {
                break;
            }

            let can_send = !queue.is_empty() && inflight.len() + awaiting_auth.len() < window;
            let now = Instant::now();
            let wait = if can_send {
                pause_until
                    .saturating_duration_since(now)
                    .max(gap.saturating_sub(last_send.map_or(gap, |t| t.elapsed())))
                    .max(Duration::from_millis(5))
                    .min(PUBLISH_IDLE)
            } else {
                PUBLISH_IDLE
            };

            match conn.next(wait).await {
                Frame::Msg(RelayMessage::Ok { id, accepted, message }) => {
                    last_heard = Instant::now();
                    let Some(idx) = inflight.remove(&id) else { continue };
                    ever_ok = true;
                    if accepted {
                        last_progress = Instant::now();
                        streak += 1;
                        if streak >= PUBLISH_SPEEDUP_STREAK && gap > PUBLISH_GAP {
                            gap = (gap * 3 / 4).max(PUBLISH_GAP);
                            streak = 0;
                            remember_gap(relay_url, gap);
                            on_event(PublishEvent::Pace { gap, rate_limited: false });
                        }
                        let r = PublishResult::ok(relay_url, id, (!message.is_empty()).then_some(message), conn.authed);
                        finish(&mut results, idx, r);
                        continue;
                    }
                    if is_rate_limited(&message) {
                        let tries = rate_tries.entry(idx).or_insert(0);
                        *tries += 1;
                        if *tries <= PUBLISH_RATE_RETRIES {
                            // Expected, not an error: slow the whole batch
                            // down, breathe, and put the event back in line.
                            streak = 0;
                            if gap < PUBLISH_MAX_GAP {
                                gap = (gap * 2).min(PUBLISH_MAX_GAP);
                                remember_gap(relay_url, gap);
                                on_event(PublishEvent::Pace { gap, rate_limited: true });
                            }
                            debug!("{relay_url} rate-limited ({message}) — pace now {}", rate_label(gap));
                            pause_until = pause_until.max(Instant::now() + gap.max(Duration::from_secs(1)));
                            queue.push_front(idx);
                            continue;
                        }
                        let r = PublishResult::failed(relay_url, id, message, None, conn.authed);
                        finish(&mut results, idx, r);
                        continue;
                    }
                    match Refusal::classify(&message) {
                        Refusal::AuthRequired if conn.authed && !resent.contains(&idx) => {
                            // Refused before our AUTH landed — re-send.
                            resent.insert(idx);
                            let (id, v) = &live[&idx];
                            if let Err(e) = conn.send(json!(["EVENT", v])).await {
                                queue.push_front(idx);
                                dropped = Some(e);
                                break;
                            }
                            inflight.insert(id.clone(), idx);
                        }
                        Refusal::AuthRequired if conn.permits() && !conn.authed && !conn.auth_attempted => {
                            if conn.challenge.is_some() {
                                match conn.authenticate().await {
                                    Ok(()) => {
                                        resent.insert(idx);
                                        let (id, v) = &live[&idx];
                                        if let Err(e) = conn.send(json!(["EVENT", v])).await {
                                            queue.push_front(idx);
                                            dropped = Some(e);
                                            break;
                                        }
                                        inflight.insert(id.clone(), idx);
                                    }
                                    Err(f) => {
                                        let r = PublishResult::failed(relay_url, id, f.message(), Some(Refusal::AuthRequired), false);
                                        finish(&mut results, idx, r);
                                    }
                                }
                            } else {
                                awaiting_auth.push(idx);
                            }
                        }
                        refusal => {
                            let refusal = match refusal {
                                Refusal::Other => None,
                                r => Some(r),
                            };
                            let r = PublishResult::failed(relay_url, id, message, refusal, conn.authed);
                            finish(&mut results, idx, r);
                        }
                    }
                }
                Frame::Msg(RelayMessage::Auth { challenge }) => {
                    last_heard = Instant::now();
                    if conn.on_challenge(challenge) {
                        match conn.authenticate().await {
                            Ok(()) => {
                                for idx in std::mem::take(&mut awaiting_auth) {
                                    resent.insert(idx);
                                    let (id, v) = &live[&idx];
                                    if let Err(e) = conn.send(json!(["EVENT", v])).await {
                                        queue.push_front(idx);
                                        dropped = Some(e);
                                        break;
                                    }
                                    inflight.insert(id.clone(), idx);
                                }
                                if dropped.is_some() {
                                    break;
                                }
                            }
                            Err(f) => {
                                for idx in std::mem::take(&mut awaiting_auth) {
                                    let r = PublishResult::failed(relay_url, live[&idx].0.clone(), f.message(), Some(Refusal::AuthRequired), false);
                                    finish(&mut results, idx, r);
                                }
                            }
                        }
                    }
                }
                Frame::Msg(RelayMessage::Notice { message }) => {
                    last_heard = Instant::now();
                    warn!("Relay notice from {}: {}", relay_url, message);
                }
                Frame::Msg(_) => {
                    last_heard = Instant::now();
                }
                Frame::Idle => {
                    if !inflight.is_empty() && last_heard.elapsed() >= PUBLISH_IDLE {
                        // A whole window went unanswered. A relay that has
                        // never sent OK doesn't send them: stop waiting per
                        // window. Either way these stay unconfirmed.
                        if !ever_ok && !fire_and_forget {
                            debug!("{relay_url} sends no OK — publishing the rest without waiting");
                            fire_and_forget = true;
                        }
                        for (id, idx) in inflight.drain() {
                            let r = PublishResult::unconfirmed(relay_url, id, conn.authed);
                            finish(&mut results, idx, r);
                        }
                        last_heard = Instant::now();
                    }
                }
                Frame::Ended => {
                    dropped = Some("connection closed".into());
                    break;
                }
                Frame::Error(e) => {
                    dropped = Some(format!("WebSocket error: {e}"));
                    break;
                }
            }
        }

        authed_any |= conn.authed;
        for idx in std::mem::take(&mut awaiting_auth) {
            let r = PublishResult::failed(relay_url, live[&idx].0.clone(), "auth-required: relay sent no AUTH challenge".into(), Some(Refusal::AuthRequired), false);
            finish(&mut results, idx, r);
        }

        if let Some(e) = dropped {
            // Re-send what the dropped socket never answered, in order.
            let mut back: Vec<usize> = inflight.drain().map(|(_, i)| i).collect();
            back.sort_unstable();
            for idx in back.into_iter().rev() {
                queue.push_front(idx);
            }
            reconnects += 1;
            if reconnects > PUBLISH_RECONNECTS {
                warn!("Failed to publish to {}: {} ({} events unsent)", relay_url, e, queue.len());
                last_error = Some(e);
                break 'conn;
            }
            warn!("{relay_url}: {e} mid-batch — reconnecting ({reconnects}/{PUBLISH_RECONNECTS}), {} events left", queue.len());
            tokio::time::sleep(Duration::from_secs(reconnects as u64)).await;
            continue 'conn;
        }

        // Stalled with events still unanswered.
        for (id, idx) in inflight.drain() {
            let r = PublishResult::unconfirmed(relay_url, id, conn.authed);
            finish(&mut results, idx, r);
        }
        conn.close().await;
        if !queue.is_empty() {
            last_error = Some(format!(
                "stalled — nothing accepted for {}s (pace {})",
                last_progress.elapsed().as_secs(),
                rate_label(gap)
            ));
            break;
        }
    }

    let err = last_error.unwrap_or_else(|| "not sent".into());
    for idx in std::mem::take(&mut queue) {
        let r = PublishResult::failed(relay_url, live[&idx].0.clone(), err.clone(), None, authed_any);
        finish(&mut results, idx, r);
    }

    let out: Vec<PublishResult> = results.into_iter().map(|r| r.expect("every event finished")).collect();
    let ok = out.iter().filter(|r| r.success && !r.unconfirmed).count();
    let unconfirmed = out.iter().filter(|r| r.unconfirmed).count();
    let failed = out.len() - ok - unconfirmed;
    if failed == 0 && unconfirmed == 0 {
        info!("Published {} event(s) to {}", ok, relay_url);
    } else {
        let first = out.iter().find(|r| !r.success).and_then(|r| r.message.clone()).unwrap_or_default();
        warn!(
            "Published to {}: {} accepted, {} unconfirmed, {} failed{}",
            relay_url,
            ok,
            unconfirmed,
            failed,
            if first.is_empty() { String::new() } else { format!(" (first: {first})") }
        );
    }
    out
}

/// `rate-limited:` (NIP-01 machine-readable prefix), any case.
fn is_rate_limited(message: &str) -> bool {
    let m = message.trim_start();
    m.len() >= 13 && m[..13].eq_ignore_ascii_case("rate-limited:")
}

/// Read events back from a relay by id — what it actually stored, not what
/// it said. Ingests what comes back (recording the relay's provenance) and
/// returns the ids found. Ids go 100 per filter, 5 filters per REQ.
pub async fn verify_on_relay(ndb: &Ndb, relay_url: &str, ids: &[String], auth: &RelayAuth) -> Result<HashSet<String>> {
    let mut found = HashSet::new();
    for group in ids.chunks(500) {
        let filters: Vec<Value> = group
            .chunks(100)
            .map(|c| json!({ "ids": c, "limit": c.len() }))
            .collect();
        let outcome = fetch_with_filters(ndb, relay_url, &filters, auth).await?;
        for ev in outcome.events {
            if let Some(id) = ev.get("id").and_then(Value::as_str) {
                found.insert(id.to_ascii_lowercase());
            }
        }
    }
    Ok(found)
}

/// Publish an event to a single relay.
/// Returns success/failure with any message from the relay.
pub async fn publish_event(relay_url: &str, event_json: &str, auth: &RelayAuth) -> PublishResult {
    publish_batch(relay_url, std::slice::from_ref(&event_json.to_string()), auth)
        .await
        .pop()
        .expect("one result per event")
}

/// Relays contacted concurrently by the multi-relay publishers.
const MAX_CONCURRENT_RELAYS: usize = 8;

/// Publish an event to multiple relays. Results are in relay order.
pub async fn publish_to_relays(relays: &[String], event_json: &str, auth: &RelayAuth) -> Vec<PublishResult> {
    let (_, _, results) = publish_events_to_relays(relays, std::slice::from_ref(&event_json.to_string()), auth).await;
    results
}

/// Progress update for broadcast operations
#[derive(Debug, Clone)]
pub struct BroadcastProgress {
    pub current_relay: usize,
    pub total_relays: usize,
    pub current_event: usize,
    pub total_events: usize,
    pub relay_name: String,
    pub status: String,
}

/// Publish multiple events to multiple relays (for publication + sections)
/// Returns (successful_relay_count, total_relay_count, details)
pub async fn publish_events_to_relays(
    relays: &[String],
    events: &[String],
    auth: &RelayAuth,
) -> (usize, usize, Vec<PublishResult>) {
    publish_events_to_relays_with_progress(relays, events, auth, |_| {}).await
}

/// Publish multiple events to multiple relays with progress callback.
///
/// One socket per relay carries the whole batch (see [`publish_batch`]);
/// relays run concurrently, capped, so total wall-time stays near the
/// slowest single relay. Results are grouped by relay, in relay order.
pub async fn publish_events_to_relays_with_progress<F>(
    relays: &[String],
    events: &[String],
    auth: &RelayAuth,
    mut on_progress: F,
) -> (usize, usize, Vec<PublishResult>)
where
    F: FnMut(BroadcastProgress),
{
    let total_relays = relays.len();
    let total_events = events.len();

    // Owned per-task inputs: an `&String` iterator item captured by the
    // async block trips rustc's higher-ranked `Send` inference (the
    // "`Send` is not general enough" error at the agent SSE boxing site).
    // Per-event results stream back while the relays run, so the caller can
    // drive a progress bar over (relay, event) pairs.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, bool)>();
    // Built eagerly so every sender clone exists before the original drops —
    // the channel then closes exactly when the last relay finishes.
    let tasks: Vec<_> = (0..total_relays)
        .map(|ri| {
            let relay_url = relays[ri].clone();
            let events = events.to_vec();
            let auth = auth.clone();
            let tx = tx.clone();
            async move {
                let cb = move |e: PublishEvent<'_>| {
                    if let PublishEvent::Result(r) = e {
                        let _ = tx.send((ri, r.success && !r.unconfirmed));
                    }
                };
                let results = publish_batch_with(&relay_url, &events, &auth, &cb).await;
                (ri, results)
            }
        })
        .collect();
    drop(tx);
    let mut stream = futures::stream::iter(tasks).buffer_unordered(MAX_CONCURRENT_RELAYS);

    let mut per_relay: Vec<Option<Vec<PublishResult>>> = (0..total_relays).map(|_| None).collect();
    let mut done_pairs = 0;
    let mut confirmed = 0;
    let total_pairs = total_relays * total_events;
    let mut relays_done = 0;
    loop {
        tokio::select! {
            Some((ri, ok)) = rx.recv() => {
                done_pairs += 1;
                if ok { confirmed += 1; }
                on_progress(BroadcastProgress {
                    current_relay: relays_done,
                    total_relays,
                    current_event: done_pairs,
                    total_events: total_pairs,
                    relay_name: relays[ri].clone(),
                    status: format!("{done_pairs}/{total_pairs} sent, {confirmed} accepted"),
                });
            }
            next = stream.next() => match next {
                Some((ri, results)) => {
                    relays_done += 1;
                    per_relay[ri] = Some(results);
                }
                None => {
                    // Drain any progress still queued, then stop.
                    while let Ok((ri, ok)) = rx.try_recv() {
                        done_pairs += 1;
                        if ok { confirmed += 1; }
                        on_progress(BroadcastProgress {
                            current_relay: relays_done,
                            total_relays,
                            current_event: done_pairs,
                            total_events: total_pairs,
                            relay_name: relays[ri].clone(),
                            status: format!("{done_pairs}/{total_pairs} sent, {confirmed} accepted"),
                        });
                    }
                    break;
                }
            }
        }
    }

    // A relay counts as a success only if every one of its events landed.
    let mut all_results = Vec::with_capacity(total_pairs);
    let mut success_count = 0;
    for results in per_relay.into_iter().flatten() {
        if !results.is_empty() && results.iter().all(|r| r.success) {
            success_count += 1;
        }
        all_results.extend(results);
    }
    (success_count, total_relays, all_results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing::InProcessSigner;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    /// A relay that accepts the TCP connection but never answers the
    /// WebSocket upgrade — the hermetic stand-in for a blackholed
    #[test]
    fn relay_permits_are_counted_and_released() {
        // Bookkeeping only — the gate itself stays off so the socket tests
        // beside this one are unaffected.
        let url = "wss://permit-test.example".to_string();
        let count = || {
            PERMITS
                .lock()
                .unwrap()
                .iter()
                .find(|(u, _)| *u == permit_key(&url))
                .map(|(_, n)| *n)
                .unwrap_or(0)
        };
        let a = permit(std::slice::from_ref(&url));
        let b = permit(&["wss://PERMIT-TEST.example/".to_string()]);
        assert_eq!(count(), 2, "normalized URLs share a slot");
        drop(a);
        assert_eq!(count(), 1);
        drop(b);
        assert_eq!(count(), 0);
    }

    /// connect (the on-device failure mode this timeout exists for).
    /// Without CONNECT_TIMEOUT, connect_async waits on it forever.
    #[tokio::test]
    async fn connect_bounded_times_out_on_silent_peer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        // Keep sockets accepted-but-mute for the duration of the test.
        let server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });

        let start = Instant::now();
        let result = connect_bounded(&format!("ws://{}", addr)).await;
        let elapsed = start.elapsed();

        let err = result.err().expect("silent peer must not connect");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        // Bounded by the timeout (small slack for scheduling), not the
        // OS TCP timeout.
        assert!(elapsed < CONNECT_TIMEOUT + Duration::from_secs(2));
        server.abort();
    }

    // ---- NIP-42 mock relay -------------------------------------------------

    type ServerWs = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

    fn test_keypair() -> (String, String) {
        let secret_hex =
            "0000000000000000000000000000000000000000000000000000000000000003".to_string();
        let pubkey_hex =
            crate::identity::derive_pubkey_from_secret(&secret_hex).expect("derive test pubkey");
        (pubkey_hex, secret_hex)
    }

    fn auth_for(url: &str) -> (RelayAuth, String) {
        let (pk, sk) = test_keypair();
        let signer = AuthSigner::new(Arc::new(InProcessSigner::new(pk.clone(), sk)));
        let mut allowed = HashSet::new();
        allowed.insert(crate::relay_url::normalize_relay_url(url));
        (
            RelayAuth {
                signer: Some(signer),
                allowed,
            },
            pk,
        )
    }

    /// Signer present but this relay is not opted in.
    fn auth_elsewhere() -> RelayAuth {
        let (pk, sk) = test_keypair();
        let signer = AuthSigner::new(Arc::new(InProcessSigner::new(pk, sk)));
        let mut allowed = HashSet::new();
        allowed.insert("wss://some.other.relay".to_string());
        RelayAuth {
            signer: Some(signer),
            allowed,
        }
    }

    async fn serve_one<F, Fut>(f: F) -> String
    where
        F: FnOnce(ServerWs) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(sock).await.unwrap();
            f(ws).await;
        });
        format!("ws://{}", addr)
    }

    async fn recv(ws: &mut ServerWs) -> Vec<Value> {
        loop {
            match timeout(Duration::from_secs(5), ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => {
                    eprintln!("mock relay <- {}", &t[..t.len().min(120)]);
                    return serde_json::from_str(&t).unwrap();
                }
                Ok(Some(Ok(_))) => continue,
                other => panic!("mock relay: unexpected {:?}", other.map(|o| o.map(|r| r.is_ok()))),
            }
        }
    }

    async fn send(ws: &mut ServerWs, v: Value) {
        eprintln!("mock relay -> {}", v);
        ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }

    fn temp_ndb() -> (tempfile::TempDir, Ndb) {
        let tmp = tempfile::TempDir::new().unwrap();
        let ndb = Ndb::new(tmp.path().to_str().unwrap(), &nostrdb::Config::new()).unwrap();
        (tmp, ndb)
    }

    /// Assert an AUTH frame is a valid kind-22242 for (relay, challenge)
    /// by `pk`, and return its id.
    fn check_auth_event(frame: &[Value], expect_challenge: &str, expect_relay: &str, pk: &str) -> String {
        assert_eq!(frame[0], "AUTH");
        let ev = &frame[1];
        crate::signing::verify_signed_event(ev).expect("AUTH event must verify");
        assert_eq!(ev["kind"], crate::nip42::KIND_AUTH);
        assert_eq!(ev["pubkey"], pk);
        let tags = ev["tags"].as_array().unwrap();
        let tag = |name: &str| {
            tags.iter()
                .find(|t| t[0] == name)
                .map(|t| t[1].as_str().unwrap().to_string())
        };
        assert_eq!(tag("challenge").as_deref(), Some(expect_challenge));
        assert_eq!(tag("relay").as_deref(), Some(expect_relay));
        ev["id"].as_str().unwrap().to_string()
    }

    fn fake_event(n: u32) -> Value {
        json!({
            "id": format!("{:064x}", n),
            "pubkey": format!("{:064x}", 0xabcu32),
            "created_at": 1_700_000_000,
            "kind": 1,
            "tags": [],
            "content": format!("note {n}"),
            "sig": format!("{:0128x}", n),
        })
    }

    #[tokio::test]
    async fn fetch_challenge_before_req_authenticates_when_allowed() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            // Proactive challenge, before the client's REQ.
            send(&mut ws, json!(["AUTH", "chal-early"])).await;
            let req = recv(&mut ws).await;
            assert_eq!(req[0], "REQ");
            let sub = req[1].as_str().unwrap().to_string();
            let auth = recv(&mut ws).await;
            let id = auth[1]["id"].as_str().unwrap().to_string();
            assert_eq!(auth[1]["tags"][1][1], "chal-early");
            send(&mut ws, json!(["OK", id, true, ""])).await;
            send(&mut ws, json!(["EVENT", sub, fake_event(1)])).await;
            send(&mut ws, json!(["EOSE", sub])).await;
        })
        .await;
        let (auth, _) = auth_for(&url);
        let out = fetch_with_filters(&ndb, &url, &[json!({"kinds": [1]})], &auth)
            .await
            .unwrap();
        assert_eq!(out.events.len(), 1);
        assert!(out.authenticated);
    }

    #[tokio::test]
    async fn fetch_closed_auth_required_then_re_req() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            let req = recv(&mut ws).await;
            let sub1 = req[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["AUTH", "chal-1"])).await;
            send(&mut ws, json!(["CLOSED", sub1, "auth-required: members only"])).await;
            // The client answers the challenge (it may have answered the
            // proactive AUTH already, before seeing CLOSED — either way
            // exactly one AUTH frame arrives).
            let auth = recv(&mut ws).await;
            let id = auth[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, true, ""])).await;
            let req2 = recv(&mut ws).await;
            assert_eq!(req2[0], "REQ");
            let sub2 = req2[1].as_str().unwrap().to_string();
            assert_ne!(sub1, sub2, "re-REQ must use a fresh sub id");
            send(&mut ws, json!(["EVENT", sub2, fake_event(2)])).await;
            send(&mut ws, json!(["EVENT", sub2, fake_event(3)])).await;
            send(&mut ws, json!(["EOSE", sub2])).await;
        })
        .await;
        let (auth, pk) = auth_for(&url);
        let out = fetch_with_filters(&ndb, &url, &[json!({"kinds": [1]})], &auth)
            .await
            .unwrap();
        assert_eq!(out.events.len(), 2);
        assert!(out.authenticated);
        // Nothing of kind 22242 was ingested locally.
        let txn = nostrdb::Transaction::new(&ndb).unwrap();
        let f = nostrdb::Filter::new().kinds([22242]).authors([&hex::decode(&pk).unwrap().try_into().unwrap()]).build();
        let res = ndb.query(&txn, &[f], 10).unwrap();
        assert!(res.is_empty(), "AUTH events must never be stored");
    }

    #[tokio::test]
    async fn fetch_closed_before_challenge_waits_for_auth_frame() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            let req = recv(&mut ws).await;
            let sub1 = req[1].as_str().unwrap().to_string();
            // CLOSED first, challenge after — allowed by the spec.
            send(&mut ws, json!(["CLOSED", sub1, "auth-required: nope"])).await;
            send(&mut ws, json!(["AUTH", "chal-late"])).await;
            let auth = recv(&mut ws).await;
            assert_eq!(auth[1]["tags"][1][1], "chal-late");
            let id = auth[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, true, ""])).await;
            let req2 = recv(&mut ws).await;
            let sub2 = req2[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["EOSE", sub2])).await;
        })
        .await;
        let (auth, _) = auth_for(&url);
        let out = fetch_with_filters(&ndb, &url, &[json!({"kinds": [1]})], &auth)
            .await
            .unwrap();
        assert!(out.authenticated);
    }

    #[tokio::test]
    async fn fetch_not_allowed_ignores_challenge_and_reports_auth_required() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal-x"])).await;
            let req = recv(&mut ws).await;
            let sub = req[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["CLOSED", sub, "auth-required: members only"])).await;
            // Must NOT receive an AUTH; the client closes instead.
            match timeout(Duration::from_secs(3), ws.next()).await {
                Ok(Some(Ok(Message::Text(t)))) => {
                    assert!(!t.starts_with("[\"AUTH\""), "client must not authenticate: {t}")
                }
                _ => {}
            }
        })
        .await;
        let auth = auth_elsewhere();
        let err = fetch_with_filters(&ndb, &url, &[json!({"kinds": [1]})], &auth)
            .await
            .err()
            .expect("must surface auth-required");
        match err {
            EngineError::RelayAuthRequired { relay, msg } => {
                assert_eq!(relay, url);
                assert!(msg.starts_with("auth-required:"), "{msg}");
            }
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn fetch_no_signer_degrades_to_auth_required() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            let req = recv(&mut ws).await;
            let sub = req[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["AUTH", "chal"])).await;
            send(&mut ws, json!(["CLOSED", sub, "auth-required: x"])).await;
        })
        .await;
        let err = fetch_with_filters(&ndb, &url, &[json!({})], &RelayAuth::none())
            .await
            .err()
            .unwrap();
        assert!(matches!(err, EngineError::RelayAuthRequired { .. }));
    }

    #[tokio::test]
    async fn fetch_proactive_challenge_on_public_relay_is_ignored_and_data_still_served() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal-public"])).await;
            let req = recv(&mut ws).await;
            let sub = req[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["EVENT", sub, fake_event(7)])).await;
            send(&mut ws, json!(["EOSE", sub])).await;
            // The next frame from the client must be CLOSE, not AUTH.
            let next = recv(&mut ws).await;
            assert_eq!(next[0], "CLOSE");
        })
        .await;
        let out = fetch_with_filters(&ndb, &url, &[json!({})], &auth_elsewhere())
            .await
            .unwrap();
        assert_eq!(out.events.len(), 1);
        assert!(!out.authenticated);
    }

    #[tokio::test]
    async fn fetch_restricted_after_auth_is_terminal() {
        let (_tmp, ndb) = temp_ndb();
        // Authenticated, but not a member: the relay refuses the sub with
        // `restricted:` after accepting our AUTH.
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal"])).await;
            let req = recv(&mut ws).await;
            let sub = req[1].as_str().unwrap().to_string();
            let auth = recv(&mut ws).await;
            let id = auth[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, true, ""])).await;
            send(&mut ws, json!(["CLOSED", sub, "restricted: not a member"])).await;
        })
        .await;
        let (auth, _) = auth_for(&url);
        let err = fetch_with_filters(&ndb, &url, &[json!({})], &auth)
            .await
            .err()
            .unwrap();
        match err {
            EngineError::Relay(msg) => assert!(msg.contains("restricted:"), "{msg}"),
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn fetch_auth_ok_false_fails_once_without_looping() {
        let (_tmp, ndb) = temp_ndb();
        let url = serve_one(|mut ws| async move {
            let req = recv(&mut ws).await;
            let sub = req[1].as_str().unwrap().to_string();
            send(&mut ws, json!(["AUTH", "chal"])).await;
            send(&mut ws, json!(["CLOSED", sub, "auth-required: x"])).await;
            let auth = recv(&mut ws).await;
            let id = auth[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, false, "invalid: bad challenge"])).await;
            // No second AUTH may arrive.
            if let Ok(Some(Ok(Message::Text(t)))) = timeout(Duration::from_secs(2), ws.next()).await {
                assert!(!t.starts_with("[\"AUTH\""), "client looped on AUTH: {t}");
            }
        })
        .await;
        let (auth, _) = auth_for(&url);
        let err = fetch_with_filters(&ndb, &url, &[json!({})], &auth)
            .await
            .err()
            .unwrap();
        match err {
            EngineError::RelayAuthRequired { msg, .. } => assert!(msg.contains("rejected"), "{msg}"),
            other => panic!("wrong error: {other}"),
        }
    }

    #[tokio::test]
    async fn publish_ok_false_auth_required_then_re_event() {
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal-pub"])).await;
            // The client may answer the proactive AUTH before or after
            // sending its EVENT — accept either order.
            let mut ev_id: Option<String> = None;
            let mut auth_id: Option<String> = None;
            for _ in 0..2 {
                let f = recv(&mut ws).await;
                match f[0].as_str().unwrap() {
                    "EVENT" => ev_id = Some(f[1]["id"].as_str().unwrap().to_string()),
                    "AUTH" => auth_id = Some(f[1]["id"].as_str().unwrap().to_string()),
                    other => panic!("unexpected {other}"),
                }
            }
            let ev_id = ev_id.unwrap();
            send(&mut ws, json!(["OK", ev_id, false, "auth-required: registered users only"])).await;
            let auth_id = match auth_id {
                Some(a) => a,
                None => {
                    let f = recv(&mut ws).await;
                    assert_eq!(f[0], "AUTH");
                    f[1]["id"].as_str().unwrap().to_string()
                }
            };
            send(&mut ws, json!(["OK", auth_id, true, ""])).await;
            let f = recv(&mut ws).await;
            assert_eq!(f[0], "EVENT");
            assert_eq!(f[1]["id"], ev_id);
            send(&mut ws, json!(["OK", ev_id, true, ""])).await;
        })
        .await;
        let (auth, _) = auth_for(&url);
        let ev = fake_event(11).to_string();
        let r = publish_event(&url, &ev, &auth).await;
        assert!(r.success, "{:?}", r.message);
        assert!(r.authenticated);
        assert_eq!(r.event_id, fake_event(11)["id"]);
    }

    #[tokio::test]
    async fn publish_not_allowed_reports_refusal_without_authenticating() {
        let url = serve_one(|mut ws| async move {
            let f = recv(&mut ws).await;
            assert_eq!(f[0], "EVENT");
            let id = f[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["AUTH", "chal"])).await;
            send(&mut ws, json!(["OK", id, false, "auth-required: no"])).await;
            if let Ok(Some(Ok(Message::Text(t)))) = timeout(Duration::from_secs(2), ws.next()).await {
                assert!(!t.starts_with("[\"AUTH\""), "client must not authenticate: {t}");
            }
        })
        .await;
        let r = publish_event(&url, &fake_event(12).to_string(), &auth_elsewhere()).await;
        assert!(!r.success);
        assert_eq!(r.refusal, Some(Refusal::AuthRequired));
        assert!(!r.authenticated);
    }

    #[tokio::test]
    async fn publish_batch_authenticates_once_for_many_events() {
        let url = serve_one(|mut ws| async move {
            // Three EVENTs arrive first.
            let mut ids = Vec::new();
            for _ in 0..3 {
                let f = recv(&mut ws).await;
                assert_eq!(f[0], "EVENT");
                ids.push(f[1]["id"].as_str().unwrap().to_string());
            }
            send(&mut ws, json!(["AUTH", "chal-batch"])).await;
            for id in &ids {
                send(&mut ws, json!(["OK", id, false, "auth-required: x"])).await;
            }
            // Exactly one AUTH, then the three re-sent EVENTs.
            let f = recv(&mut ws).await;
            assert_eq!(f[0], "AUTH");
            let auth_id = f[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", auth_id, true, ""])).await;
            for _ in 0..3 {
                let f = recv(&mut ws).await;
                assert_eq!(f[0], "EVENT", "expected re-sent EVENT, got {f:?}");
                let id = f[1]["id"].as_str().unwrap().to_string();
                send(&mut ws, json!(["OK", id, true, ""])).await;
            }
        })
        .await;
        let (auth, _) = auth_for(&url);
        let events: Vec<String> = (20..23).map(|n| fake_event(n).to_string()).collect();
        let results = publish_batch(&url, &events, &auth).await;
        assert_eq!(results.len(), 3);
        for (r, n) in results.iter().zip(20..23) {
            assert!(r.success, "{:?}", r.message);
            assert!(r.authenticated);
            assert_eq!(r.event_id, fake_event(n)["id"]);
        }
    }

    /// Accept `n` connections in turn, handing each to `f(conn_index, ws)`.
    async fn serve_many<F, Fut>(n: usize, f: F) -> String
    where
        F: Fn(usize, ServerWs) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for i in 0..n {
                let (sock, _) = listener.accept().await.unwrap();
                let ws = tokio_tungstenite::accept_async(sock).await.unwrap();
                f(i, ws).await;
            }
        });
        format!("ws://{}", addr)
    }

    #[tokio::test]
    async fn publish_batch_keeps_a_window_of_ten() {
        let url = serve_one(|mut ws| async move {
            let mut answered = 0;
            let mut outstanding: Vec<String> = Vec::new();
            while answered < 25 {
                // Read until the client stops sending (window full or done).
                loop {
                    match timeout(Duration::from_millis(400), ws.next()).await {
                        Ok(Some(Ok(Message::Text(t)))) => {
                            let f: Vec<Value> = serde_json::from_str(&t).unwrap();
                            assert_eq!(f[0], "EVENT");
                            outstanding.push(f[1]["id"].as_str().unwrap().to_string());
                            assert!(outstanding.len() <= PUBLISH_WINDOW, "window exceeded: {}", outstanding.len());
                        }
                        Ok(Some(Ok(_))) => {}
                        _ => break,
                    }
                }
                for id in outstanding.drain(..) {
                    send(&mut ws, json!(["OK", id, true, ""])).await;
                    answered += 1;
                }
            }
        })
        .await;
        let events: Vec<String> = (100..125).map(|n| fake_event(n).to_string()).collect();
        let progress = std::sync::atomic::AtomicUsize::new(0);
        let results = publish_batch_with(&url, &events, &RelayAuth::none(), &|e| {
            if let PublishEvent::Result(_) = e {
                progress.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
        .await;
        assert_eq!(results.len(), 25);
        assert!(results.iter().all(|r| r.success && !r.unconfirmed));
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 25);
    }

    #[tokio::test]
    async fn publish_batch_retries_rate_limited() {
        let url = serve_one(|mut ws| async move {
            let f = recv(&mut ws).await;
            let id = f[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, false, "rate-limited: slow down"])).await;
            let f = recv(&mut ws).await;
            assert_eq!(f[0], "EVENT");
            assert_eq!(f[1]["id"], id);
            send(&mut ws, json!(["OK", id, true, ""])).await;
        })
        .await;
        let r = publish_event(&url, &fake_event(140).to_string(), &RelayAuth::none()).await;
        assert!(r.success && !r.unconfirmed, "{:?}", r.message);
    }

    #[tokio::test]
    async fn publish_batch_slows_down_for_a_rate_limiting_relay_and_remembers() {
        // Accepts at most one event per 250 ms; anything faster is refused.
        let url = serve_one(|mut ws| async move {
            let mut last_ok: Option<Instant> = None;
            let mut accepted = 0;
            while accepted < 15 {
                let f = recv(&mut ws).await;
                let id = f[1]["id"].clone();
                if last_ok.map_or(true, |t| t.elapsed() >= Duration::from_millis(250)) {
                    last_ok = Some(Instant::now());
                    accepted += 1;
                    send(&mut ws, json!(["OK", id, true, ""])).await;
                } else {
                    send(&mut ws, json!(["OK", id, false, "rate-limited: slow down"])).await;
                }
            }
        })
        .await;
        let paces = std::sync::Mutex::new(Vec::new());
        let events: Vec<String> = (300..315).map(|n| fake_event(n).to_string()).collect();
        let results = publish_batch_with(&url, &events, &RelayAuth::none(), &|e| {
            if let PublishEvent::Pace { gap, rate_limited: true } = e {
                paces.lock().unwrap().push(gap);
            }
        })
        .await;
        assert!(results.iter().all(|r| r.success && !r.unconfirmed), "{:?}", results.iter().map(|r| &r.message).collect::<Vec<_>>());
        assert!(!paces.lock().unwrap().is_empty(), "the batch should have slowed down");
        assert!(learned_gap(&url) > PUBLISH_GAP, "the slower pace is remembered for this relay");
    }

    #[tokio::test]
    async fn publish_batch_reconnects_and_resumes_after_a_drop() {
        let url = serve_many(2, |i, mut ws| async move {
            if i == 0 {
                // Accept three, then hang up with the rest unanswered.
                for _ in 0..3 {
                    let f = recv(&mut ws).await;
                    send(&mut ws, json!(["OK", f[1]["id"], true, ""])).await;
                }
                let _ = ws.close(None).await;
            } else {
                loop {
                    match timeout(Duration::from_secs(3), ws.next()).await {
                        Ok(Some(Ok(Message::Text(t)))) => {
                            let f: Vec<Value> = serde_json::from_str(&t).unwrap();
                            send(&mut ws, json!(["OK", f[1]["id"], true, ""])).await;
                        }
                        Ok(Some(Ok(_))) => {}
                        _ => break,
                    }
                }
            }
        })
        .await;
        let events: Vec<String> = (150..162).map(|n| fake_event(n).to_string()).collect();
        let results = publish_batch(&url, &events, &RelayAuth::none()).await;
        assert_eq!(results.len(), 12);
        assert!(results.iter().all(|r| r.success && !r.unconfirmed), "{:?}", results.iter().map(|r| &r.message).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn publish_batch_marks_a_silent_relay_unconfirmed_and_sends_everything() {
        let (tx, rx) = tokio::sync::oneshot::channel::<usize>();
        let url = serve_one(|mut ws| async move {
            let mut n = 0;
            while let Ok(Some(Ok(Message::Text(_)))) = timeout(Duration::from_secs(8), ws.next()).await {
                n += 1;
            }
            let _ = tx.send(n);
        })
        .await;
        let events: Vec<String> = (170..195).map(|n| fake_event(n).to_string()).collect();
        let results = publish_batch(&url, &events, &RelayAuth::none()).await;
        assert!(results.iter().all(|r| r.success && r.unconfirmed));
        assert_eq!(rx.await.unwrap(), 25, "every event must still be sent");
    }

    #[tokio::test]
    async fn publish_restricted_after_auth_is_reported() {
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal"])).await;
            let mut ev_id = None;
            let mut auth_id = None;
            for _ in 0..2 {
                let f = recv(&mut ws).await;
                match f[0].as_str().unwrap() {
                    "EVENT" => ev_id = Some(f[1]["id"].as_str().unwrap().to_string()),
                    "AUTH" => auth_id = Some(f[1]["id"].as_str().unwrap().to_string()),
                    _ => unreachable!(),
                }
            }
            send(&mut ws, json!(["OK", auth_id.unwrap(), true, ""])).await;
            send(&mut ws, json!(["OK", ev_id.unwrap(), false, "restricted: not a member"])).await;
        })
        .await;
        let (auth, _) = auth_for(&url);
        let r = publish_event(&url, &fake_event(30).to_string(), &auth).await;
        assert!(!r.success);
        assert_eq!(r.refusal, Some(Refusal::Restricted));
        assert!(r.authenticated);
    }

    #[tokio::test]
    async fn auth_event_carries_normalized_relay_url() {
        let (_tmp, ndb) = temp_ndb();
        let (tx, rx) = tokio::sync::oneshot::channel::<Vec<Value>>();
        let url = serve_one(|mut ws| async move {
            send(&mut ws, json!(["AUTH", "chal-url"])).await;
            let _req = recv(&mut ws).await;
            let auth = recv(&mut ws).await;
            let id = auth[1]["id"].as_str().unwrap().to_string();
            send(&mut ws, json!(["OK", id, true, ""])).await;
            let _ = tx.send(auth);
        })
        .await;
        // Trailing slash on the caller's URL — the tag must be normalized.
        let dirty = format!("{url}/");
        let (auth, pk) = auth_for(&dirty);
        let _ = fetch_with_filters(&ndb, &dirty, &[json!({})], &auth).await;
        let frame = rx.await.unwrap();
        check_auth_event(
            &frame,
            "chal-url",
            &crate::relay_url::normalize_relay_url(&dirty),
            &pk,
        );
    }
}
