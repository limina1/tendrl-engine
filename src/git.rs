//! NIP-34 `git` stuff — repository announcements, issues, and their status.
//!
//! Spec: `nips/34.md`. Everything tendrl needs to show a repository "like a
//! profile" and hold a conversation on it:
//!
//! - **kind 30617** repository announcement (addressable by `d`): name,
//!   description, `web` / `clone` / `relays` / `maintainers` (all
//!   multi-valued), `r … euc` (earliest unique commit — the fork-family key),
//!   `u` (subordinate-fork pointer), `t` hashtags.
//! - **kind 30618** repository state: `refs/heads/*` + `refs/tags/*` →
//!   commit, `HEAD` → `ref: refs/heads/<branch>`.
//! - **kind 1621** issue: markdown `content`, `subject`, `t` labels, `a` →
//!   the repository coordinate, `p` → the owner.
//! - **kinds 1630–1633** status: Open / Resolved / Closed / Draft, `e … root`
//!   → the issue. The newest one by the issue author *or a maintainer* wins;
//!   no status at all means Open.
//! - Replies are plain NIP-22 comments (kind 1111, `E` = issue id) — the
//!   discussions module already builds and threads those for any
//!   non-comment root, so nothing git-specific lives here for them.
//!
//! Layout mirrors `bookshelf.rs`: pure parsers / template builders first
//! (IO-free, unit-tested against real relay.ngit.dev events), then the
//! [`GitEngine`] borrow that does the Confirm-gated fetching.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::publication::NAddr;
use crate::signing::EventTemplate;

pub const KIND_REPOSITORY: u64 = 30617;
pub const KIND_REPOSITORY_STATE: u64 = 30618;
pub const KIND_PATCH: u64 = 1617;
pub const KIND_PULL_REQUEST: u64 = 1618;
pub const KIND_ISSUE: u64 = 1621;
pub const KIND_STATUS_OPEN: u64 = 1630;
pub const KIND_STATUS_RESOLVED: u64 = 1631;
pub const KIND_STATUS_CLOSED: u64 = 1632;
pub const KIND_STATUS_DRAFT: u64 = 1633;

/// The four status kinds, in spec order.
pub const STATUS_KINDS: [u64; 4] = [
    KIND_STATUS_OPEN,
    KIND_STATUS_RESOLVED,
    KIND_STATUS_CLOSED,
    KIND_STATUS_DRAFT,
];

/// Root kinds a status event or a NIP-22 comment may point at.
pub const ROOT_KINDS: [u64; 3] = [KIND_ISSUE, KIND_PATCH, KIND_PULL_REQUEST];

