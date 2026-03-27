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
// Search API Endpoint
// ============================================================================

use crate::search::{AuthorFilter, SearchQuery, SearchResponse};

/// Search request body
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    /// Search query string (e.g. "t:python k:30041 tutorial")
    pub query: String,
    /// Maximum number of results (optional)
    pub limit: Option<usize>,
    /// Fetch policy (optional, defaults to local_first)
    pub policy: Option<String>,
    /// Override relays for this request (optional)
    pub relays: Option<Vec<String>>,
    /// Current user's pubkey hex (required for by:me queries)
    pub my_pubkey: Option<String>,
}

/// POST /api/v1/search
///
/// Search for events using the structured search query language
pub async fn search_handler(
    State(engine): State<AppState>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, EngineError> {
    debug!("Search request: query={:?}", req.query);

    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    // Check for compound query (contains |)
    if req.query.contains('|') {
        let compound = SearchQuery::parse_compound(&req.query)
            .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        let mut total_local = 0;
        let mut total_relay = 0;

        for mut branch in compound.branches {
            if let Some(limit) = req.limit {
                branch.limit = Some(limit);
            }
            resolve_author(&mut branch, &req, &engine)?;

            let resp = engine
                .search(&branch, policy, req.relays.as_deref())
                .await?;

            total_local += resp.local_count;
            total_relay += resp.relay_count;

            for result in resp.results {
                if seen_ids.insert(result.event_id.clone()) {
                    all_results.push(result);
                }
            }
        }

        let count = all_results.len();
        return Ok(Json(SearchResponse {
            results: all_results,
            count,
            local_count: total_local,
            relay_count: total_relay,
            doc_results: vec![],
        }));
    }

    // Single query path
    let mut query = SearchQuery::parse(&req.query)
        .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

    if let Some(limit) = req.limit {
        query.limit = Some(limit);
    }

    resolve_author(&mut query, &req, &engine)?;

    let response = engine
        .search(&query, policy, req.relays.as_deref())
        .await?;

    Ok(Json(response))
}

/// Resolve by:me in a query to actual pubkey
fn resolve_author(
    query: &mut SearchQuery,
    req: &SearchRequest,
    engine: &AppState,
) -> Result<(), EngineError> {
    if let Some(AuthorFilter::CurrentUser) = &query.author_filter {
        let pk = req.my_pubkey.as_deref().or_else(|| engine.my_pubkey());
        if let Some(pk) = pk {
            query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk.to_string()]));
        } else {
            return Err(EngineError::InvalidFilter(
                "by:me requires pubkey in config or request".to_string(),
            ));
        }
    }
    Ok(())
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
    /// Cursor: only return publications created before this timestamp
    pub before: Option<u64>,
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

    debug!("List publications request: limit={}, policy={:?}, before={:?}", query.limit, policy, query.before);

    let pub_engine = PublicationEngine::new(&engine);
    let publications = pub_engine.list_root_publications(policy, query.limit, query.before).await?;

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
                "section_count": p.section_count()
            })
        })
        .collect();

    Ok(Json(json!({
        "publications": summaries,
        "count": summaries.len()
    })))
}

/// Query parameters for policy override (shared by multiple handlers)
#[derive(Debug, Deserialize)]
pub struct PolicyQuery {
    pub policy: Option<String>,
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
    axum::extract::Query(query): axum::extract::Query<PolicyQuery>,
) -> Result<impl IntoResponse, EngineError> {
    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    debug!("Get publication request: {}:{} policy={:?}", params.pubkey, params.d_tag, policy);

    // Validate hex pubkey format
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let publication = pub_engine.load_publication(&addr, policy).await?;
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
    axum::extract::Query(query): axum::extract::Query<PolicyQuery>,
) -> Result<impl IntoResponse, EngineError> {
    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    debug!(
        "Get section request: {}:{} index={} policy={:?}",
        params.pubkey, params.d_tag, params.index, policy
    );

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let mut publication = pub_engine.load_publication(&addr, policy).await?;
    pub_engine
        .load_section(&mut publication, params.index, policy)
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

/// POST /api/v1/publications/:pubkey/:d_tag/sections/metadata
///
/// Lightweight metadata-only view of sections (no content).
/// Defaults to LocalOnly for instant response; accepts ?policy= override.
pub async fn load_sections_metadata_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
    axum::extract::Query(query): axum::extract::Query<PolicyQuery>,
) -> Result<impl IntoResponse, EngineError> {
    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::LocalOnly,
    };

    debug!(
        "Load sections metadata request: {}:{} policy={:?}",
        params.pubkey, params.d_tag, policy
    );

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let mut publication = pub_engine.load_publication(&addr, FetchPolicy::LocalOnly).await?;
    pub_engine
        .load_sections(&mut publication, policy)
        .await?;

    let sections_meta: Vec<Value> = publication
        .sections
        .iter()
        .map(|s| {
            json!({
                "addr": s.addr,
                "title": s.title,
                "position": s.position,
                "loaded": s.event.is_loaded()
            })
        })
        .collect();

    let total_count = sections_meta.len();

    Ok(Json(json!({
        "sections_meta": sections_meta,
        "total_count": total_count
    })))
}

