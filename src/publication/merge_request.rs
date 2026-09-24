//! Merge requests on publications — NIP-54's kind 818, target widened to
//! NKBIP-01 kinds (`nips/NKBIP-01-insertions.org` § Merge requests;
//! `docs/zettel/decision-merge-requests-818.org`).
//!
//! A proposer publishes a *derivative* under their own key (the copy
//! process: [`fork_section`] here, `fork_index` in `insertion.rs`) and one
//! kind-818 that names the owner's target (`a`), the version the change was
//! made against (`e` base), the derivative to merge (`e … source`) and the
//! owner (`p`). The owner answers with a NIP-25 reaction on the 818 — `+`
//! merged (after republishing their own version), `-` declined — and the
//! newest reaction *by the target owner* is the status; nothing else
//! records it (`resolve_status`, the `git.rs` pattern).
//!
//! Routing is inbox-side (`docs/zettel/decision-asks-routing.org`): the
//! [`destination_set`] unions the owner's read relays, the relays the target
//! was found on, the index's relay hints and the proposer's own write
//! relays, each URL carrying its reasons, and [`is_deliverable`] refuses a
//! set with neither an inbox nor a found-on entry.
//!
//! Everything in this file is pure and IO-free; the engine halves
//! (`Engine::inbox_relays_for`, `Engine::provenance_for`) and the handlers
//! (`api::merge_requests`) do the lookups.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::publication::insertion::{coord_parts, event_tags};
use crate::signing::EventTemplate;

pub const KIND_MERGE_REQUEST: u32 = 818;
pub const KIND_REACTION: u32 = 7;

/// A parsed kind-818.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRequest {
    pub id: String,
    pub pubkey: String,
    pub created_at: i64,
    pub content: String,
    /// The target coordinate (`kind:pubkey:d`): a 30040 or 30041 here, a
    /// 30818 for a NIP-54 wiki merge request.
    pub target: String,
    pub target_kind: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_relay_hint: String,
    /// The version the change was made against (`e` without a marker).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// The event to merge (`e … source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_relay_hint: String,
    /// The owner the request is addressed to (`p`).
    pub owner: String,
}

impl MergeRequest {
    /// Parse a kind-818 event. `None` for any other kind, a target that is
    /// not a coordinate, or a target kind other than 30040 / 30041 / 30818.
    pub fn from_event(ev: &Value) -> Option<Self> {
        if ev.get("kind").and_then(Value::as_u64) != Some(KIND_MERGE_REQUEST as u64) {
            return None;
        }
        let tags = event_tags(ev);
        let a = tags.iter().find(|t| t.first().map(String::as_str) == Some("a"))?;
        let target = a.get(1)?.trim().to_string();
        let (target_kind, target_pk, _) = coord_parts(&target)?;
        if !matches!(target_kind, 30040 | 30041 | 30818) {
            return None;
        }
        let target_relay_hint = a.get(2).cloned().unwrap_or_default();
        let mut base = None;
        let mut source = None;
        let mut source_relay_hint = String::new();
        for t in tags.iter().filter(|t| t.first().map(String::as_str) == Some("e")) {
            let Some(id) = t.get(1).filter(|v| v.len() == 64) else { continue };
            if t.get(3).map(String::as_str) == Some("source") {
                if source.is_none() {
                    source = Some(id.clone());
                    source_relay_hint = t.get(2).cloned().unwrap_or_default();
                }
            } else if base.is_none() {
                base = Some(id.clone());
            }
        }
        let owner = tags
            .iter()
            .find(|t| t.first().map(String::as_str) == Some("p"))
            .and_then(|t| t.get(1).cloned())
            .filter(|p| p.len() == 64)
            .unwrap_or_else(|| target_pk.to_string());
        Some(Self {
            id: ev.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: ev.get("pubkey").and_then(Value::as_str).unwrap_or("").to_string(),
            created_at: ev.get("created_at").and_then(Value::as_i64).unwrap_or(0),
            content: ev.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
            target,
            target_kind,
            target_relay_hint,
            base,
            source,
            source_relay_hint,
            owner,
        })
    }

    /// The pubkey of the target coordinate (the index / section signer).
    pub fn target_pubkey(&self) -> &str {
        coord_parts(&self.target).map(|(_, pk, _)| pk).unwrap_or("")
    }
}

/// Relay hints the 818 carries: on the `a` target and the `e … source`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeHints {
    pub target: String,
    pub source: String,
}

