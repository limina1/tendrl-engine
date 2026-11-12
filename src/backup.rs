//! Backup broadcast — push everything connected to a document to relays.
//!
//! Local mode (`NetworkMode::Local`) signs and stores but never sends.
//! When the user *wants* a copy on a relay, the reader's backup modal asks
//! for the whole connected set — the publication tree (index, nested
//! indexes, sections) and what hangs off it (highlights, comments,
//! annotations, narrations, merge requests, deletions, replies to any of
//! those) — and lets them pick events and destination relays. The pick is
//! the approval: the broadcast runs in any network mode, under a
//! [`crate::relay::RelayPermit`] for exactly the chosen relays.
//!
//! Pure: [`role_of`], [`label_of`], [`deleted_targets`], [`relay_list`],
//! [`for_relay`]. IO: [`collect`] (local store only — never a relay) and
//! the two handlers.

use crate::attached::{children_of, coord_of, dedup_newest};
use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::publication::{KIND_PUBLICATION_INDEX, KIND_PUBLICATION_SECTION};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

/// Hard ceiling on the connected set — a runaway thread can't stall the
/// modal; the plan says when it was hit.
pub const MAX_EVENTS: usize = 5000;
/// Tree depth walked from the root (nested indexes).
const MAX_DEPTH: usize = 8;
/// Hops out from the tree: 1 = things on the tree, 2 = things on those
/// (an annotation on a highlight, a reply to a comment).
const ATTACH_HOPS: usize = 2;

/// One event the modal lists.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BackupRow {
    pub id: String,
    pub kind: u64,
    pub kind_name: Option<String>,
    pub pubkey: String,
    /// Best kind-0 name held locally.
    pub author: Option<String>,
    pub created_at: u64,
    /// index · section · highlight · comment · annotation · narration ·
    /// merge request · deletion · reaction · reference.
    pub role: String,
    /// Title or a one-line preview.
    pub label: String,
    pub coord: Option<String>,
    /// Relays nostrdb says it was seen on / sent to (normalized). Empty =
    /// local only.
    pub relays: Vec<String>,
    /// A kind-5 by the same author targets it. Listed, not preselected.
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayChoice {
    pub url: String,
    /// In the user's publish (write) set — preselected.
    pub publish: bool,
    /// In the broadcast (aggregator) set.
    pub broadcast: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupPlan {
    pub root: String,
    pub title: Option<String>,
    pub rows: Vec<BackupRow>,
    pub relays: Vec<RelayChoice>,
    /// The engine is in local mode (the modal says so).
    pub local_mode: bool,
    /// The set hit [`MAX_EVENTS`].
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Pure
// ---------------------------------------------------------------------------

fn kind_of(ev: &Value) -> u64 {
    ev.get("kind").and_then(Value::as_u64).unwrap_or(0)
}
fn str_of<'a>(ev: &'a Value, k: &str) -> &'a str {
    ev.get(k).and_then(Value::as_str).unwrap_or("")
}
fn tags<'a>(ev: &'a Value, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    ev.get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .filter(move |t| t.first().and_then(Value::as_str) == Some(name))
        .filter_map(|t| t.get(1).and_then(Value::as_str))
}
fn first(ev: &Value, name: &str) -> Option<String> {
    tags(ev, name).next().map(str::to_string)
}

/// What the event is to the document.
pub fn role_of(ev: &Value, tree: &HashSet<String>, highlight_ids: &HashSet<String>) -> String {
    let kind = kind_of(ev);
    let in_tree = coord_of(ev).map(|c| tree.contains(&c)).unwrap_or(false);
    match kind {
        KIND_PUBLICATION_INDEX if in_tree => "index",
        KIND_PUBLICATION_SECTION if in_tree => "section",
        KIND_PUBLICATION_SECTION
            if tags(ev, "e")
                .chain(tags(ev, "E"))
                .any(|t| highlight_ids.contains(t)) =>
        {
            "annotation"
        }
        9802 => "highlight",
        1111 => "comment",
        1 => "reply",
        5 => "deletion",
        7 => "reaction",
        818 => "merge request",
        32176 => "narration",
        30045 => "bookshelf",
        _ => "reference",
    }
    .to_string()
}

