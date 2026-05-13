//! HTTP API handlers
//!
//! Provides REST endpoints for querying Nostr events.

use crate::engine::{Engine, FetchPolicy, QueryResponse};
use crate::error::EngineError;
use crate::network::{FetchTrigger, NetworkMode};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
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
// NIP-19 Decode Endpoint
// ============================================================================

use crate::nip19;

/// POST /api/v1/decode body.
#[derive(Debug, Deserialize)]
pub struct DecodeRequest {
    /// Bech32 NIP-19 identifier (npub / nprofile / nevent / naddr).
    /// May be prefixed with `nostr:`; the server strips that before decoding.
    pub input: String,
}

/// POST /api/v1/decode
///
/// Decode a NIP-19 bech32 identifier into its structured fields. Returns a
/// `kind`-tagged JSON object — see `nip19::Decoded` for the variants.
pub async fn decode_handler(
    Json(req): Json<DecodeRequest>,
) -> Result<Json<nip19::Decoded>, EngineError> {
    nip19::decode(&req.input)
        .map(Json)
        .map_err(|e| EngineError::InvalidFilter(e.to_string()))
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

/// Resolve by:me / by:assistant in a query to actual pubkey
fn resolve_author(
    query: &mut SearchQuery,
    req: &SearchRequest,
    engine: &AppState,
) -> Result<(), EngineError> {
    match &query.author_filter {
        Some(AuthorFilter::CurrentUser) => {
            let pk = req.my_pubkey.as_deref().or_else(|| engine.my_pubkey());
            if let Some(pk) = pk {
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk.to_string()]));
            } else {
                return Err(EngineError::InvalidFilter(
                    "by:me requires pubkey in config or request".to_string(),
                ));
            }
        }
        Some(AuthorFilter::AssistantUser) => {
            if let Some(pk) = engine.assistant_pubkey() {
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk.to_string()]));
            } else {
                return Err(EngineError::InvalidFilter(
                    "by:assistant requires identity.assistant in config".to_string(),
                ));
            }
        }
        _ => {}
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

    // Trigger background doc embedding sync
    if engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_doc_embeddings().await {
                debug!("Background doc embedding sync after import: {}", e);
            }
        });
    }

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

    // Collect pubkeys not already in nostrdb
    let missing: Vec<&str> = req.pubkeys.iter()
        .filter(|pk| pk.len() == 64 && query_profile(engine.ndb(), pk).is_none())
        .map(|pk| pk.as_str())
        .collect();

    if missing.is_empty() {
        return Ok(Json(json!({ "fetched": 0, "total": req.pubkeys.len() })));
    }

    // Batch fetch: one request per relay with ALL missing pubkeys
    let filter = json!({"kinds": [0], "authors": missing, "limit": missing.len()});
    for relay_url in relays {
        match engine.tracked_fetch(relay_url, &[filter.clone()], FetchTrigger::ProfilePrefetch).await {
            Ok(events) => {
                fetched += events.len();
            }
            Err(e) => {
                debug!("Failed to fetch profiles from {}: {}", relay_url, e);
            }
        }
    }

    // Brief wait for nostrdb to process ingested events
    if fetched > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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

    let events = engine.tracked_fetch(
        &req.relay,
        &[filter],
        FetchTrigger::UserAction,
    )
    .await?;

    let count = events.len();
    debug!("Fetched {} events from {}", count, req.relay);

    // Trigger background embedding sync for newly fetched events
    if count > 0 && engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_embeddings().await {
                debug!("Background embedding sync after fetch: {}", e);
            }
        });
    }

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

        match engine.tracked_fetch(relay_url, &[filter], FetchTrigger::UserAction).await {
            Ok(events) => {
                debug!("Fetched {} events for authors from {}", events.len(), relay_url);
                total_fetched += events.len();
            }
            Err(e) => {
                debug!("Failed to fetch authors from {}: {}", relay_url, e);
            }
        }
    }

    // Trigger background embedding sync for newly fetched events
    if total_fetched > 0 && engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_embeddings().await {
                debug!("Background embedding sync after author fetch: {}", e);
            }
        });
    }

    Ok(Json(json!({
        "fetched": total_fetched,
        "authors": authors.len(),
        "relays": rc.fetch.urls.len()
    })))
}

/// POST /api/v1/fetch/sections — bulk-fetch missing 30041 sections for all known 30040 indexes
pub async fn fetch_sections_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let (total_referenced, missing, fetched) = engine.fetch_missing_sections().await?;

    // Trigger background embedding sync for newly fetched sections
    if fetched > 0 && engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_embeddings().await {
                debug!("Background embedding sync after section fetch: {}", e);
            }
        });
    }

    Ok(Json(json!({
        "total_referenced": total_referenced,
        "missing": missing,
        "fetched": fetched
    })))
}