// ============================================================================
// Document Import API Endpoints
// ============================================================================

/// GET /api/v1/documents — list files in the documents folder
pub async fn list_documents_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let docs_dir = engine.documents_path();

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&docs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() { continue; }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            let supported = ["pdf", "docx", "epub", "html", "htm", "txt", "md", "org", "adoc", "asciidoc", "rst"];
            if !supported.contains(&ext.as_str()) { continue; }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let modified = entry.metadata().ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0u64);
            files.push(json!({
                "name": name,
                "format": ext,
                "size": size,
                "modified": modified,
            }));
        }
    }

    files.sort_by(|a, b| {
        b.get("modified").and_then(|v| v.as_u64())
            .cmp(&a.get("modified").and_then(|v| v.as_u64()))
    });

    Ok(Json(json!({
        "path": docs_dir.to_string_lossy(),
        "files": files,
        "count": files.len(),
    })))
}

/// POST /api/v1/documents/parse — parse a file from the documents folder
#[derive(Debug, Deserialize)]
pub struct ParseDocRequest {
    pub filename: String,
}

pub async fn parse_document_handler(
    State(engine): State<AppState>,
    Json(req): Json<ParseDocRequest>,
) -> Result<Json<Value>, EngineError> {
    let file_path = engine.documents_path().join(&req.filename);
    if !file_path.exists() {
        return Err(EngineError::InvalidFilter(format!("File not found: {}", req.filename)));
    }

    let file_bytes = std::fs::read(&file_path)
        .map_err(|e| EngineError::Database(format!("Failed to read file: {e}")))?;

    let sidecar = engine.sidecar_url();

    // Send to sidecar /parse as multipart
    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(req.filename.clone())
        .mime_str("application/octet-stream")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);

    let resp: Value = reqwest::Client::new()
        .post(format!("{sidecar}/parse"))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| EngineError::Database(format!("Sidecar parse failed: {e}")))?
        .json()
        .await
        .map_err(|e| EngineError::Database(format!("Invalid parse response: {e}")))?;

    Ok(Json(resp))
}

