//! HTTP API handlers
//!
//! Provides REST endpoints for querying Nostr events.

use crate::engine::{Engine, FetchPolicy, QueryResponse};
use crate::error::EngineError;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::debug;

/// Shared application state
pub type AppState = Arc<Engine>;

/// Query request body
#[derive(Debug, Deserialize)]
pub struct QueryRequest {
    /// NIP-01 filters
    pub filters: Vec<Value>,
    /// Fetch policy (optional, defaults to local_first)
    #[serde(default)]
    pub policy: Option<String>,
    /// Override relays for this request (optional)
    pub relays: Option<Vec<String>>,
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// POST /api/v1/query
///
/// Query events with NIP-01 filters
pub async fn query_handler(
    State(engine): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, EngineError> {
    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    debug!(
        "Query request: {} filters, policy={:?}",
        req.filters.len(),
        policy
    );

    let response = engine
        .get_events(req.filters, policy, req.relays.as_deref())
        .await?;

    Ok(Json(response))
}

/// GET /api/v1/events/:id
///
/// Get a single event by its ID
pub async fn get_event_handler(
    State(engine): State<AppState>,
    Path(event_id): Path<String>,
) -> Result<impl IntoResponse, EngineError> {
    debug!("Get event request: {}", event_id);

    // Validate hex ID format
    if event_id.len() != 64 || hex::decode(&event_id).is_err() {
        return Err(EngineError::InvalidHex(
            "Event ID must be a 64-character hex string".to_string(),
        ));
    }

    let event = engine.get_by_id(&event_id, FetchPolicy::LocalFirst).await?;

    match event {
        Some(e) => Ok((StatusCode::OK, Json(json!({ "event": e })))),
        None => Ok((StatusCode::NOT_FOUND, Json(json!({ "event": null, "message": "Event not found" })))),
    }
}

/// Path parameters for addressable event endpoint
#[derive(Debug, Deserialize)]
pub struct AddressablePath {
    pub kind: u64,
    pub pubkey: String,
    pub d_tag: String,
}

/// GET /api/v1/addressable/:kind/:pubkey/:d_tag
///
/// Get an addressable event by kind, pubkey, and d-tag
pub async fn get_addressable_handler(
    State(engine): State<AppState>,
    Path(params): Path<AddressablePath>,
) -> Result<impl IntoResponse, EngineError> {
    debug!(
        "Get addressable request: {}:{}:{}",
        params.kind, params.pubkey, params.d_tag
    );

    // Validate hex pubkey format
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let event = engine
        .get_addressable(
            params.kind,
            &params.pubkey,
            &params.d_tag,
            FetchPolicy::LocalFirst,
        )
        .await?;

    match event {
        Some(e) => Ok((StatusCode::OK, Json(json!({ "event": e })))),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "event": null, "message": "Addressable event not found" })),
        )),
    }
}

/// GET /health
///
/// Health check endpoint
pub async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// ============================================================================
// Publication API Endpoints
// ============================================================================

use crate::publication::{NAddr, PublicationEngine, KIND_PUBLICATION_INDEX};

/// Query parameters for publications list
#[derive(Debug, Deserialize)]
pub struct PublicationsQuery {
    /// Maximum number of publications to return
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Fetch policy
    pub policy: Option<String>,
}

fn default_limit() -> usize {
    20
}

/// GET /api/v1/publications
///
/// List root publications (kind 30040 not referenced by other 30040s)
pub async fn list_publications_handler(
    State(engine): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<PublicationsQuery>,
) -> Result<impl IntoResponse, EngineError> {
    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    debug!("List publications request: limit={}, policy={:?}", query.limit, policy);

    let pub_engine = PublicationEngine::new(&engine);
    let publications = pub_engine.list_root_publications(policy, query.limit).await?;

    // Convert to summary format
    let summaries: Vec<Value> = publications
        .iter()
        .map(|p| {
            json!({
                "addr": p.addr,
                "title": p.title,
                "summary": p.summary,
                "image": p.image,
                "author_pubkey": p.author_pubkey,
                "version": p.version,
                "created_at": p.created_at,
                "section_count": p.sections.len()
            })
        })
        .collect();

    Ok(Json(json!({
        "publications": summaries,
        "count": summaries.len()
    })))
}

