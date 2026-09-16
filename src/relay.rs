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

/// Default per-relay resolve-kind claims (see `RelaySets::resolve_kinds`):
/// targeted resolution of these kinds routes to the claimant instead of
/// the read set. Mercury serves the publication/wiki kinds tendrl
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

/// Default indexer (profile / kind-10002 discovery) relays. Used to
/// seed `indexer.default` on first boot so a fresh install can fall
/// back from the read set without manual configuration. Well-known
/// purpose-built profile relays:
///   - purplepag.es: pubkey-indexed profile + relay-list mirror
///   - user.kindpag.es: kind-10002 / 10003 / 10006 / 10007 indexer
/// Users can clear them via the Discovery section in RelaysBuffer
/// if they prefer a tighter set; new entries persist to relays.json
/// after first save so they won't be re-seeded.
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

/// `connect_async` bounded by [`CONNECT_TIMEOUT`]; error is a plain
/// string so both the fetch path (`EngineError`) and the publish path
/// (`String`) can wrap it.
async fn connect_bounded(relay_url: &str) -> std::result::Result<WsStream, String> {
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
        let normalized = crate::relay_url::normalize_relay_url(url);
        let tag_url = if normalized.is_empty() {
            url.to_string()
        } else {
            normalized
        };
        Ok(Self {
            ws,
            url: url.to_string(),
            tag_url,
            auth,
            challenge: None,
            authed: false,
            auth_attempted: false,
            pending: VecDeque::new(),
        })
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
}

impl PublishResult {
    fn ok(relay_url: &str, event_id: String, message: Option<String>, authenticated: bool) -> Self {
        Self {
            relay_url: relay_url.to_string(),
            success: true,
            message,
            event_id,
            refusal: None,
            authenticated,
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
    // Parse up front; a malformed event fails on its own, the rest go.
    let mut results: Vec<Option<PublishResult>> = Vec::with_capacity(events.len());
    let mut ids: Vec<String> = Vec::with_capacity(events.len());
    // (index, id, event) for the events that parsed.
    let mut live: Vec<(usize, String, Value)> = Vec::with_capacity(events.len());
    for (i, json_text) in events.iter().enumerate() {
        match serde_json::from_str::<Value>(json_text) {
            Ok(v) => {
                let id = event_id_of(&v);
                ids.push(id.clone());
                live.push((i, id, v));
                results.push(None);
            }
            Err(e) => {
                ids.push(String::new());
                results.push(Some(PublishResult::failed(
                    relay_url,
                    String::new(),
                    format!("Invalid event JSON: {e}"),
                    None,
                    false,
                )));
            }
        }
    }

    if live.is_empty() {
        return finish_results(results, relay_url, &ids, &Ending::Failed("Invalid event JSON".into()), false);
    }

    let mut conn = match RelayConn::connect(relay_url, auth.clone()).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to publish to {}: {}", relay_url, e);
            return finish_results(results, relay_url, &ids, &Ending::Failed(e), false);
        }
    };

    // id → index of the event awaiting an OK.
    let mut pending: HashMap<String, usize> = HashMap::new();
    for (i, id, v) in &live {
        if let Err(e) = conn.send(json!(["EVENT", v])).await {
            warn!("Failed to publish to {}: {}", relay_url, e);
            conn.close().await;
            return finish_results(results, relay_url, &ids, &Ending::Failed(e), false);
        }
        pending.insert(id.clone(), *i);
    }
    // Events the relay refused with auth-required while we had no
    // challenge yet — re-sent once the AUTH round-trip completes.
    let mut awaiting_auth: Vec<usize> = Vec::new();
    // Events already re-sent after AUTH; a second auth-required for one of
    // these is final.
    let mut resent: HashSet<usize> = HashSet::new();

    let total_budget = Duration::from_secs(10) + Duration::from_millis(500) * live.len() as u32;
    let total_budget = total_budget.min(Duration::from_secs(60));
    let start = Instant::now();
    let mut ending = Ending::Budget;