/// Title, else the first line of the content, trimmed to ~90 chars.
pub fn label_of(ev: &Value) -> String {
    let raw = first(ev, "title")
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            str_of(ev, "content")
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string()
        });
    let mut out: String = raw.chars().take(90).collect();
    if raw.chars().count() > 90 {
        out.push('…');
    }
    out
}

/// Ids and coordinates a kind-5 retracts, counted only when the deletion
/// and its target share an author (NIP-09). Returns `(ids, coords)` keyed
/// by author: `"pubkey|id"` / `"pubkey|coord"`.
pub fn deleted_targets(events: &[Value]) -> HashSet<String> {
    let mut out = HashSet::new();
    for ev in events.iter().filter(|e| kind_of(e) == 5) {
        let pk = str_of(ev, "pubkey");
        for t in tags(ev, "e").chain(tags(ev, "a")) {
            out.insert(format!("{pk}|{t}"));
        }
    }
    out
}

fn is_deleted(ev: &Value, deleted: &HashSet<String>) -> bool {
    let pk = str_of(ev, "pubkey");
    deleted.contains(&format!("{pk}|{}", str_of(ev, "id")))
        || coord_of(ev)
            .map(|c| deleted.contains(&format!("{pk}|{c}")))
            .unwrap_or(false)
}

