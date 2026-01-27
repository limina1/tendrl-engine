//! Local nostrdb query interface
//!
//! Provides NIP-01 filter-based querying against the local nostrdb database.

use crate::error::{EngineError, Result};
use nostrdb::{FilterBuilder, Ndb, Transaction};
use serde_json::Value;
use tracing::debug;

/// Query events from local nostrdb using NIP-01 filters
pub fn query_local(ndb: &Ndb, filters: &[Value]) -> Result<Vec<Value>> {
    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    let mut all_events = Vec::new();

    for filter_json in filters {
        let filter = parse_filter(filter_json)?;
        let limit = extract_limit(filter_json).unwrap_or(100) as i32;

        debug!("Querying nostrdb with filter, limit: {}", limit);

        let results = ndb
            .query(&txn, &[filter], limit)
            .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

        debug!("Found {} results in nostrdb", results.len());

        for query_result in results {
            let note = ndb
                .get_note_by_key(&txn, query_result.note_key)
                .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;

            let event = note_to_json(&note)?;
            all_events.push(event);
        }
    }

    Ok(all_events)
}

/// Query a single event by its ID
pub fn query_by_id(ndb: &Ndb, id: &str) -> Result<Option<Value>> {
    let id_bytes = parse_hex_id(id)?;

    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    let filter = FilterBuilder::new()
        .ids([id_bytes].iter())
        .limit(1)
        .build();

    let results = ndb
        .query(&txn, &[filter], 1)
        .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

    if let Some(query_result) = results.first() {
        let note = ndb
            .get_note_by_key(&txn, query_result.note_key)
            .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;
        Ok(Some(note_to_json(&note)?))
    } else {
        Ok(None)
    }
}

/// Query an addressable event by kind, pubkey, and d-tag
pub fn query_addressable(ndb: &Ndb, kind: u64, pubkey: &str, d_tag: &str) -> Result<Option<Value>> {
    let pubkey_bytes = parse_hex_id(pubkey)?;

    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    let filter = FilterBuilder::new()
        .kinds([kind])
        .authors([pubkey_bytes].iter())
        .tags([d_tag], 'd')
        .limit(1)
        .build();

    let results = ndb
        .query(&txn, &[filter], 1)
        .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

    if let Some(query_result) = results.first() {
        let note = ndb
            .get_note_by_key(&txn, query_result.note_key)
            .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;
        Ok(Some(note_to_json(&note)?))
    } else {
        Ok(None)
    }
}

/// Parse a hex string to a 32-byte array
fn parse_hex_id(hex_str: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str)?;
    if bytes.len() != 32 {
        return Err(EngineError::InvalidHex(format!(
            "Expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Extract limit from filter JSON
fn extract_limit(filter_json: &Value) -> Option<u64> {
    filter_json.as_object()?.get("limit")?.as_u64()
}

/// Parse a NIP-01 filter JSON into a nostrdb Filter
fn parse_filter(filter_json: &Value) -> Result<nostrdb::Filter> {
    let obj = filter_json
        .as_object()
        .ok_or_else(|| EngineError::InvalidFilter("Filter must be an object".to_string()))?;

    let mut builder = FilterBuilder::new();

    // Parse "ids" - event IDs
    if let Some(ids) = obj.get("ids").and_then(|v| v.as_array()) {
        let mut id_arrs: Vec<[u8; 32]> = Vec::new();
        for id in ids {
            if let Some(id_str) = id.as_str() {
                if let Ok(bytes) = hex::decode(id_str) {
                    if bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&bytes);
                        id_arrs.push(arr);
                    }
                }
            }
        }
        if !id_arrs.is_empty() {
            builder = builder.ids(id_arrs.iter());
        }
    }

    // Parse "authors" - pubkeys
    if let Some(authors) = obj.get("authors").and_then(|v| v.as_array()) {
        let mut author_arrs: Vec<[u8; 32]> = Vec::new();
        for author in authors {
            if let Some(pk_str) = author.as_str() {
                if let Ok(bytes) = hex::decode(pk_str) {
                    if bytes.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&bytes);
                        author_arrs.push(arr);
                    }
                }
            }
        }
        if !author_arrs.is_empty() {
            builder = builder.authors(author_arrs.iter());
        }
    }

    // Parse "kinds"
    if let Some(kinds) = obj.get("kinds").and_then(|v| v.as_array()) {
        let kind_nums: Vec<u64> = kinds.iter().filter_map(|k| k.as_u64()).collect();
        if !kind_nums.is_empty() {
            builder = builder.kinds(kind_nums);
        }
    }

    // Parse "since"
    if let Some(since) = obj.get("since").and_then(|v| v.as_u64()) {
        builder = builder.since(since);
    }

    // Parse "until"
    if let Some(until) = obj.get("until").and_then(|v| v.as_u64()) {
        builder = builder.until(until);
    }

    // Parse "limit"
    if let Some(limit) = obj.get("limit").and_then(|v| v.as_u64()) {
        builder = builder.limit(limit);
    }

    // Parse tag filters like "#e", "#p", "#d", etc.
    for (key, value) in obj.iter() {
        if key.starts_with('#') && key.len() == 2 {
            let tag_char = key.chars().nth(1).unwrap();
            if let Some(arr) = value.as_array() {
                let tag_values: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();

                if !tag_values.is_empty() {
                    match tag_char {
                        'e' => {
                            // Event references
                            for tag_val in tag_values {
                                if let Ok(bytes) = hex::decode(tag_val) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        builder = builder.event(&arr);
                                    }
                                }
                            }
                        }
                        'p' => {
                            // Pubkey references
                            let mut pk_arrs: Vec<[u8; 32]> = Vec::new();
                            for tag_val in tag_values {
                                if let Ok(bytes) = hex::decode(tag_val) {
                                    if bytes.len() == 32 {
                                        let mut arr = [0u8; 32];
                                        arr.copy_from_slice(&bytes);
                                        pk_arrs.push(arr);
                                    }
                                }
                            }
                            if !pk_arrs.is_empty() {
                                builder = builder.pubkey(pk_arrs.iter());
                            }
                        }
                        'd' => {
                            // D-tag for parameterized replaceable events
                            builder = builder.tags(tag_values.iter().copied(), 'd');
                        }
                        _ => {
                            debug!("Unsupported tag filter: {}", key);
                        }
                    }
                }
            }
        }
    }

    Ok(builder.build())
}

/// Convert a nostrdb Note to JSON event format
fn note_to_json(note: &nostrdb::Note) -> Result<Value> {
    let id_hex = hex::encode(note.id());
    let pubkey_hex = hex::encode(note.pubkey());
    let created_at = note.created_at();
    let kind = note.kind();
    let content = note.content();

    // Build tags array
    let mut tags = Vec::new();
    let note_tags = note.tags();
    for tag in note_tags.iter() {
        let mut tag_arr = Vec::new();
        for i in 0..tag.count() {
            if let Some(s) = tag.get_unchecked(i).variant().str() {
                tag_arr.push(Value::String(s.to_string()));
            }
        }
        if !tag_arr.is_empty() {
            tags.push(Value::Array(tag_arr));
        }
    }

    let sig_hex = hex::encode(note.sig());

    Ok(serde_json::json!({
        "id": id_hex,
        "pubkey": pubkey_hex,
        "created_at": created_at,
        "kind": kind,
        "tags": tags,
        "content": content,
        "sig": sig_hex
    }))
}
