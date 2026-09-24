//! Merge requests on publications (kind 818) — the routing payload and the
//! local list. Publishing a derivative + 818 and the owner's review /
//! merge / decline land with the relink work; this file is what they will
//! read and what the Confirm modal shows.
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

use axum::extract::{Query, State};
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::AppState;
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::insertion::coord_parts;
use crate::publication::merge_request::{destination_set, is_deliverable, reaction_template, resolve_status, verdict_of, Destination, MergeRequest, MergeStatus};

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
    let mut provenance = engine.provenance_for(&target).await;
    if kind == 30041 {
        // the indexes that list this section (local) — their provenance counts too
        let indexes = engine
            .get_events(vec![json!({ "kinds": [30040], "#a": [target.clone()], "limit": 20 })], FetchPolicy::LocalOnly, None)
            .await
            .map(|r| r.events)
            .unwrap_or_default();
        for ix in indexes {
            for r in ix.get("relays").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                let r = crate::relay_url::normalize_relay_url(r);
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

/// POST /api/v1/merge-requests/react — the target owner's kind-7 on an
/// 818: `+` merged, `-` declined. Anyone else gets 403. The republish that
/// a merge means is the review endpoint's job; this is only the verdict.
pub async fn react_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<ReactRequest>,
) -> Result<Json<ReactResponse>, EngineError> {
    let id = req.id.trim();
    if !super::is_hex64(id) {
        return Err(EngineError::BadRequest("id must be the 818's 64-hex event id".into()));
    }
    let verdict = match req.verdict.trim() {
        "+" => '+',
        "-" => '-',
        other => return Err(EngineError::BadRequest(format!("verdict must be + or -, got `{other}`"))),
    };
    let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, crate::api::SignAs::User).await?;
    let (mr, mut t) = reaction_for(&engine, id, verdict, req.reason.as_deref(), &pubkey).await?;
    t.pubkey = Some(pubkey.clone());
    let reaction = signer.sign(t).await.map_err(super::map_publish_sign_error)?;
    let rid = reaction.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let relays_used: Vec<String> = if req.broadcast {
        let mut urls: Vec<String> = req.relays.clone().unwrap_or_default();
        let policy = if req.mode_confirm { FetchPolicy::LocalFirst } else { FetchPolicy::LocalOnly };
        urls.extend(engine.inbox_relays_for(&mr.pubkey, policy, req.mode_confirm).await.unwrap_or_default());
        urls.extend(engine.provenance_for(&mr.id).await);
        let mut out: Vec<String> = Vec::new();
        for u in urls {
            let n = crate::relay_url::normalize_relay_url(&u);
            if !n.is_empty() && !out.contains(&n) {
                out.push(n);
            }
        }
        if out.is_empty() {
            out = engine.publish_relays().to_vec();
        }
        out
    } else {
        Vec::new()
    };
    let publish = super::finalize_publish(&engine, None, vec![reaction.clone()], req.broadcast, Some(relays_used.clone()), true)
        .await?
        .0;
    Ok(Json(ReactResponse {
        merge_request_id: mr.id,
        status: verdict_of(&reaction).unwrap_or_default(),
        reaction,
        id: rid,
        relays_used,
        publish,
    }))
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
}
