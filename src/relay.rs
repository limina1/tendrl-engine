//! WebSocket relay fetching
//!
//! Fetches events from Nostr relays and ingests them into nostrdb.

use crate::error::{EngineError, Result};
use futures::{SinkExt, StreamExt};
use nostrdb::{IngestMetadata, Ndb};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// Default relays to fetch from (focused on publication-supporting relays)
pub const DEFAULT_RELAYS: &[&str] = &[
    "ws://localhost:3334",
    "wss://relay.noswhere.com",
    "wss://relay.damus.io",
];

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

/// Fetch events using NIP-01 filters from a relay
pub async fn fetch_with_filters(
    ndb: &Ndb,
    relay_url: &str,
    filters: &[Value],
) -> Result<Vec<Value>> {
    debug!("Fetching events from {} with {} filters", relay_url, filters.len());

    let (mut ws, _) = connect_async(relay_url)
        .await
        .map_err(|e| EngineError::Relay(format!("Failed to connect to {}: {}", relay_url, e)))?;

    let sub_id = "nostr_engine_fetch";

    // Build REQ message: ["REQ", sub_id, filter1, filter2, ...]
    let mut req = vec![json!("REQ"), json!(sub_id)];
    for filter in filters {
        req.push(filter.clone());
    }
    let req_msg = Value::Array(req);

    ws.send(Message::Text(req_msg.to_string().into()))
        .await
        .map_err(|e| EngineError::Relay(format!("Failed to send REQ: {}", e)))?;

    let mut fetched_events = Vec::new();
    let start = Instant::now();
    let fetch_timeout = Duration::from_secs(15);

    while start.elapsed() < fetch_timeout {
        match timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(msg) = serde_json::from_str::<Vec<Value>>(&text) {
                    if msg.len() >= 2 {
                        let msg_type = msg[0].as_str().unwrap_or("");

                        match msg_type {
                            "EVENT" => {
                                if msg.len() >= 3 {
                                    // Ingest into nostrdb for caching, tagging
                                    // the source relay so `note.relays()`
                                    // records where the event was seen.
                                    // Normalize the URL so two relays differing
                                    // only in trailing slash / case / default
                                    // port don't surface as duplicate chips.
                                    let normalized =
                                        crate::relay_url::normalize_relay_url(relay_url);
                                    let tag_url = if normalized.is_empty() {
                                        relay_url
                                    } else {
                                        normalized.as_str()
                                    };
                                    if let Err(e) = ndb.process_event_with(
                                        &text,
                                        IngestMetadata::new().client(false).relay(tag_url),
                                    ) {
                                        debug!("Failed to ingest event: {}", e);
                                    }
                                    // Also collect the event to return
                                    if let Some(event) = msg.get(2) {
                                        fetched_events.push(event.clone());
                                    }
                                }
                            }
                            "EOSE" => {
                                debug!("EOSE received after {} events", fetched_events.len());
                                break;
                            }
                            "NOTICE" => {
                                if let Some(notice) = msg.get(1).and_then(|v| v.as_str()) {
                                    warn!("Relay notice from {}: {}", relay_url, notice);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(Some(Ok(_))) => {} // Other message types (binary, ping, etc.)
            Ok(Some(Err(e))) => {
                warn!("WebSocket error from {}: {}", relay_url, e);
                break;
            }
            Ok(None) => break, // Stream ended
            Err(_) => {
                debug!("Timeout waiting for events from {}", relay_url);
                break;
            }
        }
    }

    // Close subscription
    let close = json!(["CLOSE", sub_id]);
    let _ = ws.send(Message::Text(close.to_string().into())).await;
    let _ = ws.close(None).await;

    info!(
        "Fetched {} events from {} in {:?}",
        fetched_events.len(),
        relay_url,
        start.elapsed()
    );

    Ok(fetched_events)
}

/// Fetch a single event by ID from relays
pub async fn fetch_event_by_id(ndb: &Ndb, relays: &[String], event_id: &str) -> Result<Option<Value>> {
    let filter = json!({
        "ids": [event_id],
        "limit": 1
    });

    for relay_url in relays {
        match fetch_with_filters(ndb, relay_url, &[filter.clone()]).await {
            Ok(events) => {
                if let Some(event) = events.into_iter().next() {
                    return Ok(Some(event));
                }
            }
            Err(e) => {
                debug!("Failed to fetch from {}: {}", relay_url, e);
            }
        }
    }

    Ok(None)
}

/// Result of publishing an event to a relay
#[derive(Debug, Clone, serde::Serialize)]
pub struct PublishResult {
    pub relay_url: String,
    pub success: bool,
    pub message: Option<String>,
    /// The id of the event this result is for. Lets callers record relay
    /// provenance per (event, relay) without relying on result ordering.
    pub event_id: String,
}

/// Publish an event to a single relay
/// Returns success/failure with any message from the relay
pub async fn publish_event(relay_url: &str, event_json: &str) -> PublishResult {
    let event_id = serde_json::from_str::<Value>(event_json)
        .ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();

    let result = async {
        let (mut ws, _) = connect_async(relay_url)
            .await
            .map_err(|e| format!("Failed to connect: {}", e))?;

        // Send EVENT message: ["EVENT", {event}]
        let event: Value = serde_json::from_str(event_json)
            .map_err(|e| format!("Invalid event JSON: {}", e))?;
        let msg = json!(["EVENT", event]);

        ws.send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| format!("Failed to send: {}", e))?;

        // Wait for OK response (NIP-20)
        let start = Instant::now();
        let response_timeout = Duration::from_secs(10);

        while start.elapsed() < response_timeout {
            match timeout(Duration::from_secs(5), ws.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    if let Ok(msg) = serde_json::from_str::<Vec<Value>>(&text) {
                        if msg.len() >= 2 {
                            let msg_type = msg[0].as_str().unwrap_or("");
                            match msg_type {
                                "OK" => {
                                    // ["OK", event_id, success, message]
                                    let ok_event_id = msg.get(1).and_then(|v| v.as_str()).unwrap_or("");
                                    let success = msg.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                                    let message = msg.get(3).and_then(|v| v.as_str()).map(|s| s.to_string());

                                    if ok_event_id == event_id.as_str() {
                                        let _ = ws.close(None).await;
                                        if success {
                                            return Ok(message);
                                        } else {
                                            return Err(message.unwrap_or_else(|| "Rejected".to_string()));
                                        }
                                    }
                                }
                                "NOTICE" => {
                                    if let Some(notice) = msg.get(1).and_then(|v| v.as_str()) {
                                        warn!("Relay notice from {}: {}", relay_url, notice);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Ok(Some(Ok(_))) => {} // Other message types
                Ok(Some(Err(e))) => {
                    return Err(format!("WebSocket error: {}", e));
                }
                Ok(None) => {
                    return Err("Connection closed".to_string());
                }
                Err(_) => {
                    // Timeout - some relays don't send OK, assume success
                    let _ = ws.close(None).await;
                    return Ok(Some("No response (assumed success)".to_string()));
                }
            }
        }

        let _ = ws.close(None).await;
        Ok(Some("Timeout waiting for confirmation".to_string()))
    }
    .await;

    match result {
        Ok(message) => {
            info!("Published event to {}", relay_url);
            PublishResult {
                relay_url: relay_url.to_string(),
                success: true,
                message,
                event_id,
            }
        }
        Err(e) => {
            warn!("Failed to publish to {}: {}", relay_url, e);
            PublishResult {
                relay_url: relay_url.to_string(),
                success: false,
                message: Some(e),
                event_id,
            }
        }
    }
}

/// Publish an event to multiple relays
/// Returns results for each relay
pub async fn publish_to_relays(relays: &[String], event_json: &str) -> Vec<PublishResult> {
    let mut results = Vec::new();

    for relay_url in relays {
        let result = publish_event(relay_url, event_json).await;
        results.push(result);
    }

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
) -> (usize, usize, Vec<PublishResult>) {
    publish_events_to_relays_with_progress(relays, events, |_| {}).await
}

/// Publish multiple events to multiple relays with progress callback
pub async fn publish_events_to_relays_with_progress<F>(
    relays: &[String],
    events: &[String],
    mut on_progress: F,
) -> (usize, usize, Vec<PublishResult>)
where
    F: FnMut(BroadcastProgress),
{
    let total_relays = relays.len();
    let total_events = events.len();

    // Broadcast every (relay × event) pair concurrently, capped, rather
    // than in a sequential nested loop. A 60-section publication to 2
    // relays is 120 sends; each `publish_event` opens a WebSocket and
    // waits up to 10s for the OK, so done sequentially the response is
    // blocked for minutes and the client never sees the result. Capped
    // concurrency keeps total wall-time near the slowest single send.
    const MAX_CONCURRENT: usize = 16;
    let pairs: Vec<(usize, usize)> = (0..total_relays)
        .flat_map(|ri| (0..total_events).map(move |ei| (ri, ei)))
        .collect();
    let total_pairs = pairs.len();

    let mut stream = futures::stream::iter(pairs.into_iter().map(|(ri, ei)| {
        let relay_url = relays[ri].clone();
        let event_json = events[ei].clone();
        async move {
            let result = publish_event(&relay_url, &event_json).await;
            (ri, result)
        }
    }))
    .buffer_unordered(MAX_CONCURRENT);

    let mut all_results = Vec::with_capacity(total_pairs);
    // A relay counts as a success only if every one of its events landed.
    let mut relay_ok: Vec<bool> = vec![true; total_relays];
    let mut done = 0;

    while let Some((ri, result)) = stream.next().await {
        done += 1;
        on_progress(BroadcastProgress {
            current_relay: ri + 1,
            total_relays,
            current_event: done,
            total_events,
            relay_name: relays[ri].clone(),
            status: format!("{done}/{total_pairs} sent"),
        });
        if !result.success {
            relay_ok[ri] = false;
        }
        all_results.push(result);
    }

    let success_count = relay_ok.iter().filter(|&&ok| ok).count();
    (success_count, total_relays, all_results)
}

/// Fetch an addressable event by kind:pubkey:d-tag from relays
pub async fn fetch_addressable(
    ndb: &Ndb,
    relays: &[String],
    kind: u64,
    pubkey: &str,
    d_tag: &str,
) -> Result<Option<Value>> {
    let filter = json!({
        "kinds": [kind],
        "authors": [pubkey],
        "#d": [d_tag],
        "limit": 1
    });

    for relay_url in relays {
        debug!(
            "Fetching {}:{}:{}... from {}",
            kind,
            &pubkey.chars().take(8).collect::<String>(),
            d_tag,
            relay_url
        );

        match fetch_with_filters(ndb, relay_url, &[filter.clone()]).await {
            Ok(events) => {
                if let Some(event) = events.into_iter().next() {
                    info!(
                        "Found {}:{}:{}... from {}",
                        kind,
                        &pubkey.chars().take(8).collect::<String>(),
                        d_tag,
                        relay_url
                    );
                    return Ok(Some(event));
                }
            }
            Err(e) => {
                debug!("Failed to fetch from {}: {}", relay_url, e);
            }
        }
    }

    debug!(
        "Addressable event {}:{}:{}... not found on any relay",
        kind,
        &pubkey.chars().take(8).collect::<String>(),
        d_tag
    );
    Ok(None)
}
