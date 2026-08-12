//! Local database inventory — what is actually in nostrdb, in aggregate.
//!
//! Nostr relays that publish a stats page (pyramid, nostr.watch, …) answer
//! one question the reader of a *local-first* client also wants answered:
//! *what do I have?* Total events, the kind histogram, who wrote them, how
//! far back the archive reaches, what it costs on disk.
//!
//! Per the frontend/backend boundary, the whole derivation runs here: the
//! scan, the tallies, the kind labels, and the kind-0 author resolution.
//! The web renders rows.
//!
//! # Cost
//!
//! [`compute_inventory`] is a **full scan** of the note table. It reads
//! `kind` / `pubkey` / `created_at` straight off the flatbuffer note — no
//! `note_to_json`, no content copy — so a few hundred thousand notes cost
//! well under a second. Relay provenance (`include_relays`) opens an LMDB
//! cursor per note; it costs about a fifth again on top, and can be
//! switched off for a very large database.
//!
//! The result is cached on the [`Engine`](crate::engine::Engine) with a
//! short TTL; the UI's refresh button bypasses the cache.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::Result;

/// Upper bound on notes visited by one inventory scan. Far above any
/// realistic local DB — a backstop against an unbounded walk, not a
/// working limit. Hitting it sets `truncated`.
const SCAN_LIMIT: u64 = 20_000_000;

/// How many distinct authors to resolve and return. The tail of a Nostr
/// DB is a very long list of pubkeys with one event each; the head is
/// what a reader recognizes.
pub const DEFAULT_TOP_AUTHORS: usize = 40;

/// How many distinct relays to return, same reasoning.
pub const DEFAULT_TOP_RELAYS: usize = 40;

/// One row of the kind histogram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KindCount {
    pub kind: u32,
    pub count: usize,
    /// Human label for well-known kinds (`"long-form article"`,
    /// `"publication index"`, …). `None` for kinds we have no name for —
    /// the UI shows the bare number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One row of the author histogram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorCount {
    /// Hex pubkey — the stable key. The UI turns it into an npub/profile link.
    pub pubkey: String,
    pub count: usize,
    /// Best available kind-0 name (`display_name` then `name`). `None`
    /// when no profile for this pubkey is stored locally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One row of the relay-provenance histogram (only when `include_relays`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayCount {
    pub relay: String,
    pub count: usize,
}

/// Aggregate picture of the local nostrdb at one instant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    /// Every note in the DB, including ones no view currently surfaces.
    pub total_events: usize,
    /// Kind histogram, count descending.
    pub kinds: Vec<KindCount>,
    /// How many distinct kinds are present (`kinds.len()`, for symmetry
    /// with the author/relay rows, which are truncated).
    pub distinct_kinds: usize,
    /// Top authors by event count, count descending.
    pub authors: Vec<AuthorCount>,
    /// Distinct pubkeys across the whole DB — `authors` is only the head.
    pub distinct_authors: usize,
    /// How many of those pubkeys have a kind-0 stored locally.
    pub known_profiles: usize,
    /// Top relays by note count. Empty unless `include_relays` was set.
    pub relays: Vec<RelayCount>,
    /// Distinct relays seen across the scan (0 unless `include_relays`).
    pub distinct_relays: usize,
    /// Whether relay provenance was tallied at all — distinguishes "no
    /// relay data" from "not asked for".
    pub relays_included: bool,
    /// `created_at` bounds of the archive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newest: Option<u64>,
    /// Bytes the nostrdb data directory actually occupies on disk (the
    /// LMDB map is sparse — its apparent size is meaningless).
    pub db_bytes: u64,
    /// Vectors in the embedding index, when embeddings are enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_events: Option<usize>,
    /// Wall-clock cost of the scan that produced this snapshot.
    pub scan_ms: u64,
    /// Unix seconds when this snapshot was taken (the UI shows staleness).
    pub generated_at: u64,
    /// Set if the scan hit [`SCAN_LIMIT`] — the numbers are then floors.
    pub truncated: bool,
}

/// Knobs for one scan.
#[derive(Debug, Clone, Copy)]
pub struct InventoryOptions {
    /// Tally which relays each note came from. Opens an LMDB cursor per
    /// note; measured at roughly +20% over the base scan, which is worth
    /// it for a local-first client where provenance is a first-class
    /// question — hence on by default, with an off switch for very large
    /// databases.
    pub include_relays: bool,
    pub top_authors: usize,
    pub top_relays: usize,
}

