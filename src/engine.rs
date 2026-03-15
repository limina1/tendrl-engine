//! Core unified query engine
//!
//! Provides a unified interface for querying events from local nostrdb
//! with optional relay backfill based on configurable fetch policies.

use crate::error::{EngineError, Result};
use crate::search::{self, SearchQuery, SearchResponse};
use crate::{query, relay};
use nostrdb::{Config, IngestMetadata, Ndb};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info};

/// Fetch policy determines how the engine retrieves events
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchPolicy {
    /// Only query local nostrdb, never fetch from relays
    LocalOnly,
    /// Try local first, fetch from relays if no results (default)
    #[default]
    LocalFirst,
    /// Always fetch from relays, merge with local results
    FetchAlways,
}

impl std::str::FromStr for FetchPolicy {
    type Err = EngineError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "local_only" | "localonly" => Ok(FetchPolicy::LocalOnly),
            "local_first" | "localfirst" => Ok(FetchPolicy::LocalFirst),
            "fetch_always" | "fetchalways" => Ok(FetchPolicy::FetchAlways),
            _ => Err(EngineError::InvalidFilter(format!(
                "Unknown fetch policy: {}. Valid values: local_only, local_first, fetch_always",
                s
            ))),
        }
    }
}

/// Source information for query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySource {
    /// Number of events from local database
    pub local_count: usize,
    /// Number of events fetched from relays
    pub relay_count: usize,
}

/// Response from a query operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    /// The retrieved events
    pub events: Vec<Value>,
    /// Total count of events
    pub count: usize,
    /// Source breakdown
    pub source: QuerySource,
}

/// The main Nostr Engine
pub struct Engine {
    /// The nostrdb instance
    ndb: Arc<Ndb>,
    /// Default relays for fetching
    relays: Vec<String>,
    /// Request timeout in milliseconds
    #[allow(dead_code)]
    timeout_ms: u64,
    /// Data directory path
    data_dir: std::path::PathBuf,
}

impl Engine {
    /// Create a new Engine with the specified data path
    pub fn new(data_path: &Path) -> Result<Self> {
        Self::with_config(data_path, relay::DEFAULT_RELAYS, 15000)
    }

    /// Create a new Engine with custom configuration
    pub fn with_config(data_path: &Path, relays: &[&str], timeout_ms: u64) -> Result<Self> {
        // Ensure the data directory exists
        std::fs::create_dir_all(data_path)?;

        let config = Config::new().set_ingester_threads(2);

        let ndb = Ndb::new(
            data_path.to_str().ok_or_else(|| {
                EngineError::Config("Invalid data path encoding".to_string())
            })?,
            &config,
        )
        .map_err(|e| EngineError::Database(format!("Failed to open nostrdb: {}", e)))?;

        info!("Opened nostrdb at {:?}", data_path);

        Ok(Engine {
            ndb: Arc::new(ndb),
            relays: relays.iter().map(|s| s.to_string()).collect(),
            timeout_ms,
            data_dir: data_path.to_path_buf(),
        })
    }

    /// Get a reference to the underlying Ndb instance
    pub fn ndb(&self) -> &Ndb {
        &self.ndb
    }

    /// Get the configured relays
    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Query events using NIP-01 filters with the specified fetch policy
    pub async fn get_events(
        &self,
        filters: Vec<Value>,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
    ) -> Result<QueryResponse> {
        let relays = override_relays.unwrap_or(&self.relays);

        match policy {
            FetchPolicy::LocalOnly => self.query_local_only(&filters),
            FetchPolicy::LocalFirst => self.query_local_first(&filters, relays).await,
            FetchPolicy::FetchAlways => self.query_fetch_always(&filters, relays).await,
        }
    }