/// POST /api/v1/import — upload file, save to docs folder, parse
pub async fn import_document_handler(
    State(engine): State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Result<Json<Value>, EngineError> {
    let docs_dir = engine.documents_path();
    std::fs::create_dir_all(&docs_dir)
        .map_err(|e| EngineError::Database(format!("Failed to create docs dir: {e}")))?;

    // Read the uploaded file
    let mut filename = String::new();
    let mut file_bytes = Vec::new();

    while let Some(field) = multipart.next_field().await
        .map_err(|e| EngineError::Database(format!("Multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            filename = field.file_name().unwrap_or("upload").to_string();
            file_bytes = field.bytes().await
                .map_err(|e| EngineError::Database(format!("Read error: {e}")))?
                .to_vec();
        }
    }

    if file_bytes.is_empty() {
        return Err(EngineError::InvalidFilter("No file uploaded".into()));
    }

    // Save to docs folder
    let dest = docs_dir.join(&filename);
    std::fs::write(&dest, &file_bytes)
        .map_err(|e| EngineError::Database(format!("Failed to save file: {e}")))?;

    // Parse via sidecar
    let sidecar = engine.sidecar_url();
    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(filename.clone())
        .mime_str("application/octet-stream")
        .unwrap();
    let form = reqwest::multipart::Form::new().part("file", part);

    let resp: Value = reqwest::Client::new()
        .post(format!("{sidecar}/parse"))
        .multipart(form)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| EngineError::Database(format!("Sidecar parse failed: {e}")))?
        .json()
        .await
        .map_err(|e| EngineError::Database(format!("Invalid parse response: {e}")))?;

    Ok(Json(resp))
}

// ============================================================================
// Profile API Endpoint
// ============================================================================

fn profile_from_event(pubkey: &str, event: &Value) -> Value {
    let content = event.get("content").and_then(|v| v.as_str()).unwrap_or("{}");
    let profile: Value = serde_json::from_str(content).unwrap_or(json!({}));
    json!({
        "pubkey": pubkey,
        "name": profile.get("name").and_then(|v| v.as_str()),
        "display_name": profile.get("display_name").and_then(|v| v.as_str()),
        "picture": profile.get("picture").and_then(|v| v.as_str()),
        "about": profile.get("about").and_then(|v| v.as_str()),
        "nip05": profile.get("nip05").and_then(|v| v.as_str()),
        "found": true
    })
}

/// Query kind 0 profile by pubkey (no d-tag — kind 0 is regular replaceable, not parameterized)
fn query_profile(ndb: &nostrdb::Ndb, pubkey: &str) -> Option<Value> {
    let pubkey_bytes = crate::query::parse_hex_pubkey(pubkey).ok()?;
    let txn = nostrdb::Transaction::new(ndb).ok()?;
    let filter = nostrdb::FilterBuilder::new()
        .kinds([0])
        .authors([pubkey_bytes].iter())
        .limit(1)
        .build();
    let results = ndb.query(&txn, &[filter], 1).ok()?;
    let qr = results.first()?;
    let note = ndb.get_note_by_key(&txn, qr.note_key).ok()?;
    crate::query::note_to_json_pub(&note).ok()
}

/// GET /api/v1/profile/:pubkey — get kind 0 profile (local only, instant)
pub async fn profile_handler(
    State(engine): State<AppState>,
    Path(pubkey): Path<String>,
) -> Result<impl IntoResponse, EngineError> {
    if pubkey.len() != 64 || hex::decode(&pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    if let Some(event) = query_profile(engine.ndb(), &pubkey) {
        return Ok(Json(profile_from_event(&pubkey, &event)));
    }

    // Local-only for individual lookups (fast, non-blocking).
    // Use POST /api/v1/profiles/fetch for relay fetching.
    Ok(Json(json!({ "pubkey": pubkey, "found": false })))
}

/// POST /api/v1/profiles/fetch — batch-fetch profiles from general relays
#[derive(Debug, Deserialize)]
pub struct FetchProfilesRequest {
    pub pubkeys: Vec<String>,
}

pub async fn fetch_profiles_handler(
    State(engine): State<AppState>,
    Json(req): Json<FetchProfilesRequest>,
) -> Result<Json<Value>, EngineError> {
    let relays = &engine.relay_config().general.urls;
    let mut fetched = 0;

    // Fetch kind 0 for each pubkey not already in nostrdb
    for pubkey in &req.pubkeys {
        if pubkey.len() != 64 { continue; }
        // Skip if already cached locally
        if query_profile(engine.ndb(), pubkey).is_some() {
            continue;
        }
        // Fetch from general relays (kind 0 has no d-tag, use filter directly)
        let filter = json!({"kinds": [0], "authors": [pubkey], "limit": 1});
        for relay_url in relays {
            match crate::relay::fetch_with_filters(engine.ndb(), relay_url, &[filter.clone()]).await {
                Ok(events) if !events.is_empty() => {
                    fetched += 1;
                    break;
                }
                _ => continue,
            }
        }
    }

    Ok(Json(json!({
        "fetched": fetched,
        "total": req.pubkeys.len()
    })))
}

// ============================================================================
// Relay Config API Endpoints
// ============================================================================

/// Request to fetch from a specific relay
#[derive(Debug, Deserialize)]
pub struct FetchRelayRequest {
    pub relay: String,
    #[serde(default)]
    pub kinds: Vec<u64>,
    /// Pubkeys to fetch from (hex). Empty = no author filter.
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default = "default_fetch_limit")]
    pub limit: usize,
}

fn default_fetch_limit() -> usize { 200 }

/// POST /api/v1/fetch — fetch events from a specific relay
pub async fn fetch_relay_handler(
    State(engine): State<AppState>,
    Json(req): Json<FetchRelayRequest>,
) -> Result<Json<Value>, EngineError> {
    debug!("Fetch from relay: {} kinds={:?} authors={} limit={}", req.relay, req.kinds, req.authors.len(), req.limit);

    let mut filter = json!({"limit": req.limit});
    if !req.kinds.is_empty() {
        filter["kinds"] = json!(req.kinds);
    }
    if !req.authors.is_empty() {
        filter["authors"] = json!(req.authors);
    }

    let events = crate::relay::fetch_with_filters(
        engine.ndb(),
        &req.relay,
        &[filter],
    )
    .await?;

    let count = events.len();
    debug!("Fetched {} events from {}", count, req.relay);

    Ok(Json(json!({
        "fetched": count,
        "relay": req.relay,
        "kinds": req.kinds
    })))
}

