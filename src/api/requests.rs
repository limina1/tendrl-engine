//! Annotation requests over HTTP — `/api/v1/annotations/requests*`.
//!
//! Create (local record; optionally signed by the user as a kind-30041
//! annotation on the section, optionally broadcast), list, delete, answer.
//! `list` merges two sources: the local records and every signed request
//! annotation in the store (`k:30041 t:annotation-request`), and derives
//! *answered* from the notes that point back at a request with a `request`
//! tag. The answer endpoint closes a record by hand (or from the
//! `answer_request` tool) — a signed request needs no such call; the note
//! that answers it is the answer.

use super::{AppState, SignAs};
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::insertion::{coord_parts, event_coordinate};
use crate::requests::{
    request_pointer, vocabulary_task, AnnotationRequest, RequestAnswer, RequestStatus, RequestStore, OUTPUTS,
    REQUEST_TAG,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub root: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub parent_event_id: Option<String>,
    #[serde(default)]
    pub parent_title: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// The ask in the user's words. Optional for `output: vocabulary`, where
    /// it is generated from `terms` when left empty.
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub context: Option<String>,
    /// `notes` (default), `kasten` or `vocabulary`.
    #[serde(default)]
    pub output: Option<String>,
    /// `output: vocabulary` — the words highlighted in the section, one
    /// `term` tag each. Empty = find the undefined vocabulary.
    #[serde(default)]
    pub terms: Vec<String>,
    /// Coordinate of a style event (`30078:pk:d`) the answer should follow.
    #[serde(default)]
    pub style: Option<String>,
    /// Also sign it as a kind-30041 annotation on the section, as you (needs
    /// your identity). Default false — local record only.
    #[serde(default)]
    pub sign: bool,
    /// With `sign`: also send the annotation to the publish relays. Default
    /// false — it stays in the local store.
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RequestResponse {
    #[serde(flatten)]
    pub request: AnnotationRequest,
    /// The signed annotation, when `sign` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Value>,
}

/// The record a create call describes, before anything is signed or saved.
/// Shared by `create_handler` and `preview_handler` so the preview shows
/// exactly what the create path signs. For `output: vocabulary` the task is
/// generated from the terms unless the caller typed one.
async fn build_request(engine: &AppState, req: &CreateRequest, now: i64) -> Result<AnnotationRequest, EngineError> {
    let output = match req.output.as_deref().map(str::trim) {
        None | Some("") => "notes",
        Some(o) if OUTPUTS.contains(&o) => o,
        Some(_) => return Err(EngineError::BadRequest("output must be notes, kasten or vocabulary".into())),
    };
    let task = if output == "vocabulary" && req.task.trim().is_empty() {
        vocabulary_task(&req.terms)
    } else {
        req.task.clone()
    };
    let mut r = AnnotationRequest::new(&req.root, req.parent.as_deref(), &task, now)
        .map_err(EngineError::BadRequest)?
        .with_scope(&req.scope)
        .with_terms(&req.terms);
    r.output = output.to_string();
    r.parent_title = req.parent_title.clone().filter(|t| !t.trim().is_empty());
    r.title = req.title.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    r.context = req.context.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    // Pin the parent version from the local store when the caller did not.
    r.parent_event_id = req.parent_event_id.clone().filter(|e| super::is_hex64(e));
    if r.parent_event_id.is_none() {
        if let Some((k, pk, d)) = coord_parts(&r.parent) {
            if let Ok(Some(ev)) = engine.get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly).await {
                r.parent_event_id = ev.get("id").and_then(Value::as_str).map(str::to_string);
            }
        }
    }
    if let Some(coord) = req.style.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let style = super::styles::get_style(engine, coord)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("style {coord} is not in the local store")))?;
        r.style = Some(style.coordinate);
        r.style_title = Some(style.title);
    }
    Ok(r)
}