impl Default for InventoryOptions {
    fn default() -> Self {
        Self {
            include_relays: true,
            top_authors: DEFAULT_TOP_AUTHORS,
            top_relays: DEFAULT_TOP_RELAYS,
        }
    }
}

/// Every kind [`kind_label`] names, probed individually during kind
/// discovery so a sparsely-populated one is never missed. Keep in sync
/// with the match below — `well_known_kinds_all_have_labels` enforces it.
pub const WELL_KNOWN_KINDS: &[u32] = &[
    0, 1, 3, 4, 5, 6, 7, 9, 16, 20, 777, 1040, 1063, 1111, 1222, 1244, 9021, 9735, 9802, 10000,
    10002, 10003, 10006, 10007, 30000, 30002, 30023, 30040, 30041, 30078, 30617, 30618, 30817,
    30818, 31234, 39701,
];

/// Human label for a kind, where one is well established.
///
/// Covers the kinds tendrl reads and writes, plus the ambient social
/// kinds that dominate any DB that has ever pulled from a public relay
/// — so a histogram row reads as something, not just a number.
pub fn kind_label(kind: u32) -> Option<&'static str> {
    Some(match kind {
        0 => "profile metadata",
        1 => "note",
        3 => "contacts",
        4 => "encrypted DM",
        5 => "deletion",
        6 => "repost",
        7 => "reaction",
        9 => "chat message",
        16 => "generic repost",
        20 => "picture",
        777 => "spell",
        1063 => "file metadata",
        1111 => "comment",
        1222 => "voice message",
        1244 => "voice reply",
        1040 => "OpenTimestamps",
        9021 => "join request",
        9735 => "zap receipt",
        9802 => "highlight",
        10000 => "mute list",
        10002 => "relay list",
        10003 => "bookmarks",
        10006 => "blocked relays",
        10007 => "search relays",
        30000 => "follow set",
        30002 => "relay set",
        30023 => "long-form article",
        30040 => "publication index",
        30041 => "publication section",
        30078 => "app data",
        30617 => "repository",
        30618 => "repository state",
        30817 => "wiki redirect",
        30818 => "wiki article",
        31234 => "draft",
        39701 => "web bookmark",
        _ => return None,
    })
}

/// Accumulators keyed by raw values so the hot loop never allocates —
/// hex encoding happens once per surviving row, at the end.
#[derive(Default)]
struct Acc {
    kinds: HashMap<u32, usize>,
    authors: HashMap<[u8; 32], usize>,
    relays: HashMap<String, usize>,
    total: usize,
    oldest: Option<u64>,
    newest: Option<u64>,
}