/// POST /api/v1/fetch/authors — fetch from all fetch relays for configured authors
pub async fn fetch_authors_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let rc = engine.relay_config();
    let authors = rc.authors_hex();

    if authors.is_empty() {
        return Ok(Json(json!({
            "message": "No authors configured in [relay] authors list",
            "fetched": 0
        })));
    }

    let kinds = &rc.fetch.kinds;
    let mut total_fetched = 0;

    for relay_url in &rc.fetch.urls {
        let mut filter = json!({"limit": 200, "authors": authors});
        if !kinds.is_empty() {
            filter["kinds"] = json!(kinds);
        }

        match crate::relay::fetch_with_filters(engine.ndb(), relay_url, &[filter]).await {
            Ok(events) => {
                debug!("Fetched {} events for authors from {}", events.len(), relay_url);
                total_fetched += events.len();
            }
            Err(e) => {
                debug!("Failed to fetch authors from {}: {}", relay_url, e);
            }
        }
    }

    Ok(Json(json!({
        "fetched": total_fetched,
        "authors": authors.len(),
        "relays": rc.fetch.urls.len()
    })))
}

/// Request to add relay/author to config
#[derive(Debug, Deserialize)]
pub struct ConfigUpdateRequest {
    /// Add a relay URL to a set ("general", "publish", "fetch")
    pub add_relay: Option<AddRelay>,
    /// Add an author (npub or hex)
    pub add_author: Option<String>,
    /// Remove an author
    pub remove_author: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AddRelay {
    pub set: String,
    pub url: String,
}

/// POST /api/v1/config/update — update config.toml from UI
pub async fn config_update_handler(
    State(engine): State<AppState>,
    Json(req): Json<ConfigUpdateRequest>,
) -> Result<Json<Value>, EngineError> {
    let config_path = engine.config_path().ok_or_else(|| {
        EngineError::Config("No config file path set (use -c config.toml)".into())
    })?;

    // Read current config
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| EngineError::Config(format!("Failed to read config: {e}")))?;
    let mut doc: toml::Table = toml::from_str(&content)
        .map_err(|e| EngineError::Config(format!("Failed to parse config: {e}")))?;

    let mut changed = false;

    // Add relay
    if let Some(add) = &req.add_relay {
        let relay = doc.entry("relay").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(relay_table) = relay {
            let set = relay_table.entry(&add.set).or_insert_with(|| {
                let mut t = toml::Table::new();
                t.insert("urls".into(), toml::Value::Array(Vec::new()));
                toml::Value::Table(t)
            });
            if let toml::Value::Table(set_table) = set {
                let urls = set_table.entry("urls").or_insert_with(|| toml::Value::Array(Vec::new()));
                if let toml::Value::Array(arr) = urls {
                    let url_val = toml::Value::String(add.url.clone());
                    if !arr.contains(&url_val) {
                        arr.push(url_val);
                        changed = true;
                    }
                }
            }
        }
    }

    // Add author
    if let Some(author) = &req.add_author {
        let relay = doc.entry("relay").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(relay_table) = relay {
            let authors = relay_table.entry("authors").or_insert_with(|| toml::Value::Array(Vec::new()));
            if let toml::Value::Array(arr) = authors {
                let val = toml::Value::String(author.clone());
                if !arr.contains(&val) {
                    arr.push(val);
                    changed = true;
                }
            }
        }
    }

    // Remove author
    if let Some(author) = &req.remove_author {
        if let Some(toml::Value::Table(relay_table)) = doc.get_mut("relay") {
            if let Some(toml::Value::Array(arr)) = relay_table.get_mut("authors") {
                let before = arr.len();
                arr.retain(|v| v.as_str() != Some(author));
                if arr.len() != before { changed = true; }
            }
        }
    }

