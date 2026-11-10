//! Merge requests on publications (kind 818) — the routing payload, the
//! local list, and the owner's review / merge / decline. Publishing a
//! derivative + 818 lives with the relink path (`api::insertions`).
//!
//! - `GET /api/v1/merge-requests/:id/diff` — the review: the source paired
//!   with the owner's current version, a line diff per replaced section,
//!   the index diff, and the OWNER'S NEXT VERSION templates (same `d`) that
//!   a merge signs (`docs/zettel/decision-merge-requests-818.org`).
//! - `POST /api/v1/merge-requests/:id/merge` — owner only: sign those next
//!   versions, store them, then the `+` verdict.
//! - `POST /api/v1/merge-requests/:id/decline` / `POST /merge-requests/react`
//!   — the verdict alone.
//!
//! - `POST /api/v1/merge-requests/destinations` — where an ask addressed to
//!   the target's owner goes: their inbox (kind-10002 read relays) ∪ where
//!   the target was found (nostrdb provenance) ∪ the index's relay hints ∪
//!   the proposer's write relays, each URL with its reasons; `deliverable`
//!   is false when neither an inbox nor a found-on relay is known
//!   (`docs/zettel/decision-asks-routing.org`).
//! - `GET /api/v1/merge-requests` — the local 818s for me (owner) and by me
//!   (requester), each with its status resolved from the owner's newest
//!   kind-7 (`publication::merge_request::resolve_status`).

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::AppState;
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::event_util::{coord_parts, event_coordinate, event_tags, index_children};
use crate::signing::EventTemplate;
use crate::publication::merge_request::{destination_set, find_replacements, index_diff, is_deliverable, line_diff, reaction_template, resolve_status, verdict_of, Destination, DiffHunk, IndexDiff, MergeRequest, MergeStatus};

