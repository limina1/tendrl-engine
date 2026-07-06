//! Relay-list publish diff — "what will publishing overwrite".
//!
//! Kind 10002 / 30002 (and the Amethyst private-list kinds) are
//! replaceable events: publishing a new one replaces the old *wholesale*
//! on every relay that accepts it. Tendrl composes these events from its
//! local working sets, so anything on the currently published event that
//! tendrl doesn't model — tags written by other clients, encrypted
//! private members in `content` — would silently vanish. This module is
//! the pure comparison behind the publish confirmation: membership
//! added/removed, NIP-65 marker changes, and exactly what gets dropped.
//!
//! Pure and IO-free; the handler in `api.rs` looks up the current event
//! and feeds it in. The frontend may override the current membership
//! with client-decrypted URLs (NIP-44 private lists are only readable
//! next to the NIP-07 signer).

use crate::relay_url::normalize_relay_url;
use serde::Serialize;
use std::collections::BTreeMap;

/// NIP-65 usage marker, also used as the "membership" marker for kinds
/// that don't carry read/write semantics.
fn marker_label(read: bool, write: bool) -> String {
    match (read, write) {
        (true, false) => "read".into(),
        (false, true) => "write".into(),
        _ => "read+write".into(),
    }
}

/// One side of a marker disagreement on a URL present in both lists.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RelayMarkerChange {
    pub url: String,
    pub current: String,
    pub proposed: String,
}

/// The overwrite report shown before a relay-list publish proceeds.
#[derive(Debug, Clone, Serialize)]
pub struct RelayListDiff {
    pub kind: u64,
    pub current_event_id: Option<String>,
    pub current_created_at: Option<u64>,
    /// URLs the proposed event adds.
    pub added: Vec<String>,
    /// URLs on the current event the proposed one no longer carries.
    pub removed: Vec<String>,
    /// URLs in both, with different read/write markers (kind 10002).
    pub changed: Vec<RelayMarkerChange>,
    pub unchanged: usize,
    /// Tags on the current event beyond what tendrl composes for this
    /// kind — publishing drops them.
    pub dropped_tags: Vec<Vec<String>>,
    /// The current event carries non-empty `content` the proposed event
    /// won't (e.g. NIP-44 encrypted private members) — publishing drops it.
    pub drops_content: bool,
    /// The current event's membership lives in encrypted `content` we
    /// couldn't read (no client-side plaintext supplied): the publish
    /// would overwrite a list whose contents are unknown.
    pub current_opaque: bool,
}

/// A relay entry with its effective markers, keyed for comparison.
#[derive(Debug, Clone)]
struct Entry {
    url: String,
    read: bool,
    write: bool,
}

/// Tag names whose content the proposed event re-expresses — anything
/// else on the current event is reported as dropped. `relay` is the
/// NIP-51 membership tag for 30002; `r` is NIP-65's and also accepted
/// on 30002 for tolerance with non-conformant publishers.
fn known_tag_names(kind: u64) -> &'static [&'static str] {
    match kind {
        10002 => &["r"],
        30002 => &["d", "title", "alt", "r", "relay"],
        _ => &[],
    }
}

/// Membership tags for a kind: which tag names carry relay URLs.
fn membership_tag_names(kind: u64) -> &'static [&'static str] {
    match kind {
        10002 => &["r"],
        // NIP-51 says `relay`; accept `r` tolerantly (mirrors the pull).
        30002 => &["relay", "r"],
        _ => &["relay", "r"],
    }
}