    while !pending.is_empty() && start.elapsed() < total_budget {
        match conn.next(PUBLISH_IDLE).await {
            Frame::Msg(RelayMessage::Ok {
                id,
                accepted,
                message,
            }) => {
                let Some(idx) = pending.remove(&id) else {
                    continue;
                };
                if accepted {
                    results[idx] = Some(PublishResult::ok(
                        relay_url,
                        id,
                        (!message.is_empty()).then_some(message),
                        conn.authed,
                    ));
                    continue;
                }
                match Refusal::classify(&message) {
                    Refusal::AuthRequired if conn.authed && !resent.contains(&idx) => {
                        // Refused before our AUTH landed (the relay
                        // challenged and refused in one breath) — re-send.
                        resent.insert(idx);
                        if let Err(e) = resend_event(&mut conn, &live, idx, &mut pending).await {
                            ending = Ending::Failed(e);
                            break;
                        }
                    }
                    Refusal::AuthRequired if conn.permits() && !conn.authed && !conn.auth_attempted => {
                        if conn.challenge.is_some() {
                            match conn.authenticate().await {
                                Ok(()) => {
                                    resent.insert(idx);
                                    if let Err(e) = resend_event(&mut conn, &live, idx, &mut pending).await {
                                        ending = Ending::Failed(e);
                                        break;
                                    }
                                }
                                Err(f) => {
                                    results[idx] = Some(PublishResult::failed(
                                        relay_url,
                                        id,
                                        f.message(),
                                        Some(Refusal::AuthRequired),
                                        false,
                                    ));
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
                        results[idx] = Some(PublishResult::failed(relay_url, id, message, refusal, conn.authed));
                    }
                }
            }
            Frame::Msg(RelayMessage::Auth { challenge }) => {
                if conn.on_challenge(challenge) {
                    match conn.authenticate().await {
                        Ok(()) => {
                            let mut failed = None;
                            for idx in std::mem::take(&mut awaiting_auth) {
                                resent.insert(idx);
                                if let Err(e) = resend_event(&mut conn, &live, idx, &mut pending).await {
                                    failed = Some(e);
                                    break;
                                }
                            }
                            if let Some(e) = failed {
                                ending = Ending::Failed(e);
                                break;
                            }
                        }
                        Err(f) => {
                            for idx in std::mem::take(&mut awaiting_auth) {
                                results[idx] = Some(PublishResult::failed(
                                    relay_url,
                                    ids[idx].clone(),
                                    f.message(),
                                    Some(Refusal::AuthRequired),
                                    false,
                                ));
                            }
                        }
                    }
                }
            }
            Frame::Msg(RelayMessage::Notice { message }) => {
                warn!("Relay notice from {}: {}", relay_url, message);
            }
            Frame::Msg(_) => {}
            Frame::Idle => {
                ending = Ending::Silent;
                break;
            }
            Frame::Ended => {
                ending = Ending::Failed("Connection closed".into());
                break;
            }
            Frame::Error(e) => {
                ending = Ending::Failed(format!("WebSocket error: {e}"));
                break;
            }
        }
    }

    // Events refused with auth-required while we waited for a challenge
    // that never came.
    for idx in awaiting_auth {
        pending.retain(|_, i| *i != idx);
        results[idx] = Some(PublishResult::failed(
            relay_url,
            ids[idx].clone(),
            "auth-required: relay sent no AUTH challenge".into(),
            Some(Refusal::AuthRequired),
            false,
        ));
    }

    let authed = conn.authed;
    conn.close().await;

    let out = finish_results(results, relay_url, &ids, &ending, authed);
    for r in &out {
        if r.success {
            info!("Published event to {}", relay_url);
        } else {
            warn!(
                "Failed to publish to {}: {}",
                relay_url,
                r.message.as_deref().unwrap_or("rejected")
            );
        }
    }
    out
}

/// How a publish loop stopped — decides the verdict for events the relay
/// never answered.
enum Ending {
    /// Socket died / send failed: unanswered events failed with this message.
    Failed(String),
    /// Relay went quiet. Some relays don't send OK — assume success
    /// (pre-existing behaviour, kept so silent relays don't read as failures).
    Silent,
    /// Ran out of total budget with OKs still outstanding.
    Budget,
}

fn finish_results(
    results: Vec<Option<PublishResult>>,
    relay_url: &str,
    ids: &[String],
    ending: &Ending,
    authenticated: bool,
) -> Vec<PublishResult> {
    results
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            r.unwrap_or_else(|| {
                let id = ids[i].clone();
                match ending {
                    Ending::Failed(e) => PublishResult::failed(relay_url, id, e.clone(), None, authenticated),
                    Ending::Silent => PublishResult::ok(
                        relay_url,
                        id,
                        Some("No response (assumed success)".into()),
                        authenticated,
                    ),
                    Ending::Budget => PublishResult::ok(
                        relay_url,
                        id,
                        Some("Timeout waiting for confirmation".into()),
                        authenticated,
                    ),
                }
            })
        })
        .collect()
}

/// Re-send a refused event after a successful AUTH.
async fn resend_event(
    conn: &mut RelayConn,
    live: &[(usize, String, Value)],
    idx: usize,
    pending: &mut HashMap<String, usize>,
) -> std::result::Result<(), String> {
    let (_, id, v) = live.iter().find(|(i, _, _)| *i == idx).expect("live index");
    conn.send(json!(["EVENT", v])).await?;
    pending.insert(id.clone(), idx);
    Ok(())
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
    let mut stream = futures::stream::iter((0..total_relays).map(|ri| {
        let relay_url = relays[ri].clone();
        let events = events.to_vec();
        let auth = auth.clone();
        async move {
            let results = publish_batch(&relay_url, &events, &auth).await;
            (ri, results)
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_RELAYS);

    let mut per_relay: Vec<Option<Vec<PublishResult>>> = (0..total_relays).map(|_| None).collect();
    let mut done_relays = 0;
    let mut done_pairs = 0;
    let total_pairs = total_relays * total_events;

    while let Some((ri, results)) = stream.next().await {
        done_relays += 1;
        done_pairs += results.len();
        on_progress(BroadcastProgress {
            current_relay: done_relays,
            total_relays,
            current_event: done_pairs,
            total_events,
            relay_name: relays[ri].clone(),
            status: format!("{done_pairs}/{total_pairs} sent"),
        });
        per_relay[ri] = Some(results);
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
