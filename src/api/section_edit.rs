//! Edit one section (30041) or atomic article (30023 / 30818 / 30817) in place — `POST /api/v1/section-edit/{preview,publish}`.
//!
//! The planning is pure (`publication::section_edit`); this half loads the
//! base section + its containing index from the store, resolves the signer,
//! and for the request route signs the fork, then the kind-818 naming it,
//! and sends both to the owner's destination set
//! (`merge_requests::destinations`: inbox ∪ found-on ∪ hints ∪ mine).

use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::merge_requests::{destinations, DestinationsRequest, DestinationsResponse};
use crate::api::AppState;
use crate::engine::FetchPolicy;
use crate::error::EngineError;
use crate::publication::event_util::coord_parts;
use crate::publication::merge_request::MergeHints;
use crate::publication::section_edit::{
    default_message, merge_request_for, plan, EditPlan, EditRoute, SectionEdit,
};
use crate::signing::EventTemplate;

#[derive(Debug, Deserialize)]
pub struct SectionEditRequest {
    /// The edited version: its 64-hex id (preferred — pins the base) or its
    /// `30041:pk:d` coordinate (the newest local version).
    pub section: String,
    /// The containing index the reader opened it from (`30040:pk:d`).
    #[serde(default)]
    pub index: Option<String>,
    pub content: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Declared format (`markdown` / `text/org` / …); default: the base's.
    #[serde(default)]
    pub format: Option<String>,
    /// The author's tags (the tags panel): replace the base's own tags;
    /// engine-set and reference tags are kept. Absent = keep the base's.
    #[serde(default)]
    pub tags: Option<Vec<crate::publication::compose::WireTag>>,
    /// Request route only: also publish the editor's copy of the index.
    #[serde(default)]
    pub with_derivative: bool,
    /// Request route only: the 818's content; default: a one-line summary.
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub broadcast: bool,
    /// Override the relay set (default: publish relays; on the request
    /// route ∪ the owner's destination set).
    #[serde(default)]
    pub relays: Option<Vec<String>>,
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct SectionEditPreview {
    #[serde(flatten)]
    pub plan: EditPlan,
    /// The newest local version of the section is not the one edited.
    pub moved_on: bool,
    /// Request route: the 818 as it will be signed (`source` is the fork's
    /// id once signed — shown as a placeholder here).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_request: Option<EventTemplate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destinations: Option<DestinationsResponse>,
    /// The containing index resolved and listing this section.
    pub index_coordinate: Option<String>,
    /// A fork of an atomic kind keeps the `d` (NIP-54): when the editor
    /// already has an event at that coordinate, its id — publishing
    /// replaces it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrites: Option<String>,
}

const SOURCE_PLACEHOLDER: &str = "<fork id — set when signed>";

async fn load_base(engine: &AppState, section: &str) -> Result<Value, EngineError> {
    let s = section.trim();
    let ev = if super::is_hex64(s) {
        engine.get_by_id(s, FetchPolicy::LocalOnly).await?
    } else if let Some((k, pk, d)) = coord_parts(s) {
        engine
            .get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly)
            .await?
    } else {
        return Err(EngineError::BadRequest(
            "section must be a 64-hex id or a 30041 coordinate".into(),
        ));
    };
    ev.ok_or_else(|| EngineError::NotFound(format!("section {s} is not local")))
}

async fn load_index(engine: &AppState, index: Option<&str>) -> Result<Option<Value>, EngineError> {
    match index.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(c) => match coord_parts(c) {
            Some((30040, pk, d)) => {
                engine
                    .get_addressable(30040, pk, d, FetchPolicy::LocalOnly)
                    .await
            }
            _ => Err(EngineError::BadRequest(format!(
                "index must be a 30040 coordinate, got `{c}`"
            ))),
        },
    }
}

fn relay_hint(engine: &AppState) -> String {
    engine.publish_relays().first().cloned().unwrap_or_default()
}

struct Prepared {
    plan: EditPlan,
    moved_on: bool,
    index_coordinate: Option<String>,
}

async fn prepare(
    engine: &AppState,
    req: &SectionEditRequest,
    editor: &str,
) -> Result<Prepared, EngineError> {
    let base = load_base(engine, &req.section).await?;
    let index = load_index(engine, req.index.as_deref()).await?;
    let edit = SectionEdit {
        content: req.content.clone(),
        title: req.title.clone(),
        format: req.format.clone(),
        tags: req.tags.as_ref().map(|ts| {
            let entries: Vec<crate::publication::compose::TagEntry> = ts.iter().map(Into::into).collect();
            crate::publication::compose::ComposeState::tags_to_nostr_format(&entries)
        }),
    };
    let now = super::unix_now_i64();
    let p = plan(
        &base,
        index.as_ref(),
        editor,
        &edit,
        &relay_hint(engine),
        req.with_derivative,
        now,
    )
    .map_err(EngineError::BadRequest)?;
    let moved_on = match coord_parts(&p.base_coordinate) {
        Some((k, pk, d)) => engine
            .get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly)
            .await?
            .and_then(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
            .is_some_and(|newest| newest != p.base_id),
        None => false,
    };
    let index_coordinate = index
        .as_ref()
        .and_then(crate::publication::event_util::event_coordinate)
        .filter(|c| {
            crate::publication::event_util::index_children(index.as_ref().unwrap())
                .contains(&p.base_coordinate)
                && !c.is_empty()
        });
    Ok(Prepared {
        plan: p,
        moved_on,
        index_coordinate,
    })
}

