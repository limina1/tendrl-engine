//! Local nostrdb query interface
//!
//! Provides NIP-01 filter-based querying against the local nostrdb database.

use crate::error::{EngineError, Result};
use nostrdb::{FilterBuilder, Ndb, Transaction};
use serde_json::Value;
use std::sync::{Mutex, MutexGuard};
use tracing::debug;

/// Serializes nostrdb read queries.
///
/// nostrdb-rs's `Ndb::query` bitwise-copies each `ndb_filter` struct
/// (`filters.iter().map(|a| a.data)`), and every `ndb_filter` owns a
/// ~1 MiB heap buffer (`elem_buf.start`, malloc'd by
/// `ndb_filter_init_with`). Two `ndb_query` calls running concurrently
/// corrupt that buffer's heap metadata and abort the process with
/// "double free or corruption" — confirmed from a core dump:
/// `ndb_filter_destroy → free → SIGABRT` inside `query_local`.
///
/// Until the binding is fixed upstream, every nostrdb read acquires
/// this lock. For a single-user local engine the cost is negligible:
/// queries are sub-millisecond and there is no real read parallelism
/// to lose.
static NDB_QUERY_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the nostrdb query lock. Recovers from a poisoned mutex
/// (a query holder that panicked) rather than propagating the panic —
/// a recovered guard is strictly better than cascading failures.
pub fn ndb_query_lock() -> MutexGuard<'static, ()> {
    NDB_QUERY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Query events from local nostrdb using NIP-01 filters
pub fn query_local(ndb: &Ndb, filters: &[Value]) -> Result<Vec<Value>> {
    let _guard = ndb_query_lock();
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

            let event = note_to_json(&note, &txn)?;
            all_events.push(event);
        }
    }

    Ok(all_events)
}

/// Exhaustive local scan with the text filter applied at the raw
/// nostrdb-note level.
///
/// A keyword search has no usable DB index — nostrdb's full-text index
/// covers only kinds 1 and 30023, never NKBIP-01 publications — so it
/// must scan. But converting every scanned note to JSON dominates the
/// cost, so the match (`content` + `title` tag, the same test
/// `filter_by_text` does) runs on the `Note` directly and only matches
/// pay `note_to_json`. Stops once `limit` matches are collected; nostrdb
/// yields notes newest-first, so those are the newest `limit` matches.
pub fn query_local_text(
    ndb: &Ndb,
    filters: &[Value],
    text: &crate::search::TextFilter,
    limit: usize,
) -> Result<Vec<Value>> {
    use crate::search::TextFilter;
    let _guard = ndb_query_lock();
    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    // Lowercase the needles once up front. Keywords (every word must
    // appear) and Exact (the phrase must appear) both reduce to "every
    // needle is a substring of content+title".
    let needles: Vec<String> = match text {
        TextFilter::Keywords(words) => words.iter().map(|w| w.to_lowercase()).collect(),
        TextFilter::Exact(phrase) => vec![phrase.to_lowercase()],
    };

    let mut matched = Vec::new();
    for filter_json in filters {
        if matched.len() >= limit {
            break;
        }
        let filter = parse_filter(filter_json)?;
        let scan_limit = extract_limit(filter_json).unwrap_or(100) as i32;
        let results = ndb
            .query(&txn, &[filter], scan_limit)
            .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

        for query_result in results {
            if matched.len() >= limit {
                break;
            }
            let note = ndb
                .get_note_by_key(&txn, query_result.note_key)
                .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;

            // The `title` tag is searched alongside content — a
            // publication index's name lives there, not in `content`.
            let mut title = "";
            for tag in note.tags().iter() {
                if tag.count() < 2 {
                    continue;
                }
                if matches!(
                    tag.get_unchecked(0).variant(),
                    nostrdb::NdbStrVariant::Str("title")
                ) {
                    if let nostrdb::NdbStrVariant::Str(s) = tag.get_unchecked(1).variant() {
                        title = s;
                    }
                    break;
                }
            }

            let haystack = format!("{} {}", note.content(), title).to_lowercase();
            if needles.iter().all(|n| haystack.contains(n.as_str())) {
                matched.push(note_to_json(&note, &txn)?);
            }
        }
    }

    Ok(matched)
}