/// Cap on issues / status events pulled per repository load.
pub const MAX_ISSUES: usize = 500;

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn non_blank(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Every value of a multi-valued tag (`["clone", url, url, …]`), trimmed,
/// blanks dropped.
fn tag_values(arr: &[Value]) -> Vec<String> {
    arr.iter()
        .skip(1)
        .filter_map(|v| non_blank(v.as_str()))
        .collect()
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn event_str<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event.get(key).and_then(|v| v.as_str())
}

fn event_u64(event: &Value, key: &str) -> u64 {
    event.get(key).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn event_relays(event: &Value) -> Vec<String> {
    event
        .get("relays")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Newest-wins tie-break shared by every "latest event" fold: later
/// `created_at`, then lexically higher id so the choice is stable across
/// runs regardless of arrival order.
fn newer(a_created: u64, a_id: &str, b_created: u64, b_id: &str) -> bool {
    (a_created, a_id) > (b_created, b_id)
}

// ─── Repository announcement (kind 30617) ────────────────────────────────

/// A subordinate-fork pointer (`u` tag): either a repository coordinate or
/// a plain git URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkOf {
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub pubkey: String,
    pub d_tag: String,
    pub event_id: String,
    pub created_at: u64,
    /// `name` tag, falling back to the `d` tag like the reference client.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub web: Vec<String>,
    pub clone: Vec<String>,
    /// Relays this repository watches for patches and issues.
    pub relays: Vec<String>,
    /// Earliest unique commit — the key that groups forks/mirrors of one
    /// project across hosts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub euc: Option<String>,
    /// Extra recognised maintainers (the announcer is implied).
    pub maintainers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_of: Option<ForkOf>,
    pub hashtags: Vec<String>,
    /// Relays the local store saw this event on (provenance; empty = local).
    pub seen_on: Vec<String>,
}

impl Repository {
    /// Parse a kind-30617 event (JSON in the `note_to_json` shape).
    ///
    /// Errors only on the wrong kind or a missing pubkey; tag-level
    /// problems are skipped so a sloppy announcement still renders.
    pub fn from_event(event: &Value) -> Result<Self> {
        let kind = event_u64(event, "kind");
        if kind != KIND_REPOSITORY {
            return Err(EngineError::BadRequest(format!(
                "not a repository announcement (kind {kind}, want {KIND_REPOSITORY})"
            )));
        }
        let pubkey = event_str(event, "pubkey")
            .filter(|p| hex64(p))
            .ok_or_else(|| EngineError::BadRequest("repository event has no pubkey".into()))?
            .to_lowercase();

        let mut d_tag: Option<String> = None;
        let mut name = None;
        let mut description = None;
        let mut web = Vec::new();
        let mut clone = Vec::new();
        let mut relays = Vec::new();
        let mut euc = None;
        let mut maintainers = Vec::new();
        let mut fork_of = None;
        let mut hashtags = Vec::new();

        let tags = event.get("tags").and_then(|t| t.as_array());
        for tag in tags.into_iter().flatten() {
            let Some(arr) = tag.as_array() else { continue };
            let at = |i: usize| arr.get(i).and_then(|v| v.as_str());
            match at(0) {
                Some("d") if d_tag.is_none() => d_tag = non_blank(at(1)),
                Some("name") if name.is_none() => name = non_blank(at(1)),
                Some("description") if description.is_none() => {
                    description = non_blank(at(1))
                }
                Some("web") => tag_values(arr).into_iter().for_each(|v| push_unique(&mut web, v)),
                Some("clone") => tag_values(arr)
                    .into_iter()
                    .for_each(|v| push_unique(&mut clone, v)),
                Some("relays") => tag_values(arr)
                    .into_iter()
                    .for_each(|v| push_unique(&mut relays, v)),
                Some("r") if euc.is_none() && at(2) == Some("euc") => euc = non_blank(at(1)),
                Some("maintainers") => {
                    for m in tag_values(arr) {
                        if hex64(&m) {
                            push_unique(&mut maintainers, m.to_lowercase());
                        }
                    }
                }
                Some("u") if fork_of.is_none() => {
                    if let Some(target) = non_blank(at(1)) {
                        fork_of = Some(ForkOf {
                            target,
                            relay_hint: non_blank(at(2)),
                            pubkey: non_blank(at(3)).filter(|p| hex64(p)),
                        });
                    }
                }
                Some("t") => {
                    if let Some(t) = non_blank(at(1)) {
                        push_unique(&mut hashtags, t);
                    }
                }
                _ => {}
            }
        }

        // The announcer is a maintainer by definition; keep the list free of
        // that redundancy so the UI never shows the owner twice.
        maintainers.retain(|m| m != &pubkey);

        let d_tag = d_tag.unwrap_or_default();
        Ok(Self {
            name: name.unwrap_or_else(|| d_tag.clone()),
            pubkey,
            d_tag,
            event_id: event_str(event, "id").unwrap_or_default().to_string(),
            created_at: event_u64(event, "created_at"),
            description,
            web,
            clone,
            relays,
            euc,
            maintainers,
            fork_of,
            hashtags,
            seen_on: event_relays(event),
        })
    }

    /// `30617:<pubkey>:<d>` — the `a`-tag form issues point at.
    pub fn coordinate(&self) -> String {
        format!("{KIND_REPOSITORY}:{}:{}", self.pubkey, self.d_tag)
    }

    /// Everyone whose status events count: the announcer plus the listed
    /// maintainers.
    pub fn authorities(&self) -> Vec<String> {
        let mut v = vec![self.pubkey.clone()];
        for m in &self.maintainers {
            push_unique(&mut v, m.clone());
        }
        v
    }

    pub fn is_maintainer(&self, pubkey: &str) -> bool {
        let pk = pubkey.to_lowercase();
        self.pubkey == pk || self.maintainers.contains(&pk)
    }

    /// NIP-01 filter for every repository a pubkey announces.
    pub fn filter_by_author(pubkey: &str) -> Value {
        json!({
            "kinds": [KIND_REPOSITORY],
            "authors": [pubkey.to_lowercase()],
            "limit": 200
        })
    }

    /// NIP-01 filter for one announcement.
    pub fn filter_one(pubkey: &str, d_tag: &str) -> Value {
        json!({
            "kinds": [KIND_REPOSITORY],
            "authors": [pubkey.to_lowercase()],
            "#d": [d_tag],
            "limit": 1
        })
    }

    /// Newest announcement per `d` tag across a mixed batch of events
    /// (non-30617 / malformed events skipped), newest first.
    pub fn newest_per_d_tag(events: &[Value]) -> Vec<Self> {
        let mut by_d: HashMap<(String, String), Self> = HashMap::new();
        for ev in events {
            let Ok(repo) = Self::from_event(ev) else { continue };
            let key = (repo.pubkey.clone(), repo.d_tag.clone());
            match by_d.get(&key) {
                Some(cur)
                    if !newer(repo.created_at, &repo.event_id, cur.created_at, &cur.event_id) => {}
                _ => {
                    by_d.insert(key, repo);
                }
            }
        }
        let mut repos: Vec<Self> = by_d.into_values().collect();
        repos.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        repos
    }

    /// The newest announcement for `(pubkey, d_tag)` in a batch.
    pub fn newest(events: &[Value], pubkey: &str, d_tag: &str) -> Option<Self> {
        let pk = pubkey.to_lowercase();
        Self::newest_per_d_tag(events)
            .into_iter()
            .find(|r| r.pubkey == pk && r.d_tag == d_tag)
    }
}

// ─── Repository state (kind 30618) ───────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefEntry {
    /// `refs/heads/main`, `refs/tags/v1.0`, …
    pub name: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryState {
    pub pubkey: String,
    pub d_tag: String,
    pub event_id: String,
    pub created_at: u64,
    /// `HEAD` → the branch name (`refs/heads/` stripped), when announced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    pub refs: Vec<RefEntry>,
}

impl RepositoryState {
    pub fn from_event(event: &Value) -> Result<Self> {
        let kind = event_u64(event, "kind");
        if kind != KIND_REPOSITORY_STATE {
            return Err(EngineError::BadRequest(format!(
                "not a repository state (kind {kind}, want {KIND_REPOSITORY_STATE})"
            )));
        }
        let pubkey = event_str(event, "pubkey")
            .filter(|p| hex64(p))
            .ok_or_else(|| EngineError::BadRequest("repository state has no pubkey".into()))?
            .to_lowercase();
        let mut d_tag = None;
        let mut head = None;
        let mut refs = Vec::new();
        let tags = event.get("tags").and_then(|t| t.as_array());
        for tag in tags.into_iter().flatten() {
            let Some(arr) = tag.as_array() else { continue };
            let at = |i: usize| arr.get(i).and_then(|v| v.as_str());
            match at(0) {
                Some("d") if d_tag.is_none() => d_tag = non_blank(at(1)),
                Some("HEAD") if head.is_none() => {
                    head = non_blank(at(1)).map(|h| {
                        h.trim_start_matches("ref:")
                            .trim()
                            .trim_start_matches("refs/heads/")
                            .to_string()
                    })
                }
                Some(name) if name.starts_with("refs/") => {
                    if let Some(commit) = non_blank(at(1)) {
                        // Peeled annotated-tag entries (`refs/tags/x^{}`)
                        // duplicate the tag row — drop them.
                        if name.ends_with("^{}") {
                            continue;
                        }
                        refs.push(RefEntry {
                            name: name.to_string(),
                            commit,
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            pubkey,
            d_tag: d_tag.unwrap_or_default(),
            event_id: event_str(event, "id").unwrap_or_default().to_string(),
            created_at: event_u64(event, "created_at"),
            head,
            refs,
        })
    }

    /// Newest state among all announced by `authorities` for `d_tag`.
    pub fn newest(events: &[Value], authorities: &[String], d_tag: &str) -> Option<Self> {
        let mut best: Option<Self> = None;
        for ev in events {
            let Ok(st) = Self::from_event(ev) else { continue };
            if st.d_tag != d_tag || !authorities.contains(&st.pubkey) {
                continue;
            }
            let replace = match &best {
                Some(b) => newer(st.created_at, &st.event_id, b.created_at, &b.event_id),
                None => true,
            };
            if replace {
                best = Some(st);
            }
        }
        best
    }
}

// ─── Issues (kind 1621) + status (kinds 1630–1633) ───────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueStatus {
    Open,
    Resolved,
    Closed,
    Draft,
}

impl IssueStatus {
    pub fn from_kind(kind: u64) -> Option<Self> {
        Some(match kind {
            KIND_STATUS_OPEN => Self::Open,
            KIND_STATUS_RESOLVED => Self::Resolved,
            KIND_STATUS_CLOSED => Self::Closed,
            KIND_STATUS_DRAFT => Self::Draft,
            _ => return None,
        })
    }

    pub fn kind(self) -> u64 {
        match self {
            Self::Open => KIND_STATUS_OPEN,
            Self::Resolved => KIND_STATUS_RESOLVED,
            Self::Closed => KIND_STATUS_CLOSED,
            Self::Draft => KIND_STATUS_DRAFT,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Closed => "closed",
            Self::Draft => "draft",
        }
    }
}

impl std::str::FromStr for IssueStatus {
    type Err = EngineError;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "open" => Self::Open,
            "resolved" | "applied" | "merged" => Self::Resolved,
            "closed" => Self::Closed,
            "draft" => Self::Draft,
            other => {
                return Err(EngineError::BadRequest(format!(
                    "unknown status {other:?} (open | resolved | closed | draft)"
                )))
            }
        })
    }
}

/// A parsed status event, before authority filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub status: IssueStatus,
    /// The issue / patch / PR this status applies to (`e … root`, or the
    /// first `e` when unmarked).
    pub root: String,
    pub content: String,
}

impl StatusEvent {
    pub fn from_event(event: &Value) -> Option<Self> {
        let status = IssueStatus::from_kind(event_u64(event, "kind"))?;
        let pubkey = event_str(event, "pubkey").filter(|p| hex64(p))?.to_lowercase();
        let tags = event.get("tags").and_then(|t| t.as_array())?;
        let mut root = None;
        let mut first_e = None;
        for tag in tags {
            let Some(arr) = tag.as_array() else { continue };
            let at = |i: usize| arr.get(i).and_then(|v| v.as_str());
            if at(0) != Some("e") {
                continue;
            }
            let Some(id) = non_blank(at(1)).filter(|i| hex64(i)) else {
                continue;
            };
            let id = id.to_lowercase();
            if at(3) == Some("root") {
                root.get_or_insert(id);
            } else if at(3).is_none() || at(3) == Some("") {
                first_e.get_or_insert(id);
            }
        }
        Some(Self {
            id: event_str(event, "id")?.to_lowercase(),
            pubkey,
            created_at: event_u64(event, "created_at"),
            status,
            root: root.or(first_e)?,
            content: event_str(event, "content").unwrap_or_default().to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    /// The repository coordinate(s) this issue was filed against (first is
    /// primary). `mention`-marked `a` tags are excluded, as gitworkshop does.
    pub repos: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub labels: Vec<String>,
    pub content: String,
    pub status: IssueStatus,
    /// The status event that set `status`, when one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_event: Option<StatusEvent>,
    /// NIP-22 comments seen locally under this issue.
    pub comment_count: usize,
    /// Newest of: the issue, its winning status, its newest comment.
    pub last_activity: u64,
    pub seen_on: Vec<String>,
}

impl Issue {
    pub fn from_event(event: &Value) -> Result<Self> {
        let kind = event_u64(event, "kind");
        if kind != KIND_ISSUE {
            return Err(EngineError::BadRequest(format!(
                "not a git issue (kind {kind}, want {KIND_ISSUE})"
            )));
        }
        let pubkey = event_str(event, "pubkey")
            .filter(|p| hex64(p))
            .ok_or_else(|| EngineError::BadRequest("issue has no pubkey".into()))?
            .to_lowercase();
        let id = event_str(event, "id")
            .filter(|i| hex64(i))
            .ok_or_else(|| EngineError::BadRequest("issue has no id".into()))?
            .to_lowercase();
        let mut repos = Vec::new();
        let mut subject = None;
        let mut labels = Vec::new();
        let tags = event.get("tags").and_then(|t| t.as_array());
        for tag in tags.into_iter().flatten() {
            let Some(arr) = tag.as_array() else { continue };
            let at = |i: usize| arr.get(i).and_then(|v| v.as_str());
            match at(0) {
                Some("a") if at(3) != Some("mention") => {
                    if let Some(coord) = non_blank(at(1)) {
                        if let Some(addr) = NAddr::from_a_tag(&coord) {
                            if addr.kind == KIND_REPOSITORY && hex64(&addr.pubkey) {
                                push_unique(
                                    &mut repos,
                                    format!(
                                        "{KIND_REPOSITORY}:{}:{}",
                                        addr.pubkey.to_lowercase(),
                                        addr.d_tag
                                    ),
                                );
                            }
                        }
                    }
                }
                Some("subject") if subject.is_none() => subject = non_blank(at(1)),
                Some("t") => {
                    if let Some(t) = non_blank(at(1)) {
                        push_unique(&mut labels, t);
                    }
                }
                _ => {}
            }
        }
        let created_at = event_u64(event, "created_at");
        Ok(Self {
            id,
            pubkey,
            created_at,
            repos,
            subject,
            labels,
            content: event_str(event, "content").unwrap_or_default().to_string(),
            status: IssueStatus::Open,
            status_event: None,
            comment_count: 0,
            last_activity: created_at,
            seen_on: event_relays(event),
        })
    }

    /// NIP-01 filter for every issue filed against a repository.
    pub fn filter_for_repo(coordinate: &str) -> Value {
        json!({
            "kinds": [KIND_ISSUE],
            "#a": [coordinate],
            "limit": MAX_ISSUES
        })
    }

    /// NIP-01 filter for every status event on a repository's items.
    pub fn status_filter_for_repo(coordinate: &str) -> Value {
        json!({
            "kinds": STATUS_KINDS,
            "#a": [coordinate],
            "limit": MAX_ISSUES
        })
    }

    /// NIP-01 filter for the NIP-22 comments under a set of issues.
    pub fn comments_filter(issue_ids: &[String]) -> Value {
        json!({
            "kinds": [1111],
            "#E": issue_ids,
            "limit": 1000
        })
    }

    /// Fold the winning status onto each issue, per the spec's rule: the
    /// newest status event whose author is the issue author or a repository
    /// authority. Status events pointing elsewhere (or from bystanders) are
    /// ignored; an issue nobody has touched stays Open.
    pub fn apply_status(issues: &mut [Issue], statuses: &[StatusEvent], authorities: &[String]) {
        let mut by_root: HashMap<&str, &StatusEvent> = HashMap::new();
        let issue_authors: HashMap<&str, &str> = issues
            .iter()
            .map(|i| (i.id.as_str(), i.pubkey.as_str()))
            .collect();
        for st in statuses {
            let Some(author) = issue_authors.get(st.root.as_str()) else {
                continue;
            };
            if st.pubkey != *author && !authorities.contains(&st.pubkey) {
                continue;
            }
            match by_root.get(st.root.as_str()) {
                Some(cur) if !newer(st.created_at, &st.id, cur.created_at, &cur.id) => {}
                _ => {
                    by_root.insert(st.root.as_str(), st);
                }
            }
        }
        for issue in issues.iter_mut() {
            if let Some(st) = by_root.get(issue.id.as_str()) {
                issue.status = st.status;
                issue.last_activity = issue.last_activity.max(st.created_at);
                issue.status_event = Some((*st).clone());
            }
        }
    }

    /// Count NIP-22 comments per root (`E` tag) and bump `last_activity`.
    pub fn apply_comments(issues: &mut [Issue], comments: &[Value]) {
        let mut counts: HashMap<String, (usize, u64)> = HashMap::new();
        let mut seen = HashSet::new();
        for c in comments {
            if event_u64(c, "kind") != 1111 {
                continue;
            }
            let Some(id) = event_str(c, "id") else { continue };
            if !seen.insert(id.to_lowercase()) {
                continue;
            }
            let Some(tags) = c.get("tags").and_then(|t| t.as_array()) else {
                continue;
            };
            let root = tags.iter().find_map(|t| {
                let arr = t.as_array()?;
                (arr.first()?.as_str()? == "E")
                    .then(|| arr.get(1)?.as_str().map(str::to_lowercase))
                    .flatten()
            });
            let Some(root) = root else { continue };
            let e = counts.entry(root).or_default();
            e.0 += 1;
            e.1 = e.1.max(event_u64(c, "created_at"));
        }
        for issue in issues.iter_mut() {
            if let Some((n, last)) = counts.get(&issue.id) {
                issue.comment_count = *n;
                issue.last_activity = issue.last_activity.max(*last);
            }
        }
    }

    /// Parse every kind-1621 event in a batch (dedup by id), newest first.
    pub fn parse_all(events: &[Value]) -> Vec<Self> {
        let mut seen = HashSet::new();
        let mut issues: Vec<Self> = events
            .iter()
            .filter_map(|e| Self::from_event(e).ok())
            .filter(|i| seen.insert(i.id.clone()))
            .collect();
        issues.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| a.id.cmp(&b.id)));
        issues
    }
}

// ─── Templates (what tendrl publishes) ───────────────────────────────────

fn tag_with_hint(name: &str, value: &str, hint: &str) -> Vec<String> {
    if hint.is_empty() {
        vec![name.into(), value.into()]
    } else {
        vec![name.into(), value.into(), hint.into()]
    }
}

/// Unsigned kind-1621 issue against `repo`. `relay_hint` rides on the `a`
/// and `p` tags (empty = omitted). Author is stamped by the signer.
pub fn issue_template(
    repo: &Repository,
    subject: &str,
    content: &str,
    labels: &[String],
    relay_hint: &str,
    created_at: i64,
) -> Result<EventTemplate> {
    let subject = subject.trim();
    if subject.is_empty() {
        return Err(EngineError::BadRequest("issue subject is empty".into()));
    }
    let content = content.trim();
    if content.is_empty() {
        return Err(EngineError::BadRequest("issue body is empty".into()));
    }
    let mut tags = vec![
        tag_with_hint("a", &repo.coordinate(), relay_hint),
        tag_with_hint("p", &repo.pubkey, relay_hint),
        vec!["subject".into(), subject.into()],
    ];
    let mut seen = HashSet::new();
    for l in labels {
        let l = l.trim();
        if !l.is_empty() && seen.insert(l.to_lowercase()) {
            tags.push(vec!["t".into(), l.into()]);
        }
    }
    tags.push(vec!["alt".into(), format!("git issue: {subject}")]);
    Ok(EventTemplate {
        kind: KIND_ISSUE as u32,
        created_at,
        tags,
        content: content.to_string(),
        pubkey: None,
    })
}

/// Unsigned status event for `issue` (kind 1630–1633 per `status`).
/// `signer` is the pubkey about to sign — it is excluded from the `p`
/// notifications so nobody is told about their own change.
pub fn status_template(
    repo: &Repository,
    issue: &Issue,
    status: IssueStatus,
    content: &str,
    signer: &str,
    relay_hint: &str,
    created_at: i64,
) -> EventTemplate {
    let signer = signer.to_lowercase();
    let mut tags = vec![
        vec!["e".into(), issue.id.clone(), String::new(), "root".into()],
        tag_with_hint("a", &repo.coordinate(), relay_hint),
    ];
    for pk in std::iter::once(&issue.pubkey).chain(std::iter::once(&repo.pubkey)) {
        if *pk != signer && !tags.iter().any(|t| t[0] == "p" && t[1] == *pk) {
            tags.push(tag_with_hint("p", pk, relay_hint));
        }
    }
    tags.push(vec!["alt".into(), format!("git issue status: {}", status.as_str())]);
    EventTemplate {
        kind: status.kind() as u32,
        created_at,
        tags,
        content: content.trim().to_string(),
        pubkey: None,
    }
}

// ─── Engine wrapper ──────────────────────────────────────────────────────

/// A repository with everything the buffer renders.
#[derive(Debug, Clone, Serialize)]
pub struct LoadedRepository {
    pub repository: Repository,
    /// The raw announcement (for the JSON view / re-broadcast).
    pub event: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<RepositoryState>,
    pub issues: Vec<Issue>,
    pub open_count: usize,
    /// Relays actually consulted for this load (empty = local read only).
    pub fetched_from: Vec<String>,
}

pub struct GitEngine<'a> {
    engine: &'a Engine,
}

impl<'a> GitEngine<'a> {
    pub fn new(engine: &'a Engine) -> Self {
        Self { engine }
    }

    async fn local(&self, filters: Vec<Value>) -> Result<Vec<Value>> {
        Ok(self
            .engine
            .get_events(filters, FetchPolicy::LocalOnly, None)
            .await?
            .events)
    }

    /// One Confirm-gated relay round for `filters` against `relays`.
    /// `Ok(None)` when the user declined or the modal timed out — callers
    /// fall through to what the store already holds.
    async fn gated_fetch(
        &self,
        label: String,
        steps: Vec<String>,
        filters: Vec<Value>,
        relays: Vec<String>,
    ) -> Result<Option<(Vec<Value>, Vec<String>)>> {
        if relays.is_empty() {
            return Ok(None);
        }
        let composition = crate::network::CompositionShape {
            phases: vec![crate::network::PhaseStage {
                label: "primary".into(),
                members: vec![(crate::network::Phase::Read, relays.clone())],
                start_delay_ms: 0,
            }],
        };
        let summary = crate::network::RequestSummary {
            filters: filters
                .iter()
                .map(crate::network::nip_filter_from_json)
                .collect(),
            dsl: crate::network::dsl_for_composition(&filters, &composition),
            composition,
        };
        let op = match self
            .engine
            .begin_fetch_operation_with_summary(
                crate::network::FetchPattern::Custom,
                label,
                steps,
                relays,
                Some(summary),
            )
            .await
        {
            Ok(op) => op,
            Err(_) => return Ok(None),
        };
        let chosen = op.relays().to_vec();
        let res = self
            .engine
            .get_events_with_options(filters, FetchPolicy::FetchAlways, Some(&chosen), true)
            .await;
        let count = res.as_ref().map(|r| r.events.len()).unwrap_or(0);
        op.complete(count);
        Ok(Some((res?.events, chosen)))
    }

    /// Relays worth asking about a repository: the engine's read set plus
    /// whatever the announcement itself names.
    fn relays_for(&self, repo: Option<&Repository>) -> Vec<String> {
        let mut relays = self.engine.relays();
        if let Some(r) = repo {
            for url in &r.relays {
                push_unique(&mut relays, url.clone());
            }
        }
        relays
    }

    /// Every repository `pubkey` announces. `LocalFirst` goes to relays
    /// only when the store holds none; `FetchAlways` always asks.
    pub async fn list(&self, pubkey: &str, policy: FetchPolicy) -> Result<Vec<Repository>> {
        let filter = Repository::filter_by_author(pubkey);
        let mut events = self.local(vec![filter.clone()]).await?;
        let mut repos = Repository::newest_per_d_tag(&events);
        let fetch = match policy {
            FetchPolicy::FetchAlways => true,
            FetchPolicy::LocalFirst => repos.is_empty(),
            FetchPolicy::LocalOnly => false,
        };
        if fetch {
            let short = &pubkey[..pubkey.len().min(8)];
            if let Some((remote, _)) = self
                .gated_fetch(
                    format!("Repositories — {short}…"),
                    vec![format!("kind 30617 announcements by {short}…")],
                    vec![filter],
                    self.relays_for(None),
                )
                .await?
            {
                events.extend(remote);
                repos = Repository::newest_per_d_tag(&events);
            }
        }
        Ok(repos)
    }

    /// One repository with its issues, statuses folded in, comment counts,
    /// and the newest state announcement from any maintainer.
    ///
    /// A relay round (when the policy asks for one) is two-phase under a
    /// single confirm: the announcement + issues + statuses + state first,
    /// then the NIP-22 comments under the issues found — the second set of
    /// ids isn't known until the first lands.
    pub async fn load(
        &self,
        pubkey: &str,
        d_tag: &str,
        policy: FetchPolicy,
    ) -> Result<LoadedRepository> {
        let pubkey = pubkey.to_lowercase();
        let coordinate = format!("{KIND_REPOSITORY}:{pubkey}:{d_tag}");

        let repo_filter = Repository::filter_one(&pubkey, d_tag);
        let mut repo_events = self.local(vec![repo_filter.clone()]).await?;
        let mut repo = Repository::newest(&repo_events, &pubkey, d_tag);

        let fetch = match policy {
            FetchPolicy::FetchAlways => true,
            FetchPolicy::LocalFirst => repo.is_none(),
            FetchPolicy::LocalOnly => false,
        };

        let mut fetched_from = Vec::new();
        let mut remote_items: Vec<Value> = Vec::new();
        if fetch {
            let relays = self.relays_for(repo.as_ref());
            let label = repo
                .as_ref()
                .map(|r| r.name.clone())
                .unwrap_or_else(|| d_tag.to_string());
            let filters = vec![
                repo_filter.clone(),
                Issue::filter_for_repo(&coordinate),
                Issue::status_filter_for_repo(&coordinate),
                json!({
                    "kinds": [KIND_REPOSITORY_STATE],
                    "#d": [d_tag],
                    "limit": 20
                }),
            ];
            let steps = vec![
                "the repository announcement".to_string(),
                "issues filed against it".to_string(),
                "status changes (open / resolved / closed / draft)".to_string(),
                "branch + tag state".to_string(),
                "comments under the issues".to_string(),
            ];
            if let Some((remote, chosen)) = self
                .gated_fetch(format!("Repository — {label}"), steps, filters, relays)
                .await?
            {
                fetched_from = chosen.clone();
                repo_events.extend(remote.iter().cloned());
                repo = Repository::newest(&repo_events, &pubkey, d_tag);
                let ids: Vec<String> = Issue::parse_all(&remote).into_iter().map(|i| i.id).collect();
                if !ids.is_empty() {
                    // Same approved relay set, no second modal.
                    let res = self
                        .engine
                        .get_events_with_options(
                            vec![Issue::comments_filter(&ids)],
                            FetchPolicy::FetchAlways,
                            Some(&chosen),
                            true,
                        )
                        .await;
                    if let Ok(r) = res {
                        remote_items.extend(r.events);
                    }
                }
                remote_items.extend(remote);
            }
        }

        let repo = repo.ok_or_else(|| {
            EngineError::NotFound(format!("repository {coordinate} is not in the store"))
        })?;
        let event = repo_events
            .iter()
            .find(|e| event_str(e, "id") == Some(repo.event_id.as_str()))
            .cloned()
            .unwrap_or(Value::Null);
        let authorities = repo.authorities();

        // Everything below reads the store (remote results were ingested on
        // the way in), plus the remote batch as a belt-and-braces merge —
        // nostrdb ingest is async and a just-fetched issue may not be
        // queryable yet.
        let mut local_items = self
            .local(vec![
                Issue::filter_for_repo(&coordinate),
                Issue::status_filter_for_repo(&coordinate),
                json!({
                    "kinds": [KIND_REPOSITORY_STATE],
                    "authors": authorities,
                    "#d": [d_tag],
                    "limit": 20
                }),
            ])
            .await?;
        local_items.extend(remote_items);

        let mut issues = Issue::parse_all(&local_items);
        let statuses: Vec<StatusEvent> = {
            let mut seen = HashSet::new();
            local_items
                .iter()
                .filter_map(StatusEvent::from_event)
                .filter(|s| seen.insert(s.id.clone()))
                .collect()
        };
        Issue::apply_status(&mut issues, &statuses, &authorities);

        let ids: Vec<String> = issues.iter().map(|i| i.id.clone()).collect();
        if !ids.is_empty() {
            let comments = self.local(vec![Issue::comments_filter(&ids)]).await?;
            Issue::apply_comments(&mut issues, &comments);
        }
        issues.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then_with(|| a.id.cmp(&b.id))
        });

        let state = RepositoryState::newest(&local_items, &authorities, d_tag);
        let open_count = issues
            .iter()
            .filter(|i| matches!(i.status, IssueStatus::Open | IssueStatus::Draft))
            .count();
        Ok(LoadedRepository {
            repository: repo,
            event,
            state,
            issues,
            open_count,
            fetched_from,
        })
    }

    /// The store's copy of one issue, with its status resolved against its
    /// primary repository (when that announcement is held). Local only.
    pub async fn issue(&self, issue_id: &str) -> Result<Option<(Issue, Option<Repository>)>> {
        let Some(ev) = crate::query::query_by_id(self.engine.ndb(), issue_id)? else {
            return Ok(None);
        };
        let mut issue = Issue::from_event(&ev)?;
        let repo = match issue.repos.first().and_then(|c| NAddr::from_a_tag(c)) {
            Some(addr) => {
                let evs = self
                    .local(vec![Repository::filter_one(&addr.pubkey, &addr.d_tag)])
                    .await?;
                Repository::newest(&evs, &addr.pubkey, &addr.d_tag)
            }
            None => None,
        };
        let authorities = repo
            .as_ref()
            .map(|r| r.authorities())
            .unwrap_or_default();
        let statuses: Vec<StatusEvent> = self
            .local(vec![json!({
                "kinds": STATUS_KINDS,
                "#e": [issue.id],
                "limit": 100
            })])
            .await?
            .iter()
            .filter_map(StatusEvent::from_event)
            .collect();
        Issue::apply_status(std::slice::from_mut(&mut issue), &statuses, &authorities);
        Ok(Some((issue, repo)))
    }

    /// Relays a NIP-34 conversation SHOULD reach: the repository's own
    /// `relays` for whichever repository `root_event` (an issue / patch /
    /// PR, or a comment under one) belongs to. Empty when unknown.
    pub async fn conversation_relays(&self, root_event: &Value) -> Vec<String> {
        let kind = event_u64(root_event, "kind");
        let tags = root_event.get("tags").and_then(|t| t.as_array());
        let coord = tags.into_iter().flatten().find_map(|t| {
            let arr = t.as_array()?;
            let name = arr.first()?.as_str()?;
            let is_root_ref = if kind == 1111 { name == "A" } else { name == "a" };
            if !is_root_ref {
                return None;
            }
            let v = arr.get(1)?.as_str()?;
            NAddr::from_a_tag(v).filter(|a| a.kind == KIND_REPOSITORY)
        });
        // A comment carries `E` (the issue) but no `A`; resolve through the
        // issue the store holds.
        let coord = match coord {
            Some(c) => Some(c),
            None if kind == 1111 => {
                let root_id = tags.into_iter().flatten().find_map(|t| {
                    let arr = t.as_array()?;
                    (arr.first()?.as_str()? == "E").then(|| arr.get(1)?.as_str().map(str::to_string)).flatten()
                });
                match root_id.and_then(|id| crate::query::query_by_id(self.engine.ndb(), &id).ok().flatten()) {
                    Some(root) if ROOT_KINDS.contains(&event_u64(&root, "kind")) => {
                        Issue::from_event(&root)
                            .ok()
                            .and_then(|i| i.repos.first().and_then(|c| NAddr::from_a_tag(c)))
                    }
                    _ => None,
                }
            }
            None => None,
        };
        let Some(addr) = coord else { return Vec::new() };
        let evs = self
            .local(vec![Repository::filter_one(&addr.pubkey, &addr.d_tag)])
            .await
            .unwrap_or_default();
        Repository::newest(&evs, &addr.pubkey, &addr.d_tag)
            .map(|r| r.relays)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "a008def15796fba9a0d6fab04e8fd57089285d9fd505da5a83fe8aad57a3564d";
    const MAINT: &str = "657c9f566a2627ad76196695361e1d814bdb49e87b90f4db818fb91c59dc142a";
    const OTHER: &str = "28d7ca9cba0e40f59843cd1cd507c1a912919769f4495a0f38df2b0ab0238bc4";

    fn ev(kind: u64, id: &str, pubkey: &str, created_at: u64, tags: Value, content: &str) -> Value {
        json!({
            "kind": kind, "id": id, "pubkey": pubkey, "created_at": created_at,
            "tags": tags, "content": content, "sig": ""
        })
    }

    fn id(n: u8) -> String {
        format!("{:02x}", n).repeat(32)
    }

    /// Real announcement from relay.ngit.dev (trimmed).
    fn ngit_repo() -> Value {
        ev(
            30617,
            &id(1),
            OWNER,
            1787151942,
            json!([
                ["d", "ngit"],
                ["name", "ngit"],
                ["description", "cli and git plugin for code collaboration over nostr"],
                ["clone", "https://relay.ngit.dev/npub15qy/ngit.git", "https://gitnostr.com/npub15qy/ngit.git"],
                ["relays", "wss://relay.ngit.dev", "wss://gitnostr.com"],
                ["alt", "git repository: ngit"],
                ["r", "26689f97810fc656c7134c76e2a37d33b2e40ce7", "euc"],
                ["maintainers", OWNER, MAINT],
                ["web", "https://gitworkshop.dev/danconwaydev.com/ngit"],
                ["t", "git"], ["t", "git"]
            ]),
            "",
        )
    }

    #[test]
    fn parses_announcement_multivalue_tags() {
        let r = Repository::from_event(&ngit_repo()).unwrap();
        assert_eq!(r.d_tag, "ngit");
        assert_eq!(r.name, "ngit");
        assert_eq!(r.clone.len(), 2);
        assert_eq!(r.relays, vec!["wss://relay.ngit.dev", "wss://gitnostr.com"]);
        assert_eq!(r.euc.as_deref(), Some("26689f97810fc656c7134c76e2a37d33b2e40ce7"));
        // The announcer is implied; only the *other* maintainer is listed.
        assert_eq!(r.maintainers, vec![MAINT.to_string()]);
        assert_eq!(r.hashtags, vec!["git"]);
        assert_eq!(r.web.len(), 1);
        assert_eq!(r.coordinate(), format!("30617:{OWNER}:ngit"));
        assert!(r.is_maintainer(OWNER) && r.is_maintainer(MAINT) && !r.is_maintainer(OTHER));
    }

    #[test]
    fn name_falls_back_to_d_tag_and_fork_parsed() {
        let e = ev(
            30617,
            &id(2),
            OWNER,
            10,
            json!([["d", "my-fork"], ["u", format!("30617:{MAINT}:ngit"), "wss://r", MAINT]]),
            "",
        );
        let r = Repository::from_event(&e).unwrap();
        assert_eq!(r.name, "my-fork");
        let f = r.fork_of.unwrap();
        assert_eq!(f.target, format!("30617:{MAINT}:ngit"));
        assert_eq!(f.pubkey.as_deref(), Some(MAINT));
    }

    #[test]
    fn wrong_kind_rejected() {
        assert!(Repository::from_event(&ev(1, &id(3), OWNER, 1, json!([]), "")).is_err());
        assert!(Issue::from_event(&ev(1, &id(3), OWNER, 1, json!([]), "")).is_err());
        assert!(StatusEvent::from_event(&ev(1, &id(3), OWNER, 1, json!([]), "")).is_none());
    }

    #[test]
    fn newest_per_d_tag_keeps_latest_and_sorts() {
        let old = ev(30617, &id(4), OWNER, 100, json!([["d", "a"], ["name", "old"]]), "");
        let new = ev(30617, &id(5), OWNER, 200, json!([["d", "a"], ["name", "new"]]), "");
        let other = ev(30617, &id(6), OWNER, 150, json!([["d", "b"]]), "");
        let repos = Repository::newest_per_d_tag(&[old, other, new, json!({"kind": 1})]);
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "new");
        assert_eq!(repos[1].d_tag, "b");
    }

    #[test]
    fn state_parses_head_and_refs_dropping_peeled() {
        let e = ev(
            30618,
            &id(7),
            OWNER,
            5,
            json!([
                ["d", "ngit"],
                ["HEAD", "ref: refs/heads/master"],
                ["refs/heads/master", "aaaa"],
                ["refs/tags/v1", "bbbb"],
                ["refs/tags/v1^{}", "cccc"]
            ]),
            "",
        );
        let s = RepositoryState::from_event(&e).unwrap();
        assert_eq!(s.head.as_deref(), Some("master"));
        assert_eq!(s.refs.len(), 2);
        assert_eq!(s.refs[1].name, "refs/tags/v1");
    }

    fn issue(n: u8, author: &str, created_at: u64) -> Value {
        ev(
            1621,
            &id(n),
            author,
            created_at,
            json!([
                ["a", format!("30617:{OWNER}:ngit")],
                ["a", format!("30617:{OTHER}:elsewhere"), "", "mention"],
                ["subject", format!("issue {n}")],
                ["t", "bug"],
                ["p", OWNER]
            ]),
            "body",
        )
    }

    fn status(n: u8, kind: u64, author: &str, root: &str, created_at: u64) -> Value {
        ev(
            kind,
            &id(n),
            author,
            created_at,
            json!([["e", root, "", "root"], ["a", format!("30617:{OWNER}:ngit")]]),
            "",
        )
    }

    #[test]
    fn issue_parse_skips_mention_coordinates() {
        let i = Issue::from_event(&issue(10, OTHER, 50)).unwrap();
        assert_eq!(i.repos, vec![format!("30617:{OWNER}:ngit")]);
        assert_eq!(i.subject.as_deref(), Some("issue 10"));
        assert_eq!(i.labels, vec!["bug"]);
        assert_eq!(i.status, IssueStatus::Open);
    }

    #[test]
    fn status_resolution_follows_the_spec() {
        let repo = Repository::from_event(&ngit_repo()).unwrap();
        let auth = repo.authorities();
        let mut issues = Issue::parse_all(&[issue(10, OTHER, 50), issue(11, OTHER, 60)]);
        let statuses: Vec<StatusEvent> = [
            // maintainer closes 10
            status(20, 1632, MAINT, &id(10), 70),
            // bystander "resolves" 10 later — ignored
            status(21, 1631, "ff".repeat(32).as_str(), &id(10), 80),
            // issue author reopens 11 after the owner resolved it
            status(22, 1631, OWNER, &id(11), 70),
            status(23, 1630, OTHER, &id(11), 71),
            // status for an unknown root — ignored
            status(24, 1632, OWNER, &id(99), 90),
        ]
        .iter()
        .filter_map(StatusEvent::from_event)
        .collect();
        assert_eq!(statuses.len(), 5);
        Issue::apply_status(&mut issues, &statuses, &auth);
        let by_id = |n: u8| issues.iter().find(|i| i.id == id(n)).unwrap().clone();
        assert_eq!(by_id(10).status, IssueStatus::Closed);
        assert_eq!(by_id(10).status_event.unwrap().id, id(20));
        assert_eq!(by_id(11).status, IssueStatus::Open);
        assert_eq!(by_id(11).last_activity, 71);
    }

    #[test]
    fn status_root_falls_back_to_unmarked_e() {
        let e = ev(1631, &id(30), OWNER, 1, json!([["e", id(10)]]), "");
        assert_eq!(StatusEvent::from_event(&e).unwrap().root, id(10));
        let none = ev(1631, &id(31), OWNER, 1, json!([["a", "x"]]), "");
        assert!(StatusEvent::from_event(&none).is_none());
    }

    #[test]
    fn comments_counted_by_uppercase_e() {
        let mut issues = Issue::parse_all(&[issue(10, OTHER, 50)]);
        let c = |n: u8, root: &str, t: u64| {
            ev(1111, &id(n), OTHER, t, json!([["E", root, "", OTHER], ["K", "1621"], ["e", id(40)], ["k", "1111"]]), "hi")
        };
        Issue::apply_comments(
            &mut issues,
            &[c(40, &id(10), 60), c(41, &id(10), 65), c(41, &id(10), 65), c(42, &id(99), 70)],
        );
        assert_eq!(issues[0].comment_count, 2);
        assert_eq!(issues[0].last_activity, 65);
    }

    #[test]
    fn issue_template_shape() {
        let repo = Repository::from_event(&ngit_repo()).unwrap();
        let t = issue_template(
            &repo,
            "  Crash on start ",
            "steps…",
            &["bug".into(), "Bug".into(), "".into()],
            "wss://relay.ngit.dev",
            123,
        )
        .unwrap();
        assert_eq!(t.kind, 1621);
        assert_eq!(t.tags[0], vec!["a", &format!("30617:{OWNER}:ngit"), "wss://relay.ngit.dev"]);
        assert_eq!(t.tags[1], vec!["p", OWNER, "wss://relay.ngit.dev"]);
        assert_eq!(t.tags[2], vec!["subject", "Crash on start"]);
        assert_eq!(t.tags.iter().filter(|t| t[0] == "t").count(), 1);
        assert_eq!(t.tags.last().unwrap(), &vec!["alt", "git issue: Crash on start"]);
        assert!(issue_template(&repo, "", "x", &[], "", 1).is_err());
        assert!(issue_template(&repo, "x", "  ", &[], "", 1).is_err());
    }

    #[test]
    fn status_template_shape_excludes_signer() {
        let repo = Repository::from_event(&ngit_repo()).unwrap();
        let i = Issue::from_event(&issue(10, OTHER, 50)).unwrap();
        let t = status_template(&repo, &i, IssueStatus::Closed, "done", OWNER, "", 5);
        assert_eq!(t.kind, 1632);
        assert_eq!(t.tags[0], vec!["e", &id(10), "", "root"]);
        assert_eq!(t.tags[1], vec!["a", &format!("30617:{OWNER}:ngit")]);
        let ps: Vec<&String> = t.tags.iter().filter(|t| t[0] == "p").map(|t| &t[1]).collect();
        assert_eq!(ps, vec![OTHER]);
        assert_eq!(t.content, "done");
        assert_eq!("merged".parse::<IssueStatus>().unwrap(), IssueStatus::Resolved);
        assert!("bogus".parse::<IssueStatus>().is_err());
    }
}
