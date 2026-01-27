//! Core unified query engine
//!
//! Provides a unified interface for querying events from local nostrdb
//! with optional relay backfill based on configurable fetch policies.

use crate::error::{EngineError, Result};
use crate::{query, relay};
use nostrdb::{Config, Ndb};
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
    pub fn ingest_event(&self, event_json: &str) -> Result<()> {
        self.ndb
            .process_event(event_json)
            .map_err(|e| EngineError::Database(format!("Failed to ingest event: {}", e)))?;
        Ok(())
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
}
