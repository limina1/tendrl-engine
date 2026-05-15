//! NIP-11 (Relay Information Document) fetch + cache.
//!
//! Per `docs/relay-classes-and-info-port.md` §4 + §7.1, the canonical
//! cache for NIP-11 lives in the engine — process-wide, normalized
//! URL keys, 1-hour TTL, four-state lifecycle (`Pending | Loading |
//! Loaded | Failed`), 256 KB body cap, 5-second timeout, semaphore of
//! 5 concurrent fetches. The web/Emacs clients all consume the
//! same cache through `GET /api/v1/relay/info?url=...`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

const TTL: Duration = Duration::from_secs(60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_CONCURRENT: usize = 5;

/// Subset of NIP-11 fields the UI actually renders. Every field is
/// optional per spec; we tolerate absence everywhere. `supported_nips`
/// is permissively decoded (relays in the wild ship a mix of ints and
/// stringified ints).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Nip11Doc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub software: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_nips: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_of_service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posting_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limitation: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relay_countries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub language_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fees: Option<Value>,
}

/// Four-state lifecycle, mirrored on the wire so the web client can
/// render skeleton/error states without inventing its own taxonomy.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Nip11Status {
    Pending,
    Loading,
    Loaded { doc: Nip11Doc, fetched_at: u64 },
    Failed { error: String, fetched_at: u64 },
}

#[derive(Debug, Clone)]
struct Entry {
    status: Nip11Status,
    inserted: Instant,
}

/// Process-wide cache. Cheap to clone (`Arc` everywhere).
#[derive(Clone)]
pub struct Nip11Cache {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
    inflight: Arc<Mutex<HashMap<String, Arc<tokio::sync::Notify>>>>,
    sem: Arc<Semaphore>,
    client: reqwest::Client,
}

impl Nip11Cache {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("tendrl-engine/0.1 (NIP-11 retriever)")
            .build()
            .expect("reqwest client");
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            sem: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            client,
        }
    }

    /// Returns the current cached status. If absent or stale, kicks
    /// off a background fetch (deduplicated under a per-URL Notify)
    /// and returns `Loading` so the caller can render a skeleton.
    pub async fn get(&self, url: &str) -> Nip11Status {
        let key = normalize_relay_url(url);

        {
            let entries = self.entries.lock().await;
            if let Some(entry) = entries.get(&key) {
                if entry.inserted.elapsed() < TTL {
                    return entry.status.clone();
                }
            }
        }

        // Mark as in-flight. Per port doc §4: per-URL dedup so 50
        // concurrent screen renders for the same relay don't spawn 50
        // fetches.
        let notify = {
            let mut flight = self.inflight.lock().await;
            if let Some(notify) = flight.get(&key) {
                Arc::clone(notify)
            } else {
                let notify = Arc::new(tokio::sync::Notify::new());
                flight.insert(key.clone(), Arc::clone(&notify));
                drop(flight);
                self.spawn_fetch(url.to_string(), key.clone(), Arc::clone(&notify));
                return Nip11Status::Loading;
            }
        };

        // Await the in-flight fetch from another caller.
        notify.notified().await;
        let entries = self.entries.lock().await;
        entries
            .get(&key)
            .map(|e| e.status.clone())
            .unwrap_or(Nip11Status::Pending)
    }

    fn spawn_fetch(&self, url: String, key: String, notify: Arc<tokio::sync::Notify>) {
        let this = self.clone();
        tokio::spawn(async move {
            let _permit = this.sem.acquire().await;
            let status = fetch_doc(&this.client, &url).await;
            {
                let mut entries = this.entries.lock().await;
                entries.insert(
                    key.clone(),
                    Entry {
                        status,
                        inserted: Instant::now(),
                    },
                );
            }
            {
                let mut flight = this.inflight.lock().await;
                flight.remove(&key);
            }
            notify.notify_waiters();
        });
    }
}

impl Default for Nip11Cache {
    fn default() -> Self {
        Self::new()
    }
}