#[derive(Debug, Deserialize)]
pub struct DestinationsRequest {
    /// The target coordinate (`30040:pk:d` or `30041:pk:d`).
    pub target: String,
    /// The owner to reach; default: the target coordinate's pubkey.
    #[serde(default)]
    pub owner: Option<String>,
    /// `local_only` never touches a relay; `local_first` (default) pulls the
    /// owner's kind 10002 through the indexer composition when it is not
    /// cached; `fetch_always` likewise.
    #[serde(default)]
    pub policy: Option<String>,
    /// User-initiated: in Confirm mode the 10002 pull raises the intent
    /// modal instead of resolving local-only.
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DestinationsResponse {
    pub target: String,
    pub owner: String,
    pub destinations: Vec<Destination>,
    /// At least one inbox or found-on relay — the ask can reach the owner.
    pub deliverable: bool,
    pub inbox_known: bool,
    pub provenance_known: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The destination set for something addressed to the target's owner.
pub async fn destinations(engine: &AppState, req: &DestinationsRequest) -> Result<DestinationsResponse, EngineError> {
    let target = req.target.trim().to_string();
    let (kind, target_pk, _) = coord_parts(&target)
        .ok_or_else(|| EngineError::BadRequest(format!("target is not a coordinate: `{target}`")))?;
    if !matches!(kind, 30040 | 30041 | 30818) {
        return Err(EngineError::BadRequest(format!("a merge request targets a 30040 or 30041, not kind {kind}")));
    }
    let owner = req
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .unwrap_or(target_pk)
        .to_string();
    if owner.len() != 64 || hex::decode(&owner).is_err() {
        return Err(EngineError::BadRequest("owner must be a 64-hex pubkey".into()));
    }
    let policy: FetchPolicy = req.policy.as_deref().unwrap_or("local_first").parse()?;
    let inbox = engine.inbox_relays_for(&owner, policy, req.mode_confirm).await?;
    // Provenance of the target and, for a section, of its index too: the
    // relay that served the publication is one the owner published to.
    let mut provenance = engine.provenance_for(&target);
    if kind == 30041 {
        // the indexes that list this section (local) — their provenance counts too
        let indexes = engine
            .get_events(vec![json!({ "kinds": [30040], "#a": [target.clone()], "limit": 20 })], FetchPolicy::LocalOnly, None)
            .await
            .map(|r| r.events)
            .unwrap_or_default();
        for ix in indexes {
            for r in ix.get("relays").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                let r = crate::nip11::normalize_relay_url(r);
                if !r.is_empty() && !provenance.contains(&r) {
                    provenance.push(r);
                }
            }
        }
    }
    let hints = engine.relay_hints_for_target(&target);
    let mine = engine.publish_relays();
    let set = destination_set(&inbox, &provenance, &hints, &mine);
    let deliverable = is_deliverable(&set);
    let note = (!deliverable).then(|| {
        format!(
            "no inbox known for {} and no provenance for this event — fetch its kind 10002 (policy: local_first) or pull the publication from a relay first",
            crate::identity::encode_npub(&owner).map(|n| format!("{}…", &n[..12])).unwrap_or_else(|_| owner.clone())
        )
    });
    Ok(DestinationsResponse {
        target,
        owner,
        destinations: set,
        deliverable,
        inbox_known: !inbox.is_empty(),
        provenance_known: !provenance.is_empty(),
        note,
    })
}

/// POST /api/v1/merge-requests/destinations
pub async fn destinations_handler(
    State(engine): State<AppState>,
    Json(req): Json<DestinationsRequest>,
) -> Result<Json<DestinationsResponse>, EngineError> {
    Ok(Json(destinations(&engine, &req).await?))
}

// ---------------------------------------------------------------------------
// The local list
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// `owner` (818s addressed to me or targeting my events), `requester`
    /// (818s I sent), `all` (default).
    #[serde(default)]
    pub role: Option<String>,
    /// `open` (default) or `all`.
    #[serde(default)]
    pub status: Option<String>,
    /// Whose list; default: the logged-in user.
    #[serde(default)]
    pub pubkey: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergeRequestRow {
    #[serde(flatten)]
    pub request: MergeRequest,
    pub status: MergeStatus,
    /// `owner` when it is addressed to me / targets mine, `requester` when I
    /// sent it, `both` when I asked myself (a fork of my own work).
    pub role: String,
    /// The reaction that set the status, when any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Value>,
    /// The raw 818.
    pub event: Value,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub pubkey: String,
    pub requests: Vec<MergeRequestRow>,
    pub open: usize,
}

/// The 818s in the local store that concern `me`, with status.
pub async fn list(engine: &AppState, me: &str, role: &str, status: &str) -> Result<ListResponse, EngineError> {
    let want_owner = matches!(role, "owner" | "all");
    let want_requester = matches!(role, "requester" | "all");
    let mut filters = Vec::new();
    if want_owner {
        filters.push(json!({ "kinds": [818], "#p": [me], "limit": 500 }));
    }
    if want_requester {
        filters.push(json!({ "kinds": [818], "authors": [me], "limit": 500 }));
    }
    let events = engine.get_events(filters, FetchPolicy::LocalOnly, None).await?.events;
    let mut seen = std::collections::HashSet::new();
    let mut rows: Vec<MergeRequestRow> = Vec::new();
    for ev in events {
        let Some(mr) = MergeRequest::from_event(&ev) else { continue };
        if !seen.insert(mr.id.clone()) {
            continue;
        }
        let is_owner = mr.owner == me || mr.target_pubkey() == me;
        let is_requester = mr.pubkey == me;
        let role = match (is_owner, is_requester) {
            (true, true) => "both",
            (true, false) => "owner",
            (false, true) => "requester",
            (false, false) => continue,
        };
        if (role == "owner" && !want_owner) || (role == "requester" && !want_requester) {
            continue;
        }
        let reactions = engine
            .get_events(vec![json!({ "kinds": [7], "#e": [mr.id.clone()], "authors": [mr.owner.clone()], "limit": 50 })], FetchPolicy::LocalOnly, None)
            .await?
            .events;
        let (st, verdict) = resolve_status(&mr, &reactions);
        if status == "open" && st != MergeStatus::Open {
            continue;
        }
        rows.push(MergeRequestRow { request: mr, status: st, role: role.to_string(), verdict: verdict.cloned(), event: ev });
    }
    rows.sort_by(|a, b| b.request.created_at.cmp(&a.request.created_at));
    let open = rows.iter().filter(|r| r.status == MergeStatus::Open).count();
    Ok(ListResponse { pubkey: me.to_string(), requests: rows, open })
}

/// GET /api/v1/merge-requests?role=owner|requester|all&status=open|all
pub async fn list_handler(State(engine): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<ListResponse>, EngineError> {
    let me = match q.pubkey.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => engine
            .my_pubkey()
            .ok_or_else(|| EngineError::Auth("no identity — log in, or pass ?pubkey=".into()))?,
    };
    if me.len() != 64 || hex::decode(&me).is_err() {
        return Err(EngineError::BadRequest("pubkey must be 64 hex".into()));
    }
    let role = q.role.as_deref().unwrap_or("all");
    if !matches!(role, "owner" | "requester" | "all") {
        return Err(EngineError::BadRequest("role must be owner, requester or all".into()));
    }
    let status = q.status.as_deref().unwrap_or("open");
    if !matches!(status, "open" | "all") {
        return Err(EngineError::BadRequest("status must be open or all".into()));
    }
    Ok(Json(list(&engine, &me, role, status).await?))
}


// ---------------------------------------------------------------------------
// The owner's verdict
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ReactRequest {
    /// The kind-818's event id.
    pub id: String,
    /// `+` (merged) or `-` (declined).
    pub verdict: String,
    /// Optional one-line reason, appended after the sign.
    #[serde(default)]
    pub reason: Option<String>,
    /// Default false: sign and keep local.
    #[serde(default)]
    pub broadcast: bool,
    /// Extra relays; the requester's inbox and the 818's provenance are
    /// always unioned in when broadcasting.
    #[serde(default)]
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct ReactResponse {
    pub merge_request_id: String,
    pub status: MergeStatus,
    /// The signed kind-7, ingested.
    pub reaction: Value,
    pub id: String,
    pub relays_used: Vec<String>,
    #[serde(flatten)]
    pub publish: crate::api::PublishResponse,
}

/// The owner's unsigned kind-7 on the local 818 `id`, or why not: 404 when
/// the 818 is not local, 403 when `pubkey` is neither the `p` the request
/// names nor the target's author.
pub(crate) async fn reaction_for(
    engine: &AppState,
    id: &str,
    verdict: char,
    reason: Option<&str>,
    pubkey: &str,
) -> Result<(MergeRequest, crate::signing::EventTemplate), EngineError> {
    let ev = engine
        .get_by_id(id, FetchPolicy::LocalOnly)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("no local kind-818 with id {id}")))?;
    let mr = MergeRequest::from_event(&ev).ok_or_else(|| EngineError::BadRequest(format!("{id} is not a merge request")))?;
    if pubkey != mr.owner && pubkey != mr.target_pubkey() {
        return Err(EngineError::Forbidden(format!(
            "only the owner the merge request is addressed to can answer it ({}…)",
            &mr.owner[..12]
        )));
    }
    let t = reaction_template(&mr, verdict, reason, super::unix_now_i64()).map_err(EngineError::BadRequest)?;
    Ok((mr, t))
}