    if changed {
        let output = toml::to_string_pretty(&doc)
            .map_err(|e| EngineError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(config_path, &output)
            .map_err(|e| EngineError::Config(format!("Failed to write config: {e}")))?;
    }

    Ok(Json(json!({
        "updated": changed,
        "message": if changed { "Config updated. Restart to apply relay changes." } else { "No changes needed." }
    })))
}

/// GET /api/v1/relays — get relay configuration
pub async fn relay_config_handler(
    State(engine): State<AppState>,
) -> Json<Value> {
    let rc = engine.relay_config();
    Json(json!({
        "general": { "urls": rc.general.urls, "kinds": rc.general.kinds },
        "publish": { "urls": rc.publish.urls, "kinds": rc.publish.kinds },
        "fetch": { "urls": rc.fetch.urls, "kinds": rc.fetch.kinds },
        "authors": rc.authors_hex(),
    }))
}

// ============================================================================
// Ignore List API Endpoints
// ============================================================================

/// Response for ignore list operations
#[derive(Debug, Serialize)]
pub struct IgnoreListResponse {
    pub ignored_event_count: usize,
    pub ignored_pubkey_count: usize,
    pub event_ids: Vec<String>,
    pub pubkeys: Vec<String>,
}

/// GET /api/v1/ignore — get current ignore list
pub async fn ignore_list_handler(
    State(engine): State<AppState>,
) -> Result<Json<IgnoreListResponse>, EngineError> {
    let list = engine.ignore_list().read().await;
    Ok(Json(IgnoreListResponse {
        ignored_event_count: list.event_ids.len(),
        ignored_pubkey_count: list.pubkeys.len(),
        event_ids: list.event_ids.iter().cloned().collect(),
        pubkeys: list.pubkeys.iter().cloned().collect(),
    }))
}

/// Request to ignore/unignore
#[derive(Debug, Deserialize)]
pub struct IgnoreRequest {
    /// Event IDs to add to ignore list
    #[serde(default)]
    pub event_ids: Vec<String>,
    /// Pubkeys to add to ignore list
    #[serde(default)]
    pub pubkeys: Vec<String>,
}

/// POST /api/v1/ignore — add events/pubkeys to ignore list
pub async fn ignore_add_handler(
    State(engine): State<AppState>,
    Json(req): Json<IgnoreRequest>,
) -> Result<Json<IgnoreListResponse>, EngineError> {
    for id in &req.event_ids {
        engine.ignore_event(id).await?;
    }
    for pk in &req.pubkeys {
        engine.ignore_pubkey(pk).await?;
    }
    let list = engine.ignore_list().read().await;
    Ok(Json(IgnoreListResponse {
        ignored_event_count: list.event_ids.len(),
        ignored_pubkey_count: list.pubkeys.len(),
        event_ids: list.event_ids.iter().cloned().collect(),
        pubkeys: list.pubkeys.iter().cloned().collect(),
    }))
}

/// DELETE /api/v1/ignore — remove events/pubkeys from ignore list
pub async fn ignore_remove_handler(
    State(engine): State<AppState>,
    Json(req): Json<IgnoreRequest>,
) -> Result<Json<IgnoreListResponse>, EngineError> {
    for id in &req.event_ids {
        engine.unignore_event(id).await?;
    }
    for pk in &req.pubkeys {
        engine.unignore_pubkey(pk).await?;
    }
    let list = engine.ignore_list().read().await;
    Ok(Json(IgnoreListResponse {
        ignored_event_count: list.event_ids.len(),
        ignored_pubkey_count: list.pubkeys.len(),
        event_ids: list.event_ids.iter().cloned().collect(),
        pubkeys: list.pubkeys.iter().cloned().collect(),
    }))
}

// ============================================================================
// Purge API Endpoint
// ============================================================================

/// POST /api/v1/purge — delete nostrdb and restart fresh
pub async fn purge_handler(
    State(engine): State<AppState>,
) -> Result<Json<serde_json::Value>, EngineError> {
    let data_dir = engine.data_dir().to_path_buf();
    // Can't actually delete while nostrdb is open — return the path for manual purge
    Ok(Json(json!({
        "message": "Purge requires restart. Delete the data directory and restart the engine.",
        "data_dir": data_dir.to_string_lossy(),
        "command": format!("rm -rf {} && cargo run -- -c config.toml", data_dir.display())
    })))
}

// ============================================================================
// Publish API Endpoints
// ============================================================================

use crate::publication::{build_publication_events, build_signed_publication_events};
use crate::tree::state::{ComposeState, SectionCompose};

/// Request to publish/draft a publication
#[derive(Debug, Deserialize)]
pub struct PublishRequest {
    pub title: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    pub sections: Vec<PublishSectionRequest>,
    /// Whether to sign the events (requires secret key in engine)
    #[serde(default)]
    pub sign: bool,
    /// Whether to broadcast to relays after creating
    #[serde(default)]
    pub broadcast: bool,
    /// Specific relays to broadcast to (defaults to configured relays)
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct PublishSectionRequest {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
}

/// Response from publish endpoint
#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub publication_id: String,
    pub section_ids: Vec<String>,
    pub signed: bool,
    pub ingested: bool,
    pub broadcast_results: Option<Vec<BroadcastResult>>,
}

#[derive(Debug, Serialize)]
pub struct BroadcastResult {
    pub relay: String,
    pub success: bool,
    pub message: Option<String>,
}

/// POST /api/v1/publish — create a publication (draft or signed)
pub async fn publish_handler(
    State(engine): State<AppState>,
    Json(req): Json<PublishRequest>,
) -> Result<impl IntoResponse, EngineError> {
    let pubkey = engine.my_pubkey().ok_or_else(|| {
        EngineError::Config("Publishing requires [identity] pubkey in config".into())
    })?;
    let pubkey = pubkey.to_string();

    // Map request to ComposeState
    use crate::tree::state::TagEntry;
    let mut compose = ComposeState::new();
    compose.title = req.title;
    for (name, value) in &req.tags {
        compose.tags.push(TagEntry { name: name.clone(), value: value.clone() });
    }
    compose.sections = req
        .sections
        .iter()
        .map(|s| {
            let mut sc = SectionCompose::default();
            sc.title = s.title.clone();
            sc.content = s.content.clone();
            sc.tags = s.tags.iter().map(|(n, v)| TagEntry { name: n.clone(), value: v.clone() }).collect();
            sc
        })
        .collect();

    // Build events (signed or unsigned)
    let (pub_event, section_events) = if req.sign {
        // Try to get secret from engine's keyring
        let secret = crate::identity::IdentityKeyring::new()
            .get_secret(&pubkey)
            .map_err(|e| EngineError::Config(format!("Cannot sign: {e}")))?;
        build_signed_publication_events(&compose, &pubkey, &secret)
    } else {
        build_publication_events(&compose, &pubkey)
    };

    let pub_id = pub_event
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let section_ids: Vec<String> = section_events
        .iter()
        .map(|e| {
            e.get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect();

    // Ingest into local nostrdb
    let mut ingested = true;
    for event in section_events.iter().chain(std::iter::once(&pub_event)) {
        let json_str = serde_json::to_string(event)
            .map_err(|e| EngineError::Database(format!("JSON error: {e}")))?;
        if let Err(e) = engine.ingest_event(&json_str) {
            debug!("Ingest warning: {}", e);
            ingested = false;
        }
    }

    // Broadcast to relays if requested
    let broadcast_results = if req.broadcast {
        let relays = req
            .relays
            .as_deref()
            .map(|r| r.to_vec())
            .unwrap_or_else(|| engine.publish_relays().to_vec());

        let event_jsons: Vec<String> = section_events
            .iter()
            .chain(std::iter::once(&pub_event))
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();

        let (_, _, results) =
            crate::relay::publish_events_to_relays(&relays, &event_jsons).await;

        Some(
            results
                .into_iter()
                .map(|r| BroadcastResult {
                    relay: r.relay_url,
                    success: r.success,
                    message: r.message,
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    Ok(Json(PublishResponse {
        publication_id: pub_id,
        section_ids,
        signed: req.sign,
        ingested,
        broadcast_results,
    }))
}

// ============================================================================
// Embedding API Endpoints
// ============================================================================

use crate::embedding::EmbeddingStatus;

/// GET /api/v1/embed/status — current embedding index status
pub async fn embed_status_handler(
    State(engine): State<AppState>,
) -> Result<Json<EmbeddingStatus>, EngineError> {
    let emb = match engine.embedding_index() {
        Some(e) => e,
        None => {
            return Ok(Json(EmbeddingStatus {
                enabled: false,
                indexed_count: 0,
                total_events: 0,
                sidecar_available: false,
                model: None,
            }));
        }
    };

    let index = emb.read().await;
    let sidecar_available = index.health_check().await.is_ok();
    let model = index.model().to_string();
    let indexed_count = index.len();

    // Count total events in nostrdb
    let filter = serde_json::json!({"limit": 100000});
    let total_events = crate::query::query_local(engine.ndb(), &[filter])
        .map(|e| e.len())
        .unwrap_or(0);

    Ok(Json(EmbeddingStatus {
        enabled: true,
        indexed_count,
        total_events,
        sidecar_available,
        model: Some(model),
    }))
}

/// POST /api/v1/embed/sync — embed unembedded events
pub async fn embed_sync_handler(
    State(engine): State<AppState>,
) -> Result<Json<EmbeddingStatus>, EngineError> {
    let status = engine.sync_embeddings().await?;
    // Also sync document embeddings
    let _ = engine.sync_doc_embeddings().await;
    Ok(Json(status))
}

/// POST /api/v1/embed/reindex — clear and re-embed everything
pub async fn embed_reindex_handler(
    State(engine): State<AppState>,
) -> Result<Json<EmbeddingStatus>, EngineError> {
    let status = engine.reindex_embeddings().await?;
    Ok(Json(status))
}

// ============================================================================
// Chat API Endpoints
// ============================================================================

use crate::chat::{ChatState, InjectedNote};
use crate::llm::LLMProvider;
use std::sync::Mutex;

/// Shared chat + LLM provider state
#[derive(Clone)]
pub struct ChatAppState {
    pub chat: Arc<Mutex<ChatState>>,
    pub provider: Arc<dyn LLMProvider>,
}

/// A single fragment in the API response
#[derive(Debug, Serialize)]
pub struct FragmentResponse {
    pub id: usize,
    pub role: String,
    pub content: String,
}

/// Unified response for all chat endpoints
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub fragments: Vec<FragmentResponse>,
    pub fragment_count: usize,
    pub edit_mode: bool,
    pub edit_buffer: Option<String>,
    pub system_prompt: Option<String>,
    pub context_count: usize,
    pub generating: bool,
}

fn build_chat_response(state: &ChatState) -> ChatResponse {
    let fragments: Vec<FragmentResponse> = state
        .fragments
        .iter()
        .map(|f| FragmentResponse {
            id: f.id,
            role: f.role.as_str().to_string(),
            content: f.content.clone(),
        })
        .collect();
    let fragment_count = fragments.len();
    ChatResponse {
        fragments,
        fragment_count,
        edit_mode: state.edit_mode,
        edit_buffer: if state.edit_mode {
            Some(state.edit_buffer.clone())
        } else {
            None
        },
        system_prompt: state.system_prompt.clone(),
        context_count: state.injected_context.len(),
        generating: state.generating,
    }
}

/// Request to send a chat message
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub content: String,
}

/// Request to submit an edited buffer
#[derive(Debug, Deserialize)]
pub struct EditBufferRequest {
    pub buffer: String,
}

/// Request to set the system prompt
#[derive(Debug, Deserialize)]
pub struct SystemPromptRequest {
    pub prompt: String,
}

/// A context note to inject
#[derive(Debug, Deserialize)]
pub struct NoteRequest {
    pub title: String,
    pub content: String,
}

/// Request to inject context notes
#[derive(Debug, Deserialize)]
pub struct InjectContextRequest {
    pub notes: Vec<NoteRequest>,
}

/// GET /api/v1/chat — get current conversation state
pub async fn chat_get(State(state): State<ChatAppState>) -> Json<ChatResponse> {
    let chat = state.chat.lock().unwrap();
    Json(build_chat_response(&chat))
}

/// DELETE /api/v1/chat — reset conversation
pub async fn chat_reset(State(state): State<ChatAppState>) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    *chat = ChatState::new();
    Json(build_chat_response(&chat))
}

/// POST /api/v1/chat/message — send message, get LLM response
pub async fn chat_send(
    State(state): State<ChatAppState>,
    Json(req): Json<SendMessageRequest>,
) -> Json<ChatResponse> {
    let messages = {
        let mut chat = state.chat.lock().unwrap();
        chat.input = req.content;
        chat.generating = true;
        chat.send_message()
    };

    let response = state
        .provider
        .chat(messages)
        .await
        .unwrap_or_else(|e| format!("Error: {}", e));

    let mut chat = state.chat.lock().unwrap();
    chat.generating = false;
    chat.receive_response(response);
    Json(build_chat_response(&chat))
}

/// POST /api/v1/chat/edit — enter edit mode
pub async fn chat_enter_edit(State(state): State<ChatAppState>) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    chat.enter_edit_mode();
    Json(build_chat_response(&chat))
}

/// PUT /api/v1/chat/edit — submit modified buffer, exit edit mode
pub async fn chat_exit_edit(
    State(state): State<ChatAppState>,
    Json(req): Json<EditBufferRequest>,
) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    chat.edit_buffer = req.buffer;
    chat.exit_edit_mode();
    Json(build_chat_response(&chat))
}

/// POST /api/v1/chat/system — set system prompt
pub async fn chat_set_system(
    State(state): State<ChatAppState>,
    Json(req): Json<SystemPromptRequest>,
) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    chat.system_prompt = Some(req.prompt);
    Json(build_chat_response(&chat))
}

/// POST /api/v1/chat/context — inject context notes
pub async fn chat_inject_context(
    State(state): State<ChatAppState>,
    Json(req): Json<InjectContextRequest>,
) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    let notes: Vec<InjectedNote> = req
        .notes
        .into_iter()
        .map(|n| InjectedNote {
            addr: None,
            title: n.title,
            content: n.content,
        })
        .collect();
    chat.inject_context(notes);
    Json(build_chat_response(&chat))
}