/// The unsigned kind-818, tags in the spec's order: `a` target (+ hint),
/// `e` base, `e` source with the `source` marker, `p` owner.
pub fn template(
    target: &str,
    base: Option<&str>,
    source: Option<&str>,
    owner: &str,
    content: &str,
    hints: &MergeHints,
    created_at: i64,
) -> Result<EventTemplate, String> {
    let (kind, target_pk, _) =
        coord_parts(target).ok_or_else(|| format!("target is not a coordinate: `{target}`"))?;
    if !matches!(kind, 30040 | 30041 | 30818) {
        return Err(format!("a merge request targets a 30040, 30041 or 30818, not kind {kind}"));
    }
    let owner = if owner.trim().is_empty() { target_pk } else { owner.trim() };
    if owner.len() != 64 {
        return Err("owner must be a 64-hex pubkey".into());
    }
    let mut tags: Vec<Vec<String>> = Vec::new();
    let mut a = vec!["a".to_string(), target.to_string()];
    if !hints.target.is_empty() {
        a.push(hints.target.clone());
    }
    tags.push(a);
    if let Some(b) = base.map(str::trim).filter(|b| b.len() == 64) {
        tags.push(vec!["e".to_string(), b.to_string()]);
    }
    if let Some(s) = source.map(str::trim).filter(|s| s.len() == 64) {
        tags.push(vec![
            "e".to_string(),
            s.to_string(),
            hints.source.clone(),
            "source".to_string(),
        ]);
    }
    tags.push(vec!["p".to_string(), owner.to_string()]);
    Ok(EventTemplate {
        kind: KIND_MERGE_REQUEST,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// Where a merge request stands: the newest reaction by the target owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    #[default]
    Open,
    Merged,
    Declined,
}

/// Status = the newest kind-7 whose `pubkey` is the request's owner and
/// whose `e` names the 818's id; `+` merged, `-` declined, anything else
/// (an emoji, a stranger's reaction) ignored. No such reaction = open.
pub fn resolve_status<'a>(mr: &MergeRequest, reactions: &'a [Value]) -> (MergeStatus, Option<&'a Value>) {
    let mut best: Option<(&Value, i64, MergeStatus)> = None;
    for r in reactions {
        if r.get("kind").and_then(Value::as_u64) != Some(KIND_REACTION as u64) {
            continue;
        }
        if r.get("pubkey").and_then(Value::as_str) != Some(mr.owner.as_str()) {
            continue;
        }
        let names_it = event_tags(r)
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("e") && t.get(1).map(String::as_str) == Some(mr.id.as_str()));
        if !names_it {
            continue;
        }
        let verdict = match r.get("content").and_then(Value::as_str).map(str::trim) {
            Some("+") => MergeStatus::Merged,
            Some("-") => MergeStatus::Declined,
            Some(c) if c.starts_with('+') => MergeStatus::Merged,
            Some(c) if c.starts_with('-') => MergeStatus::Declined,
            _ => continue,
        };
        let at = r.get("created_at").and_then(Value::as_i64).unwrap_or(0);
        if best.map(|(_, t, _)| at > t).unwrap_or(true) {
            best = Some((r, at, verdict));
        }
    }
    match best {
        Some((r, _, s)) => (s, Some(r)),
        None => (MergeStatus::Open, None),
    }
}

/// The owner's reaction on an 818: `+` (merged) or `-` (declined), an
/// optional reason appended after the sign so a bare `+`/`-` stays a bare
/// reaction. Tags: `e` the 818, `p` its author, `k` 818, and the target
/// `a` so a subscriber on the publication sees the verdict too.
pub fn reaction_template(mr: &MergeRequest, verdict: char, reason: Option<&str>, created_at: i64) -> Result<EventTemplate, String> {
    if verdict != '+' && verdict != '-' {
        return Err("a verdict is + or -".into());
    }
    let mut content = verdict.to_string();
    if let Some(r) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        content.push(' ');
        content.push_str(r);
    }
    let mut tags = vec![
        vec!["e".to_string(), mr.id.clone()],
        vec!["p".to_string(), mr.pubkey.clone()],
        vec!["k".to_string(), KIND_MERGE_REQUEST.to_string()],
    ];
    if !mr.target.is_empty() {
        tags.push(vec!["a".to_string(), mr.target.clone()]);
    }
    Ok(EventTemplate { kind: KIND_REACTION, created_at, tags, content, pubkey: None })
}