/// Query a single event by its ID
pub fn query_by_id(ndb: &Ndb, id: &str) -> Result<Option<Value>> {
    let id_bytes = parse_hex_id(id)?;

    let _guard = ndb_query_lock();
    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    let filter = FilterBuilder::new().ids([id_bytes].iter()).limit(1).build();

    let results = ndb
        .query(&txn, &[filter], 1)
        .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

    if let Some(query_result) = results.first() {
        let note = ndb
            .get_note_by_key(&txn, query_result.note_key)
            .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;
        Ok(Some(note_to_json(&note, &txn)?))
    } else {
        Ok(None)
    }
}

/// Query an addressable event by kind, pubkey, and d-tag
///
/// For NIP-33 replaceable events, this returns the newest version (highest created_at)
/// since nostrdb stores all versions without auto-replacement.
pub fn query_addressable(ndb: &Ndb, kind: u64, pubkey: &str, d_tag: &str) -> Result<Option<Value>> {
    let pubkey_bytes = parse_hex_id(pubkey)?;

    let _guard = ndb_query_lock();
    let txn = Transaction::new(ndb)
        .map_err(|e| EngineError::Database(format!("Failed to create transaction: {}", e)))?;

    // Query multiple results since nostrdb may have multiple versions of the same addressable event
    let filter = FilterBuilder::new()
        .kinds([kind])
        .authors([pubkey_bytes].iter())
        .tags([d_tag], 'd')
        .limit(100)
        .build();

    let results = ndb
        .query(&txn, &[filter], 100)
        .map_err(|e| EngineError::Database(format!("Query failed: {}", e)))?;

    // Find the newest version by created_at
    let mut newest: Option<(u64, Value)> = None;
    for query_result in results {
        let note = ndb
            .get_note_by_key(&txn, query_result.note_key)
            .map_err(|e| EngineError::Database(format!("Failed to get note: {}", e)))?;
        let created_at = note.created_at();

        match &newest {
            Some((best_time, _)) if created_at <= *best_time => continue,
            _ => {
                let event = note_to_json(&note, &txn)?;
                newest = Some((created_at, event));
            }
        }
    }

    Ok(newest.map(|(_, event)| event))
}

/// Parse a hex pubkey to a 32-byte array (public for profile queries)
pub fn parse_hex_pubkey(hex_str: &str) -> Result<[u8; 32]> {
    parse_hex_id(hex_str)
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
                            builder = builder.tags(tag_values.iter().copied(), tag_char);
                        }
                    }
                }
            }
        }
    }

    Ok(builder.build())
}

/// Does an event satisfy at least one of the NIP-01 filters?
///
/// Filters are OR'd (NIP-01 semantics). Used to keep relay over-returns
/// out of merged results: a relay that ignores or loosely matches a
/// tag filter (notably uppercase `#A`/`#E` root-scope tags) can echo
/// back far more than was asked for, and those raw events must not be
/// trusted into the response just because they aren't in the local DB.
pub fn event_matches_filters(event: &Value, filters: &[Value]) -> bool {
    filters.iter().any(|f| event_matches_filter(event, f))
}