/// PUT /api/v1/chat/context — clear and replace all context notes
pub async fn chat_replace_context(
    State(state): State<ChatAppState>,
    Json(req): Json<InjectContextRequest>,
) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    chat.clear_context();
    let notes: Vec<InjectedNote> = req
        .notes
        .into_iter()
        .map(|n| InjectedNote {
            addr: None,
            title: n.title,
            content: n.content,
        })
        .collect();
    chat.inject_context(notes);
    Json(build_chat_response(&chat))
}

#[cfg(test)]
mod chat_api_tests {
    use super::*;

    fn make_state() -> ChatAppState {
        use crate::llm::NoopProvider;
        ChatAppState {
            chat: Arc::new(Mutex::new(ChatState::new())),
            provider: Arc::new(NoopProvider::echo()),
        }
    }

    #[tokio::test]
    async fn test_chat_api_get_empty() {
        let state = make_state();
        let Json(resp) = chat_get(State(state)).await;
        assert_eq!(resp.fragment_count, 0);
        assert!(!resp.edit_mode);
        assert!(resp.edit_buffer.is_none());
        assert!(resp.system_prompt.is_none());
        assert_eq!(resp.context_count, 0);
        assert!(!resp.generating);
    }

    #[tokio::test]
    async fn test_chat_api_send_message() {
        let state = make_state();
        let req = SendMessageRequest {
            content: "Hello world".into(),
        };
        let Json(resp) = chat_send(State(state), Json(req)).await;
        assert_eq!(resp.fragment_count, 2);
        assert_eq!(resp.fragments[0].role, "user");
        assert_eq!(resp.fragments[0].content, "Hello world");
        assert_eq!(resp.fragments[1].role, "assistant");
        assert_eq!(resp.fragments[1].content, "Echo: Hello world");
    }