/// The reaction's verdict when it is one: `+` merged, `-` declined.
pub fn verdict_of(reaction: &Value) -> Option<MergeStatus> {
    match reaction.get("content").and_then(Value::as_str).map(str::trim) {
        Some(c) if c.starts_with('+') => Some(MergeStatus::Merged),
        Some(c) if c.starts_with('-') => Some(MergeStatus::Declined),
        _ => None,
    }
}

/// The copy process for a section: every tag verbatim, a minted `d`, `E` to
/// the version copied from (any earlier `E` dropped — a derivative of a
/// derivative points one hop back) and `p` to its author. Content is copied
/// unchanged; the caller edits it afterwards (a relink, say).
pub fn fork_section(section: &Value, signer_pubkey: &str, relay_hint: &str, created_at: i64) -> Result<EventTemplate, String> {
    if section.get("kind").and_then(Value::as_u64) != Some(30041) {
        return Err("only a kind-30041 section can be forked".into());
    }
    let author = section.get("pubkey").and_then(Value::as_str).ok_or("section has no pubkey")?;
    let id = section.get("id").and_then(Value::as_str).ok_or("section has no id")?;
    let mut tags = event_tags(section);
    let d = crate::publication::mint_d_tag();
    let mut had_d = false;
    for t in tags.iter_mut() {
        if t.first().map(String::as_str) == Some("d") {
            if t.len() >= 2 {
                t[1] = d.clone();
            } else {
                t.push(d.clone());
            }
            had_d = true;
        }
    }
    if !had_d {
        tags.insert(0, vec!["d".to_string(), d]);
    }
    tags.retain(|t| t.first().map(String::as_str) != Some("E"));
    if signer_pubkey != author
        && !tags
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("p") && t.get(1).map(String::as_str) == Some(author))
    {
        tags.push(vec!["p".to_string(), author.to_string()]);
    }
    tags.push(vec!["E".to_string(), id.to_string(), relay_hint.to_string(), author.to_string()]);
    Ok(EventTemplate {
        kind: 30041,
        created_at,
        tags,
        content: section.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
        pubkey: Some(signer_pubkey.to_string()),
    })
}

/// The copy process for an index whose sections were rewritten: every tag
/// verbatim, each `a` in `replacements` (original → derivative coordinate)
/// swapped in place (hint column kept), `glossary` appended after the last
/// `a` when given and not already listed, a minted `d`, any earlier `E`
/// dropped, `p` to the index author and `E` to the copied version. Returns
/// the template and the minted `d`.
pub fn derivative_index(
    index: &Value,
    replacements: &[(String, String)],
    glossary: Option<&str>,
    signer_pubkey: &str,
    relay_hint: &str,
    created_at: i64,
) -> Result<(EventTemplate, String), String> {
    if index.get("kind").and_then(Value::as_u64) != Some(30040) {
        return Err("only a kind-30040 index can be forked".into());
    }
    let author = index.get("pubkey").and_then(Value::as_str).ok_or("index has no pubkey")?;
    let id = index.get("id").and_then(Value::as_str).ok_or("index has no id")?;
    let mut tags = event_tags(index);
    let mut has_glossary = false;
    for t in tags.iter_mut() {
        if t.first().map(String::as_str) != Some("a") {
            continue;
        }
        if let Some(v) = t.get(1).cloned() {
            if let Some((_, to)) = replacements.iter().find(|(from, _)| *from == v) {
                t[1] = to.clone();
            }
            if glossary == Some(t[1].as_str()) {
                has_glossary = true;
            }
        }
    }
    if let Some(g) = glossary {
        if !has_glossary {
            let mut a = vec!["a".to_string(), g.to_string()];
            if !relay_hint.is_empty() {
                a.push(relay_hint.to_string());
            }
            let at = tags
                .iter()
                .rposition(|t| t.first().map(String::as_str) == Some("a"))
                .map(|p| p + 1)
                .unwrap_or(tags.len());
            tags.insert(at, a);
        }
    }
    let d = crate::publication::mint_d_tag();
    let mut had_d = false;
    for t in tags.iter_mut() {
        if t.first().map(String::as_str) == Some("d") {
            if t.len() >= 2 {
                t[1] = d.clone();
            } else {
                t.push(d.clone());
            }
            had_d = true;
        }
    }
    if !had_d {
        tags.insert(0, vec!["d".to_string(), d.clone()]);
    }
    tags.retain(|t| t.first().map(String::as_str) != Some("E"));
    if signer_pubkey != author
        && !tags
            .iter()
            .any(|t| t.first().map(String::as_str) == Some("p") && t.get(1).map(String::as_str) == Some(author))
    {
        tags.push(vec!["p".to_string(), author.to_string()]);
    }
    tags.push(vec!["E".to_string(), id.to_string(), relay_hint.to_string(), author.to_string()]);
    Ok((
        EventTemplate {
            kind: 30040,
            created_at,
            tags,
            content: index.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: Some(signer_pubkey.to_string()),
        },
        d,
    ))
}