// ============================================================================
// Network Mode & Activity API
// ============================================================================

/// GET /api/v1/network/status — current mode, active fetches, recent activity
pub async fn network_status_handler(
    State(engine): State<AppState>,
) -> Json<Value> {
    let status = engine.network().status();
    Json(serde_json::to_value(status).unwrap_or_default())
}

/// POST /api/v1/network/mode — toggle online/offline
#[derive(Debug, Deserialize)]
pub struct SetNetworkModeRequest {
    pub mode: String,
}

pub async fn set_network_mode_handler(
    State(engine): State<AppState>,
    Json(req): Json<SetNetworkModeRequest>,
) -> Result<Json<Value>, EngineError> {
    let mode: NetworkMode = req.mode.parse().map_err(|e: String| {
        EngineError::InvalidFilter(e)
    })?;

    engine.set_network_mode(mode);

    // Persist to config.toml in a blocking task to avoid stalling the runtime
    if let Some(config_path) = engine.config_path() {
        let config_path = config_path.to_path_buf();
        let mode_str = mode.to_string();
        tokio::task::spawn_blocking(move || {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(mut doc) = content.parse::<toml::Table>() {
                    let network = doc.entry("network")
                        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                    if let toml::Value::Table(table) = network {
                        table.insert("mode".into(), toml::Value::String(mode_str));
                    }
                    if let Ok(serialized) = toml::to_string_pretty(&doc) {
                        let _ = std::fs::write(&config_path, serialized);
                    }
                }
            }
        });
    }

    // Return immediately with current mode — don't wait for status lock
    Ok(Json(json!({ "mode": mode })))
}

// ============================================================================
// Relay Config API Endpoints
// ============================================================================

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

#[derive(Debug, Deserialize)]
pub struct RelayInfoQuery {
    pub url: String,
}

