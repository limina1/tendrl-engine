//! Network mode and fetch activity tracking
//!
//! Provides an engine-level Online/Offline toggle and a ring buffer
//! of recent relay fetch activity so users can see exactly when
//! the engine is talking to relays.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const MAX_LOG_ENTRIES: usize = 64;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Online,
    Offline,
}

impl std::str::FromStr for NetworkMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "online" => Ok(NetworkMode::Online),
            "offline" => Ok(NetworkMode::Offline),
            _ => Err(format!("Unknown network mode: {s}. Valid: online, offline")),
        }
    }
}

impl std::fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkMode::Online => write!(f, "online"),
            NetworkMode::Offline => write!(f, "offline"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchTrigger {
    BackgroundSync,
    UserAction,
    LocalFirst,
    FetchAlways,
    ProfilePrefetch,
}

impl std::fmt::Display for FetchTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchTrigger::BackgroundSync => write!(f, "background_sync"),
            FetchTrigger::UserAction => write!(f, "user_action"),
            FetchTrigger::LocalFirst => write!(f, "local_first"),
            FetchTrigger::FetchAlways => write!(f, "fetch_always"),
            FetchTrigger::ProfilePrefetch => write!(f, "profile_prefetch"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchRecord {
    pub id: u64,
    pub relay: String,
    pub filter_summary: String,
    pub event_count: usize,
    pub duration_ms: u64,
    pub trigger: FetchTrigger,
    pub timestamp: u64,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NetworkStatus {
    pub mode: NetworkMode,
    pub active_fetches: u64,
    pub total_events_fetched: u64,
    pub last_fetch_timestamp: u64,
    pub recent: Vec<FetchRecord>,
}

// ---------------------------------------------------------------------------
// NetworkActivity — the shared tracker
// ---------------------------------------------------------------------------

pub struct NetworkActivity {
    mode: AtomicBool, // true = online
    log: RwLock<VecDeque<FetchRecord>>,
    active_fetches: AtomicU64,
    next_id: AtomicU64,
    total_events_fetched: AtomicU64,
    last_fetch_timestamp: AtomicU64,
}

impl NetworkActivity {
    pub fn new(initial_mode: NetworkMode) -> Self {
        Self {
            mode: AtomicBool::new(matches!(initial_mode, NetworkMode::Online)),
            log: RwLock::new(VecDeque::with_capacity(MAX_LOG_ENTRIES)),
            active_fetches: AtomicU64::new(0),
            next_id: AtomicU64::new(1),
            total_events_fetched: AtomicU64::new(0),
            last_fetch_timestamp: AtomicU64::new(0),
        }
    }

    pub fn is_online(&self) -> bool {
        self.mode.load(Ordering::Relaxed)
    }

    pub fn mode(&self) -> NetworkMode {
        if self.is_online() {
            NetworkMode::Online
        } else {
            NetworkMode::Offline
        }
    }

    pub fn set_mode(&self, mode: NetworkMode) {
        self.mode
            .store(matches!(mode, NetworkMode::Online), Ordering::Relaxed);
    }

    pub fn active_count(&self) -> u64 {
        self.active_fetches.load(Ordering::Relaxed)
    }

    /// Begin tracking a fetch. Returns a guard that records the result
    /// on completion. The guard decrements active_fetches on Drop even
    /// if the future is cancelled.
    pub fn begin_fetch(
        self: &std::sync::Arc<Self>,
        relay: &str,
        filter_summary: String,
        trigger: FetchTrigger,
    ) -> FetchGuard {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.active_fetches.fetch_add(1, Ordering::Relaxed);
        FetchGuard {
            activity: std::sync::Arc::clone(self),
            id,
            relay: relay.to_string(),
            filter_summary,
            trigger,
            start: Instant::now(),
            completed: false,
        }
    }

    async fn record(&self, record: FetchRecord) {
        self.total_events_fetched
            .fetch_add(record.event_count as u64, Ordering::Relaxed);
        self.last_fetch_timestamp
            .store(record.timestamp, Ordering::Relaxed);

        let mut log = self.log.write().await;
        if log.len() >= MAX_LOG_ENTRIES {
            log.pop_front();
        }
        log.push_back(record);
    }

    pub async fn status(&self) -> NetworkStatus {
        let log = self.log.read().await;
        NetworkStatus {
            mode: self.mode(),
            active_fetches: self.active_fetches.load(Ordering::Relaxed),
            total_events_fetched: self.total_events_fetched.load(Ordering::Relaxed),
            last_fetch_timestamp: self.last_fetch_timestamp.load(Ordering::Relaxed),
            recent: log.iter().rev().cloned().collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// FetchGuard — RAII tracker for individual fetches
// ---------------------------------------------------------------------------

pub struct FetchGuard {
    activity: std::sync::Arc<NetworkActivity>,
    id: u64,
    relay: String,
    filter_summary: String,
    trigger: FetchTrigger,
    start: Instant,
    completed: bool,
}

impl FetchGuard {
    pub async fn complete(mut self, event_count: usize) {
        self.completed = true;
        self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
        let record = FetchRecord {
            id: self.id,
            relay: self.relay.clone(),
            filter_summary: self.filter_summary.clone(),
            event_count,
            duration_ms: self.start.elapsed().as_millis() as u64,
            trigger: self.trigger.clone(),
            timestamp: now_unix(),
            success: true,
            error: None,
        };
        self.activity.record(record).await;
    }

    pub async fn fail(mut self, error: String) {
        self.completed = true;
        self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
        let record = FetchRecord {
            id: self.id,
            relay: self.relay.clone(),
            filter_summary: self.filter_summary.clone(),
            event_count: 0,
            duration_ms: self.start.elapsed().as_millis() as u64,
            trigger: self.trigger.clone(),
            timestamp: now_unix(),
            success: false,
            error: Some(error),
        };
        self.activity.record(record).await;
    }
}

impl Drop for FetchGuard {
    fn drop(&mut self) {
        if !self.completed {
            // Future was cancelled — still decrement active count
            self.activity.active_fetches.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Produce a human-readable summary of NIP-01 filters for the activity log.
pub fn summarize_filters(filters: &[Value]) -> String {
    let mut parts = Vec::new();
    for f in filters {
        if let Some(kinds) = f.get("kinds").and_then(|v| v.as_array()) {
            let ks: Vec<String> = kinds.iter().filter_map(|k| k.as_u64().map(|n| n.to_string())).collect();
            if !ks.is_empty() {
                parts.push(format!("kinds=[{}]", ks.join(",")));
            }
        }
        if let Some(authors) = f.get("authors").and_then(|v| v.as_array()) {
            if !authors.is_empty() {
                parts.push(format!("authors={}", authors.len()));
            }
        }
        if let Some(ids) = f.get("ids").and_then(|v| v.as_array()) {
            if !ids.is_empty() {
                parts.push(format!("ids={}", ids.len()));
            }
        }
        if let Some(d_tags) = f.get("#d").and_then(|v| v.as_array()) {
            if !d_tags.is_empty() {
                parts.push(format!("#d={}", d_tags.len()));
            }
        }
        if let Some(limit) = f.get("limit").and_then(|v| v.as_u64()) {
            parts.push(format!("limit={}", limit));
        }
    }
    if parts.is_empty() {
        "no filters".to_string()
    } else {
        parts.join(" ")
    }
}