// ---------------------------------------------------------------------------
// Routing: the destination set
// ---------------------------------------------------------------------------

/// Why a relay is in the destination set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// A read relay from the owner's kind 10002.
    Inbox,
    /// A relay the target event was found on (nostrdb provenance).
    FoundOn,
    /// A relay hint on the index's `a` tag / the target coordinate.
    Hint,
    /// One of the proposer's own write relays.
    Mine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub url: String,
    pub reasons: Vec<Reason>,
}

fn norm(u: &str) -> String {
    let n = crate::relay_url::normalize_relay_url(u);
    if n.is_empty() {
        u.trim().to_string()
    } else {
        n
    }
}

/// Union the four sources, deduped by normalized URL with reasons merged,
/// ordered inbox → found-on → hint → mine (a relay keeps the position of
/// its first appearance). Empty and unparseable URLs are dropped.
pub fn destination_set(inbox: &[String], provenance: &[String], hints: &[String], mine: &[String]) -> Vec<Destination> {
    let mut out: Vec<Destination> = Vec::new();
    for (list, reason) in [(inbox, Reason::Inbox), (provenance, Reason::FoundOn), (hints, Reason::Hint), (mine, Reason::Mine)] {
        for u in list {
            let url = norm(u);
            if url.is_empty() || !(url.starts_with("wss://") || url.starts_with("ws://")) {
                continue;
            }
            match out.iter_mut().find(|d| d.url == url) {
                Some(d) => {
                    if !d.reasons.contains(&reason) {
                        d.reasons.push(reason);
                    }
                }
                None => out.push(Destination { url, reasons: vec![reason] }),
            }
        }
    }
    out
}