    /// Get a single event by its ID
    pub async fn get_by_id(
        &self,
        event_id: &str,
        policy: FetchPolicy,
    ) -> Result<Option<Value>> {
        // Try local first (unless FetchAlways)
        if policy != FetchPolicy::FetchAlways {
            if let Some(event) = query::query_by_id(&self.ndb, event_id)? {
                debug!("Found event {} locally", event_id);
                return Ok(Some(event));
            }
        }

        // Fetch from relays if needed
        if policy != FetchPolicy::LocalOnly {
            debug!("Fetching event {} from relays", event_id);
            return relay::fetch_event_by_id(&self.ndb, &self.relays, event_id).await;
        }

        Ok(None)
    }

    /// Get an addressable event by kind, pubkey, and d-tag
    pub async fn get_addressable(
        &self,
        kind: u64,
        pubkey: &str,
        d_tag: &str,
        policy: FetchPolicy,
    ) -> Result<Option<Value>> {
        // Try local first (unless FetchAlways)
        if policy != FetchPolicy::FetchAlways {
            if let Some(event) = query::query_addressable(&self.ndb, kind, pubkey, d_tag)? {
                debug!("Found addressable event {}:{}:{}... locally", kind, &pubkey.chars().take(8).collect::<String>(), d_tag);
                return Ok(Some(event));
            }
        }

        // Fetch from relays if needed
        if policy != FetchPolicy::LocalOnly {
            debug!("Fetching addressable event {}:{}:{}... from relays", kind, &pubkey.chars().take(8).collect::<String>(), d_tag);
            return relay::fetch_addressable(&self.ndb, &self.relays, kind, pubkey, d_tag).await;
        }

        Ok(None)
    }

