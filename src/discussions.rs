//! NIP-22 (kind 1111) comment threading. Pure logic — no engine, no IO, no
//! view state — so it's the frontend-agnostic source of truth for every
//! frontend (web today; emacs/nvim planned).
//!
//! Parent detection follows NIP-22: the first lowercase `e` tag pointing to
//! another event *in the set* is the immediate parent; an event with no such
//! resolvable parent edge is surfaced as a root. That's the least-lossy
//! fallback — an `e` target outside the set references a comment that simply
//! hasn't been ingested yet, and surfacing the reply at the top level beats
//! hiding it. All ids are matched lowercase (relays return mixed case).
//!
//! Moved from the web's `discussions/thread.ts::buildThread` per the
//! frontend/backend boundary (docs/eval/08). The web keeps only the
//! depth→indent rendering and tree-walk view helpers.

use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// A node in a NIP-22 thread tree. Serialized to match the web's `ThreadNode`:
/// `{ event, children, depth }`.
#[derive(Debug, Clone, Serialize)]
pub struct ThreadNode {
    pub event: Value,
    pub children: Vec<ThreadNode>,
    /// 0 = top-level root, increments per nesting level.
    pub depth: usize,
}

fn event_id_lower(ev: &Value) -> Option<String> {
    ev.get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase())
}

fn created_at(ev: &Value) -> i64 {
    ev.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// Build a NIP-22 thread forest from a flat list of events. Returns root nodes
/// (top-level comments); each node carries its replies in `children`, sorted by
/// `created_at` ascending (roots too). Events are deduped by lowercase id,
/// first occurrence winning.
///
/// Mixed kinds are accepted but only events whose lowercase `e` tag matches
/// another event in the set thread — callers typically pre-filter to kind 1111.
pub fn build_thread(events: &[Value]) -> Vec<ThreadNode> {
    // Dedup by lowercase id, preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for ev in events {
        let Some(id) = event_id_lower(ev) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = by_id.entry(id.clone()) {
            order.push(id);
            slot.insert(ev.clone());
        }
    }
    if order.is_empty() {
        return Vec::new();
    }

    let id_set: HashSet<&str> = order.iter().map(String::as_str).collect();

    // Resolve each event's parent: first lowercase `e` tag pointing to a known
    // event in the set (skipping self). `e` tags resolving outside the set are
    // ignored — see the module note.
    let mut parent_of: HashMap<&str, String> = HashMap::new();
    for id in &order {
        let ev = &by_id[id];
        let Some(tags) = ev.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => continue,
            };
            if arr[0].as_str() != Some("e") {
                continue;
            }
            let candidate = arr[1].as_str().unwrap_or("").to_lowercase();
            if candidate == *id {
                continue;
            }
            if id_set.contains(candidate.as_str()) {
                parent_of.insert(id.as_str(), candidate);
                break;
            }
        }
    }

    // Bucket children under parents (in first-seen order); ids with no resolved
    // parent become roots. A 2+ cycle leaves every member parented, so none
    // appears in `roots` and the cycle is simply unreachable — no recursion
    // guard needed (matches the TS twin's behaviour).
    let mut children_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut roots: Vec<String> = Vec::new();
    for id in &order {
        match parent_of.get(id.as_str()) {
            Some(p) => children_of.entry(p.clone()).or_default().push(id.clone()),
            None => roots.push(id.clone()),
        }
    }

    roots.sort_by_key(|id| created_at(&by_id[id]));
    roots
        .iter()
        .map(|id| build_node(id, 0, &by_id, &children_of))
        .collect()
}

fn build_node(
    id: &str,
    depth: usize,
    by_id: &HashMap<String, Value>,
    children_of: &HashMap<String, Vec<String>>,
) -> ThreadNode {
    let mut child_ids = children_of.get(id).cloned().unwrap_or_default();
    child_ids.sort_by_key(|cid| created_at(&by_id[cid]));
    let children = child_ids
        .iter()
        .map(|cid| build_node(cid, depth + 1, by_id, children_of))
        .collect();
    ThreadNode {
        event: by_id[id].clone(),
        children,
        depth,
    }
}