/// Where a verdict goes when broadcast: `extra` ∪ the requester's inbox ∪
/// the 818's provenance, falling back to the publish set.
async fn verdict_relays(engine: &AppState, mr: &MergeRequest, extra: Option<&[String]>, mode_confirm: bool) -> Vec<String> {
    let mut urls: Vec<String> = extra.map(|v| v.to_vec()).unwrap_or_default();
    let policy = if mode_confirm { FetchPolicy::LocalFirst } else { FetchPolicy::LocalOnly };
    urls.extend(engine.inbox_relays_for(&mr.pubkey, policy, mode_confirm).await.unwrap_or_default());
    urls.extend(engine.provenance_of_id(&mr.id));
    let mut out: Vec<String> = Vec::new();
    for u in urls {
        let n = crate::nip11::normalize_relay_url(&u);
        if !n.is_empty() && !out.contains(&n) {
            out.push(n);
        }
    }
    if out.is_empty() {
        out = engine.publish_relays().to_vec();
    }
    out
}

/// Sign the owner's verdict on `id` as the user and store it (broadcast
/// when asked). Shared by `/react`, `/:id/decline` and the merge.
pub(crate) async fn react_inner(
    engine: &AppState,
    signing: &crate::signing::SigningController,
    id: &str,
    verdict: char,
    reason: Option<&str>,
    broadcast: bool,
    relays: Option<&[String]>,
    mode_confirm: bool,
) -> Result<ReactResponse, EngineError> {
    if !super::is_hex64(id) {
        return Err(EngineError::BadRequest("id must be the 818's 64-hex event id".into()));
    }
    let (pubkey, signer) = super::resolve_publish_signer(engine, signing, crate::api::SignAs::User).await?;
    let (mr, mut t) = reaction_for(engine, id, verdict, reason, &pubkey).await?;
    t.pubkey = Some(pubkey.clone());
    let reaction = signer.sign(t).await.map_err(super::map_publish_sign_error)?;
    let rid = reaction.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let relays_used = if broadcast { verdict_relays(engine, &mr, relays, mode_confirm).await } else { Vec::new() };
    let publish = super::finalize_publish(engine, None, vec![reaction.clone()], broadcast, Some(relays_used.clone()), true)
        .await?
        .0;
    Ok(ReactResponse {
        merge_request_id: mr.id,
        status: verdict_of(&reaction).unwrap_or_default(),
        reaction,
        id: rid,
        relays_used,
        publish,
    })
}

/// POST /api/v1/merge-requests/react — the target owner's kind-7 on an
/// 818: `+` merged, `-` declined. Anyone else gets 403. The republish that
/// a merge means is `/merge-requests/:id/merge`; this is only the verdict.
pub async fn react_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<ReactRequest>,
) -> Result<Json<ReactResponse>, EngineError> {
    let verdict = match req.verdict.trim() {
        "+" => '+',
        "-" => '-',
        other => return Err(EngineError::BadRequest(format!("verdict must be + or -, got `{other}`"))),
    };
    Ok(Json(react_inner(&engine, &signing, req.id.trim(), verdict, req.reason.as_deref(), req.broadcast, req.relays.as_deref(), req.mode_confirm).await?))
}

#[derive(Debug, Deserialize, Default)]
pub struct DeclineRequest {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub mode_confirm: bool,
}