/// POST /api/v1/annotations/requests/preview — the unsigned kind-30041 the
/// create path would sign for this body (same builder), pubkey filled when
/// the user has an identity. Persists nothing; the modal renders it live.
pub async fn preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<CreateRequest>,
) -> Result<Json<Value>, EngineError> {
    let now = super::unix_now_i64();
    let r = build_request(&engine, &req, now).await?;
    let relay_hint = super::discussion_relay_hint(&engine);
    let mut template = r.to_template(&relay_hint, now).map_err(EngineError::BadRequest)?;
    crate::signing::ensure_client_tag(&mut template.tags);
    template.pubkey = signing.active_pubkey().await;
    Ok(Json(json!({ "event": template, "request": r })))
}

/// POST /api/v1/annotations/requests
pub async fn create_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<CreateRequest>,
) -> Result<Json<RequestResponse>, EngineError> {
    let now = super::unix_now_i64();
    let mut r = build_request(&engine, &req, now).await?;

    let mut event = None;
    if req.sign {
        // The user signs the ask; the same path an insertion takes.
        let (pubkey, signer) = super::resolve_publish_signer(&engine, &signing, SignAs::User).await?;
        let relay_hint = super::discussion_relay_hint(&engine);
        let mut template = r.to_template(&relay_hint, now).map_err(EngineError::BadRequest)?;
        template.pubkey = Some(pubkey);
        let signed = signer
            .sign(template)
            .await
            .map_err(super::map_publish_sign_error)?;
        // Local by default; `broadcast` sends it to the publish relays.
        let _ = super::finalize_publish(
            &engine,
            None,
            vec![signed.clone()],
            req.broadcast,
            req.relays.clone(),
            true,
        )
        .await?;
        r.event_id = signed.get("id").and_then(Value::as_str).map(str::to_string);
        r.coordinate = event_coordinate(&signed);
        event = Some(signed);
    }

    RequestStore::new(engine.data_dir())?.save(&r)?;
    emit_request(&engine, &r);
    Ok(Json(RequestResponse { request: r, event }))
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct ListQuery {
    pub root: Option<String>,
    pub parent: Option<String>,
    /// `open` · `answered` · `all` (default all).
    pub status: Option<String>,
}

fn parse_status(s: Option<&str>) -> Result<Option<RequestStatus>, EngineError> {
    match s.map(str::trim) {
        None | Some("") | Some("all") => Ok(None),
        Some("open") => Ok(Some(RequestStatus::Open)),
        Some("answered") => Ok(Some(RequestStatus::Answered)),
        Some(other) => Err(EngineError::BadRequest(format!(
            "status must be open, answered or all — got `{other}`"
        ))),
    }
}

/// Newest version per coordinate of a batch of addressable events.
fn newest_per_coordinate(events: Vec<Value>) -> Vec<Value> {
    let mut newest: HashMap<String, Value> = HashMap::new();
    for ev in events {
        let Some(coord) = event_coordinate(&ev) else {
            continue;
        };
        let ts = ev.get("created_at").and_then(Value::as_i64).unwrap_or(0);
        let replace = newest
            .get(&coord)
            .map(|old| old.get("created_at").and_then(Value::as_i64).unwrap_or(0) < ts)
            .unwrap_or(true);
        if replace {
            newest.insert(coord, ev);
        }
    }
    newest.into_values().collect()
}

/// Signed requests in the local store, as records. `root` narrows the scan.
fn signed_requests(engine: &AppState, root: Option<&str>) -> Result<Vec<AnnotationRequest>, EngineError> {
    let mut filter = json!({ "kinds": [30041], "#t": [REQUEST_TAG], "limit": 500 });
    if let Some(root) = root {
        filter["#A"] = json!([root]);
    }
    let events = crate::query::query_local(engine.ndb(), &[filter])?;
    Ok(newest_per_coordinate(events)
        .iter()
        .filter_map(AnnotationRequest::from_event)
        .collect())
}

/// Mark every request that a local note points at as answered. A request
/// is answered iff a 30041 — or a bundle's root 30040 — anchored to its root carries
/// `["request", <its coordinate>]`; a record's own answer (from
/// `answer_request`) is kept when it has one.
fn derive_answers(engine: &AppState, requests: &mut [AnnotationRequest]) -> Result<(), EngineError> {
    let coords: HashSet<&str> = requests.iter().filter_map(|r| r.coordinate.as_deref()).collect();
    if coords.is_empty() {
        return Ok(());
    }
    let roots: HashSet<&str> = requests
        .iter()
        .filter(|r| r.coordinate.is_some())
        .map(|r| r.root.as_str())
        .collect();
    // request coordinate → (answer coordinates, newest answer time, its author)
    let mut answers: HashMap<String, (Vec<String>, i64, Option<String>)> = HashMap::new();
    for root in roots {
        // A note (30041) or a bundle root (30040 — a whole kasten) may answer.
        let filter = json!({ "kinds": [30040, 30041], "#A": [root], "limit": 1000 });
        let events = crate::query::query_local(engine.ndb(), &[filter])?;
        for ev in newest_per_coordinate(events) {
            let Some(target) = request_pointer(&ev) else {
                continue;
            };
            if !coords.contains(target.as_str()) {
                continue;
            }
            let Some(coord) = event_coordinate(&ev) else {
                continue;
            };
            let ts = ev.get("created_at").and_then(Value::as_i64).unwrap_or(0);
            let by = ev.get("pubkey").and_then(Value::as_str).map(str::to_string);
            let entry = answers.entry(target).or_insert_with(|| (Vec::new(), 0, None));
            entry.0.push(coord);
            if ts >= entry.1 {
                entry.1 = ts;
                entry.2 = by;
            }
        }
    }
    for r in requests.iter_mut() {
        let Some(coord) = r.coordinate.as_deref() else {
            continue;
        };
        let Some((coords, ts, by)) = answers.get(coord) else {
            continue;
        };
        let mut coords = coords.clone();
        coords.sort();
        match r.answer.as_mut() {
            Some(a) => {
                for c in coords {
                    if !a.coordinates.contains(&c) {
                        a.coordinates.push(c);
                    }
                }
            }
            None => {
                let n = coords.len();
                r.answer = Some(RequestAnswer {
                    summary: format!("{n} note{} point{} at this request", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }),
                    coordinates: coords,
                    answered_at: *ts,
                    by: by.clone(),
                });
            }
        }
        r.status = RequestStatus::Answered;
    }
    Ok(())
}