async fn editor_pubkey(
    engine: &AppState,
    signing: &crate::signing::SigningController,
) -> Result<String, EngineError> {
    signing
        .active_pubkey()
        .await
        .or_else(|| engine.my_pubkey())
        .ok_or_else(|| EngineError::Auth("no identity — log in to edit".into()))
}

/// POST /api/v1/section-edit/preview — the route, the events as they will
/// be signed, the diff, and (request route) the 818 + destination set.
pub async fn preview_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<SectionEditRequest>,
) -> Result<Json<SectionEditPreview>, EngineError> {
    let editor = editor_pubkey(&engine, &signing).await?;
    let Prepared {
        plan,
        moved_on,
        index_coordinate,
    } = prepare(&engine, &req, &editor).await?;
    let (merge_request, dests) = if plan.route == EditRoute::ForkAndRequest {
        let mut t = merge_request_for(
            &plan,
            &"0".repeat(64),
            req.message.as_deref().unwrap_or(""),
            &MergeHints::default(),
            super::unix_now_i64(),
        )
        .map_err(EngineError::BadRequest)?;
        for row in t.tags.iter_mut() {
            if row.get(3).map(String::as_str) == Some("source") {
                row[1] = SOURCE_PLACEHOLDER.into();
            }
        }
        let d = destinations(
            &engine,
            &DestinationsRequest {
                target: plan.base_coordinate.clone(),
                owner: Some(plan.owner.clone()),
                policy: Some("local_only".into()),
                mode_confirm: false,
            },
        )
        .await?;
        (Some(t), Some(d))
    } else {
        (None, None)
    };
    let overwrites = match (plan.route, coord_parts(&plan.section_coordinate)) {
        (EditRoute::Republish, _) | (_, None) => None,
        (_, Some((k, pk, d))) => engine
            .get_addressable(k as u64, pk, d, FetchPolicy::LocalOnly)
            .await?
            .and_then(|e| e.get("id").and_then(Value::as_str).map(str::to_string)),
    };
    Ok(Json(SectionEditPreview {
        plan,
        moved_on,
        merge_request,
        destinations: dests,
        index_coordinate,
        overwrites,
    }))
}

#[derive(Debug, Serialize)]
pub struct SectionEditPublished {
    pub route: EditRoute,
    pub section: Value,
    pub section_coordinate: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_request: Option<Value>,
    pub relays_used: Vec<String>,
    #[serde(flatten)]
    pub publish: crate::api::PublishResponse,
}

/// POST /api/v1/section-edit/publish — sign as the user and store; broadcast
/// when asked. Own section → same `d`; foreign in my index → fork + my index;
/// otherwise fork + kind-818 to the owner.
pub async fn publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<SectionEditRequest>,
) -> Result<Json<SectionEditPublished>, EngineError> {
    let (editor, signer) =
        super::resolve_publish_signer(&engine, &signing, crate::api::SignAs::User).await?;
    let Prepared { plan, .. } = prepare(&engine, &req, &editor).await?;
    if plan.unchanged {
        return Err(EngineError::BadRequest(
            "nothing changed — the section is as published".into(),
        ));
    }
    let section = signer
        .sign(plan.section.clone())
        .await
        .map_err(super::map_publish_sign_error)?;
    let index = match plan.index.clone() {
        Some(t) => Some(
            signer
                .sign(t)
                .await
                .map_err(super::map_publish_sign_error)?,
        ),
        None => None,
    };
    let mut relays: Vec<String> = req
        .relays
        .clone()
        .unwrap_or_else(|| engine.publish_relays().to_vec());
    let mut extra: Vec<Value> = Vec::new();
    let mut merge_request = None;
    if plan.route == EditRoute::ForkAndRequest {
        let d = destinations(
            &engine,
            &DestinationsRequest {
                target: plan.base_coordinate.clone(),
                owner: Some(plan.owner.clone()),
                policy: Some(
                    if req.broadcast {
                        "local_first"
                    } else {
                        "local_only"
                    }
                    .into(),
                ),
                mode_confirm: req.mode_confirm,
            },
        )
        .await?;
        if req.relays.is_none() {
            for dst in &d.destinations {
                if !relays.contains(&dst.url) {
                    relays.push(dst.url.clone());
                }
            }
        }
        let hints = MergeHints {
            target: engine
                .provenance_for(&plan.base_coordinate)
                .first()
                .cloned()
                .unwrap_or_default(),
            source: relays.first().cloned().unwrap_or_default(),
        };
        let fork_id = section
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let message = req
            .message
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| default_message(&plan));
        let mut t = merge_request_for(&plan, &fork_id, &message, &hints, super::unix_now_i64())
            .map_err(EngineError::BadRequest)?;
        t.pubkey = Some(editor.clone());
        let mr = signer
            .sign(t)
            .await
            .map_err(super::map_publish_sign_error)?;
        extra.push(mr.clone());
        merge_request = Some(mr);
    }
    let mut events = vec![section.clone()];
    events.extend(extra);
    let publish = super::finalize_publish(
        &engine,
        index.clone(),
        events,
        req.broadcast,
        Some(relays.clone()),
        true,
    )
    .await?
    .0;
    Ok(Json(SectionEditPublished {
        route: plan.route,
        section_coordinate: plan.section_coordinate,
        section,
        index,
        merge_request,
        relays_used: if req.broadcast { relays } else { Vec::new() },
        publish,
    }))
}