    /// Ingest a raw event JSON string into nostrdb
    ///
    /// The event_json should be a raw event object `{...}`. This function wraps it
    /// in the client message format `["EVENT", {...}]` that nostrdb expects for client events.
    ///
    /// Uses `client(true)` to skip signature verification for locally-created events.
    /// This allows storing unsigned/placeholder-signed events created by the compose flow.
    pub fn ingest_event(&self, event_json: &str) -> Result<()> {
        // Client events use format: ["EVENT", {...event...}] (no subscription ID)
        // Relay events use format: ["EVENT", "subid", {...event...}] (with subscription ID)
        let wrapped = format!(r#"["EVENT",{}]"#, event_json);
        debug!("Ingesting event: {}", &wrapped[..wrapped.len().min(200)]);
        self.ndb
            .process_event_with(&wrapped, IngestMetadata::new().client(true))
            .map_err(|e| {
                debug!("Ingest failed: {}", e);
                EngineError::Database(format!("Failed to ingest event: {}", e))
            })?;
        debug!("Ingest succeeded");
        Ok(())
    }

    /// Search for events using a structured search query.
    ///
    /// Compiles the query to NIP-01 filters, fetches events, applies text
    /// post-filtering, and builds SearchResult objects.
    pub async fn search(
        &self,
        query: &SearchQuery,
        policy: FetchPolicy,
        override_relays: Option<&[String]>,
    ) -> Result<SearchResponse> {
        let limit = query.limit.unwrap_or(100);

        let mut filters = query.to_nip01_filters();
        if filters.is_empty() {
            // No NIP-01 filters — fetch broadly with a limit
            filters = vec![serde_json::json!({"limit": limit})];
        }

        let response = self.get_events(filters, policy, override_relays).await?;

        let filtered = if let Some(text_filter) = &query.text_filter {
            query::filter_by_text(&response.events, text_filter)
        } else {
            response.events
        };

        let results = search::build_search_results(&filtered, limit);
        let count = results.len();

        Ok(SearchResponse {
            results,
            count,
            local_count: response.source.local_count,
            relay_count: response.source.relay_count,
        })
    }

    // ---- Private helper methods ----

    fn query_local_only(&self, filters: &[Value]) -> Result<QueryResponse> {
        let events = query::query_local(&self.ndb, filters)?;
        let count = events.len();

        Ok(QueryResponse {
            events,
            count,
            source: QuerySource {
                local_count: count,
                relay_count: 0,
            },
        })
    }

    async fn query_local_first(&self, filters: &[Value], relays: &[String]) -> Result<QueryResponse> {
        // Try local first
        let local_events = query::query_local(&self.ndb, filters)?;
        let local_count = local_events.len();

        // Extract the requested limit from filters (use first filter's limit, default to 50)
        let requested_limit = filters
            .first()
            .and_then(|f| f.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // If we have enough local results, return them
        if local_count >= requested_limit {
            debug!(
                "Found {} local results (requested {}), returning local only",
                local_count, requested_limit
            );
            return Ok(QueryResponse {
                events: local_events,
                count: local_count,
                source: QuerySource {
                    local_count,
                    relay_count: 0,
                },
            });
        }

        // Not enough local results, fetch from relays and merge
        debug!(
            "Found {} local results (requested {}), fetching from relays",
            local_count, requested_limit
        );
        let relay_events = relay::fetch_from_multiple_relays(&self.ndb, relays, filters).await?;
        let relay_count = relay_events.len();

        // Merge and deduplicate by event ID
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged = Vec::new();

        // Add local events first
        for event in local_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        // Add relay events
        for event in relay_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        let count = merged.len();
        Ok(QueryResponse {
            events: merged,
            count,
            source: QuerySource {
                local_count,
                relay_count,
            },
        })
    }

    async fn query_fetch_always(&self, filters: &[Value], relays: &[String]) -> Result<QueryResponse> {
        // Fetch from relays first (this also ingests into nostrdb)
        let relay_events = relay::fetch_from_multiple_relays(&self.ndb, relays, filters).await?;
        let relay_count = relay_events.len();

        // Now query local (which includes freshly ingested events)
        let local_events = query::query_local(&self.ndb, filters)?;
        let local_count = local_events.len();

        // Merge and deduplicate (local results may include more than relay returned due to limits)
        let mut seen_ids = std::collections::HashSet::new();
        let mut merged = Vec::new();

        // Add local events first (they're from the DB, so authoritative)
        for event in local_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        // Add any relay events not in local (edge case: just fetched but not yet queryable)
        for event in relay_events {
            if let Some(id) = event.get("id").and_then(|v| v.as_str()) {
                if seen_ids.insert(id.to_string()) {
                    merged.push(event);
                }
            }
        }

        let count = merged.len();

        Ok(QueryResponse {
            events: merged,
            count,
            source: QuerySource {
                local_count,
                relay_count,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_fetch_policy_parsing() {
        assert_eq!("local_only".parse::<FetchPolicy>().unwrap(), FetchPolicy::LocalOnly);
        assert_eq!("local_first".parse::<FetchPolicy>().unwrap(), FetchPolicy::LocalFirst);
        assert_eq!("fetch_always".parse::<FetchPolicy>().unwrap(), FetchPolicy::FetchAlways);
        assert_eq!("LocalFirst".parse::<FetchPolicy>().unwrap(), FetchPolicy::LocalFirst);
        assert!("invalid".parse::<FetchPolicy>().is_err());
    }

    #[test]
    fn test_fetch_policy_serde() {
        let policy = FetchPolicy::LocalFirst;
        let json = serde_json::to_string(&policy).unwrap();
        assert_eq!(json, "\"local_first\"");

        let parsed: FetchPolicy = serde_json::from_str("\"fetch_always\"").unwrap();
        assert_eq!(parsed, FetchPolicy::FetchAlways);
    }

    #[tokio::test]
    async fn test_ingest_valid_signed_event() {
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use a known-valid event from nostrdb tests (kind 1 with valid signature)
        let event_json = r#"{"id": "702555e52e82cc24ad517ba78c21879f6e47a7c0692b9b20df147916ae8731a3","pubkey": "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15","created_at": 1702675561,"kind": 1,"tags": [],"content": "hello, world","sig": "2275c5f5417abfd644b7bc74f0388d70feb5d08b6f90fa18655dda5c95d013bfbc5258ea77c05b7e40e0ee51d8a2efa931dc7a0ec1db4c0a94519762c6625675"}"#;

        // Subscribe BEFORE ingesting (nostrdb processes asynchronously)
        let filter = FilterBuilder::new().kinds([1]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest the event
        let result = engine.ingest_event(event_json);
        assert!(result.is_ok(), "Ingest failed: {:?}", result.err());

        // Wait for the event to be processed (with timeout)
        let note_keys = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            engine.ndb.wait_for_notes(sub, 1)
        ).await.expect("timeout waiting for notes").expect("waiting for notes");

        println!("Got {} note keys after ingest", note_keys.len());
        assert!(!note_keys.is_empty(), "No notes received from subscription");
    }

    #[tokio::test]
    async fn test_ingest_signed_event() {
        use nostrdb::FilterBuilder;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use the EXACT known-working event from nostrdb tests but change kind to 30040
        // First, let's verify the original kind 1 event works with our ingest
        let valid_event = r#"{"id": "702555e52e82cc24ad517ba78c21879f6e47a7c0692b9b20df147916ae8731a3","pubkey": "32bf915904bfde2d136ba45dde32c88f4aca863783999faea2e847a8fafd2f15","created_at": 1702675561,"kind": 1,"tags": [],"content": "hello, world","sig": "2275c5f5417abfd644b7bc74f0388d70feb5d08b6f90fa18655dda5c95d013bfbc5258ea77c05b7e40e0ee51d8a2efa931dc7a0ec1db4c0a94519762c6625675"}"#;

        // Subscribe BEFORE ingesting
        let filter = FilterBuilder::new().kinds([1]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest the event
        let result = engine.ingest_event(valid_event);
        assert!(result.is_ok(), "Ingest failed: {:?}", result.err());

        // Wait for the event with timeout
        let note_keys_result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.ndb.wait_for_notes(sub, 1)
        ).await;

        match note_keys_result {
            Ok(Ok(keys)) => {
                println!("SUCCESS: Got {} note keys for signed event", keys.len());
                assert!(!keys.is_empty(), "No notes received");
            }
            Ok(Err(e)) => {
                panic!("Subscription error: {:?}", e);
            }
            Err(_) => {
                panic!("TIMEOUT: Signed event was not processed");
            }
        }
    }

    #[tokio::test]
    async fn test_build_and_ingest_signed_publication() {
        use nostrdb::FilterBuilder;
        use crate::publication::build_signed_publication_events;
        use crate::tree::state::ComposeState;

        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        // Use the known test secret from identity tests
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";

        // Derive the pubkey
        use secp256k1::{Secp256k1, SecretKey, PublicKey};
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized = public_key.serialize();
        let pubkey = hex::encode(&serialized[1..33]);

        println!("Derived pubkey: {}", pubkey);

        // Create a simple publication
        use crate::tree::state::SectionCompose;
        let mut compose = ComposeState::new();
        compose.title = "Test Publication".to_string();
        compose.sections.push(SectionCompose {
            title: "Section 1".to_string(),
            content: "Hello, world!".to_string(),
            ..Default::default()
        });

        // Build signed events
        let (pub_event, section_events) = build_signed_publication_events(&compose, &pubkey, secret_hex);

        println!("Publication event: {}", serde_json::to_string_pretty(&pub_event).unwrap());
        println!("Section event: {}", serde_json::to_string_pretty(&section_events[0]).unwrap());

        // Subscribe BEFORE ingesting
        let filter = FilterBuilder::new().kinds([30040, 30041]).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        // Ingest section event first
        let section_json = serde_json::to_string(&section_events[0]).unwrap();
        let result = engine.ingest_event(&section_json);
        assert!(result.is_ok(), "Section ingest failed: {:?}", result.err());

        // Ingest publication event
        let pub_json = serde_json::to_string(&pub_event).unwrap();
        let result = engine.ingest_event(&pub_json);
        assert!(result.is_ok(), "Publication ingest failed: {:?}", result.err());

        // Wait for the events with timeout
        let note_keys_result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine.ndb.wait_for_notes(sub, 2)
        ).await;

        match note_keys_result {
            Ok(Ok(keys)) => {
                println!("SUCCESS: Got {} note keys for signed events", keys.len());
                assert!(keys.len() >= 1, "Expected at least 1 note, got {}", keys.len());
            }
            Ok(Err(e)) => {
                panic!("Subscription error: {:?}", e);
            }
            Err(_) => {
                panic!("TIMEOUT: Signed events were not processed");
            }
        }
    }

    #[tokio::test]
    async fn test_events_persist_across_restart() {
        use nostrdb::FilterBuilder;
        use crate::publication::build_signed_publication_events;
        use crate::tree::state::ComposeState;

        // Use a non-temp directory so we can reopen it
        let test_dir = std::path::PathBuf::from("target/testdbs/persistence_test");
        let _ = std::fs::remove_dir_all(&test_dir);
        std::fs::create_dir_all(&test_dir).unwrap();

        // Known test secret
        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";

        // Derive pubkey
        use secp256k1::{Secp256k1, SecretKey, PublicKey};
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let serialized = public_key.serialize();
        let pubkey = hex::encode(&serialized[1..33]);

        let event_id: String;

        // PHASE 1: Create engine, ingest event, drop engine
        {
            let engine = Engine::with_config(&test_dir, &[], 1000).unwrap();

            // Create a publication
            use crate::tree::state::SectionCompose;
            let mut compose = ComposeState::new();
            compose.title = "Persistence Test".to_string();
            compose.sections.push(SectionCompose {
                title: "Section 1".to_string(),
                content: "This should persist!".to_string(),
                ..Default::default()
            });

            let (pub_event, _section_events) = build_signed_publication_events(&compose, &pubkey, secret_hex);
            event_id = pub_event.get("id").unwrap().as_str().unwrap().to_string();

            println!("Created event with ID: {}", event_id);

            // Subscribe and ingest
            let filter = FilterBuilder::new().kinds([30040]).build();
            let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

            let pub_json = serde_json::to_string(&pub_event).unwrap();
            engine.ingest_event(&pub_json).expect("ingest should work");

            // Wait for processing
            let keys = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                engine.ndb.wait_for_notes(sub, 1)
            ).await.expect("timeout").expect("wait_for_notes");

            println!("Phase 1: Ingested and got {} note keys", keys.len());
            assert!(!keys.is_empty(), "Event not processed in phase 1");

            // Drop engine (should flush to disk)
            drop(engine);
            println!("Phase 1: Engine dropped");
        }

        // Small delay to ensure filesystem sync
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // PHASE 2: Reopen engine, query for the event
        {
            let engine = Engine::with_config(&test_dir, &[], 1000).unwrap();
            println!("Phase 2: Engine reopened");

            // Query for the event
            let filter = serde_json::json!({
                "kinds": [30040],
                "limit": 10
            });
            let events = query::query_local(&engine.ndb, &[filter]).unwrap();

            println!("Phase 2: Found {} events", events.len());

            // Check if our event is there
            let found = events.iter().any(|e| {
                e.get("id").and_then(|v| v.as_str()) == Some(&event_id)
            });

            if !found {
                println!("Events found: {:?}", events.iter().map(|e| e.get("id")).collect::<Vec<_>>());
            }

            assert!(found, "Event {} not found after restart!", event_id);
            println!("SUCCESS: Event persisted across restart!");
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&test_dir);
    }

    /// Helper: build a signed event with given kind, content, tags
    fn build_test_event(kind: u64, content: &str, tags: Vec<Vec<&str>>, created_at: u64) -> String {
        use sha2::{Sha256, Digest};
        use secp256k1::{Secp256k1, SecretKey, PublicKey};

        let secret_hex = "e698fdd6e2e780b7d9800266bfc02d56630835856a0146969cc984bb21b068c6";
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&secret_bytes).unwrap();
        let public_key = PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey = hex::encode(&public_key.serialize()[1..33]);

        let tags_json: Vec<Vec<String>> = tags.iter()
            .map(|t| t.iter().map(|s| s.to_string()).collect())
            .collect();

        let serial = serde_json::json!([0, pubkey, created_at, kind, tags_json, content]);
        let serial_str = serde_json::to_string(&serial).unwrap();

        let mut hasher = Sha256::new();
        hasher.update(serial_str.as_bytes());
        let hash = hasher.finalize();
        let id_hex = hex::encode(&hash);

        let sig = crate::identity::sign_event_hash(&id_hex, secret_hex).unwrap();

        serde_json::to_string(&serde_json::json!({
            "id": id_hex,
            "pubkey": pubkey,
            "created_at": created_at,
            "kind": kind,
            "tags": tags_json,
            "content": content,
            "sig": sig
        })).unwrap()
    }

    /// Helper: ingest events and wait for processing
    async fn ingest_and_wait(engine: &Engine, events: &[String]) {
        use nostrdb::FilterBuilder;

        let filter = FilterBuilder::new().limit(events.len() as u64 + 10).build();
        let sub = engine.ndb.subscribe(&[filter]).expect("subscription");

        for event_json in events {
            engine.ingest_event(event_json).expect("ingest");
        }

        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            engine.ndb.wait_for_notes(sub, events.len() as u32),
        ).await;
    }

    #[tokio::test]
    async fn test_search_by_tag() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(30041, "Python tutorial", vec![vec!["t", "python"], vec!["d", "s1"]], 1700000001),
            build_test_event(30041, "Rust guide", vec![vec!["t", "rust"], vec!["d", "s2"]], 1700000002),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("t:python").unwrap();
        let response = engine.search(&query, FetchPolicy::LocalOnly, None).await.unwrap();

        assert!(!response.results.is_empty(), "Should find python-tagged events");
        assert!(response.results.iter().all(|r| {
            r.tags.iter().any(|t| t.get(0).map(|s| s.as_str()) == Some("t") && t.get(1).map(|s| s.as_str()) == Some("python"))
        }));
    }

    #[tokio::test]
    async fn test_search_by_text_keyword() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(1, "neural network training", vec![], 1700000001),
            build_test_event(1, "hello world", vec![], 1700000002),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("neural").unwrap();
        let response = engine.search(&query, FetchPolicy::LocalOnly, None).await.unwrap();

        assert_eq!(response.results.len(), 1);
        assert!(response.results[0].preview.contains("neural"));
    }