/// POST /api/v1/merge-requests/:id/decline — the owner's `-`, with a reason.
pub async fn decline_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Path(id): Path<String>,
    Json(req): Json<DeclineRequest>,
) -> Result<Json<ReactResponse>, EngineError> {
    Ok(Json(react_inner(&engine, &signing, id.trim(), '-', req.reason.as_deref(), req.broadcast, req.relays.as_deref(), req.mode_confirm).await?))
}

// ---------------------------------------------------------------------------
// The review: the owner's next version, as a diff
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ReviewBase {
    pub coordinate: String,
    /// The version the 818 says the change was made against (`e`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_id: Option<String>,
    /// The owner's current version of the target.
    pub current_id: String,
    /// The target has a newer version than the one the change was made against.
    pub moved_on: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewSource {
    pub coordinate: String,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewSection {
    pub original: String,
    pub derivative: String,
    pub title: String,
    pub format: String,
    /// Hunks of the owner's CURRENT content → the derivative's content.
    pub hunks: Vec<Vec<DiffHunk>>,
    pub tags_added: Vec<Vec<String>>,
    pub tags_removed: Vec<Vec<String>>,
    /// The owner's next version: same `d`, the derivative's content, the
    /// original's tags with the derivative's reference tags merged in.
    pub next: EventTemplate,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewIndex {
    pub diff: IndexDiff,
    /// The owner's next index: same `d`, replaced sections kept at their own
    /// coordinates, added entries inserted at their slots.
    pub next: EventTemplate,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewResponse {
    pub merge_request: MergeRequest,
    pub status: MergeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Value>,
    pub role: String,
    pub base: ReviewBase,
    pub source: ReviewSource,
    pub sections: Vec<ReviewSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<ReviewIndex>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Tag names that carry references — the ones a relink adds and a review
/// compares; everything else on the original stays verbatim.
const REF_TAGS: &[&str] = &["a", "r", "w", "ref", "q"];

fn tag_name(t: &[String]) -> &str {
    t.first().map(String::as_str).unwrap_or("")
}

fn first_tag<'a>(tags: &'a [Vec<String>], name: &str) -> Option<&'a str> {
    tags.iter().find(|t| tag_name(t) == name).and_then(|t| t.get(1)).map(String::as_str)
}

/// The owner's next version of `original` with `derivative`'s content and
/// its reference tags merged in; the `(added, removed)` reference tags.
fn next_section(original: &Value, derivative: &Value, owner: &str, created_at: i64) -> (EventTemplate, Vec<Vec<String>>, Vec<Vec<String>>) {
    let o_tags = event_tags(original);
    let d_tags = event_tags(derivative);
    let is_ref = |t: &Vec<String>| REF_TAGS.contains(&tag_name(t));
    let same = |x: &Vec<String>, y: &Vec<String>| x.first() == y.first() && x.get(1) == y.get(1);
    let added: Vec<Vec<String>> = d_tags.iter().filter(|t| is_ref(t) && !o_tags.iter().any(|o| same(o, t))).cloned().collect();
    let removed: Vec<Vec<String>> = o_tags.iter().filter(|t| is_ref(t) && !d_tags.iter().any(|d| same(d, t))).cloned().collect();
    let mut tags: Vec<Vec<String>> = o_tags.iter().filter(|t| tag_name(t) != "client").cloned().collect();
    tags.retain(|t| !(is_ref(t) && removed.iter().any(|r| same(r, t))));
    tags.extend(added.iter().cloned());
    // A section edit may retitle or re-declare the format: the derivative's
    // `title` and `m` win (everything else stays the owner's).
    for name in ["title", "m"] {
        if let Some(v) = first_tag(&d_tags, name) {
            match tags.iter_mut().find(|t| tag_name(t) == name) {
                Some(t) if t.len() >= 2 => t[1] = v.to_string(),
                _ => tags.push(vec![name.to_string(), v.to_string()]),
            }
        }
    }
    (
        EventTemplate {
            kind: 30041,
            created_at,
            tags,
            content: derivative.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
            pubkey: Some(owner.to_string()),
        },
        added,
        removed,
    )
}

/// The owner's next index: their current tags, with `added` entries
/// inserted at their slots (clamped to the `a` list).
fn next_index(current: &Value, diff: &IndexDiff, owner: &str, created_at: i64) -> EventTemplate {
    let mut tags: Vec<Vec<String>> = event_tags(current).into_iter().filter(|t| tag_name(t) != "client").collect();
    for add in &diff.added {
        if tags.iter().any(|t| tag_name(t) == "a" && t.get(1).map(String::as_str) == Some(add.coordinate.as_str())) {
            continue;
        }
        // the position of the slot-th `a` row, else after the last `a`
        let a_positions: Vec<usize> = tags.iter().enumerate().filter(|(_, t)| tag_name(t) == "a").map(|(i, _)| i).collect();
        let at = a_positions.get(add.slot).copied().unwrap_or_else(|| a_positions.last().map(|p| p + 1).unwrap_or(tags.len()));
        tags.insert(at, vec!["a".to_string(), add.coordinate.clone()]);
    }
    EventTemplate {
        kind: 30040,
        created_at,
        tags,
        content: current.get("content").and_then(Value::as_str).unwrap_or("").to_string(),
        pubkey: Some(owner.to_string()),
    }
}

async fn local_addressable(engine: &AppState, coord: &str) -> Result<Option<Value>, EngineError> {
    match coord_parts(coord) {
        Some((k, pk, d)) => engine.get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly).await,
        None => Ok(None),
    }
}

/// The review of a local 818 for `me` (its owner or its requester; 403 for
/// anyone else): status, the base pairing, one entry per replaced section
/// with the owner's next version, the index diff.
pub async fn review(engine: &AppState, id: &str, me: &str) -> Result<ReviewResponse, EngineError> {
    let ev = engine
        .get_by_id(id, FetchPolicy::LocalOnly)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("no local kind-818 with id {id}")))?;
    let mr = MergeRequest::from_event(&ev).ok_or_else(|| EngineError::BadRequest(format!("{id} is not a merge request")))?;
    let is_owner = me == mr.owner || me == mr.target_pubkey();
    let is_requester = me == mr.pubkey;
    let role = match (is_owner, is_requester) {
        (true, true) => "both",
        (true, false) => "owner",
        (false, true) => "requester",
        (false, false) => return Err(EngineError::Forbidden("only the owner or the requester can review a merge request".into())),
    };
    let reactions = engine
        .get_events(vec![json!({ "kinds": [7], "#e": [mr.id.clone()], "authors": [mr.owner.clone()], "limit": 50 })], FetchPolicy::LocalOnly, None)
        .await?
        .events;
    let (status, verdict) = resolve_status(&mr, &reactions);
    let owner = mr.target_pubkey().to_string();
    let current = local_addressable(engine, &mr.target)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("the target {} is not local", mr.target)))?;
    let current_id = current.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let moved_on = mr.base.as_deref().is_some_and(|b| b != current_id);
    let source_id = mr.source.clone().ok_or_else(|| EngineError::BadRequest("the merge request names no source".into()))?;
    let source = engine
        .get_by_id(&source_id, FetchPolicy::LocalOnly)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("the source {source_id} is not local — fetch it first")))?;
    let source_coord = event_coordinate(&source).unwrap_or_default();
    let now = super::unix_now_i64();
    let mut sections = Vec::new();
    let mut index = None;
    if mr.target_kind == 30041 {
        let (next, added, removed) = next_section(&current, &source, &owner, now);
        let tags = event_tags(&current);
        sections.push(ReviewSection {
            original: mr.target.clone(),
            derivative: source_coord.clone(),
            title: first_tag(&tags, "title").unwrap_or("").to_string(),
            format: format_of(&current),
            hunks: line_diff(current.get("content").and_then(Value::as_str).unwrap_or(""), source.get("content").and_then(Value::as_str).unwrap_or("")),
            tags_added: added,
            tags_removed: removed,
            next,
        });
    } else {
        // base sections (current versions) and the source's children
        let mut base_sections = Vec::new();
        for c in index_children(&current) {
            if let Some(e) = local_addressable(engine, &c).await? {
                base_sections.push(e);
            }
        }
        let mut src_children = Vec::new();
        for c in index_children(&source) {
            if let Some(e) = local_addressable(engine, &c).await? {
                src_children.push(e);
            }
        }
        // a derivative section: a 30041 not by the owner, or one carrying E
        let derivatives: Vec<Value> = src_children
            .iter()
            .filter(|e| e.get("kind").and_then(Value::as_u64) == Some(30041))
            .filter(|e| e.get("pubkey").and_then(Value::as_str) != Some(owner.as_str()) || event_tags(e).iter().any(|t| tag_name(t) == "E"))
            .cloned()
            .collect();
        // `E` may name an older version of a base section: resolve those ids to coordinates too
        let mut base_versions = base_sections.clone();
        for d in &derivatives {
            if let Some(eid) = event_tags(d).iter().find(|t| tag_name(t) == "E").and_then(|t| t.get(1).cloned()) {
                if !base_versions.iter().any(|b| b.get("id").and_then(Value::as_str) == Some(eid.as_str())) {
                    if let Some(old) = engine.get_by_id(&eid, FetchPolicy::LocalOnly).await? {
                        base_versions.push(old);
                    }
                }
            }
        }
        let replacements = find_replacements(&derivatives, &base_versions);
        for (orig_coord, deriv_coord) in &replacements {
            let Some(original) = base_sections.iter().find(|b| event_coordinate(b).as_deref() == Some(orig_coord.as_str())) else { continue };
            let Some(derivative) = derivatives.iter().find(|d| event_coordinate(d).as_deref() == Some(deriv_coord.as_str())) else { continue };
            let (next, added, removed) = next_section(original, derivative, &owner, now);
            let tags = event_tags(original);
            sections.push(ReviewSection {
                original: orig_coord.clone(),
                derivative: deriv_coord.clone(),
                title: first_tag(&tags, "title").unwrap_or("").to_string(),
                format: format_of(original),
                hunks: line_diff(original.get("content").and_then(Value::as_str).unwrap_or(""), derivative.get("content").and_then(Value::as_str).unwrap_or("")),
                tags_added: added,
                tags_removed: removed,
                next,
            });
        }
        let diff = index_diff(&current, &source, &replacements);
        let next = next_index(&current, &diff, &owner, now);
        index = Some(ReviewIndex { diff, next });
    }
    let note = moved_on.then(|| format!("based on an older version of {} — the diff is against the owner's current one", mr.target));
    Ok(ReviewResponse {
        status,
        verdict: verdict.cloned(),
        role: role.to_string(),
        base: ReviewBase { coordinate: mr.target.clone(), base_id: mr.base.clone(), current_id, moved_on },
        source: ReviewSource { coordinate: source_coord, id: source_id },
        sections,
        index,
        note,
        merge_request: mr,
    })
}