/// `wss://Relay.Example/` and `wss://relay.example` must hit the same
/// cache slot. Lowercase the host, drop trailing slashes; preserve
/// scheme so ws/wss/http/https don't collide accidentally.
pub fn normalize_relay_url(url: &str) -> String {
    let trimmed = url.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.trim_end_matches('/').to_string()
}

fn to_http(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("wss://") {
        format!("https://{}", rest)
    } else if let Some(rest) = url.strip_prefix("ws://") {
        format!("http://{}", rest)
    } else {
        url.to_string()
    }
}

async fn fetch_doc(client: &reqwest::Client, url: &str) -> Nip11Status {
    let http_url = to_http(url);
    let now = unix_now();
    let resp = match client
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Nip11Status::Failed {
                error: e.to_string(),
                fetched_at: now,
            };
        }
    };

    if !resp.status().is_success() {
        return Nip11Status::Failed {
            error: format!("HTTP {}", resp.status()),
            fetched_at: now,
        };
    }

    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return Nip11Status::Failed {
                error: format!("body read: {}", e),
                fetched_at: now,
            };
        }
    };
    if bytes.len() > MAX_BODY_BYTES {
        return Nip11Status::Failed {
            error: format!("response exceeds {} bytes", MAX_BODY_BYTES),
            fetched_at: now,
        };
    }

    let raw: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            return Nip11Status::Failed {
                error: format!("parse: {}", e),
                fetched_at: now,
            };
        }
    };

    Nip11Status::Loaded {
        doc: parse_doc(&raw),
        fetched_at: now,
    }
}

/// Permissive decoder. `supported_nips` accepts ints and stringified
/// ints (port doc §3 principle 1). Unknown / mistyped fields are
/// silently dropped — forward-compatible with future spec additions.
fn parse_doc(raw: &Value) -> Nip11Doc {
    let obj = match raw.as_object() {
        Some(o) => o,
        None => return Nip11Doc::default(),
    };

    let str_opt = |k: &str| obj.get(k).and_then(|v| v.as_str()).map(String::from);
    let str_arr = |k: &str| -> Vec<String> {
        obj.get(k)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };

    let supported_nips: Vec<u32> = obj
        .get("supported_nips")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    if let Some(n) = v.as_u64() {
                        u32::try_from(n).ok()
                    } else if let Some(s) = v.as_str() {
                        s.parse::<u32>().ok()
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Nip11Doc {
        name: str_opt("name"),
        description: str_opt("description"),
        pubkey: str_opt("pubkey"),
        contact: str_opt("contact"),
        software: str_opt("software"),
        version: str_opt("version"),
        icon: str_opt("icon"),
        banner: str_opt("banner"),
        supported_nips,
        privacy_policy: str_opt("privacy_policy"),
        terms_of_service: str_opt("terms_of_service"),
        posting_policy: str_opt("posting_policy"),
        limitation: obj.get("limitation").cloned(),
        retention: obj.get("retention").cloned(),
        relay_countries: str_arr("relay_countries"),
        language_tags: str_arr("language_tags"),
        tags: str_arr("tags"),
        fees: obj.get("fees").cloned(),
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_url() {
        assert_eq!(normalize_relay_url("wss://Relay.Example/"), "wss://relay.example");
        assert_eq!(normalize_relay_url("WSS://relay.example"), "wss://relay.example");
        assert_eq!(normalize_relay_url("  wss://relay.example/  "), "wss://relay.example");
    }

    #[test]
    fn parses_sloppy_supported_nips() {
        let raw = serde_json::json!({
            "supported_nips": [1, "11", 50, "garbage", 9999]
        });
        let doc = parse_doc(&raw);
        assert_eq!(doc.supported_nips, vec![1, 11, 50, 9999]);
    }

    #[test]
    fn tolerates_missing_fields() {
        let raw = serde_json::json!({});
        let doc = parse_doc(&raw);
        assert!(doc.name.is_none());
        assert!(doc.supported_nips.is_empty());
    }
}