    #[tokio::test]
    async fn test_chat_api_edit_roundtrip() {
        let state = make_state();

        // Send a message first
        let req = SendMessageRequest {
            content: "Hello".into(),
        };
        let _ = chat_send(State(state.clone()), Json(req)).await;

        // Enter edit mode
        let Json(resp) = chat_enter_edit(State(state.clone())).await;
        assert!(resp.edit_mode);
        assert!(resp.edit_buffer.is_some());
        let buffer = resp.edit_buffer.unwrap();
        assert!(buffer.contains("[user]"));
        assert!(buffer.contains("Hello"));

        // Modify and exit
        let req = EditBufferRequest {
            buffer: "[user]\nHello\n---\n[assistant]\nCustom response\n---\n[user]\nFollow-up"
                .into(),
        };
        let Json(resp) = chat_exit_edit(State(state.clone()), Json(req)).await;
        assert!(!resp.edit_mode);
        assert_eq!(resp.fragment_count, 3);
        assert_eq!(resp.fragments[2].role, "user");
        assert_eq!(resp.fragments[2].content, "Follow-up");
    }

    #[tokio::test]
    async fn test_chat_api_reset() {
        let state = make_state();

        let req = SendMessageRequest {
            content: "Hello".into(),
        };
        let _ = chat_send(State(state.clone()), Json(req)).await;

        let Json(resp) = chat_reset(State(state)).await;
        assert_eq!(resp.fragment_count, 0);
    }