/// Group kind-1111 comments by the requested addresses they reference (via `a`
/// or `A` tags), then thread each group independently. Mirrors the reader's
/// per-section grouping: an event is bucketed under every requested address it
/// tags. Every requested address appears in the result (empty forest if
/// nothing references it).
pub fn group_threads_by_address(
    events: &[Value],
    addresses: &[String],
) -> HashMap<String, Vec<ThreadNode>> {
    let address_set: HashSet<&str> = addresses.iter().map(String::as_str).collect();
    let mut by_addr: HashMap<String, Vec<Value>> =
        addresses.iter().map(|a| (a.clone(), Vec::new())).collect();

    for ev in events {
        if ev.get("kind").and_then(|v| v.as_u64()) != Some(1111) {
            continue;
        }
        let Some(tags) = ev.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        let mut matched: HashSet<&str> = HashSet::new();
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => continue,
            };
            if !matches!(arr[0].as_str().unwrap_or(""), "a" | "A") {
                continue;
            }
            if let Some(val) = arr[1].as_str() {
                if let Some(&hit) = address_set.get(val) {
                    matched.insert(hit);
                }
            }
        }
        for addr in matched {
            // `addr` is a key we inserted above, so the entry exists.
            if let Some(bucket) = by_addr.get_mut(addr) {
                bucket.push(ev.clone());
            }
        }
    }

    by_addr
        .into_iter()
        .map(|(addr, evs)| (addr, build_thread(&evs)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn comment(id: &str, created_at: i64, e_parent: Option<&str>, a_scope: Option<&str>) -> Value {
        let mut tags = Vec::new();
        if let Some(p) = e_parent {
            tags.push(json!(["e", p]));
        }
        if let Some(a) = a_scope {
            tags.push(json!(["a", a]));
        }
        json!({
            "id": id,
            "kind": 1111,
            "pubkey": "ff",
            "created_at": created_at,
            "content": format!("c{id}"),
            "tags": tags,
        })
    }

    fn ids(nodes: &[ThreadNode]) -> Vec<String> {
        nodes
            .iter()
            .map(|n| n.event["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn empty_input_is_empty_forest() {
        assert!(build_thread(&[]).is_empty());
    }

    #[test]
    fn flat_roots_sorted_by_created_at() {
        let evs = vec![comment("b", 20, None, None), comment("a", 10, None, None)];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["a", "b"]); // created_at ascending
        assert!(forest.iter().all(|n| n.depth == 0 && n.children.is_empty()));
    }

    #[test]
    fn nests_replies_under_parent_with_depth() {
        // root <- reply1 <- reply2
        let evs = vec![
            comment("root", 1, None, None),
            comment("reply2", 3, Some("reply1"), None),
            comment("reply1", 2, Some("root"), None),
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["root"]);
        let root = &forest[0];
        assert_eq!(root.depth, 0);
        assert_eq!(ids(&root.children), vec!["reply1"]);
        let r1 = &root.children[0];
        assert_eq!(r1.depth, 1);
        assert_eq!(ids(&r1.children), vec!["reply2"]);
        assert_eq!(r1.children[0].depth, 2);
    }

    #[test]
    fn children_sorted_by_created_at() {
        let evs = vec![
            comment("root", 1, None, None),
            comment("late", 30, Some("root"), None),
            comment("early", 10, Some("root"), None),
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest[0].children), vec!["early", "late"]);
    }

    #[test]
    fn parent_outside_set_becomes_root() {
        // `orphan` replies to an id not in the set → surfaced as a root.
        let evs = vec![comment("orphan", 5, Some("not-loaded"), None)];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["orphan"]);
        assert_eq!(forest[0].depth, 0);
    }

    #[test]
    fn dedup_by_lowercase_id() {
        let evs = vec![
            comment("AbC", 1, None, None),
            comment("abc", 2, None, None), // same id, lowercased
        ];
        assert_eq!(build_thread(&evs).len(), 1);
    }

    #[test]
    fn mixed_case_e_tag_resolves() {
        let evs = vec![
            comment("root", 1, None, None),
            comment("reply", 2, Some("ROOT"), None), // uppercase e target
        ];
        let forest = build_thread(&evs);
        assert_eq!(ids(&forest), vec!["root"]);
        assert_eq!(ids(&forest[0].children), vec!["reply"]);
    }

    #[test]
    fn group_by_address_buckets_and_threads() {
        let sec_a = "30041:ff:intro";
        let sec_b = "30041:ff:method";
        let evs = vec![
            comment("a-root", 1, None, Some(sec_a)),
            comment("a-reply", 2, Some("a-root"), Some(sec_a)),
            comment("b-root", 1, None, Some(sec_b)),
        ];
        let grouped =
            group_threads_by_address(&evs, &[sec_a.to_string(), sec_b.to_string()]);

        let a = &grouped[sec_a];
        assert_eq!(ids(a), vec!["a-root"]);
        assert_eq!(ids(&a[0].children), vec!["a-reply"]);

        let b = &grouped[sec_b];
        assert_eq!(ids(b), vec!["b-root"]);

        // Every requested address present, even with no comments.
        let none = group_threads_by_address(&evs, &["30041:ff:absent".to_string()]);
        assert!(none["30041:ff:absent"].is_empty());
    }

    #[test]
    fn group_by_address_ignores_non_comments() {
        let sec = "30041:ff:intro";
        let mut highlight = comment("hl", 1, None, Some(sec));
        highlight["kind"] = json!(9802); // NIP-84 highlight, not a comment
        let grouped = group_threads_by_address(&[highlight], &[sec.to_string()]);
        assert!(grouped[sec].is_empty());
    }
}
