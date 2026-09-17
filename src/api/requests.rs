//! Annotation requests over HTTP — `/api/v1/annotations/requests*`.
//!
//! Create (local record; optionally signed as a kind-1111 mention of the
//! assistant, optionally broadcast), list, delete, answer. The answer path
//! is what an agent calls through the `answer_request` tool; the web uses it
//! to mark a request done by hand.

use super::{AppState, SignAs};
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::insertion::coord_parts;
use crate::requests::{
    build_answer_template, AnnotationRequest, RequestAnswer, RequestStatus, RequestStore,
};
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    pub task: String,
    #[serde(default)]
    pub scope: Vec<String>,
    #[serde(default)]
    pub context: Option<String>,
    /// `notes` (default) or `kasten`.
    #[serde(default)]
    pub output: Option<String>,
    /// Coordinate of a style event (`30078:pk:d`) the answer should follow.
    #[serde(default)]
    pub style: Option<String>,
    /// Also sign it as a kind-1111 mention of the assistant (needs your
    /// identity and an assistant identity). Default false — local record.
    #[serde(default)]
    pub sign: bool,
    /// With `sign`: also send the mention to the publish relays. Default
    /// false — the mention stays in the local store.
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct RequestResponse {
    #[serde(flatten)]
    pub request: AnnotationRequest,
    /// The signed mention, when `sign` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Value>,
}

async fn assistant_pubkey(engine: &AppState) -> Option<String> {
    match engine.assistant_signing() {
        Some(ctl) => ctl.active_pubkey().await,
        None => None,
    }
}

/// POST /api/v1/annotations/requests
pub async fn create_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<CreateRequest>,
) -> Result<Json<RequestResponse>, EngineError> {
    let now = super::unix_now_i64();
    let mut r = AnnotationRequest::new(&req.root, req.parent.as_deref(), &req.task, now)
        .map_err(EngineError::BadRequest)?
        .with_scope(&req.scope);
    r.parent_title = req.parent_title.filter(|t| !t.trim().is_empty());
    r.title = req.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    r.context = req.context.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    if let Some(o) = req.output.as_deref().map(str::trim) {
        if o == "kasten" || o == "notes" {
            r.output = o.to_string();
        } else if !o.is_empty() {
            return Err(EngineError::BadRequest("output must be notes or kasten".into()));
        }
    }
    // Pin the parent version from the local store when the caller did not.
    r.parent_event_id = req.parent_event_id.filter(|e| super::is_hex64(e));
    if r.parent_event_id.is_none() {
        if let Some((k, pk, d)) = coord_parts(&r.parent) {
            if let Ok(Some(ev)) = engine.get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly).await {
                r.parent_event_id = ev.get("id").and_then(Value::as_str).map(str::to_string);
            }
        }
    }
    r.assistant_pubkey = assistant_pubkey(&engine).await;
    if let Some(coord) = req.style.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let style = super::styles::get_style(&engine, coord)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("style {coord} is not in the local store")))?;
        r.style = Some(style.coordinate);
        r.style_title = Some(style.title);
    }

    let mut event = None;
    if req.sign {
        let asst = r.assistant_pubkey.clone().ok_or_else(|| {
            EngineError::BadRequest(
                "signing a request mentions the assistant — set an assistant identity first, or leave it unsigned".into(),
            )
        })?;
        let relay_hint = super::discussion_relay_hint(&engine);
        let template = r
            .to_template(&asst, &relay_hint, now)
            .map_err(EngineError::BadRequest)?;
        // Local by default: an empty relay override means "no broadcast".
        let relays = if req.broadcast {
            req.relays.clone()
        } else {
            Some(Vec::new())
        };
        let resp = super::sign_ingest_broadcast(&engine, &signing, template, relays).await?;
        r.event_id = resp
            .event
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        event = Some(resp.event);
    }

    RequestStore::new(engine.data_dir())?.save(&r)?;
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

/// Shared by the endpoint and the `list_requests` tool.
pub(crate) fn list_requests(
    engine: &AppState,
    root: Option<&str>,
    parent: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<AnnotationRequest>, EngineError> {
    let status = parse_status(status)?;
    RequestStore::new(engine.data_dir())?.query(root, parent, status)
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
    Ok(Json(RequestStore::new(engine.data_dir())?.load(&id)?))
}

/// DELETE /api/v1/annotations/requests/:id — the local record only; a signed
/// mention stays an event (NIP-09 it from its menu if you want it gone).
pub async fn delete_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, EngineError> {
    RequestStore::new(engine.data_dir())?.delete(&id)?;
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub summary: String,
    #[serde(default)]
    pub coordinates: Vec<String>,
    /// When the request was a signed mention, also reply to it as the
    /// assistant (kind 1111, local). Default true.
    #[serde(default = "default_true")]
    pub sign_reply: bool,
    /// Who is answering. `assistant` (default) signs the reply with the
    /// assistant's key; `user` marks it answered by hand, no reply event.
    #[serde(default)]
    pub sign_as: Option<SignAs>,
}

fn default_true() -> bool {
    true
}

/// Shared by the endpoint and the `answer_request` tool.
pub(crate) async fn answer_request(
    engine: &AppState,
    id: &str,
    req: AnswerRequest,
) -> Result<AnnotationRequest, EngineError> {
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
        reply_event_id: None,
        answered_at: now,
        by: None,
    };
    if sign_as == SignAs::Assistant {
        if let Some(ctl) = engine.assistant_signing() {
            answer.by = ctl.active_pubkey().await;
            // Close the thread on the wire too when the request was signed.
            if req.sign_reply {
                if let Some(req_id) = r.event_id.as_deref() {
                    if let Some(request_event) = crate::query::query_by_id(engine.ndb(), req_id)? {
                        if let Some(pubkey) = answer.by.clone() {
                            let relay_hint = super::discussion_relay_hint(engine);
                            let mut template = build_answer_template(
                                &request_event,
                                &answer.summary,
                                &answer.coordinates,
                                &relay_hint,
                                now,
                            )
                            .map_err(EngineError::BadRequest)?;
                            template.pubkey = Some(pubkey);
                            let signed = ctl
                                .sign(template)
                                .await
                                .map_err(super::map_assistant_sign_error)?;
                            let resp = super::finalize_publish(
                                engine,
                                None,
                                vec![signed.clone()],
                                false,
                                None,
                                true,
                            )
                            .await?;
                            let _ = resp;
                            answer.reply_event_id = signed
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                    }
                }
            }
        }
    }
    r.answer(answer);
    store.save(&r)?;
    Ok(r)
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
    Ok(Json(r))
}