/// Path parameters for publication endpoint
#[derive(Debug, Deserialize)]
pub struct PublicationPath {
    pub pubkey: String,
    pub d_tag: String,
}

/// GET /api/v1/publications/:pubkey/:d_tag
///
/// Get a publication with its table of contents
pub async fn get_publication_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
) -> Result<impl IntoResponse, EngineError> {
    debug!("Get publication request: {}:{}", params.pubkey, params.d_tag);

    // Validate hex pubkey format
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let publication = pub_engine.load_publication(&addr, FetchPolicy::LocalFirst).await?;
    let toc = pub_engine.build_toc(&publication, 0);

    Ok((
        StatusCode::OK,
        Json(json!({
            "publication": {
                "addr": publication.addr,
                "title": publication.title,
                "summary": publication.summary,
                "image": publication.image,
                "author_pubkey": publication.author_pubkey,
                "version": publication.version,
                "created_at": publication.created_at,
                "index": publication.index
            },
            "toc": toc,
            "section_count": publication.sections.len()
        })),
    ))
}

/// POST /api/v1/publications/:pubkey/:d_tag/sections
///
/// Load all sections for a publication
pub async fn load_sections_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
) -> Result<impl IntoResponse, EngineError> {
    debug!("Load sections request: {}:{}", params.pubkey, params.d_tag);

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let mut publication = pub_engine.load_publication(&addr, FetchPolicy::LocalFirst).await?;
    let loaded_count = pub_engine.load_sections(&mut publication, FetchPolicy::LocalFirst).await?;

    // Build response with section content
    let sections: Vec<Value> = publication
        .sections
        .iter()
        .map(|s| {
            json!({
                "addr": s.addr,
                "title": s.title,
                "content": s.content,
                "position": s.position,
                "loaded": s.event.is_loaded()
            })
        })
        .collect();

    Ok(Json(json!({
        "sections": sections,
        "loaded_count": loaded_count,
        "total_count": publication.sections.len()
    })))
}

/// Path parameters for section endpoint
#[derive(Debug, Deserialize)]
pub struct SectionPath {
    pub pubkey: String,
    pub d_tag: String,
    pub index: usize,
}

/// GET /api/v1/publications/:pubkey/:d_tag/sections/:index
///
/// Load a single section by index
pub async fn get_section_handler(
    State(engine): State<AppState>,
    Path(params): Path<SectionPath>,
) -> Result<impl IntoResponse, EngineError> {
    debug!(
        "Get section request: {}:{} index={}",
        params.pubkey, params.d_tag, params.index
    );

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let mut publication = pub_engine.load_publication(&addr, FetchPolicy::LocalFirst).await?;
    pub_engine
        .load_section(&mut publication, params.index, FetchPolicy::LocalFirst)
        .await?;

    let section = publication.sections.get(params.index).ok_or_else(|| {
        EngineError::InvalidFilter("Section index out of bounds".into())
    })?;

    Ok(Json(json!({
        "section": {
            "addr": section.addr,
            "title": section.title,
            "content": section.content,
            "position": section.position,
            "loaded": section.event.is_loaded(),
            "event": section.event.data()
        }
    })))
}

/// Path parameters for section versions endpoint
#[derive(Debug, Deserialize)]
pub struct SectionVersionsPath {
    pub pubkey: String,
    pub d_tag: String,
}

/// GET /api/v1/sections/:pubkey/:d_tag/versions
///
/// Find alternate versions of a section (for forking UI)
pub async fn get_section_versions_handler(
    State(engine): State<AppState>,
    Path(params): Path<SectionVersionsPath>,
) -> Result<impl IntoResponse, EngineError> {
    debug!(
        "Get section versions request: {}:{}",
        params.pubkey, params.d_tag
    );

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(crate::publication::KIND_PUBLICATION_SECTION, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let versions = pub_engine
        .find_section_versions(&addr, FetchPolicy::FetchAlways)
        .await?;

    let version_summaries: Vec<Value> = versions
        .iter()
        .map(|v| {
            json!({
                "author": v.author,
                "created_at": v.created_at,
                "version": v.version,
                "content_preview": v.event.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.chars().take(200).collect::<String>())
            })
        })
        .collect();

    Ok(Json(json!({
        "versions": version_summaries,
        "count": versions.len()
    })))
}