    #[tokio::test]
    async fn test_chat_api_system_prompt() {
        let state = make_state();
        let req = SystemPromptRequest {
            prompt: "You are a helpful assistant.".into(),
        };
        let Json(resp) = chat_set_system(State(state), Json(req)).await;
        assert_eq!(
            resp.system_prompt,
            Some("You are a helpful assistant.".to_string())
        );
    }

    #[tokio::test]
    async fn test_chat_api_inject_context() {
        let state = make_state();
        let req = InjectContextRequest {
            notes: vec![NoteRequest {
                title: "Test Note".into(),
                content: "Some context".into(),
            }],
        };
        let Json(resp) = chat_inject_context(State(state), Json(req)).await;
        assert_eq!(resp.context_count, 1);
    }

    #[tokio::test]
    async fn test_chat_api_replace_context() {
        let state = make_state();

        // Inject initial context
        let req = InjectContextRequest {
            notes: vec![
                NoteRequest { title: "A".into(), content: "aaa".into() },
                NoteRequest { title: "B".into(), content: "bbb".into() },
            ],
        };
        let Json(resp) = chat_inject_context(State(state.clone()), Json(req)).await;
        assert_eq!(resp.context_count, 2);

        // Replace with single note
        let req = InjectContextRequest {
            notes: vec![NoteRequest { title: "C".into(), content: "ccc".into() }],
        };
        let Json(resp) = chat_replace_context(State(state), Json(req)).await;
        assert_eq!(resp.context_count, 1);
    }
}