fn event_matches_filter(event: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return false;
    };

    if let Some(kinds) = filter.get("kinds").and_then(|v| v.as_array()) {
        let k = event.get("kind").and_then(|v| v.as_u64());
        if !kinds.iter().filter_map(|v| v.as_u64()).any(|kk| Some(kk) == k) {
            return false;
        }
    }
    if let Some(authors) = filter.get("authors").and_then(|v| v.as_array()) {
        let pk = event.get("pubkey").and_then(|v| v.as_str());
        if !authors.iter().filter_map(|v| v.as_str()).any(|a| Some(a) == pk) {
            return false;
        }
    }
    if let Some(ids) = filter.get("ids").and_then(|v| v.as_array()) {
        let id = event.get("id").and_then(|v| v.as_str());
        if !ids.iter().filter_map(|v| v.as_str()).any(|i| Some(i) == id) {
            return false;
        }
    }
    let created = event.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0);
    if let Some(since) = filter.get("since").and_then(|v| v.as_i64()) {
        if created < since {
            return false;
        }
    }
    if let Some(until) = filter.get("until").and_then(|v| v.as_i64()) {
        if created > until {
            return false;
        }
    }

    // `#<x>` tag filters — the event must carry a matching tag for each.
    let tags = event.get("tags").and_then(|v| v.as_array());
    for (key, value) in filter {
        if !key.starts_with('#') || key.len() != 2 {
            continue;
        }
        let tag_name = &key[1..];
        let wanted: Vec<&str> = value
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        if wanted.is_empty() {
            continue;
        }
        let has = tags
            .map(|tags| {
                tags.iter().any(|t| {
                    t.as_array()
                        .map(|t| {
                            t.first().and_then(|v| v.as_str()) == Some(tag_name)
                                && t.get(1)
                                    .and_then(|v| v.as_str())
                                    .map(|val| wanted.contains(&val))
                                    .unwrap_or(false)
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if !has {
            return false;
        }
    }
    true
}

/// Post-filter events by text content (and title tag), case-insensitive.
///
/// Post-filter events by tag filters that NIP-01 / nostrdb can't index.
///
/// NIP-01 only indexes single-letter tag-filter keys (`#t`, `#d`, etc.).
/// Multi-char tag names like `author`, `client`, `imeta`, `alt`, …
/// pass through nostrdb without filtering — we apply them here by
/// walking each event's `tags` array.
///
/// **Matching is case-insensitive substring** so `author:liminal` finds
/// events whose author tag is `"liminal 🌑"` or `"Liminal Day"`. To force
/// exact match, callers can post-process the results themselves; in
/// practice, surface noise from substring is preferable to missing hits
/// when display-name conventions vary (emoji, casing, last names).
///
/// Each event must satisfy ALL multi-char filters (AND across filters,
/// OR within a single filter's values — same semantics as NIP-01).
/// Single-char filters are skipped here since they were already applied
/// in the DB query.
pub fn filter_by_tags(events: &[Value], tag_filters: &[crate::search::TagFilter]) -> Vec<Value> {
    let multi_char: Vec<&crate::search::TagFilter> = tag_filters
        .iter()
        .filter(|tf| tf.tag_name.chars().count() > 1)
        .collect();
    if multi_char.is_empty() {
        return events.to_vec();
    }

    // Lowercase the filter values once up front, expanding each to its
    // lowercase/slug variants so a typed `author:"Some Name"` also hits a
    // stored `some-name` (the same normalization single-char tags get in
    // `SearchQuery::to_nip01_filters`).
    let needles: Vec<(String, Vec<String>)> = multi_char
        .iter()
        .map(|tf| {
            (
                tf.tag_name.clone(),
                tf.values
                    .iter()
                    .flat_map(|v| crate::search::tag_value_variants(v))
                    .map(|v| v.to_lowercase())
                    .collect(),
            )
        })
        .collect();

    events
        .iter()
        .filter(|event| {
            let tags = match event.get("tags").and_then(|t| t.as_array()) {
                Some(t) => t,
                None => return false,
            };
            needles.iter().all(|(name, values)| {
                tags.iter().any(|tag| {
                    let arr = match tag.as_array() {
                        Some(a) => a,
                        None => return false,
                    };
                    let tag_name = arr.first().and_then(|v| v.as_str()).unwrap_or("");
                    if tag_name != name.as_str() {
                        return false;
                    }
                    let tag_val = arr.get(1).and_then(|v| v.as_str()).unwrap_or("");
                    let tag_val_lc = tag_val.to_lowercase();
                    values.iter().any(|needle| tag_val_lc.contains(needle))
                })
            })
        })
        .cloned()
        .collect()
}

/// Aggregate distinct tag values for each requested name across `events`.
///
/// Backs `count:NAME` queries. For each requested tag name, walks the
/// events and tallies how many events carry each distinct value, AND
/// records the contributing event ids so the UI can unfold a group into
/// its members. Returns a map keyed by tag name; each list is sorted by
/// count descending, then alphabetically for ties.
///
/// Counting is per-event (an event with two `["author","Claude"]` tags
/// still only adds 1 to Claude's bucket), and case-sensitive (matches
/// the raw tag value).
pub fn count_tag_values(
    events: &[Value],
    names: &[String],
) -> std::collections::HashMap<String, Vec<crate::search::TagValueCount>> {
    use std::collections::HashMap;

    // tag_name → value → Vec<event_id> (insertion-ordered = recency-ordered
    // because callers pass events in the order they came back from the DB).
    let mut buckets: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for name in names {
        buckets.insert(name.clone(), HashMap::new());
    }

    for event in events {
        let event_id = event
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let tags = match event.get("tags").and_then(|t| t.as_array()) {
            Some(t) => t,
            None => continue,
        };
        // Per-event de-dup so repeated tags on one event don't double-count.
        let mut seen: HashMap<&str, std::collections::HashSet<String>> = HashMap::new();
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) => a,
                None => continue,
            };
            let name = match arr.first().and_then(|v| v.as_str()) {
                Some(s) => s,
                None => continue,
            };
            if !buckets.contains_key(name) {
                continue;
            }
            let value = arr
                .get(1)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            seen.entry(name).or_default().insert(value);
        }
        for (name, values) in seen {
            let bucket = buckets.get_mut(name).expect("bucket exists");
            for v in values {
                bucket.entry(v).or_default().push(event_id.clone());
            }
        }
    }

    buckets
        .into_iter()
        .map(|(name, value_buckets)| {
            let mut entries: Vec<crate::search::TagValueCount> = value_buckets
                .into_iter()
                .map(|(value, event_ids)| crate::search::TagValueCount {
                    value,
                    count: event_ids.len(),
                    event_ids,
                })
                .collect();
            entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
            (name, entries)
        })
        .collect()
}

/// Keep only events that have at least one tag for each named key.
///
/// Backing `has:NAME` queries. Pure presence test (no value match), AND
/// across multiple names.
pub fn filter_by_has_tags(events: &[Value], names: &[String]) -> Vec<Value> {
    if names.is_empty() {
        return events.to_vec();
    }
    events
        .iter()
        .filter(|event| {
            let tags = match event.get("tags").and_then(|t| t.as_array()) {
                Some(t) => t,
                None => return false,
            };
            names.iter().all(|name| {
                tags.iter().any(|tag| {
                    tag.as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .map(|n| n == name)
                        .unwrap_or(false)
                })
            })
        })
        .cloned()
        .collect()
}

/// - Keywords: all words must appear (AND, order-independent)
/// - Exact: content must contain the substring
pub fn filter_by_text(events: &[Value], filter: &crate::search::TextFilter) -> Vec<Value> {
    events
        .iter()
        .filter(|event| {
            let content = event.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let title = event
                .get("tags")
                .and_then(|t| t.as_array())
                .and_then(|tags| {
                    tags.iter().find_map(|tag| {
                        let arr = tag.as_array()?;
                        if arr.first()?.as_str()? == "title" {
                            arr.get(1)?.as_str()
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or("");

            let searchable = format!("{} {}", content, title);
            let lower = searchable.to_lowercase();

            match filter {
                crate::search::TextFilter::Keywords(words) => {
                    words.iter().all(|w| lower.contains(&w.to_lowercase()))
                }
                crate::search::TextFilter::Exact(phrase) => lower.contains(&phrase.to_lowercase()),
            }
        })
        .cloned()
        .collect()
}

/// Convert a nostrdb Note to JSON event format (public for profile queries)
pub fn note_to_json_pub(note: &nostrdb::Note, txn: &Transaction) -> Result<Value> {
    note_to_json(note, txn)
}

/// True when a kind-0 (profile metadata) event for `pubkey` is already
/// cached locally. Used to decide whether an author needs a profile
/// backfill from relays. A malformed pubkey reports `false`.
pub fn profile_exists(ndb: &Ndb, pubkey: &str) -> bool {
    let Ok(pk) = parse_hex_id(pubkey) else {
        return false;
    };
    let _guard = ndb_query_lock();
    let Ok(txn) = Transaction::new(ndb) else {
        return false;
    };
    let filter = FilterBuilder::new()
        .kinds([0])
        .authors([pk].iter())
        .limit(1)
        .build();
    ndb.query(&txn, &[filter], 1)
        .map(|r| !r.is_empty())
        .unwrap_or(false)
}

/// How many kind-0 events the profile scan walks, and how many hits it
/// returns. The scan is brute-force (NIP-01 has no name index); the cap
/// keeps a noisy term from flooding the people category.
const PROFILE_SCAN_LIMIT: u64 = 2000;
const PROFILE_RESULT_CAP: usize = 50;

/// Score a profile's text fields against lowercased keyword `needles`.
///
/// `None` = no match. Otherwise: `0` = the term is a prefix of `name`
/// or `display_name`; `1` = every keyword appears somewhere in the name
/// fields; `2` = the match only lands on an identifier field
/// (nip05 / lud16 / website). Pure — unit-tested without an `Ndb`.
fn score_profile_fields(
    name: &str,
    display_name: &str,
    nip05: &str,
    lud16: &str,
    website: &str,
    needles: &[String],
) -> Option<u8> {
    if needles.is_empty() {
        return None;
    }
    let name_lc = name.to_lowercase();
    let display_lc = display_name.to_lowercase();
    let name_hay = format!("{name_lc} {display_lc}");
    let id_hay = format!(
        "{} {} {}",
        nip05.to_lowercase(),
        lud16.to_lowercase(),
        website.to_lowercase()
    );
    let full = format!("{name_hay} {id_hay}");
    if !needles.iter().all(|n| full.contains(n.as_str())) {
        return None;
    }
    let first = needles[0].as_str();
    if name_lc.starts_with(first) || display_lc.starts_with(first) {
        Some(0)
    } else if needles.iter().all(|n| name_hay.contains(n.as_str())) {
        Some(1)
    } else {
        Some(2)
    }
}

/// Scan local kind-0 events and return the profiles matching `term`.
///
/// The "people" half of search's fan-out — a port of Amethyst's
/// `findUsersStartingWith` (see `docs/search-architecture.org` §3.1,
/// §14). `term` is split into whitespace keywords; a profile matches
/// when *every* keyword appears (case-insensitive substring) across its
/// name, display_name, nip05, lud16, or website. A `term` that is a
/// bare 64-hex pubkey short-circuits to a direct lookup of that author.
///
/// Only the newest kind-0 per pubkey is considered. Results are scored
/// (`score_profile_fields`) and returned sorted by score then display
/// name, capped at `PROFILE_RESULT_CAP`.
pub fn find_profiles_matching(ndb: &Ndb, term: &str) -> Vec<crate::search::ProfileResult> {
    use crate::search::{ProfileResult, ResultSource};

    let term = term.trim();
    if term.is_empty() {
        return Vec::new();
    }
    let needles: Vec<String> = term.split_whitespace().map(|w| w.to_lowercase()).collect();
    if needles.is_empty() {
        return Vec::new();
    }

    // A bare 64-hex term is a pubkey, not a name — look it up directly.
    let direct_hex = term.len() == 64 && term.chars().all(|c| c.is_ascii_hexdigit());

    let _guard = ndb_query_lock();
    let txn = match Transaction::new(ndb) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    let filter = if direct_hex {
        let mut pk = [0u8; 32];
        match hex::decode(term) {
            Ok(b) if b.len() == 32 => pk.copy_from_slice(&b),
            _ => return Vec::new(),
        }
        FilterBuilder::new()
            .kinds([0])
            .authors([pk].iter())
            .limit(20)
            .build()
    } else {
        FilterBuilder::new()
            .kinds([0])
            .limit(PROFILE_SCAN_LIMIT)
            .build()
    };

    let results = match ndb.query(&txn, &[filter], PROFILE_SCAN_LIMIT as i32) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    // Collapse to the newest kind-0 per pubkey.
    let mut newest: std::collections::HashMap<String, (u64, Value)> =
        std::collections::HashMap::new();
    for qr in results {
        let Ok(note) = ndb.get_note_by_key(&txn, qr.note_key) else {
            continue;
        };
        let Ok(event) = note_to_json(&note, &txn) else {
            continue;
        };
        let pubkey = event
            .get("pubkey")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if pubkey.len() != 64 {
            continue;
        }
        let created_at = event
            .get("created_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        match newest.get(&pubkey) {
            Some((ts, _)) if *ts >= created_at => {}
            _ => {
                newest.insert(pubkey, (created_at, event));
            }
        }
    }

    let mut out: Vec<ProfileResult> = Vec::new();
    for (pubkey, (_, event)) in newest {
        let content = event.get("content").and_then(|v| v.as_str()).unwrap_or("");
        // Single source of truth for kind-0 parsing (shared with the profile
        // endpoint); the inline JSON field-picking twin was deleted.
        let meta = crate::user_data::Metadata::from_event_content(content, 0).unwrap_or_default();
        let name = meta.name.unwrap_or_default();
        let display_name = meta.display_name.unwrap_or_default();
        let nip05 = meta.nip05.unwrap_or_default();
        let lud16 = meta.lud16.unwrap_or_default();
        let website = meta.website.unwrap_or_default();
        let picture = meta.picture.unwrap_or_default();
        let about = meta.about.unwrap_or_default();

        // A direct hex lookup always yields its profile (strongest score);
        // a name scan requires an actual field match.
        let score = if direct_hex {
            0
        } else {
            match score_profile_fields(&name, &display_name, &nip05, &lud16, &website, &needles) {
                Some(s) => s,
                None => continue,
            }
        };

        let opt = |s: String| (!s.is_empty()).then_some(s);
        out.push(ProfileResult {
            pubkey,
            name: opt(name),
            display_name: opt(display_name),
            nip05: opt(nip05),
            picture: opt(picture),
            about: opt(about),
            score,
            source: ResultSource::Local,
        });
    }

    out.sort_by(|a, b| {
        a.score.cmp(&b.score).then_with(|| {
            let an = a
                .display_name
                .as_deref()
                .or(a.name.as_deref())
                .unwrap_or("")
                .to_lowercase();
            let bn = b
                .display_name
                .as_deref()
                .or(b.name.as_deref())
                .unwrap_or("")
                .to_lowercase();
            an.cmp(&bn).then_with(|| a.pubkey.cmp(&b.pubkey))
        })
    });
    out.truncate(PROFILE_RESULT_CAP);
    out
}

/// Convert a nostrdb Note to JSON event format
fn note_to_json(note: &nostrdb::Note, txn: &Transaction) -> Result<Value> {
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
            // nostrdb stores 32-byte ids/pubkeys (in `e`/`p`/`E`/`P`/`a`
            // etc. tags) as a binary `Id` variant, not a string. `.str()`
            // returns `None` for those — handling only `str()` silently
            // drops every id and pubkey, collapsing `["e", id, relay,
            // pubkey]` into `["e", relay]`. Hex-encode the `Id` variant.
            match tag.get_unchecked(i).variant() {
                nostrdb::NdbStrVariant::Str(s) => tag_arr.push(Value::String(s.to_string())),
                nostrdb::NdbStrVariant::Id(id) => {
                    tag_arr.push(Value::String(hex::encode(id)))
                }
            }
        }
        if !tag_arr.is_empty() {
            tags.push(Value::Array(tag_arr));
        }
    }

    let sig_hex = hex::encode(note.sig());

    // Relays this note has been seen on (empty = written locally, never
    // fetched from or broadcast to a relay). nostrdb records relay
    // provenance per event id via `IngestMetadata::relay`.
    let relays: Vec<Value> = note
        .relays(txn)
        .map(|r| Value::String(r.to_string()))
        .collect();

    Ok(serde_json::json!({
        "id": id_hex,
        "pubkey": pubkey_hex,
        "created_at": created_at,
        "kind": kind,
        "tags": tags,
        "content": content,
        "sig": sig_hex,
        "relays": relays
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::TextFilter;
    use serde_json::json;

    #[test]
    fn test_filter_by_tags_multi_char() {
        use crate::search::TagFilter;
        let events = vec![
            json!({"tags": [["author", "Claude"], ["t", "tech"]]}),
            json!({"tags": [["author", "Pablo"]]}),
            json!({"tags": [["t", "tech"]]}),
            json!({"tags": [["author", "Claude"], ["author", "Pablo"]]}),
        ];
        let filters = vec![TagFilter {
            tag_name: "author".to_string(),
            values: vec!["Claude".to_string()],
        }];
        let result = filter_by_tags(&events, &filters);
        // Events 0 and 3 have an "author"=>"Claude" tag.
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_tags_substring_case_insensitive() {
        use crate::search::TagFilter;
        // `author:liminal` should match "liminal 🌑", "Liminal Day",
        // "the liminal night" — substring + case-insensitive.
        let events = vec![
            json!({"tags": [["author", "liminal 🌑"]]}),
            json!({"tags": [["author", "Liminal Day"]]}),
            json!({"tags": [["author", "the liminal night"]]}),
            json!({"tags": [["author", "Claude"]]}),
        ];
        let filters = vec![TagFilter {
            tag_name: "author".to_string(),
            values: vec!["liminal".to_string()],
        }];
        let result = filter_by_tags(&events, &filters);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_filter_by_tags_quoted_phrase() {
        use crate::search::TagFilter;
        // `author:"word sequence"` — the tokenizer hands us the spaces
        // intact; we substring-match the phrase.
        let events = vec![
            json!({"tags": [["author", "alice in wonderland"]]}),
            json!({"tags": [["author", "alice"]]}),
        ];
        let filters = vec![TagFilter {
            tag_name: "author".to_string(),
            values: vec!["in wonderland".to_string()],
        }];
        let result = filter_by_tags(&events, &filters);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_count_tag_values_basic() {
        let events = vec![
            json!({"id": "e1", "tags": [["author", "Claude"]]}),
            json!({"id": "e2", "tags": [["author", "Claude"]]}),
            json!({"id": "e3", "tags": [["author", "Pablo"]]}),
            json!({"id": "e4", "tags": [["author", "liminal 🌑"]]}),
            json!({"id": "e5", "tags": [["t", "tech"]]}),
        ];
        let result = count_tag_values(&events, &["author".to_string()]);
        let authors = &result["author"];
        // Sorted desc by count; ties broken alphabetically.
        assert_eq!(authors[0].value, "Claude");
        assert_eq!(authors[0].count, 2);
        assert_eq!(
            authors[0].event_ids,
            vec!["e1".to_string(), "e2".to_string()]
        );
        assert_eq!(authors[1].value, "Pablo");
        assert_eq!(authors[1].event_ids, vec!["e3".to_string()]);
        assert_eq!(authors[2].value, "liminal 🌑");
        assert_eq!(authors[2].event_ids, vec!["e4".to_string()]);
    }

    #[test]
    fn test_count_tag_values_dedupes_within_event() {
        // Two `["author", "Claude"]` on one event count as 1, and the id
        // appears once in event_ids (not twice).
        let events =
            vec![json!({"id": "e1", "tags": [["author", "Claude"], ["author", "Claude"]]})];
        let result = count_tag_values(&events, &["author".to_string()]);
        assert_eq!(result["author"][0].count, 1);
        assert_eq!(result["author"][0].event_ids, vec!["e1".to_string()]);
    }

    #[test]
    fn test_count_tag_values_multiple_names() {
        let events = vec![
            json!({"id": "e1", "tags": [["author", "Claude"], ["client", "tendrl"]]}),
            json!({"id": "e2", "tags": [["author", "Pablo"], ["client", "amethyst"]]}),
        ];
        let result = count_tag_values(&events, &["author".to_string(), "client".to_string()]);
        assert_eq!(result["author"].len(), 2);
        assert_eq!(result["client"].len(), 2);
        // Same event can appear in multiple buckets — once per tag name.
        let claude_bucket = result["author"]
            .iter()
            .find(|b| b.value == "Claude")
            .unwrap();
        assert_eq!(claude_bucket.event_ids, vec!["e1".to_string()]);
        let tendrl_bucket = result["client"]
            .iter()
            .find(|b| b.value == "tendrl")
            .unwrap();
        assert_eq!(tendrl_bucket.event_ids, vec!["e1".to_string()]);
    }

    #[test]
    fn test_filter_by_has_tags_presence() {
        let events = vec![
            json!({"tags": [["author", "Claude"]]}),
            json!({"tags": [["author", "Pablo"], ["t", "tech"]]}),
            json!({"tags": [["t", "tech"]]}),
            json!({"tags": []}),
        ];
        let result = filter_by_has_tags(&events, &["author".to_string()]);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_has_tags_and_across_names() {
        let events = vec![
            json!({"tags": [["author", "Claude"], ["client", "tendrl"]]}),
            json!({"tags": [["author", "Pablo"]]}),
            json!({"tags": [["client", "amethyst"]]}),
        ];
        // Both names must be present (AND).
        let result = filter_by_has_tags(&events, &["author".to_string(), "client".to_string()]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_by_has_tags_empty_names() {
        // No filters → pass through everything.
        let events = vec![json!({"tags": []})];
        let result = filter_by_has_tags(&events, &[]);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_by_tags_single_char_passthrough() {
        use crate::search::TagFilter;
        // Single-char filters are applied by nostrdb, not here — this helper
        // ignores them and returns everything.
        let events = vec![
            json!({"tags": [["t", "rust"]]}),
            json!({"tags": [["t", "python"]]}),
        ];
        let filters = vec![TagFilter {
            tag_name: "t".to_string(),
            values: vec!["rust".to_string()],
        }];
        let result = filter_by_tags(&events, &filters);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_by_text_keywords_single() {
        let events = vec![
            json!({"content": "hello world", "tags": []}),
            json!({"content": "goodbye world", "tags": []}),
        ];
        let filter = TextFilter::Keywords(vec!["hello".to_string()]);
        let result = filter_by_text(&events, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["content"], "hello world");
    }

    #[test]
    fn test_filter_by_text_keywords_multi_and() {
        let events = vec![
            json!({"content": "hello world tutorial", "tags": []}),
            json!({"content": "hello universe", "tags": []}),
        ];
        let filter = TextFilter::Keywords(vec!["hello".to_string(), "tutorial".to_string()]);
        let result = filter_by_text(&events, &filter);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["content"], "hello world tutorial");
    }

    #[test]
    fn test_filter_by_text_keywords_no_match() {
        let events = vec![json!({"content": "hello world", "tags": []})];
        let filter = TextFilter::Keywords(vec!["python".to_string()]);
        let result = filter_by_text(&events, &filter);
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_by_text_exact() {
        let events = vec![
            json!({"content": "this is an exact phrase test", "tags": []}),
            json!({"content": "exact test", "tags": []}),
        ];
        let filter = TextFilter::Exact("exact phrase".to_string());
        let result = filter_by_text(&events, &filter);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_filter_by_text_case_insensitive() {
        let events = vec![json!({"content": "Hello WORLD", "tags": []})];
        let filter = TextFilter::Exact("hello world".to_string());
        let result = filter_by_text(&events, &filter);
        assert_eq!(result.len(), 1);
    }

    fn needles(term: &str) -> Vec<String> {
        term.split_whitespace().map(|w| w.to_lowercase()).collect()
    }

    #[test]
    fn test_score_profile_name_prefix() {
        // Term is a prefix of `name` → strongest (0).
        let s = score_profile_fields("fiatjaf", "", "", "", "", &needles("fia"));
        assert_eq!(s, Some(0));
        // Prefix of display_name also counts.
        let s = score_profile_fields("", "Fiatjaf", "", "", "", &needles("fia"));
        assert_eq!(s, Some(0));
    }

    #[test]
    fn test_score_profile_name_substring() {
        // Match inside the name, not at the start → 1.
        let s = score_profile_fields("the fiatjaf", "", "", "", "", &needles("fiat"));
        assert_eq!(s, Some(1));
    }

    #[test]
    fn test_score_profile_identifier_only() {
        // Match lands only on nip05 / website → weakest (2).
        let s = score_profile_fields("alice", "Alice", "bob@example.com", "", "", &needles("bob"));
        assert_eq!(s, Some(2));
        let s = score_profile_fields("alice", "", "", "", "https://carol.dev", &needles("carol"));
        assert_eq!(s, Some(2));
    }

    #[test]
    fn test_score_profile_no_match() {
        let s = score_profile_fields("alice", "Alice", "alice@x.com", "", "", &needles("zzz"));
        assert_eq!(s, None);
        // Empty needles never match.
        assert_eq!(score_profile_fields("alice", "", "", "", "", &[]), None);
    }

    #[test]
    fn test_score_profile_multi_keyword_and() {
        // Every keyword must appear; order-independent, across fields.
        let s = score_profile_fields("Jack Dorsey", "", "", "", "", &needles("dorsey jack"));
        assert_eq!(s, Some(1));
        // One keyword missing → no match.
        let s = score_profile_fields("Jack Dorsey", "", "", "", "", &needles("jack twitter"));
        assert_eq!(s, None);
    }

    #[test]
    fn test_filter_by_text_matches_title_tag() {
        let events = vec![
            json!({"content": "body text", "tags": [["title", "Python Tutorial"]]}),
            json!({"content": "other content", "tags": []}),
        ];
        let filter = TextFilter::Keywords(vec!["python".to_string()]);
        let result = filter_by_text(&events, &filter);
        assert_eq!(result.len(), 1);
    }
}