/// Scan the whole note table and tally it.
///
/// # Why this walks per kind
///
/// The obvious implementation — one pass over a limit-only filter —
/// **undercounts**, and silently. nostrdb picks a query plan from the
/// filter's shape, and a filter with no ids/kinds/authors/tags falls to
/// the created-at plan, which seeks into the note-id index starting from
/// a 32-byte "high key" declared as `{0xFF}` — C initializes only the
/// first byte, leaving the other 31 zero. Every note whose id sorts above
/// `FF 00 … 00` is therefore never visited: about 1/256 of a database,
/// measured at 0.37% on a real 39k-note store (146 notes, spread across
/// kinds).
///
/// The kinds plan keys off `(kind, timestamp)` and has no such truncated
/// key, so per-kind walks are exact. We use the cheap-but-lossy walk only
/// to *enumerate* which kinds exist — a kind can only be missed there if
/// every one of its events lands in the blind spot — and then recount
/// each one through the kind index. Well-known kinds are probed directly
/// so even a single-event one can't be lost to chance.
///
/// Takes the process-wide nostrdb read lock for the duration (see
/// `query.rs` — concurrent `ndb_query` calls corrupt the heap), so it is
/// a blocking call; callers on an async runtime should hand it to
/// `spawn_blocking`.
pub fn compute_inventory(
    ndb: &nostrdb::Ndb,
    data_dir: &std::path::Path,
    opts: InventoryOptions,
) -> Result<Inventory> {
    let started = std::time::Instant::now();

    let acc = {
        let _guard = crate::query::ndb_query_lock();
        let txn = nostrdb::Transaction::new(ndb)
            .map_err(|e| crate::error::EngineError::Database(format!("txn: {:?}", e)))?;

        let kinds = discover_kinds(ndb, &txn)?;

        let mut acc = Acc::default();
        for kind in kinds {
            tally_kind(ndb, &txn, kind, opts.include_relays, &mut acc)?;
        }
        acc
    };

    let truncated = acc.total as u64 >= SCAN_LIMIT;
    let distinct_authors = acc.authors.len();

    let mut kinds: Vec<KindCount> = acc
        .kinds
        .into_iter()
        .map(|(kind, count)| KindCount {
            kind,
            count,
            label: kind_label(kind).map(str::to_string),
        })
        .collect();
    kinds.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.kind.cmp(&b.kind)));
    let distinct_kinds = kinds.len();

    // Head of the author distribution only — the tail is thousands of
    // one-event pubkeys nobody scrolls to.
    let mut author_rows: Vec<([u8; 32], usize)> = acc.authors.into_iter().collect();
    author_rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    author_rows.truncate(opts.top_authors);

    // Name resolution is a second pass under a fresh lock: kind-0 lookups
    // per surviving row, not per note.
    let authors = resolve_author_names(ndb, &author_rows);
    let known_profiles = authors.iter().filter(|a| a.name.is_some()).count();

    let mut relays: Vec<RelayCount> = acc
        .relays
        .into_iter()
        .map(|(relay, count)| RelayCount { relay, count })
        .collect();
    relays.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.relay.cmp(&b.relay)));
    let distinct_relays = relays.len();
    relays.truncate(opts.top_relays);

    Ok(Inventory {
        total_events: acc.total,
        kinds,
        distinct_kinds,
        authors,
        distinct_authors,
        known_profiles,
        relays,
        distinct_relays,
        relays_included: opts.include_relays,
        oldest: acc.oldest,
        newest: acc.newest,
        db_bytes: dir_size_on_disk(data_dir),
        embedded_events: None,
        scan_ms: started.elapsed().as_millis() as u64,
        generated_at: now_secs(),
        truncated,
    })
}

/// Every kind present in the local database.
///
/// Takes the read lock and opens its own transaction. Costs a full walk,
/// so callers that need it per-request should cache it — the set changes
/// only when a kind is stored for the very first time.
pub fn known_kinds(ndb: &nostrdb::Ndb) -> Result<Vec<u32>> {
    let _guard = crate::query::ndb_query_lock();
    let txn = nostrdb::Transaction::new(ndb)
        .map_err(|e| crate::error::EngineError::Database(format!("txn: {:?}", e)))?;
    Ok(discover_kinds(ndb, &txn)?.into_iter().collect())
}

/// Which kinds are present in the database.
///
/// The broad walk is lossy by ~0.4% (see [`compute_inventory`]), which is
/// harmless for *enumeration* — a kind survives if any one of its events
/// is visited. The well-known kinds are then probed individually so a
/// database holding, say, a single spell can't miss it by coincidence.
fn discover_kinds(
    ndb: &nostrdb::Ndb,
    txn: &nostrdb::Transaction,
) -> Result<std::collections::BTreeSet<u32>> {
    let filter = nostrdb::FilterBuilder::new().limit(SCAN_LIMIT).build();
    let mut kinds = ndb
        .fold(
            txn,
            &[filter],
            std::collections::BTreeSet::new(),
            |mut set, note| {
                set.insert(note.kind());
                set
            },
        )
        .map_err(|e| crate::error::EngineError::Database(format!("kind discovery: {:?}", e)))?;

    for kind in WELL_KNOWN_KINDS {
        if kinds.contains(kind) {
            continue;
        }
        let probe = nostrdb::FilterBuilder::new()
            .kinds([*kind as u64])
            .limit(1)
            .build();
        let present = ndb.fold(txn, &[probe], false, |_, _| true).unwrap_or(false);
        if present {
            kinds.insert(*kind);
        }
    }

    Ok(kinds)
}

