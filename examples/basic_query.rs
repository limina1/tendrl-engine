//! Basic query example for nostr-engine
//!
//! Demonstrates using nostr-engine as a library to query Nostr events.

use nostr_engine::{Engine, FetchPolicy};
use serde_json::json;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("nostr_engine=info")
        .init();

    // Determine data directory
    let data_dir = std::env::var("NOSTR_ENGINE_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("nostr-engine")
                .join("nostrdb")
        });

    println!("Using data directory: {:?}", data_dir);

    // Create the engine
    let engine = Engine::new(&data_dir)?;

    // Example 1: Query for short notes (kind 1)
    println!("\n--- Querying for kind 1 notes ---");
    let filter = json!({"kinds": [1], "limit": 5});
    let response = engine
        .get_events(vec![filter], FetchPolicy::LocalFirst, None)
        .await?;

    println!(
        "Found {} events (local: {}, relay: {})",
        response.count, response.source.local_count, response.source.relay_count
    );

    for event in response.events.iter().take(3) {
        if let Some(content) = event.get("content").and_then(|v| v.as_str()) {
            let truncated: String = content.chars().take(80).collect();
            println!("  - {}", truncated);
        }
    }

    // Example 2: Query for long-form articles (kind 30023)
    println!("\n--- Querying for kind 30023 articles ---");
    let filter = json!({"kinds": [30023], "limit": 3});
    let response = engine
        .get_events(vec![filter], FetchPolicy::FetchAlways, None)
        .await?;

    println!(
        "Found {} articles (local: {}, relay: {})",
        response.count, response.source.local_count, response.source.relay_count
    );

    for event in &response.events {
        // Extract title from tags
        if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    if arr.first().and_then(|v| v.as_str()) == Some("title") {
                        if let Some(title) = arr.get(1).and_then(|v| v.as_str()) {
                            println!("  - {}", title);
                        }
                    }
                }
            }
        }
    }

    // Example 3: Query with custom relays
    println!("\n--- Querying with custom relay ---");
    let filter = json!({"kinds": [0], "limit": 3}); // Profile metadata
    let custom_relays = vec!["wss://relay.damus.io".to_string()];
    let response = engine
        .get_events(vec![filter], FetchPolicy::FetchAlways, Some(&custom_relays))
        .await?;

    println!("Found {} profiles from custom relay", response.count);

    println!("\nDone!");
    Ok(())
}