/// GET /api/v1/relay/info?url=wss://… — return cached NIP-11 doc
/// for the relay (or kick off a fetch if missing/stale and return
/// `Loading`). See `docs/relay-classes-and-info-port.md` §4 for the
/// caching contract.
pub async fn relay_nip11_handler(
    State(engine): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RelayInfoQuery>,
) -> Json<Value> {
    let status = engine.nip11_cache().get(&q.url).await;
    Json(json!({
        "url": q.url,
        "status": status,
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

use crate::publication::{build_block_publication_events, build_publication_events};
use crate::tree::state::{
    BlockKind, ComposeBlock, ComposeBlockState, ComposeState, SectionCompose,
};

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
    Extension(identity): Extension<IdentityAppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<PublishRequest>,
) -> Result<impl IntoResponse, EngineError> {
    // Resolve pubkey: prefer identity session, fall back to config
    let pubkey = {
        let session = identity.lock().unwrap();
        session.pubkey().map(|s| s.to_string())
    }
    .or_else(|| engine.my_pubkey().map(|s| s.to_string()))
    .ok_or_else(|| {
        EngineError::Config("Publishing requires identity login or [identity] pubkey in config".into())
    })?;

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
        // Sign every event through the SigningController. For engine
        // source this resolves InProcessSigner (same fallback chain as
        // before); for nip07 / nip46 source this round-trips each
        // template through the registered ExternalSigner via the SSE
        // back-channel. Either way the publish handler is unaware.
        let active_pubkey = signing.active_pubkey().await.ok_or_else(|| {
            EngineError::Config(
                "No identity configured (engine source needs login; nip07 needs a connected signer)"
                    .into(),
            )
        })?;
        crate::publication::build_signed_publication_events_via_signer(
            &compose,
            &active_pubkey,
            &signing,
        )
        .await
        .map_err(|e| match e {
            crate::signing::SigningError::Locked => {
                EngineError::Locked("Identity is locked — unlock with password first".into())
            }
            crate::signing::SigningError::SignerNotConnected => EngineError::Config(
                "External signer not connected — open a tab with the signer extension".into(),
            ),
            other => EngineError::Config(format!("Cannot sign: {other}")),
        })?
    } else {
        // Track unsigned events if identity is present but locked
        let should_track = {
            let session = identity.lock().unwrap();
            session.pubkey().is_some()
        };
        let events = build_publication_events(&compose, &pubkey);
        if should_track {
            let pub_id = events
                .0
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !pub_id.is_empty() {
                let mut session = identity.lock().unwrap();
                session.track_unsigned(pub_id);
            }
        }
        events
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
    // process_event is async — it queues events for background processing.
    // We ingest all events, wait for nostrdb to process them, then verify.
    for event in section_events.iter().chain(std::iter::once(&pub_event)) {
        let json_str = serde_json::to_string(event)
            .map_err(|e| EngineError::Database(format!("JSON error: {e}")))?;
        if let Err(e) = engine.ingest_event(&json_str) {
            debug!("Ingest queue warning: {}", e);
        }
    }

    // Wait for nostrdb to process the queued events
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // Verify the publication event was actually stored
    let ingested = crate::query::query_by_id(engine.ndb(), &pub_id)
        .ok()
        .flatten()
        .is_some();
    if !ingested {
        debug!("Publication {} was not persisted by nostrdb after ingest", pub_id);
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

    // Trigger background embedding sync so new events are searchable immediately
    if ingested && engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_embeddings().await {
                debug!("Background embedding sync after publish: {}", e);
            }
        });
    }

    Ok(Json(PublishResponse {
        publication_id: pub_id,
        section_ids,
        signed: req.sign,
        ingested,
        broadcast_results,
    }))
}

// ----------------------------------------------------------------------------
// Block-based publish (NIP-54-style fork support)
// ----------------------------------------------------------------------------
//
// The legacy /publish endpoint always emits a fresh 30041 per section. The
// block endpoint accepts a richer payload where each block can be:
//
//   - editable  → emit a new 30041 (no fork lineage).
//   - imported  → no 30041 emitted; the new 30040 references the source addr.
//   - forked    → emit a new 30041 with `a`/`e` `fork`-marker tags.
//
// When the payload's `source_publication_addr` is set, the new 30040 also
// carries `a`/`e` `fork`-marker tags pointing at the parent 30040, per
// NIP-54.
//
// The web client computes structural-change locally and only calls this
// endpoint when there's something to publish.

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublishBlockKind {
    Editable {
        content: String,
    },
    Imported {
        source_addr: NAddrPayload,
        content: String,
        author: String,
    },
    Forked {
        original_addr: NAddrPayload,
        content: String,
        original_author: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NAddrPayload {
    pub kind: u64,
    pub pubkey: String,
    pub d_tag: String,
}

impl NAddrPayload {
    fn into_naddr(self) -> NAddr {
        NAddr {
            kind: self.kind,
            pubkey: self.pubkey,
            d_tag: self.d_tag,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PublishBlockEntry {
    pub title: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    #[serde(flatten)]
    pub kind: PublishBlockKind,
}

#[derive(Debug, Deserialize)]
pub struct PublishBlocksRequest {
    pub title: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    pub blocks: Vec<PublishBlockEntry>,
    /// If set, the new 30040 emits `["a", ..., "fork"]` (and optionally
    /// `["e", ..., "fork"]`) pointing at this source publication.
    pub source_publication_addr: Option<NAddrPayload>,
    pub source_publication_event_id: Option<String>,
    #[serde(default)]
    pub sign: bool,
    #[serde(default)]
    pub broadcast: bool,
    pub relays: Option<Vec<String>>,
}

/// POST /api/v1/publish/blocks — publish a block-based draft.
pub async fn publish_blocks_handler(
    State(engine): State<AppState>,
    Extension(identity): Extension<IdentityAppState>,
    Json(req): Json<PublishBlocksRequest>,
) -> Result<impl IntoResponse, EngineError> {
    let pubkey = {
        let session = identity.lock().unwrap();
        session.pubkey().map(|s| s.to_string())
    }
    .or_else(|| engine.my_pubkey().map(|s| s.to_string()))
    .ok_or_else(|| {
        EngineError::Config(
            "Publishing requires identity login or [identity] pubkey in config".into(),
        )
    })?;

    use crate::tree::state::TagEntry;
    let mut state = ComposeBlockState::new();
    state.title = req.title;
    for (name, value) in &req.tags {
        state.tags.push(TagEntry {
            name: name.clone(),
            value: value.clone(),
        });
    }
    state.source_publication_addr = req.source_publication_addr.map(|n| n.into_naddr());
    state.source_publication_event_id = req.source_publication_event_id;

    let mut next_id: usize = 0;
    for entry in req.blocks {
        let block_id = next_id;
        next_id += 1;
        let kind = match entry.kind {
            PublishBlockKind::Editable { content } => BlockKind::Editable {
                content,
                cursor: 0,
            },
            PublishBlockKind::Imported {
                source_addr,
                content,
                author,
            } => BlockKind::Imported {
                source_addr: source_addr.into_naddr(),
                content,
                author,
                fork_requested: false,
            },
            PublishBlockKind::Forked {
                original_addr,
                content,
                original_author,
            } => BlockKind::Forked {
                original_addr: original_addr.into_naddr(),
                content,
                cursor: 0,
                original_author,
            },
        };
        state.blocks.push(ComposeBlock {
            block_id,
            kind,
            title: entry.title,
            tags: entry
                .tags
                .into_iter()
                .map(|(name, value)| TagEntry { name, value })
                .collect(),
            collapsed: false,
        });
    }

    // Resolve signing key — same fallback chain as the legacy publish handler.
    let secret_hex: Option<String> = if req.sign {
        let resolved = {
            let mut session = identity.lock().unwrap();
            if session.can_sign() {
                session.touch();
                Some(session.secret().unwrap().to_string())
            } else if session.pubkey().is_some() {
                return Err(EngineError::Locked(
                    "Identity is locked — unlock with password first".into(),
                ));
            } else {
                None
            }
        };
        if let Some(s) = resolved {
            Some(s)
        } else {
            // Fall back to keyring or .env
            let from_keyring = crate::identity::IdentityKeyring::new()
                .get_secret(&pubkey)
                .ok();
            if let Some(s) = from_keyring {
                Some(s)
            } else {
                let env_content = std::fs::read_to_string(".env").ok();
                let mut secret: Option<String> = None;
                if let Some(content) = env_content {
                    let mut ncryptsec: Option<String> = None;
                    let mut password: Option<String> = None;
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("NOSTR_NCRYPTSEC=") {
                            ncryptsec = Some(val.to_string());
                        } else if let Some(val) = line.strip_prefix("NOSTR_PASSWORD=") {
                            password = Some(val.to_string());
                        }
                    }
                    if let (Some(nc), Some(pw)) = (ncryptsec, password) {
                        if let Ok((s_hex, _)) = crate::identity::decrypt_ncryptsec(&nc, &pw) {
                            secret = Some(s_hex);
                        }
                    }
                }
                secret
            }
        }
    } else {
        None
    };

    let (pub_event, section_events) =
        build_block_publication_events(&state, &pubkey, secret_hex.as_deref());

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

    for event in section_events.iter().chain(std::iter::once(&pub_event)) {
        let json_str = serde_json::to_string(event)
            .map_err(|e| EngineError::Database(format!("JSON error: {e}")))?;
        if let Err(e) = engine.ingest_event(&json_str) {
            debug!("Ingest queue warning: {}", e);
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let ingested = crate::query::query_by_id(engine.ndb(), &pub_id)
        .ok()
        .flatten()
        .is_some();

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

    if ingested && engine.embedding_index().is_some() {
        let eng = engine.clone();
        tokio::spawn(async move {
            if let Err(e) = eng.sync_embeddings().await {
                debug!("Background embedding sync after block publish: {}", e);
            }
        });
    }

    Ok(Json(PublishResponse {
        publication_id: pub_id,
        section_ids,
        signed: req.sign,
        ingested,
        broadcast_results,
    }))
}

// ============================================================================
// Ingest API Endpoint
// ============================================================================
// Export API
// ============================================================================

/// GET /api/v1/export — export all local events as JSONL (one event per line)
///
/// Query params:
/// - kinds: comma-separated kind numbers (e.g. ?kinds=30040,30041)
/// - authors: comma-separated hex pubkeys
/// - since: unix timestamp lower bound
/// - until: unix timestamp upper bound
/// - limit: max events (default: 100000)
pub async fn export_handler(
    State(engine): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, EngineError> {
    let mut filter = serde_json::json!({});

    if let Some(kinds_str) = params.get("kinds") {
        let kinds: Vec<u64> = kinds_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if !kinds.is_empty() {
            filter["kinds"] = json!(kinds);
        }
    }
    if let Some(authors_str) = params.get("authors") {
        let authors: Vec<&str> = authors_str.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !authors.is_empty() {
            filter["authors"] = json!(authors);
        }
    }
    if let Some(since) = params.get("since").and_then(|s| s.parse::<u64>().ok()) {
        filter["since"] = json!(since);
    }
    if let Some(until) = params.get("until").and_then(|s| s.parse::<u64>().ok()) {
        filter["until"] = json!(until);
    }
    let limit = params.get("limit")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(100_000);
    filter["limit"] = json!(limit);

    let events = crate::query::query_local(engine.ndb(), &[filter])?;
    let count = events.len();

    // Build JSONL body
    let mut body = String::new();
    for event in &events {
        if let Ok(line) = serde_json::to_string(event) {
            body.push_str(&line);
            body.push('\n');
        }
    }

    let mut headers = axum::http::HeaderMap::new();
    headers.insert("content-type", "application/x-ndjson".parse().unwrap());
    headers.insert("x-event-count", count.to_string().parse().unwrap());

    Ok((headers, body))
}

/// GET /api/v1/export/manifest — summary of what an export would contain
pub async fn export_manifest_handler(
    State(engine): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, EngineError> {
    // Query with same filters to count
    let mut filter = serde_json::json!({});
    if let Some(kinds_str) = params.get("kinds") {
        let kinds: Vec<u64> = kinds_str.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if !kinds.is_empty() {
            filter["kinds"] = json!(kinds);
        }
    }
    if let Some(authors_str) = params.get("authors") {
        let authors: Vec<&str> = authors_str.split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !authors.is_empty() {
            filter["authors"] = json!(authors);
        }
    }
    if let Some(since) = params.get("since").and_then(|s| s.parse::<u64>().ok()) {
        filter["since"] = json!(since);
    }
    if let Some(until) = params.get("until").and_then(|s| s.parse::<u64>().ok()) {
        filter["until"] = json!(until);
    }
    filter["limit"] = json!(100_000u64);

    let events = crate::query::query_local(engine.ndb(), &[filter])?;

    // Count by kind
    let mut kinds_count = std::collections::HashMap::<u64, usize>::new();
    let mut authors_set = std::collections::HashSet::<String>::new();
    for event in &events {
        if let Some(kind) = event.get("kind").and_then(|v| v.as_u64()) {
            *kinds_count.entry(kind).or_default() += 1;
        }
        if let Some(author) = event.get("pubkey").and_then(|v| v.as_str()) {
            authors_set.insert(author.to_string());
        }
    }

    let emb_count = if let Some(e) = engine.embedding_index() {
        e.read().await.len()
    } else {
        0
    };

    Ok(Json(json!({
        "event_count": events.len(),
        "kinds": kinds_count,
        "authors": authors_set.len(),
        "embedding_count": emb_count,
        "filters_used": params,
    })))
}

// ============================================================================
// Ingest API
// ============================================================================

/// POST /api/v1/ingest — ingest Nostr events into nostrdb
///
/// Accepts either:
/// - A single JSON event object: `{"id":"...", ...}`
/// - A JSONL body (one event per line) with Content-Type: application/x-ndjson
///
/// Embedding sync is NOT triggered per event. The background 60s loop
/// handles it, or call POST /api/v1/embed/sync explicitly after bulk ingest.
pub async fn ingest_handler(
    State(engine): State<AppState>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<impl IntoResponse, EngineError> {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json");

    let is_ndjson = content_type.contains("ndjson") || content_type.contains("jsonl");

    if is_ndjson {
        // Bulk JSONL ingest
        let start = std::time::Instant::now();
        let mut ingested = 0usize;
        let mut skipped = 0usize;
        let mut errors = 0usize;

        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(event) => {
                    let event_json = event.to_string();
                    match engine.ingest_event(&event_json) {
                        Ok(()) => ingested += 1,
                        Err(_) => skipped += 1, // nostrdb rejects duplicates
                    }
                }
                Err(_) => errors += 1,
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        // Don't trigger embedding sync per chunk — the background 60s loop
        // handles it, or the caller can POST /api/v1/embed/sync after import.
        let embedding_sync = "deferred";

        Ok(Json(json!({
            "ingested": ingested,
            "skipped": skipped,
            "errors": errors,
            "duration_ms": duration_ms,
            "embedding_sync": embedding_sync
        })))
    } else {
        // Single event ingest (no embedding sync — background loop handles it)
        let event: Value = serde_json::from_str(&body)
            .map_err(|e| EngineError::Database(format!("JSON error: {e}")))?;
        let event_json = event.to_string();
        engine.ingest_event(&event_json)?;
        let id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
        Ok(Json(json!({ "ingested": true, "id": id })))
    }
}

// ============================================================================
// Embedding API Endpoints
// ============================================================================

use crate::embedding::EmbeddingStatus;

/// GET /api/v1/embed/status — current embedding index status
pub async fn embed_status_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let emb = match engine.embedding_index() {
        Some(e) => e,
        None => {
            return Ok(Json(json!({
                "enabled": false,
                "indexed_count": 0,
                "total_events": 0,
                "stale_count": 0,
                "missing_sections": 0,
                "sidecar_available": false,
                "model": null,
            })));
        }
    };

    let index = emb.read().await;
    let sidecar_available = index.health_check().await.is_ok();
    let model = index.model().to_string();
    let indexed_count = index.len();

    // Count embeddable events in nostrdb (content kinds only; skip 30040 index events)
    let filter = serde_json::json!({"kinds": [30041, 30023, 30818, 9802], "limit": 100000});
    let local_events = crate::query::query_local(engine.ndb(), &[filter])
        .unwrap_or_default();
    let total_events = local_events.len();

    // Count stale embeddings (indexed but no longer in nostrdb)
    let local_ids: std::collections::HashSet<&str> = local_events.iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()))
        .collect();
    let stale_count = indexed_count.saturating_sub(
        index.all_ids().iter().filter(|id| local_ids.contains(id.as_str())).count()
    );

    // Count missing sections (30040 indexes reference 30041s not yet fetched)
    let idx_filter = serde_json::json!({"kinds": [30040], "limit": 100000});
    let indexes = crate::query::query_local(engine.ndb(), &[idx_filter]).unwrap_or_default();
    let mut referenced_sections = 0usize;
    for event in &indexes {
        if let Some(tags) = event.get("tags").and_then(|v| v.as_array()) {
            for tag in tags {
                if let Some(arr) = tag.as_array() {
                    if arr.first().and_then(|v| v.as_str()) == Some("a") {
                        if let Some(addr) = arr.get(1).and_then(|v| v.as_str()) {
                            if addr.starts_with("30041:") {
                                referenced_sections += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    let missing_sections = referenced_sections.saturating_sub(total_events);

    Ok(Json(json!({
        "enabled": true,
        "indexed_count": indexed_count,
        "total_events": total_events,
        "stale_count": stale_count,
        "missing_sections": missing_sections,
        "sidecar_available": sidecar_available,
        "model": model,
    })))
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
// Claude Code Sessions
// ============================================================================

/// GET /api/v1/claude-sessions — list available Claude Code conversation sessions
pub async fn list_claude_sessions_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let dir = engine.claude_sessions_dir().ok_or_else(|| {
        EngineError::Config("Claude Code sessions directory not found".into())
    })?;
    let sessions = crate::claude_sessions::list_sessions(dir)?;
    let count = sessions.len();
    Ok(Json(json!({
        "sessions": sessions,
        "count": count,
    })))
}

/// Query parameters for session endpoint
#[derive(Debug, Deserialize)]
pub struct SessionQuery {
    /// Skip the first N messages (for polling new messages)
    pub offset: Option<usize>,
}

/// GET /api/v1/claude-sessions/:id — get messages for a specific session
pub async fn get_claude_session_handler(
    State(engine): State<AppState>,
    Path(session_id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<SessionQuery>,
) -> Result<Json<Value>, EngineError> {
    let dir = engine.claude_sessions_dir().ok_or_else(|| {
        EngineError::Config("Claude Code sessions directory not found".into())
    })?;

    let offset = query.offset.unwrap_or(0);
    if offset > 0 {
        // Polling mode: find the file and return only new messages
        let matches: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| EngineError::Database(format!("Failed to read sessions dir: {e}")))?
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                name.starts_with(&session_id) && name.ends_with(".jsonl")
            })
            .map(|e| e.path())
            .collect();

        if matches.is_empty() {
            return Err(EngineError::NotFound(format!("No session matching '{session_id}'")));
        }

        let messages = crate::claude_sessions::parse_session_messages(&matches[0], offset)?;
        return Ok(Json(json!({
            "id": session_id,
            "messages": messages,
            "count": messages.len(),
            "offset": offset,
        })));
    }

    let detail = crate::claude_sessions::get_session(dir, &session_id)?;
    let count = detail.messages.len();
    Ok(Json(json!({
        "id": detail.id,
        "messages": detail.messages,
        "count": count,
    })))
}

/// POST /api/v1/claude-sessions/:id/message — append a user message to a session
pub async fn append_claude_session_handler(
    State(engine): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<AppendSessionRequest>,
) -> Result<Json<Value>, EngineError> {
    let dir = engine.claude_sessions_dir().ok_or_else(|| {
        EngineError::Config("Claude Code sessions directory not found".into())
    })?;
    let uuid = crate::claude_sessions::append_message(dir, &session_id, &req.content)?;
    Ok(Json(json!({
        "uuid": uuid,
        "session_id": session_id,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AppendSessionRequest {
    pub content: String,
}

// ============================================================================
// Chat API Endpoints
// ============================================================================

use crate::chat::{ChatRole, ChatState, InjectedNote};
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

/// PUT /api/v1/chat/load — load fragments directly (for importing sessions)
pub async fn chat_load_fragments(
    State(state): State<ChatAppState>,
    Json(req): Json<Vec<FragmentInput>>,
) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    let fragments: Vec<(ChatRole, String)> = req
        .into_iter()
        .filter_map(|f| {
            let role = ChatRole::from_str(&f.role)?;
            Some((role, f.content))
        })
        .collect();
    chat.load_fragments(fragments);
    Json(build_chat_response(&chat))
}

/// Input for loading a fragment
#[derive(Debug, Deserialize)]
pub struct FragmentInput {
    pub role: String,
    pub content: String,
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

// ---------------------------------------------------------------------------
// Identity session endpoints
// ---------------------------------------------------------------------------

use crate::identity::{IdentitySession, IdentityStatusResponse};

/// Shared identity session state
pub type IdentityAppState = Arc<std::sync::Mutex<IdentitySession>>;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub ncryptsec: String,
}

#[derive(Debug, Deserialize)]
pub struct UnlockRequest {
    pub password: String,
}

/// GET /api/v1/identity — current identity status
pub async fn identity_status_handler(
    State(identity): State<IdentityAppState>,
) -> Json<IdentityStatusResponse> {
    let mut session = identity.lock().unwrap();
    Json(session.status())
}

/// POST /api/v1/identity/login — provide ncryptsec, transition to locked
pub async fn identity_login_handler(
    State(identity): State<IdentityAppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    let mut session = identity.lock().unwrap();
    session
        .login_ncryptsec(&req.ncryptsec)
        .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;
    Ok(Json(session.status()))
}

/// POST /api/v1/identity/unlock — decrypt ncryptsec with password
pub async fn identity_unlock_handler(
    State(identity): State<IdentityAppState>,
    Json(req): Json<UnlockRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    // Quick state check
    {
        let mut session = identity.lock().unwrap();
        let state = session.status().state;
        if state == "none" {
            return Err(EngineError::Auth("No identity loaded — login first".into()));
        }
        if state == "unlocked" {
            session.touch();
            return Ok(Json(session.status()));
        }
    }

    let password = req.password.clone();
    let identity_clone = identity.clone();

    // spawn_blocking to avoid blocking the async runtime with scrypt
    let result = tokio::task::spawn_blocking(move || {
        let mut session = identity_clone.lock().unwrap();
        session.unlock(&password)
    })
    .await
    .map_err(|e| EngineError::Other(format!("Task join error: {e}")))?;

    match result {
        Ok(pubkey) => {
            debug!("Identity unlocked for pubkey {}", pubkey);
            let mut session = identity.lock().unwrap();
            Ok(Json(session.status()))
        }
        Err(e) => Err(EngineError::Auth(format!("Unlock failed: {e}"))),
    }
}

/// POST /api/v1/identity/lock — re-lock (clear secret, keep ncryptsec)
pub async fn identity_lock_handler(
    State(identity): State<IdentityAppState>,
) -> Json<IdentityStatusResponse> {
    let mut session = identity.lock().unwrap();
    session.lock();
    Json(session.status())
}

/// POST /api/v1/identity/logout — clear everything
pub async fn identity_logout_handler(
    State(identity): State<IdentityAppState>,
) -> Json<IdentityStatusResponse> {
    let mut session = identity.lock().unwrap();
    session.logout();
    Json(session.status())
}

#[derive(Debug, Deserialize)]
pub struct UseSourceRequest {
    /// "engine" | "nip07" | "nip46"
    pub source: String,
    /// Required when source is nip07 / nip46 (returned by /signer-register).
    #[serde(default)]
    pub signer_id: Option<String>,
}

/// POST /api/v1/identity/use — switch the active signing source.
pub async fn identity_use_source_handler(
    State(identity): State<IdentityAppState>,
    Json(req): Json<UseSourceRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    use crate::identity::IdentitySource;
    let new_source = match req.source.as_str() {
        "engine" => IdentitySource::Engine,
        "nip07" => {
            let signer_id = req
                .signer_id
                .ok_or_else(|| EngineError::Config("nip07 source requires signer_id".into()))?;
            IdentitySource::Nip07 { signer_id }
        }
        "nip46" => {
            let signer_id = req
                .signer_id
                .ok_or_else(|| EngineError::Config("nip46 source requires signer_id".into()))?;
            IdentitySource::Nip46 { signer_id }
        }
        other => {
            return Err(EngineError::Config(format!("unknown source: {other}")));
        }
    };
    let mut session = identity.lock().unwrap();
    session.set_source(new_source);
    Ok(Json(session.status()))
}

#[derive(Debug, Deserialize)]
pub struct SignTemplateRequest {
    pub template: crate::signing::EventTemplate,
}

#[derive(Debug, Serialize)]
pub struct SignTemplateResponse {
    pub signed_event: crate::signing::SignedEvent,
}

/// POST /api/v1/identity/sign — sign a single event template through the
/// active source. Used by callers that need one-shot signing without
/// going through the full publish flow (chat publish, profile updates).
pub async fn identity_sign_handler(
    State(controller): State<crate::signing::SigningController>,
    Json(req): Json<SignTemplateRequest>,
) -> Result<Json<SignTemplateResponse>, EngineError> {
    let signed_event = controller.sign(req.template).await.map_err(|e| match e {
        crate::signing::SigningError::Locked => {
            EngineError::Locked("Identity is locked — unlock with password first".into())
        }
        crate::signing::SigningError::NoIdentity => {
            EngineError::Config("No identity configured".into())
        }
        crate::signing::SigningError::SignerNotConnected => EngineError::Config(
            "External signer not connected — open a tab with the signer extension".into(),
        ),
        other => EngineError::Config(format!("Sign failed: {other}")),
    })?;
    Ok(Json(SignTemplateResponse { signed_event }))
}

// ---------------------------------------------------------------------------
// Generic broadcast — pushes a fully-signed event to relays
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BroadcastRequest {
    /// Already-signed event JSON (must include `id`, `pubkey`, `sig`).
    pub event: Value,
    /// Optional explicit relay list. Defaults to the engine's publish set.
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct BroadcastResponse {
    pub successful: usize,
    pub total: usize,
    pub results: Vec<crate::relay::PublishResult>,
}

/// POST /api/v1/broadcast — push a fully-signed event to relays.
///
/// Used by clients that signed via `/identity/sign` (or any other path
/// that produced a signed event JSON) and want to fan it out without
/// going through the publication-shaped publish handler. Used today by
/// the profile-edit buffer to push kind-0 metadata.
pub async fn broadcast_handler(
    State(engine): State<AppState>,
    Json(req): Json<BroadcastRequest>,
) -> Result<Json<BroadcastResponse>, EngineError> {
    // Basic shape check — the relay layer will reject malformed events
    // anyway, but a clear error here saves a round trip.
    let event = req.event.as_object().ok_or_else(|| {
        EngineError::Config("event must be a JSON object".into())
    })?;
    for field in ["id", "pubkey", "sig", "kind", "created_at", "tags", "content"] {
        if !event.contains_key(field) {
            return Err(EngineError::Config(format!(
                "event missing required field `{field}`"
            )));
        }
    }

    let relays = req
        .relays
        .unwrap_or_else(|| engine.publish_relays().to_vec());
    if relays.is_empty() {
        return Err(EngineError::Config(
            "no relays configured (set [relays.publish] in config or pass `relays`)".into(),
        ));
    }

    let event_json = serde_json::to_string(&req.event).map_err(|e| {
        EngineError::Config(format!("event serialize: {e}"))
    })?;
    let results = crate::relay::publish_to_relays(&relays, &event_json).await;
    let successful = results.iter().filter(|r| r.success).count();
    let total = results.len();
    Ok(Json(BroadcastResponse {
        successful,
        total,
        results,
    }))
}

// ---------------------------------------------------------------------------
// External signer channel (Phase 4)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SignerRegisterRequest {
    pub kind: String,
    pub pubkey: String,
    #[serde(default)]
    pub capabilities: crate::signing::SignerCapabilities,
}

#[derive(Debug, Serialize)]
pub struct SignerRegisterResponse {
    pub signer_id: String,
    pub token: String,
}

/// POST /api/v1/identity/signer-register
pub async fn signer_register_handler(
    State(controller): State<crate::signing::SigningController>,
    Json(req): Json<SignerRegisterRequest>,
) -> Json<SignerRegisterResponse> {
    let (signer_id, token) = controller
        .register_external(req.kind, req.pubkey, req.capabilities)
        .await;
    Json(SignerRegisterResponse { signer_id, token })
}

#[derive(Debug, Deserialize)]
pub struct SignerChannelQuery {
    pub token: String,
}

/// GET /api/v1/identity/signer-channel?token=...
///
/// Long-lived SSE stream. Each `sign_request` event carries a `req_id`
/// and the `EventTemplate`; the client signs (e.g. via
/// `window.nostr.signEvent`) and POSTs back to `/sign-response`.
pub async fn signer_channel_handler(
    State(controller): State<crate::signing::SigningController>,
    axum::extract::Query(q): axum::extract::Query<SignerChannelQuery>,
) -> Result<axum::response::Sse<futures::stream::BoxStream<'static, Result<axum::response::sse::Event, std::convert::Infallible>>>, EngineError>
{
    use axum::response::sse::{Event as SseHttpEvent, KeepAlive, Sse};
    use futures::stream::StreamExt;

    let signer = controller
        .lookup_by_token(&q.token)
        .await
        .ok_or_else(|| EngineError::Auth("unknown signer token".into()))?;
    let rx = signer
        .take_receiver()
        .ok_or_else(|| EngineError::Auth("signer channel already claimed".into()))?;
    signer.touch();

    // Bridge the mpsc receiver into a stream of SSE events. On send-side
    // disconnect (signer dropped from registry), the stream ends.
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(ev) => {
                let payload = serde_json::to_string(&ev).unwrap_or_default();
                let sse_event = SseHttpEvent::default().data(payload);
                Some((
                    Ok::<SseHttpEvent, std::convert::Infallible>(sse_event),
                    rx,
                ))
            }
            None => None,
        }
    });

    Ok(Sse::new(stream.boxed()).keep_alive(KeepAlive::default()))
}

#[derive(Debug, Deserialize)]
pub struct SignResponseRequest {
    pub signer_id: String,
    pub req_id: String,
    #[serde(default)]
    pub signed_event: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SignResponseResponse {
    pub resolved: bool,
}

/// POST /api/v1/identity/sign-response
pub async fn sign_response_handler(
    State(controller): State<crate::signing::SigningController>,
    Json(req): Json<SignResponseRequest>,
) -> Json<SignResponseResponse> {
    let reply = match (req.signed_event, req.error) {
        (Some(ev), _) => crate::signing::SignerReply::Ok(ev),
        (None, Some(msg)) => crate::signing::SignerReply::Err(msg),
        (None, None) => crate::signing::SignerReply::Err("empty response".into()),
    };
    let resolved = controller
        .resolve_sign_response(&req.signer_id, &req.req_id, reply)
        .await;
    Json(SignResponseResponse { resolved })
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