/// Extract relay entries (with NIP-65 markers where present) from tags.
fn entries_from_tags(kind: u64, tags: &[Vec<String>]) -> BTreeMap<String, Entry> {
    let names = membership_tag_names(kind);
    let mut map = BTreeMap::new();
    for tag in tags {
        let (Some(name), Some(url)) = (tag.first(), tag.get(1)) else {
            continue;
        };
        if !names.contains(&name.as_str()) || url.is_empty() {
            continue;
        }
        let (read, write) = if kind == 10002 {
            match tag.get(2).map(String::as_str) {
                Some("read") => (true, false),
                Some("write") => (false, true),
                _ => (true, true),
            }
        } else {
            (true, true)
        };
        // First occurrence wins on duplicates, matching relay dedup.
        map.entry(normalize_relay_url(url)).or_insert(Entry {
            url: url.clone(),
            read,
            write,
        });
    }
    map
}

fn entries_from_urls(urls: &[String]) -> BTreeMap<String, Entry> {
    let mut map = BTreeMap::new();
    for url in urls {
        if url.is_empty() {
            continue;
        }
        map.entry(normalize_relay_url(url)).or_insert(Entry {
            url: url.clone(),
            read: true,
            write: true,
        });
    }
    map
}

/// The current event as the handler found it. `tags`/`content` come
/// straight off the stored event JSON.
#[derive(Debug, Clone, Default)]
pub struct CurrentEvent {
    pub id: Option<String>,
    pub created_at: Option<u64>,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

/// Compare a proposed relay-list event against the currently published
/// one. Returns `None` when there is no current event at all — a first
/// publish overwrites nothing and needs no gate.
///
/// `current_urls` is the client-decrypted membership for private-list
/// kinds; when the current event's membership is in `content` and no
/// plaintext was supplied, the diff comes back `current_opaque`.
/// `proposed` is either tags (public kinds) or a plain URL list
/// (private kinds, pre-encryption).
pub fn compute_relay_list_diff(
    kind: u64,
    current: Option<&CurrentEvent>,
    current_urls: Option<&[String]>,
    proposed_tags: &[Vec<String>],
    proposed_urls: Option<&[String]>,
) -> Option<RelayListDiff> {
    let current = current?;

    let current_entries = match current_urls {
        Some(urls) => entries_from_urls(urls),
        None => entries_from_tags(kind, &current.tags),
    };
    let proposed_entries = match proposed_urls {
        Some(urls) => entries_from_urls(urls),
        None => entries_from_tags(kind, proposed_tags),
    };

    // Encrypted membership we couldn't read: no tag-borne entries, no
    // client plaintext, but content is present.
    let current_opaque =
        current_entries.is_empty() && current_urls.is_none() && !current.content.is_empty();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = 0usize;

    for (key, p) in &proposed_entries {
        match current_entries.get(key) {
            None => added.push(p.url.clone()),
            Some(c) if kind == 10002 && (c.read != p.read || c.write != p.write) => {
                changed.push(RelayMarkerChange {
                    url: p.url.clone(),
                    current: marker_label(c.read, c.write),
                    proposed: marker_label(p.read, p.write),
                });
            }
            Some(_) => unchanged += 1,
        }
    }
    for (key, c) in &current_entries {
        if !proposed_entries.contains_key(key) {
            removed.push(c.url.clone());
        }
    }

    let known = known_tag_names(kind);
    let dropped_tags: Vec<Vec<String>> = current
        .tags
        .iter()
        .filter(|t| {
            t.first()
                .map(|name| !known.contains(&name.as_str()))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    // Tendrl publishes empty content for the public kinds; a private
    // kind's proposed event carries (re-encrypted) content, so nothing
    // is dropped there.
    let drops_content = !current.content.is_empty() && matches!(kind, 10002 | 30002);

    Some(RelayListDiff {
        kind,
        current_event_id: current.id.clone(),
        current_created_at: current.created_at,
        added,
        removed,
        changed,
        unchanged,
        dropped_tags,
        drops_content,
        current_opaque,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(spec: &[&[&str]]) -> Vec<Vec<String>> {
        spec.iter()
            .map(|t| t.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    fn current(tags_spec: &[&[&str]], content: &str) -> CurrentEvent {
        CurrentEvent {
            id: Some("ev1".into()),
            created_at: Some(1_700_000_000),
            tags: tags(tags_spec),
            content: content.into(),
        }
    }

    #[test]
    fn first_publish_has_no_diff() {
        let proposed = tags(&[&["r", "wss://a.example"]]);
        assert!(compute_relay_list_diff(10002, None, None, &proposed, None).is_none());
    }

    #[test]
    fn membership_and_marker_diff_10002() {
        let cur = current(
            &[
                &["r", "wss://keep.example"],
                &["r", "wss://gone.example", "read"],
                &["r", "wss://flip.example", "read"],
            ],
            "",
        );
        let proposed = tags(&[
            &["r", "wss://keep.example"],
            &["r", "wss://new.example", "write"],
            &["r", "wss://flip.example"],
        ]);
        let d = compute_relay_list_diff(10002, Some(&cur), None, &proposed, None).unwrap();
        assert_eq!(d.added, vec!["wss://new.example"]);
        assert_eq!(d.removed, vec!["wss://gone.example"]);
        assert_eq!(
            d.changed,
            vec![RelayMarkerChange {
                url: "wss://flip.example".into(),
                current: "read".into(),
                proposed: "read+write".into(),
            }]
        );
        assert_eq!(d.unchanged, 1);
        assert!(!d.drops_content);
        assert!(!d.current_opaque);
        assert!(d.dropped_tags.is_empty());
    }

    #[test]
    fn url_normalization_matches_variants() {
        // Trailing slash / case / missing scheme are the same relay.
        let cur = current(&[&["r", "wss://A.Example/"]], "");
        let proposed = tags(&[&["r", "a.example"]]);
        let d = compute_relay_list_diff(10002, Some(&cur), None, &proposed, None).unwrap();
        assert!(d.added.is_empty() && d.removed.is_empty());
        assert_eq!(d.unchanged, 1);
    }

    #[test]
    fn unknown_tags_and_content_report_as_dropped() {
        // A 30002 written by another client: `relay` members, an
        // unmodeled `description` tag, and encrypted private members.
        let cur = current(
            &[
                &["d", "research"],
                &["title", "Research"],
                &["relay", "wss://a.example"],
                &["description", "my set"],
            ],
            "nip44-ciphertext",
        );
        let proposed = tags(&[
            &["d", "research"],
            &["title", "Research"],
            &["r", "wss://a.example"],
        ]);
        let d = compute_relay_list_diff(30002, Some(&cur), None, &proposed, None).unwrap();
        // `relay` on the current side matches `r` on the proposed side.
        assert!(d.added.is_empty() && d.removed.is_empty());
        assert_eq!(d.unchanged, 1);
        assert_eq!(d.dropped_tags, tags(&[&["description", "my set"]]));
        assert!(d.drops_content);
        // Content is present but membership was readable from tags.
        assert!(!d.current_opaque);
    }

    #[test]
    fn encrypted_current_without_plaintext_is_opaque() {
        let cur = current(&[], "nip44-ciphertext");
        let d =
            compute_relay_list_diff(10088, Some(&cur), None, &[], Some(&["wss://a.example".into()]))
                .unwrap();
        assert!(d.current_opaque);
        assert_eq!(d.added, vec!["wss://a.example"]);
        // Private kinds re-encrypt content, so nothing content-drops.
        assert!(!d.drops_content);
    }

    #[test]
    fn client_supplied_plaintext_overrides_opaque() {
        let cur = current(&[], "nip44-ciphertext");
        let cur_urls = vec!["wss://a.example".to_string(), "wss://b.example".to_string()];
        let proposed_urls = vec!["wss://a.example".to_string()];
        let d = compute_relay_list_diff(10088, Some(&cur), Some(&cur_urls), &[], Some(&proposed_urls))
            .unwrap();
        assert!(!d.current_opaque);
        assert_eq!(d.removed, vec!["wss://b.example"]);
        assert_eq!(d.unchanged, 1);
    }
}
