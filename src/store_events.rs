//! The local store's change stream — what just landed in nostrdb, and what
//! changed among the annotation requests.
//!
//! The reader listens on `GET /api/v1/store/events` and re-reads its own
//! root when something anchored to it arrives: a note the agent wrote over
//! MCP, a comment, an answer. Local only — nothing here opens a relay; it is
//! the ingest path announcing itself. Payloads are small handles (kind, id,
//! coordinate, the `A`/`a` targets), never content.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoreEvent {
    /// An event was ingested into nostrdb.
    Ingested {
        kind: u64,
        id: String,
        pubkey: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        coordinate: Option<String>,
        /// Root scopes it names (`A` tags) — publications it is attached to.
        roots: Vec<String>,
        /// Parent targets (`a` tags) — the sections it points at.
        addrs: Vec<String>,
    },
    /// An annotation request record was created, answered, reopened or
    /// deleted.
    Request {
        id: String,
        root: String,
        parent: String,
        status: String,
    },
}

impl StoreEvent {
    /// The handle for a raw event JSON, or `None` when it has no id.
    pub fn ingested(ev: &Value) -> Option<Self> {
        let id = ev.get("id")?.as_str()?.to_string();
        let kind = ev.get("kind")?.as_u64()?;
        let pubkey = ev.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string();
        let tags = crate::publication::insertion::event_tags(ev);
        let vals = |name: &str| -> Vec<String> {
            tags.iter()
                .filter(|t| t.first().map(String::as_str) == Some(name) && t.len() >= 2)
                .map(|t| t[1].clone())
                .collect()
        };
        Some(Self::Ingested {
            kind,
            id,
            pubkey,
            coordinate: crate::publication::insertion::event_coordinate(ev),
            roots: vals("A"),
            addrs: vals("a"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ingested_handle_carries_kind_coordinate_and_anchors() {
        let ev = json!({
            "id": "ab", "kind": 30041, "pubkey": "pk",
            "tags": [["d","x"],["A","30040:pk:root"],["a","30041:pk:sec"],["title","t"]]
        });
        let h = StoreEvent::ingested(&ev).unwrap();
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["type"], "ingested");
        assert_eq!(v["kind"], 30041);
        assert_eq!(v["coordinate"], "30041:pk:x");
        assert_eq!(v["roots"][0], "30040:pk:root");
        assert_eq!(v["addrs"][0], "30041:pk:sec");
        assert!(StoreEvent::ingested(&json!({"kind": 1})).is_none());
    }
}
