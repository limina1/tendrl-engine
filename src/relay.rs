//! WebSocket relay fetching
//!
//! Fetches events from Nostr relays and ingests them into nostrdb.

use crate::error::{EngineError, Result};
use futures::{SinkExt, StreamExt};
use nostrdb::Ndb;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, info, warn};

/// Default relays to fetch from (focused on publication-supporting relays)
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://theforest.nostr1.com",
    "wss://thecitadel.nostr1.com",
    "wss://relay.damus.io",
];

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

    ws.send(Message::Text(req_msg.to_string()))
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
                                    // Ingest into nostrdb for caching
                                    if let Err(e) = ndb.process_event(&text) {
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
    let _ = ws.send(Message::Text(close.to_string())).await;
    let _ = ws.close(None).await;

    info!(
        "Fetched {} events from {} in {:?}",
        fetched_events.len(),
        relay_url,
        start.elapsed()
    );

    Ok(fetched_events)
}

/// Fetch events from multiple relays and merge results
pub async fn fetch_from_multiple_relays(
    ndb: &Ndb,
    relays: &[String],
    filters: &[Value],
) -> Result<Vec<Value>> {
    let mut all_events = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    for relay_url in relays {
        match fetch_with_filters(ndb, relay_url, filters).await {
            Ok(events) => {
                for event in events {
                    // Deduplicate by event ID
                    if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                        if seen_ids.insert(id.to_string()) {
                            all_events.push(event);
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to fetch from {}: {}", relay_url, e);
                // Continue with other relays
            }
        }
    }

    Ok(all_events)
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