/// Shared by the endpoint and the `list_requests` tool: local records plus
/// signed request annotations in the store, answered derived from the
/// notes that point at them, newest first.
pub(crate) fn list_requests(
    engine: &AppState,
    root: Option<&str>,
    parent: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<AnnotationRequest>, EngineError> {
    let status = parse_status(status)?;
    let mut out = RequestStore::new(engine.data_dir())?.query(root, parent, None)?;
    let known: HashSet<String> = out.iter().filter_map(|r| r.coordinate.clone()).collect();
    for r in signed_requests(engine, root)? {
        // A record that signed this event already stands for it.
        if r.coordinate.as_deref().map(|c| known.contains(c)).unwrap_or(false) {
            continue;
        }
        if parent.map(|p| r.parent == p).unwrap_or(true) {
            out.push(r);
        }
    }
    derive_answers(engine, &mut out)?;
    if let Some(s) = status {
        out.retain(|r| r.status == s);
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    Ok(out)
}

/// One request by id: a record (`rq_…`) or a signed request annotation
/// (`ev_…`), answered derived either way.
pub(crate) fn get_request(engine: &AppState, id: &str) -> Result<AnnotationRequest, EngineError> {
    let mut one = if id.starts_with("rq_") {
        vec![RequestStore::new(engine.data_dir())?.load(id)?]
    } else {
        signed_requests(engine, None)?
            .into_iter()
            .filter(|r| r.id == id)
            .take(1)
            .collect::<Vec<_>>()
    };
    if one.is_empty() {
        return Err(EngineError::NotFound(format!("request {id}")));
    }
    derive_answers(engine, &mut one)?;
    Ok(one.remove(0))
}

/// A request as the agent sees it: the record plus the style's text, so it
/// need not fetch the style event itself.
pub(crate) async fn request_with_style(engine: &AppState, r: AnnotationRequest) -> Value {
    let mut v = serde_json::to_value(&r).unwrap_or(Value::Null);
    if let Some(coord) = &r.style {
        if let Ok(Some(style)) = super::styles::get_style(engine, coord).await {
            v["style_instructions"] = json!(style.content);
        }
    }
    v
}

/// GET /api/v1/annotations/requests?root=&parent=&status=
pub async fn list_handler(
    State(engine): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, EngineError> {
    let requests = list_requests(
        &engine,
        q.root.as_deref(),
        q.parent.as_deref(),
        q.status.as_deref(),
    )?;
    let open = requests
        .iter()
        .filter(|r| r.status == RequestStatus::Open)
        .count();
    Ok(Json(json!({ "requests": requests, "open": open, "count": requests.len() })))
}

/// GET /api/v1/annotations/requests/:id
pub async fn get_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AnnotationRequest>, EngineError> {
    Ok(Json(get_request(&engine, &id)?))
}

/// DELETE /api/v1/annotations/requests/:id — the local record only; a signed
/// request stays an event (NIP-09 it from its menu if you want it gone).
pub async fn delete_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, EngineError> {
    let store = RequestStore::new(engine.data_dir())?;
    let gone = store.load(&id).ok();
    store.delete(&id)?;
    if let Some(r) = gone {
        engine.emit_store_event(crate::store_events::StoreEvent::Request {
            id: r.id,
            root: r.root,
            parent: r.parent,
            status: "deleted".into(),
        });
    }
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub summary: String,
    #[serde(default)]
    pub coordinates: Vec<String>,
    /// Who is answering. `assistant` (default) records the assistant's
    /// pubkey; `user` marks it answered by hand. No event is signed here —
    /// the notes that answer a signed request point at it themselves.
    #[serde(default)]
    pub sign_as: Option<SignAs>,
}

/// Shared by the endpoint and the `answer_request` tool. Closes a local
/// record; a signed request with no record (`ev_…`) is closed by the notes
/// that point at it, not by this call.
pub(crate) async fn answer_request(
    engine: &AppState,
    id: &str,
    req: AnswerRequest,
) -> Result<AnnotationRequest, EngineError> {
    if !id.starts_with("rq_") {
        return Err(EngineError::BadRequest(
            "a signed request is answered by the notes that point at it — write them with `request` set to its coordinate"
                .into(),
        ));
    }
    let store = RequestStore::new(engine.data_dir())?;
    let mut r = store.load(id)?;
    let summary = req.summary.trim().to_string();
    if summary.is_empty() {
        return Err(EngineError::BadRequest("an answer needs a summary".into()));
    }
    let coordinates: Vec<String> = req
        .coordinates
        .iter()
        .map(|c| c.trim().to_string())
        .filter(|c| coord_parts(c).is_some())
        .collect();
    if coordinates.len() != req.coordinates.len() {
        return Err(EngineError::BadRequest(
            "coordinates must be kind:pubkey:d".into(),
        ));
    }
    let sign_as = req.sign_as.unwrap_or(SignAs::Assistant);
    let now = super::unix_now_i64();
    let mut answer = RequestAnswer {
        summary,
        coordinates,
        answered_at: now,
        by: None,
    };
    if sign_as == SignAs::Assistant {
        if let Some(ctl) = engine.assistant_signing() {
            answer.by = ctl.active_pubkey().await;
        }
    }
    r.answer(answer);
    store.save(&r)?;
    emit_request(engine, &r);
    Ok(r)
}

/// Announce a request-record change on the store stream.
fn emit_request(engine: &AppState, r: &AnnotationRequest) {
    engine.emit_store_event(crate::store_events::StoreEvent::Request {
        id: r.id.clone(),
        root: r.root.clone(),
        parent: r.parent.clone(),
        status: match r.status {
            RequestStatus::Open => "open".into(),
            RequestStatus::Answered => "answered".into(),
        },
    });
}

/// POST /api/v1/annotations/requests/:id/answer
pub async fn answer_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<AnswerRequest>,
) -> Result<Json<AnnotationRequest>, EngineError> {
    Ok(Json(answer_request(&engine, &id, req).await?))
}

/// POST /api/v1/annotations/requests/:id/reopen
pub async fn reopen_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<AnnotationRequest>, EngineError> {
    let store = RequestStore::new(engine.data_dir())?;
    let mut r = store.load(&id)?;
    r.status = RequestStatus::Open;
    r.answer = None;
    store.save(&r)?;
    emit_request(&engine, &r);
    Ok(Json(r))
}