    #[tokio::test]
    async fn test_search_by_kind() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events = vec![
            build_test_event(30041, "A section", vec![vec!["d", "sec1"]], 1700000001),
            build_test_event(1, "A note", vec![], 1700000002),
        ];
        ingest_and_wait(&engine, &events).await;

        let query = SearchQuery::parse("k:30041").unwrap();
        let response = engine.search(&query, FetchPolicy::LocalOnly, None).await.unwrap();

        assert!(response.results.iter().all(|r| r.kind == 30041));
    }

    #[tokio::test]
    async fn test_search_empty_results() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let query = SearchQuery::parse("t:nonexistent").unwrap();
        let response = engine.search(&query, FetchPolicy::LocalOnly, None).await.unwrap();

        assert_eq!(response.count, 0);
        assert!(response.results.is_empty());
    }

    #[tokio::test]
    async fn test_search_respects_limit() {
        let dir = tempdir().unwrap();
        let engine = Engine::with_config(dir.path(), &[], 1000).unwrap();

        let events: Vec<String> = (0..10)
            .map(|i| build_test_event(1, &format!("note {}", i), vec![], 1700000000 + i))
            .collect();
        ingest_and_wait(&engine, &events).await;

        let mut query = SearchQuery::parse("note").unwrap();
        query.limit = Some(3);
        let response = engine.search(&query, FetchPolicy::LocalOnly, None).await.unwrap();

        assert!(response.results.len() <= 3);
    }
}