/// Provenance as stored: the `relays` field, normalized, `local` dropped.
fn provenance(ev: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in ev
        .get("relays")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let n = crate::relay_url::normalize_relay_url(r);
        if !n.is_empty() && n != "local" && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

/// Destination choices: publish set (preselected) then broadcast set.
pub fn relay_list(publish: &[String], broadcast: &[String]) -> Vec<RelayChoice> {
    let mut out: Vec<RelayChoice> = Vec::new();
    for (url, is_pub) in publish
        .iter()
        .map(|u| (u, true))
        .chain(broadcast.iter().map(|u| (u, false)))
    {
        let n = crate::relay_url::normalize_relay_url(url);
        if n.is_empty() {
            continue;
        }
        match out.iter_mut().find(|c| c.url == n) {
            Some(c) => {
                c.publish |= is_pub;
                c.broadcast |= !is_pub;
            }
            None => out.push(RelayChoice {
                url: n,
                publish: is_pub,
                broadcast: !is_pub,
            }),
        }
    }
    out
}

/// The event as a relay should receive it: the NIP-01 fields only (local
/// reads carry a `relays` provenance field the wire format doesn't have).
pub fn for_relay(ev: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for k in [
        "id",
        "pubkey",
        "created_at",
        "kind",
        "tags",
        "content",
        "sig",
    ] {
        if let Some(v) = ev.get(k) {
            out.insert(k.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

/// Order for the modal: the tree first in TOC order, then attachments
/// oldest first (a thread reads top-down).
fn sort_rows(rows: &mut [BackupRow], toc: &[String]) {
    let pos = |r: &BackupRow| {
        r.coord
            .as_ref()
            .and_then(|c| toc.iter().position(|t| t == c))
            .unwrap_or(usize::MAX)
    };
    rows.sort_by(|a, b| pos(a).cmp(&pos(b)).then(a.created_at.cmp(&b.created_at)));
}

// ---------------------------------------------------------------------------
// IO — local store only
// ---------------------------------------------------------------------------

async fn local(engine: &Engine, filters: Vec<Value>) -> Result<Vec<Value>> {
    if filters.is_empty() {
        return Ok(Vec::new());
    }
    Ok(engine
        .get_events(filters, FetchPolicy::LocalOnly, None)
        .await?
        .events)
}

async fn newest_at(engine: &Engine, coord: &str) -> Result<Option<Value>> {
    let Some(a) = crate::publication::NAddr::from_a_tag(coord) else {
        return Ok(None);
    };
    let evs = local(
        engine,
        vec![json!({"kinds": [a.kind], "authors": [a.pubkey], "#d": [a.d_tag], "limit": 5})],
    )
    .await?;
    Ok(evs
        .into_iter()
        .max_by_key(|e| e.get("created_at").and_then(Value::as_u64).unwrap_or(0)))
}

/// Everything local connected to `root` (a coordinate).
pub async fn collect(engine: &Engine, root: &str) -> Result<BackupPlan> {
    let root_ev = newest_at(engine, root)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("{root} is not in the local store")))?;
    let title = first(&root_ev, "title");

    // 1. The tree, breadth-first, in TOC order.
    let mut toc: Vec<String> = vec![root.to_string()];
    let mut tree_events: Vec<Value> = vec![root_ev];
    let mut tree: HashSet<String> = HashSet::from([root.to_string()]);
    let mut queue: VecDeque<(Value, usize)> = VecDeque::from([(tree_events[0].clone(), 0)]);
    while let Some((node, depth)) = queue.pop_front() {
        if kind_of(&node) != KIND_PUBLICATION_INDEX || depth >= MAX_DEPTH {
            continue;
        }
        for child in children_of(&node) {
            if !tree.insert(child.clone()) || tree.len() > MAX_EVENTS {
                continue;
            }
            toc.push(child.clone());
            if let Some(ev) = newest_at(engine, &child).await? {
                queue.push_back((ev.clone(), depth + 1));
                tree_events.push(ev);
            }
        }
    }

    // 2. Hops out: anything referencing a tree node (by coordinate or id),
    // then anything referencing those. Foreign indexes that merely list
    // one of our sections are another publication — not ours to back up.
    let mut all: Vec<Value> = tree_events.clone();
    let mut seen: HashSet<String> = all.iter().map(|e| str_of(e, "id").to_string()).collect();
    let mut frontier = tree_events;
    let mut truncated = false;
    for _ in 0..ATTACH_HOPS {
        let coords: Vec<String> = frontier.iter().filter_map(coord_of).collect();
        let ids: Vec<String> = frontier
            .iter()
            .map(|e| str_of(e, "id").to_string())
            .collect();
        let mut filters = Vec::new();
        for chunk in coords.chunks(100) {
            filters.push(json!({"#a": chunk, "limit": MAX_EVENTS}));
            filters.push(json!({"#A": chunk, "limit": MAX_EVENTS}));
        }
        for chunk in ids.chunks(100) {
            filters.push(json!({"#e": chunk, "limit": MAX_EVENTS}));
            filters.push(json!({"#E": chunk, "limit": MAX_EVENTS}));
        }
        let mut next = Vec::new();
        for ev in local(engine, filters).await? {
            let id = str_of(&ev, "id").to_string();
            if id.is_empty() || seen.contains(&id) {
                continue;
            }
            if kind_of(&ev) == KIND_PUBLICATION_INDEX
                && !coord_of(&ev).map(|c| tree.contains(&c)).unwrap_or(false)
            {
                continue;
            }
            if all.len() >= MAX_EVENTS {
                truncated = true;
                break;
            }
            seen.insert(id);
            all.push(ev.clone());
            next.push(ev);
        }
        if next.is_empty() || truncated {
            break;
        }
        frontier = next;
    }

    // 3. Newest version per coordinate, then rows.
    let all = dedup_newest(all);
    let deleted = deleted_targets(&all);
    let highlight_ids: HashSet<String> = all
        .iter()
        .filter(|e| kind_of(e) == 9802)
        .map(|e| str_of(e, "id").to_string())
        .collect();
    let pubkeys: Vec<String> = {
        let mut v: Vec<String> = all
            .iter()
            .map(|e| str_of(e, "pubkey").to_string())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let keys: Vec<[u8; 32]> = pubkeys
        .iter()
        .filter_map(|p| hex::decode(p).ok()?.try_into().ok())
        .collect();
    let names: HashMap<String, Option<String>> = keys
        .iter()
        .map(hex::encode)
        .zip(crate::stats::display_names(engine.ndb(), &keys))
        .collect();

    let mut rows: Vec<BackupRow> = all
        .iter()
        .map(|ev| {
            let kind = kind_of(ev);
            let pk = str_of(ev, "pubkey").to_string();
            BackupRow {
                id: str_of(ev, "id").to_string(),
                kind,
                kind_name: u32::try_from(kind)
                    .ok()
                    .and_then(crate::stats::kind_label)
                    .map(str::to_string),
                author: names.get(&pk).cloned().flatten(),
                pubkey: pk,
                created_at: ev.get("created_at").and_then(Value::as_u64).unwrap_or(0),
                role: role_of(ev, &tree, &highlight_ids),
                label: label_of(ev),
                coord: coord_of(ev),
                relays: provenance(ev),
                deleted: is_deleted(ev, &deleted),
            }
        })
        .collect();
    sort_rows(&mut rows, &toc);

    Ok(BackupPlan {
        root: root.to_string(),
        title,
        rows,
        relays: relay_list(&engine.publish_relays(), &engine.broadcast_relays()),
        local_mode: engine.is_local(),
        truncated,
    })
}

pub mod handlers {
    use super::*;
    use crate::api::ChatAppState;
    use axum::extract::{Query, State};
    use axum::response::Json;
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct PlanQuery {
        pub coord: String,
    }

    /// `GET /api/v1/backup/plan?coord=kind:pubkey:d` — the connected set.
    pub async fn plan(
        State(st): State<ChatAppState>,
        Query(q): Query<PlanQuery>,
    ) -> std::result::Result<Json<BackupPlan>, EngineError> {
        Ok(Json(collect(&st.engine, q.coord.trim()).await?))
    }

    #[derive(Deserialize)]
    pub struct BroadcastBody {
        pub ids: Vec<String>,
        pub relays: Vec<String>,
    }

    /// `POST /api/v1/backup/broadcast {ids, relays}` — send the picked
    /// events to the picked relays. Runs in every network mode: the modal
    /// is the approval, and a permit opens the local-mode gate for exactly
    /// these relays while the broadcast runs.
    pub async fn broadcast(
        State(st): State<ChatAppState>,
        Json(body): Json<BroadcastBody>,
    ) -> std::result::Result<Json<Value>, EngineError> {
        let engine = &st.engine;
        let relays: Vec<String> = body
            .relays
            .iter()
            .map(|u| crate::relay_url::normalize_relay_url(u))
            .filter(|u| !u.is_empty())
            .collect();
        if relays.is_empty() {
            return Err(EngineError::BadRequest("pick at least one relay".into()));
        }
        if body.ids.is_empty() {
            return Err(EngineError::BadRequest("pick at least one event".into()));
        }
        let mut events: Vec<Value> = Vec::with_capacity(body.ids.len());
        for id in &body.ids {
            let ev = engine
                .get_by_id(id, FetchPolicy::LocalOnly)
                .await?
                .ok_or_else(|| EngineError::NotFound(format!("no local event with id {id}")))?;
            events.push(for_relay(&ev));
        }
        let event_jsons: Vec<String> = events.iter().map(|e| e.to_string()).collect();
        let ids: Vec<String> = events.iter().map(|e| str_of(e, "id").to_string()).collect();

        let _permit = crate::relay::permit(&relays);
        let manifest = crate::network::PublishManifest::from_events(events.iter());
        let op = engine.network().begin_publish_operation_approved(
            format!(
                "Backup — {} event(s) to {} relay(s)",
                events.len(),
                relays.len()
            ),
            relays.clone(),
            ids,
            None,
            Some(manifest),
        );
        for url in &relays {
            op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
        }
        let results = crate::api::publish_and_verify(engine, &op, &relays, &event_jsons).await;
        for r in &results {
            op.relay_status(r.relay_url.clone(), crate::api::publish_result_status(r));
        }
        let accepted = results.iter().filter(|r| r.success).count();
        op.complete(accepted);

        // Provenance per (event, relay); a publication that landed somewhere
        // stops reading as local.
        let by_id: HashMap<&str, &String> = events
            .iter()
            .zip(event_jsons.iter())
            .map(|(e, j)| (str_of(e, "id"), j))
            .collect();
        let mut landed: HashSet<&str> = HashSet::new();
        for r in results.iter().filter(|r| r.success) {
            if let Some(j) = by_id.get(r.event_id.as_str()) {
                let _ = engine.record_event_relay(j, &r.relay_url);
                landed.insert(r.event_id.as_str());
            }
        }
        if let Ok(tracker) = crate::drafts::LocalPublicationTracker::new(engine.data_dir()) {
            for ev in events.iter().filter(|e| landed.contains(str_of(e, "id"))) {
                if let Some(c) = coord_of(ev) {
                    let _ = tracker.mark_published(&c);
                }
            }
        }

        let per_relay: Vec<Value> = relays
            .iter()
            .map(|url| {
                let mine: Vec<_> = results.iter().filter(|r| &r.relay_url == url).collect();
                let ok = mine.iter().filter(|r| r.success).count();
                let first_err = mine
                    .iter()
                    .find(|r| !r.success)
                    .and_then(|r| r.message.clone());
                json!({ "relay": url, "accepted": ok, "rejected": mine.len() - ok, "message": first_err })
            })
            .collect();
        Ok(Json(json!({
            "events": events.len(),
            "landed": landed.len(),
            "relays": per_relay,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, kind: u64, pk: &str, tags: Vec<Vec<&str>>) -> Value {
        json!({"id": id, "kind": kind, "pubkey": pk, "created_at": 1, "content": "", "tags": tags, "sig": "s"})
    }

    #[test]
    fn roles_follow_the_tree_and_highlights() {
        let tree: HashSet<String> = ["30040:aa:book".into(), "30041:aa:one".into()].into();
        let hl: HashSet<String> = ["h1".into()].into();
        assert_eq!(
            role_of(&ev("1", 30040, "aa", vec![vec!["d", "book"]]), &tree, &hl),
            "index"
        );
        assert_eq!(
            role_of(&ev("2", 30041, "aa", vec![vec!["d", "one"]]), &tree, &hl),
            "section"
        );
        assert_eq!(
            role_of(
                &ev("3", 30041, "bb", vec![vec!["d", "x"], vec!["e", "h1"]]),
                &tree,
                &hl
            ),
            "annotation"
        );
        assert_eq!(
            role_of(&ev("4", 30041, "bb", vec![vec!["d", "y"]]), &tree, &hl),
            "reference"
        );
        assert_eq!(
            role_of(&ev("5", 9802, "bb", vec![]), &tree, &hl),
            "highlight"
        );
    }

    #[test]
    fn deletions_count_only_for_the_same_author() {
        let evs = vec![
            ev(
                "d1",
                5,
                "aa",
                vec![vec!["e", "h1"], vec!["a", "30041:aa:one"]],
            ),
            ev("d2", 5, "bb", vec![vec!["e", "h2"]]),
        ];
        let del = deleted_targets(&evs);
        assert!(is_deleted(&ev("h1", 9802, "aa", vec![]), &del));
        assert!(
            !is_deleted(&ev("h2", 9802, "aa", vec![]), &del),
            "bb can't delete aa's"
        );
        assert!(is_deleted(
            &ev("s", 30041, "aa", vec![vec!["d", "one"]]),
            &del
        ));
    }

    #[test]
    fn relay_list_merges_sets_and_preselects_publish() {
        let l = relay_list(
            &["wss://a.example".into(), "wss://b.example/".into()],
            &["wss://b.example".into(), "wss://c.example".into()],
        );
        let urls: Vec<&str> = l.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls.len(), 3, "{urls:?}");
        let b = l.iter().find(|c| c.url.contains("b.example")).unwrap();
        assert!(b.publish && b.broadcast);
        let c = l.iter().find(|c| c.url.contains("c.example")).unwrap();
        assert!(!c.publish && c.broadcast);
    }

    #[test]
    fn for_relay_drops_local_fields() {
        let mut e = ev("1", 1, "aa", vec![]);
        e["relays"] = json!(["wss://x"]);
        let out = for_relay(&e);
        assert!(out.get("relays").is_none());
        assert_eq!(out["sig"], "s");
    }

    #[test]
    fn labels_prefer_title_and_trim() {
        let t = ev("1", 30041, "aa", vec![vec!["title", "Chapter One"]]);
        assert_eq!(label_of(&t), "Chapter One");
        let mut c = ev("2", 1111, "aa", vec![]);
        c["content"] = json!(format!("\n\n{}", "x".repeat(200)));
        let l = label_of(&c);
        assert!(l.ends_with('…') && l.chars().count() == 91);
    }
}