fn format_of(ev: &Value) -> String {
    crate::mimetype::from_event(ev).and_then(crate::mimetype::short_name).unwrap_or("plain").to_string()
}

/// GET /api/v1/merge-requests/:id/diff
pub async fn review_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Path(id): Path<String>,
) -> Result<Json<ReviewResponse>, EngineError> {
    let id = id.trim().to_string();
    if !super::is_hex64(&id) {
        return Err(EngineError::BadRequest("id must be the 818's 64-hex event id".into()));
    }
    let me = signing
        .active_pubkey()
        .await
        .or_else(|| engine.my_pubkey())
        .ok_or_else(|| EngineError::Auth("no identity — log in to review a merge request".into()))?;
    Ok(Json(review(&engine, &id, &me).await?))
}

#[derive(Debug, Deserialize, Default)]
pub struct MergeRequestBody {
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct MergedSection {
    pub coordinate: String,
    pub event: Value,
}

#[derive(Debug, Serialize)]
pub struct MergeResponse {
    pub merged: bool,
    pub merge_request_id: String,
    pub sections: Vec<MergedSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Value>,
    pub reaction: Value,
    pub status: MergeStatus,
    pub relays_used: Vec<String>,
    #[serde(flatten)]
    pub publish: crate::api::PublishResponse,
}

/// The merge: sign the owner's next versions (same `d`), store them, then
/// the `+` verdict. Owner only.
pub(crate) async fn merge(
    engine: &AppState,
    signing: &crate::signing::SigningController,
    id: &str,
    broadcast: bool,
    relays: Option<&[String]>,
    mode_confirm: bool,
) -> Result<MergeResponse, EngineError> {
    if !super::is_hex64(id) {
        return Err(EngineError::BadRequest("id must be the 818's 64-hex event id".into()));
    }
    let (pubkey, signer) = super::resolve_publish_signer(engine, signing, crate::api::SignAs::User).await?;
    let rv = review(engine, id, &pubkey).await?;
    if rv.role == "requester" {
        return Err(EngineError::Forbidden("only the owner can merge; the requester waits for the verdict".into()));
    }
    if rv.sections.is_empty() && rv.index.as_ref().map(|i| i.diff.added.is_empty() && i.diff.removed.is_empty() && i.diff.moved.is_empty()).unwrap_or(true) {
        return Err(EngineError::BadRequest("nothing to merge: the source does not change the target".into()));
    }
    let mut sections = Vec::new();
    let mut events = Vec::new();
    for s in rv.sections {
        let mut t = s.next;
        t.pubkey = Some(pubkey.clone());
        let ev = signer.sign(t).await.map_err(super::map_publish_sign_error)?;
        sections.push(MergedSection { coordinate: event_coordinate(&ev).unwrap_or(s.original), event: ev.clone() });
        events.push(ev);
    }
    let index = match rv.index {
        Some(ix) => {
            let mut t = ix.next;
            t.pubkey = Some(pubkey.clone());
            Some(signer.sign(t).await.map_err(super::map_publish_sign_error)?)
        }
        None => None,
    };
    let publish = super::finalize_publish(engine, index.clone(), events, broadcast, relays.map(|r| r.to_vec()), true)
        .await?
        .0;
    let verdict = react_inner(engine, signing, id, '+', None, broadcast, relays, mode_confirm).await?;
    Ok(MergeResponse {
        merged: true,
        merge_request_id: rv.merge_request.id,
        sections,
        index,
        reaction: verdict.reaction,
        status: MergeStatus::Merged,
        relays_used: verdict.relays_used,
        publish,
    })
}

/// POST /api/v1/merge-requests/:id/merge
pub async fn merge_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Path(id): Path<String>,
    Json(req): Json<MergeRequestBody>,
) -> Result<Json<MergeResponse>, EngineError> {
    Ok(Json(merge(&engine, &signing, id.trim(), req.broadcast, req.relays.as_deref(), req.mode_confirm).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publication::merge_request::{template, MergeHints};
    use std::sync::Arc;

    const OWNER_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000011";
    const ME_SEC: &str = "0000000000000000000000000000000000000000000000000000000000000012";

    fn pk_of(secret_hex: &str) -> String {
        use secp256k1::{PublicKey, Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&hex::decode(secret_hex).unwrap()).unwrap();
        hex::encode(&PublicKey::from_secret_key(&secp, &sk).serialize()[1..33])
    }

    /// A signed event JSON for `secret_hex` (the engine's relay-form ingest
    /// verifies signatures).
    fn signed(secret_hex: &str, kind: u64, content: &str, tags: Vec<Vec<String>>, created_at: u64) -> (String, Value) {
        use sha2::{Digest, Sha256};
        let pubkey = pk_of(secret_hex);
        let serial = serde_json::to_string(&json!([0, pubkey, created_at, kind, tags, content])).unwrap();
        let id = hex::encode(Sha256::digest(serial.as_bytes()));
        let sig = crate::identity::sign_event_hash(&id, secret_hex).unwrap();
        let ev = json!({ "id": id, "pubkey": pubkey, "created_at": created_at, "kind": kind, "tags": tags, "content": content, "sig": sig });
        (id, ev)
    }

    async fn ingest(engine: &Arc<crate::engine::Engine>, ev: &Value, id: &str) {
        engine.ingest_event(&serde_json::to_string(ev).unwrap()).unwrap();
        for _ in 0..40 {
            if engine.get_by_id(id, FetchPolicy::LocalOnly).await.unwrap().is_some() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("event {id} never landed");
    }

    #[tokio::test]
    async fn merge_request_list_resolves_status_from_the_owners_reaction() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(crate::engine::Engine::new(dir.path()).unwrap());
        std::mem::forget(dir);
        let owner = pk_of(OWNER_SEC);
        let me = pk_of(ME_SEC);
        let target = format!("30040:{owner}:on-method");
        let base = "a".repeat(64);
        let source = "b".repeat(64);
        let t = template(&target, Some(&base), Some(&source), &owner, "relink 2 terms", &MergeHints::default(), 1000).unwrap();
        let (mr_id, mr_ev) = signed(ME_SEC, 818, &t.content, t.tags.clone(), 1000);
        ingest(&engine, &mr_ev, &mr_id).await;

        // Open, seen from both sides.
        let mine = list(&engine, &me, "requester", "open").await.unwrap();
        assert_eq!(mine.requests.len(), 1);
        assert_eq!(mine.requests[0].role, "requester");
        assert_eq!(mine.requests[0].status, MergeStatus::Open);
        assert_eq!(mine.open, 1);
        let theirs = list(&engine, &owner, "owner", "open").await.unwrap();
        assert_eq!(theirs.requests.len(), 1);
        assert_eq!(theirs.requests[0].role, "owner");
        assert_eq!(theirs.requests[0].request.source.as_deref(), Some(source.as_str()));
        assert!(list(&engine, &"c".repeat(64), "all", "all").await.unwrap().requests.is_empty(), "a stranger sees nothing");

        // A stranger's + changes nothing; the owner's - declines it.
        let (sid, s_ev) = signed("0000000000000000000000000000000000000000000000000000000000000013", 7, "+", vec![vec!["e".into(), mr_id.clone()], vec!["k".into(), "818".into()]], 1001);
        ingest(&engine, &s_ev, &sid).await;
        assert_eq!(list(&engine, &owner, "owner", "all").await.unwrap().requests[0].status, MergeStatus::Open);
        let (rid, r_ev) = signed(OWNER_SEC, 7, "- not now", vec![vec!["e".into(), mr_id.clone()], vec!["p".into(), me.clone()], vec!["k".into(), "818".into()]], 1002);
        ingest(&engine, &r_ev, &rid).await;
        let after = list(&engine, &me, "all", "all").await.unwrap();
        assert_eq!(after.requests[0].status, MergeStatus::Declined);
        assert_eq!(after.requests[0].verdict.as_ref().unwrap()["id"], rid);
        assert_eq!(after.open, 0);
        assert!(list(&engine, &me, "all", "open").await.unwrap().requests.is_empty(), "declined is not open");

        // Destinations: nothing on the wire yet → not deliverable, with a note;
        // hints and my own relays alone never count.
        let d = destinations(&engine, &DestinationsRequest { target: target.clone(), owner: None, policy: Some("local_only".into()), mode_confirm: false }).await.unwrap();
        assert_eq!(d.owner, owner);
        assert!(!d.deliverable && !d.inbox_known && !d.provenance_known);
        assert!(d.note.as_deref().unwrap_or("").contains("no inbox known"));
        // The owner's 10002 lands locally → their read relays are the inbox.
        let (lid, l_ev) = signed(OWNER_SEC, 10002, "", vec![vec!["r".into(), "wss://relay.pkf.example".into(), "read".into()], vec!["r".into(), "wss://write.only".into(), "write".into()], vec!["r".into(), "wss://both.example".into()]], 1003);
        ingest(&engine, &l_ev, &lid).await;
        let d2 = destinations(&engine, &DestinationsRequest { target, owner: None, policy: Some("local_only".into()), mode_confirm: false }).await.unwrap();
        use crate::publication::merge_request::Reason;
        let inbox: Vec<&str> = d2.destinations.iter().filter(|x| x.reasons.contains(&Reason::Inbox)).map(|x| x.url.as_str()).collect();
        assert_eq!(inbox, vec!["wss://relay.pkf.example", "wss://both.example"], "read + read/write entries, never the write-only one");
        assert_eq!(&d2.destinations[0].url, "wss://relay.pkf.example", "inbox entries come first");
        assert!(d2.deliverable && d2.inbox_known && d2.note.is_none());
        // the engine's default publish set rides along as `mine` — never as inbox / found-on
        assert!(d2.destinations.iter().filter(|x| !x.reasons.contains(&Reason::Inbox)).all(|x| x.reasons == vec![Reason::Mine]));
        assert!(destinations(&engine, &DestinationsRequest { target: "30023:x:y".into(), owner: None, policy: None, mode_confirm: false }).await.is_err());
    }

    /// A stranger edits one section of the owner's publication: the planner
    /// forks it and names it in an 818; the owner's review pairs fork and
    /// base and its next version keeps the owner's `d` with the new text
    /// and title.
    #[tokio::test]
    async fn a_section_edit_by_a_stranger_reviews_as_the_owners_next_version() {
        use crate::publication::section_edit::{merge_request_for, plan, EditRoute, SectionEdit};
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(crate::engine::Engine::new(dir.path()).unwrap());
        std::mem::forget(dir);
        let owner = pk_of(OWNER_SEC);
        let me = pk_of(ME_SEC);
        let s_tags = vec![vec!["d".into(), "s1".into()], vec!["title".into(), "One".into()], vec!["m".into(), "text/markdown".into()], vec!["t".into(), "keep".into()]];
        let (sid, s_ev) = signed(OWNER_SEC, 30041, "first line\nsecond line", s_tags, 900);
        ingest(&engine, &s_ev, &sid).await;
        let ix_tags = vec![vec!["d".into(), "book".into()], vec!["title".into(), "Book".into()], vec!["a".into(), format!("30041:{owner}:s1")]];
        let (iid, ix_ev) = signed(OWNER_SEC, 30040, "", ix_tags, 901);
        ingest(&engine, &ix_ev, &iid).await;

        let edit = SectionEdit { content: "first line\nsecond line, sharper".into(), title: Some("Uno".into()), format: None };
        let p = plan(&s_ev, Some(&ix_ev), &me, &edit, "", false, 950).unwrap();
        assert_eq!(p.route, EditRoute::ForkAndRequest);
        let (fid, f_ev) = signed(ME_SEC, 30041, &p.section.content, p.section.tags.clone(), 950);
        ingest(&engine, &f_ev, &fid).await;
        let t = merge_request_for(&p, &fid, "", &MergeHints::default(), 951).unwrap();
        let (mid, m_ev) = signed(ME_SEC, 818, &t.content, t.tags.clone(), 951);
        ingest(&engine, &m_ev, &mid).await;

        let rv = review(&engine, &mid, &owner).await.unwrap();
        assert_eq!(rv.role, "owner");
        assert!(!rv.base.moved_on);
        assert!(rv.index.is_none(), "a section target merges without touching the index");
        assert_eq!(rv.sections.len(), 1);
        let next = &rv.sections[0].next;
        assert_eq!(next.content, "first line\nsecond line, sharper");
        let tag = |n: &str| next.tags.iter().find(|t| t[0] == n).map(|t| t[1].clone());
        assert_eq!(tag("d").as_deref(), Some("s1"), "the owner's own d");
        assert_eq!(tag("title").as_deref(), Some("Uno"), "the retitle carries");
        assert_eq!(tag("t").as_deref(), Some("keep"));
        assert!(next.tags.iter().all(|t| t[0] != "E"), "the owner's version is not a derivative");
        assert!(rv.sections[0].hunks.iter().flatten().any(|h| h.line == "second line, sharper"));
        // the requester sees the same review; a stranger is refused
        assert_eq!(review(&engine, &mid, &me).await.unwrap().role, "requester");
        assert!(review(&engine, &mid, &"c".repeat(64)).await.is_err());
    }
}