/// Fold one kind's notes into `acc`, exactly.
fn tally_kind(
    ndb: &nostrdb::Ndb,
    txn: &nostrdb::Transaction,
    kind: u32,
    include_relays: bool,
    acc: &mut Acc,
) -> Result<()> {
    let filter = nostrdb::FilterBuilder::new()
        .kinds([kind as u64])
        .limit(SCAN_LIMIT)
        .build();

    // The visitor borrows `acc` mutably; fold's accumulator is the unit
    // value because everything lands in `acc` directly.
    ndb.fold(txn, &[filter], (), |_, note| {
        acc.total += 1;
        *acc.kinds.entry(note.kind()).or_insert(0) += 1;
        *acc.authors.entry(*note.pubkey()).or_insert(0) += 1;

        // Guard against the 0 timestamps that leak in from the open
        // network — they would swallow the real span.
        let ts = note.created_at();
        if ts > 0 {
            acc.oldest = Some(acc.oldest.map_or(ts, |o: u64| o.min(ts)));
            acc.newest = Some(acc.newest.map_or(ts, |n: u64| n.max(ts)));
        }

        if include_relays {
            for relay in note.relays(txn) {
                *acc.relays.entry(relay.to_string()).or_insert(0) += 1;
            }
        }
    })
    .map_err(|e| crate::error::EngineError::Database(format!("kind {kind} scan: {:?}", e)))
}

/// Attach the best kind-0 name we hold for each pubkey.
fn resolve_author_names(ndb: &nostrdb::Ndb, rows: &[([u8; 32], usize)]) -> Vec<AuthorCount> {
    let keys: Vec<[u8; 32]> = rows.iter().map(|(pk, _)| *pk).collect();
    let names = display_names(ndb, &keys);
    rows.iter()
        .zip(names)
        .map(|((pk, count), name)| AuthorCount {
            pubkey: hex::encode(pk),
            count: *count,
            name,
        })
        .collect()
}

/// Best kind-0 name for each pubkey, positionally.
///
/// `display_name` then `name`, blanks dropped. One lock and one
/// transaction for the whole batch — callers resolve a bounded row list,
/// never one lookup per event. Shared with the search layer, which
/// labels `count:by` buckets the same way.
pub fn display_names(ndb: &nostrdb::Ndb, pubkeys: &[[u8; 32]]) -> Vec<Option<String>> {
    let _guard = crate::query::ndb_query_lock();
    // A missing txn only costs us the names, never the counts.
    let Ok(txn) = nostrdb::Transaction::new(ndb) else {
        return vec![None; pubkeys.len()];
    };

    pubkeys
        .iter()
        .map(|pk| {
            ndb.get_profile_by_pubkey(&txn, pk)
                .ok()
                .and_then(|rec| {
                    let profile = rec.record().profile()?;
                    profile
                        .display_name()
                        .or_else(|| profile.name())
                        .map(|s| s.to_string())
                })
                .filter(|s| !s.trim().is_empty())
        })
        .collect()
}

/// Bytes a directory actually occupies.
///
/// LMDB's `data.mdb` is a sparse file whose apparent length is the map
/// size (gigabytes, mostly hole), so `metadata().len()` would report a
/// number an order of magnitude too large. On unix we use the allocated
/// block count instead; elsewhere the apparent size is the best available.
fn dir_size_on_disk(dir: &std::path::Path) -> u64 {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                meta.blocks() * 512
            }
            #[cfg(not(unix))]
            {
                meta.len()
            }
        })
        .sum()
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_cover_the_app_kinds() {
        // The kinds tendrl itself reads and writes must never render as a
        // bare number in the inventory.
        for kind in [0, 1, 777, 9802, 30023, 30040, 30041, 30818] {
            assert!(kind_label(kind).is_some(), "kind {kind} has no label");
        }
    }

    #[test]
    fn unknown_kinds_have_no_label() {
        assert_eq!(kind_label(424242), None);
    }

    #[test]
    fn well_known_kinds_all_have_labels() {
        // The probe list and the label table are two halves of one fact;
        // a kind in one and not the other is a bug in whichever was
        // edited alone.
        for kind in WELL_KNOWN_KINDS {
            assert!(
                kind_label(*kind).is_some(),
                "kind {kind} probed but unnamed"
            );
        }
    }

    #[test]
    fn default_options_bound_the_row_lists() {
        // The author/relay tails are unbounded in principle (one row per
        // distinct pubkey in the DB); the defaults must cap them so a
        // response never grows with database size.
        let opts = InventoryOptions::default();
        assert!(opts.top_authors > 0 && opts.top_authors <= 100);
        assert!(opts.top_relays > 0 && opts.top_relays <= 100);
    }
}