/// A set reaches the owner only through their inbox or a relay the target
/// was found on; hints and the proposer's own relays alone do not count.
pub fn is_deliverable(set: &[Destination]) -> bool {
    set.iter().any(|d| d.reasons.iter().any(|r| matches!(r, Reason::Inbox | Reason::FoundOn)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const ME: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const STRANGER: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SRC: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const MR_ID: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn target() -> String {
        format!("30040:{OWNER}:on-method")
    }

    fn as_event(t: &EventTemplate, id: &str, pubkey: &str) -> Value {
        json!({ "id": id, "pubkey": pubkey, "kind": t.kind, "created_at": t.created_at, "tags": t.tags, "content": t.content })
    }

    #[test]
    fn template_round_trips_through_from_event_in_spec_order() {
        let hints = MergeHints { target: "wss://nos.lol".into(), source: "wss://relay.me".into() };
        let t = template(&target(), Some(BASE), Some(SRC), OWNER, "relink 2 terms", &hints, 100).unwrap();
        assert_eq!(t.kind, 818);
        assert_eq!(t.tags[0], vec!["a", &target(), "wss://nos.lol"]);
        assert_eq!(t.tags[1], vec!["e", BASE]);
        assert_eq!(t.tags[2], vec!["e", SRC, "wss://relay.me", "source"]);
        assert_eq!(t.tags[3], vec!["p", OWNER]);
        assert_eq!(t.tags.len(), 4);
        let mr = MergeRequest::from_event(&as_event(&t, MR_ID, ME)).unwrap();
        assert_eq!(mr.target, target());
        assert_eq!(mr.target_kind, 30040);
        assert_eq!(mr.target_relay_hint, "wss://nos.lol");
        assert_eq!(mr.base.as_deref(), Some(BASE));
        assert_eq!(mr.source.as_deref(), Some(SRC));
        assert_eq!(mr.source_relay_hint, "wss://relay.me");
        assert_eq!(mr.owner, OWNER);
        assert_eq!(mr.pubkey, ME);
        assert_eq!(mr.content, "relink 2 terms");
        assert_eq!(mr.target_pubkey(), OWNER);
    }

    #[test]
    fn from_event_accepts_30818_flags_kind_and_refuses_other_targets() {
        let wiki = template(&format!("30818:{OWNER}:bitcoin"), None, Some(SRC), "", "", &MergeHints::default(), 1).unwrap();
        let mr = MergeRequest::from_event(&as_event(&wiki, MR_ID, ME)).unwrap();
        assert_eq!(mr.target_kind, 30818);
        assert_eq!(mr.owner, OWNER, "owner defaults to the target's pubkey");
        assert!(mr.base.is_none());
        assert!(template(&format!("30023:{OWNER}:x"), None, None, OWNER, "", &MergeHints::default(), 1).is_err());
        let not_818 = json!({ "kind": 1, "tags": [["a", target()]], "content": "" });
        assert!(MergeRequest::from_event(&not_818).is_none());
        let bad_target = json!({ "kind": 818, "tags": [["a", "30023:x:y"], ["p", OWNER]], "content": "" });
        assert!(MergeRequest::from_event(&bad_target).is_none());
    }

    fn reaction(pk: &str, content: &str, at: i64, e: &str) -> Value {
        json!({ "kind": 7, "pubkey": pk, "created_at": at, "content": content, "tags": [["e", e], ["k", "818"]] })
    }

    #[test]
    fn status_is_the_owners_newest_verdict_and_strangers_are_ignored() {
        let t = template(&target(), None, Some(SRC), OWNER, "", &MergeHints::default(), 1).unwrap();
        let mr = MergeRequest::from_event(&as_event(&t, MR_ID, ME)).unwrap();
        assert_eq!(resolve_status(&mr, &[]).0, MergeStatus::Open);
        let rs = vec![
            reaction(STRANGER, "+", 50, MR_ID),         // not the owner
            reaction(OWNER, "+", 10, MR_ID),            // merged, older
            reaction(OWNER, "- not now", 20, MR_ID),    // declined, newest
            reaction(OWNER, "🔥", 30, MR_ID),           // not a verdict
            reaction(OWNER, "+", 40, "d".repeat(64).as_str()), // another event
        ];
        let (s, r) = resolve_status(&mr, &rs);
        assert_eq!(s, MergeStatus::Declined);
        assert_eq!(r.unwrap()["created_at"], 20);
        let rt = reaction_template(&mr, '+', Some("thanks"), 99).unwrap();
        assert_eq!(rt.kind, 7);
        assert_eq!(rt.content, "+ thanks");
        assert!(rt.tags.contains(&vec!["e".to_string(), MR_ID.to_string()]));
        assert!(rt.tags.contains(&vec!["p".to_string(), ME.to_string()]));
        assert!(rt.tags.contains(&vec!["k".to_string(), "818".to_string()]));
        assert!(rt.tags.contains(&vec!["a".to_string(), target()]));
        assert!(reaction_template(&mr, 'x', None, 1).is_err());
        assert_eq!(verdict_of(&reaction(OWNER, "+", 1, MR_ID)), Some(MergeStatus::Merged));
        assert_eq!(verdict_of(&reaction(OWNER, "?", 1, MR_ID)), None);
    }

    #[test]
    fn fork_section_keeps_tags_mints_d_and_points_back() {
        let sec = json!({
            "kind": 30041, "id": BASE, "pubkey": OWNER, "created_at": 5, "content": "Against method …",
            "tags": [["d", "m3x8pz"], ["title", "Against method"], ["T", "against-method"], ["m", "text/markdown"], ["a", "30041:x:y"], ["E", "old", "", "z"]]
        });
        let t = fork_section(&sec, ME, "wss://nos.lol", 9).unwrap();
        assert_eq!(t.kind, 30041);
        assert_eq!(t.content, "Against method …");
        assert_eq!(t.pubkey.as_deref(), Some(ME));
        let d = t.tags.iter().find(|x| x[0] == "d").unwrap();
        assert_ne!(d[1], "m3x8pz");
        assert_eq!(d[1].len(), 21, "minted nanoid d");
        for keep in [vec!["title", "Against method"], vec!["T", "against-method"], vec!["m", "text/markdown"], vec!["a", "30041:x:y"]] {
            let keep: Vec<String> = keep.into_iter().map(String::from).collect();
            assert!(t.tags.contains(&keep), "{keep:?} kept verbatim");
        }
        assert_eq!(t.tags.iter().filter(|x| x[0] == "E").count(), 1, "the old E is dropped");
        assert_eq!(t.tags.last().unwrap(), &vec!["E".to_string(), BASE.to_string(), "wss://nos.lol".to_string(), OWNER.to_string()]);
        assert!(t.tags.contains(&vec!["p".to_string(), OWNER.to_string()]));
        // the owner forking their own section gets no p to themselves
        let own = fork_section(&sec, OWNER, "", 9).unwrap();
        assert!(!own.tags.iter().any(|x| x[0] == "p" && x[1] == OWNER));
        assert!(fork_section(&json!({"kind": 30040, "id": BASE, "pubkey": OWNER, "tags": []}), ME, "", 1).is_err());
    }

    #[test]
    fn destination_set_dedups_merges_reasons_and_keeps_order() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let set = destination_set(
            &s(&["wss://relay.pkf.example", "wss://nos.lol/"]),
            &s(&["wss://NOS.LOL", "wss://theforest.nostr1.com"]),
            &s(&["wss://relay.damus.io", "", "not a url"]),
            &s(&["wss://relay.me", "wss://relay.damus.io"]),
        );
        let urls: Vec<&str> = set.iter().map(|d| d.url.as_str()).collect();
        assert_eq!(urls, vec!["wss://relay.pkf.example", "wss://nos.lol", "wss://theforest.nostr1.com", "wss://relay.damus.io", "wss://relay.me"]);
        assert_eq!(set[1].reasons, vec![Reason::Inbox, Reason::FoundOn]);
        assert_eq!(set[3].reasons, vec![Reason::Hint, Reason::Mine]);
        assert!(is_deliverable(&set));
        let weak = destination_set(&[], &[], &s(&["wss://relay.damus.io"]), &s(&["wss://relay.me"]));
        assert!(!is_deliverable(&weak), "hints + mine alone do not reach the owner");
        assert!(!is_deliverable(&[]));
        let json = serde_json::to_value(&set[1]).unwrap();
        assert_eq!(json["reasons"], json!(["inbox", "found_on"]));
    }

    #[test]
    fn derivative_index_swaps_sections_appends_glossary_and_points_back() {
        let owner = "a".repeat(64);
        let me = "b".repeat(64);
        let s1 = format!("30041:{owner}:s1");
        let s2 = format!("30041:{owner}:s2");
        let index = serde_json::json!({ "id": "c".repeat(64), "pubkey": owner, "kind": 30040, "created_at": 1, "content": "",
            "tags": [["d", "book"], ["title", "On Method"], ["a", s1, "wss://hint"], ["a", s2], ["t", "x"]] });
        let s2p = format!("30041:{me}:s2prime");
        let gl = format!("30040:{me}:book-glossary");
        let (t, d) = derivative_index(&index, &[(s2.clone(), s2p.clone())], Some(&gl), &me, "wss://mine", 5).unwrap();
        assert_ne!(d, "book");
        let a: Vec<&Vec<String>> = t.tags.iter().filter(|t| t[0] == "a").collect();
        assert_eq!(a.len(), 3);
        assert_eq!(a[0][1], s1);
        assert_eq!(a[0][2], "wss://hint", "hint column kept");
        assert_eq!(a[1][1], s2p, "rewritten in place");
        assert_eq!(a[2][1], gl, "glossary appended after the last a");
        assert!(t.tags.iter().any(|t| t[0] == "title" && t[1] == "On Method"));
        assert!(t.tags.iter().any(|t| t[0] == "p" && t[1] == owner));
        let e = t.tags.iter().find(|t| t[0] == "E").unwrap();
        assert_eq!(e[1], "c".repeat(64));
        assert_eq!(e[3], owner);
        assert_eq!(t.pubkey.as_deref(), Some(me.as_str()));
        // the glossary already listed is not appended twice
        let index2 = serde_json::json!({ "id": "c".repeat(64), "pubkey": owner, "kind": 30040, "created_at": 1, "tags": [["d", "book"], ["a", s1], ["a", gl]] });
        let (t2, _) = derivative_index(&index2, &[], Some(&gl), &me, "", 5).unwrap();
        assert_eq!(t2.tags.iter().filter(|t| t[0] == "a").count(), 2);
    }
}
