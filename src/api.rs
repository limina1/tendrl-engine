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
use tracing::{debug, warn};

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
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "event": null, "message": "Event not found" })),
        )),
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

use crate::search::{AuthorFilter, ProfileResult, SearchQuery, SearchResponse, TextFilter};

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
    /// Set true for explicit user-initiated searches that should reach
    /// the network even when global network mode is offline (the web's
    /// "No events in local DB — search relays?" CTA).
    #[serde(default)]
    pub mode_confirm: bool,
}

/// POST /api/v1/search
///
/// Search for events using the structured search query language
pub async fn search_handler(
    State(engine): State<AppState>,
    Json(mut req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, EngineError> {
    debug!("Search request: query={:?}", req.query);

    let mut policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    // An explicit relay search is a user-initiated fetch operation —
    // gate it. In Auto mode this returns at once; in Confirm mode it
    // blocks for the modal. Declined → fall back to a local-only search.
    let mut op: Option<crate::network::FetchOperation> = None;
    let mut override_relays = req.relays.clone();
    if req.mode_confirm {
        let relays = req
            .relays
            .clone()
            .unwrap_or_else(|| engine.relay_config().all_urls());
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Search,
                format!("Search relays: {}", req.query.trim()),
                describe_search_steps(&req.query),
                relays,
            )
            .await
        {
            Ok(o) => {
                override_relays = Some(o.relays().to_vec());
                op = Some(o);
            }
            Err(_) => {
                req.mode_confirm = false;
                policy = FetchPolicy::LocalOnly;
            }
        }
    }

    // Check for compound query (contains |)
    if req.query.contains('|') {
        let compound = SearchQuery::parse_compound(&req.query)
            .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        let mut total_local = 0;
        let mut total_relay = 0;
        let mut profile_terms: Vec<String> = Vec::new();

        for mut branch in compound.branches {
            if let Some(limit) = req.limit {
                branch.limit = Some(limit);
            }
            resolve_author(&mut branch, &req, &engine)?;

            if let Some(term) = query_free_text(&branch) {
                if !profile_terms.contains(&term) {
                    profile_terms.push(term);
                }
            }

            let resp = engine
                .search_with_options(&branch, policy, override_relays.as_deref(), req.mode_confirm)
                .await?;

            total_local += resp.local_count;
            total_relay += resp.relay_count;

            for result in resp.results {
                if seen_ids.insert(result.event_id.clone()) {
                    all_results.push(result);
                }
            }
        }

        let profiles = merge_profile_search(&engine, &profile_terms).await;

        let count = all_results.len();
        let response = SearchResponse {
            results: all_results,
            profiles,
            count,
            local_count: total_local,
            relay_count: total_relay,
            doc_results: vec![],
            tag_counts: std::collections::HashMap::new(),
        };
        backfill_result_profiles(&engine, &response, req.mode_confirm).await;
        if let Some(op) = op {
            op.complete(response.count);
        }
        return Ok(Json(response));
    }

    // Single query path
    let mut query =
        SearchQuery::parse(&req.query).map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

    if let Some(limit) = req.limit {
        query.limit = Some(limit);
    }

    resolve_author(&mut query, &req, &engine)?;

    let mut response = engine
        .search_with_options(&query, policy, override_relays.as_deref(), req.mode_confirm)
        .await?;

    // Fan out to the people category: any free-text term also scans
    // local profiles, surfaced alongside content in `response.profiles`.
    if let Some(term) = query_free_text(&query) {
        response.profiles = engine.search_profiles(&term).await;
    }

    backfill_result_profiles(&engine, &response, req.mode_confirm).await;
    if let Some(op) = op {
        op.complete(response.count);
    }
    Ok(Json(response))
}

/// Cache kind-0 profiles for the authors of these search results.
/// Awaited — the profiles are fetched into nostrdb *before* the search
/// response returns, so the client's first render resolves author
/// metadata locally with no follow-up round-trip. The engine method
/// no-ops for authors already cached.
///
/// `mode_confirm` is the search request's own flag: a search the user
/// authorized to reach relays despite offline mode carries its profile
/// backfill along on the same okay.
async fn backfill_result_profiles(
    engine: &AppState,
    response: &SearchResponse,
    mode_confirm: bool,
) {
    let pubkeys: Vec<String> = response.results.iter().map(|r| r.author.clone()).collect();
    if pubkeys.is_empty() {
        return;
    }
    engine
        .backfill_missing_profiles(pubkeys, mode_confirm)
        .await;
}

/// The free-text component of a query — the keywords or exact phrase
/// profile search matches author names against. `None` for an
/// operator-only or semantic query (semantic fan-out stays
/// content-only — see `docs/search-architecture.org` §20).
fn query_free_text(q: &SearchQuery) -> Option<String> {
    if q.semantic_filter.is_some() {
        return None;
    }
    let term = match &q.text_filter {
        Some(TextFilter::Keywords(words)) => words.join(" "),
        Some(TextFilter::Exact(phrase)) => phrase.clone(),
        None => return None,
    };
    let term = term.trim();
    (!term.is_empty()).then(|| term.to_string())
}

/// Run profile search for each distinct term and merge the hits into one
/// list, de-duplicated by pubkey, sorted by match score. Used by the
/// compound (`|`) query path, where each OR-branch contributes a term.
async fn merge_profile_search(engine: &AppState, terms: &[String]) -> Vec<ProfileResult> {
    let mut merged: Vec<ProfileResult> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for term in terms {
        for p in engine.search_profiles(term).await {
            if seen.insert(p.pubkey.clone()) {
                merged.push(p);
            }
        }
    }
    merged.sort_by(|a, b| a.score.cmp(&b.score));
    merged
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
        Some(AuthorFilter::Name(partial)) => {
            let matches = resolve_pubkeys_by_name(engine.ndb(), partial);
            if matches.is_empty() {
                // No match: short-circuit by inserting an obviously
                // empty pubkey set. We could error instead, but a zero-
                // result query is the more discoverable feedback — the
                // user sees the search ran with no hits and can adjust
                // the partial, rather than getting a 400.
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![]));
            } else {
                query.author_filter = Some(AuthorFilter::Pubkeys(matches));
            }
        }
        _ => {}
    }
    Ok(())
}

/// Resolve `by:name:<partial>` into a concrete pubkey list.
///
/// A thin wrapper over `query::find_profiles_matching` — the same local
/// kind-0 scan that powers profile search — mapped down to pubkeys.
/// Matching the partial therefore also reaches nip05 / lud16 / website,
/// not just name / display_name.
fn resolve_pubkeys_by_name(ndb: &nostrdb::Ndb, partial: &str) -> Vec<String> {
    crate::query::find_profiles_matching(ndb, partial)
        .into_iter()
        .map(|p| p.pubkey)
        .collect()
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

    debug!(
        "List publications request: limit={}, policy={:?}, before={:?}",
        query.limit, policy, query.before
    );

    let pub_engine = PublicationEngine::new(&engine);
    let publications = pub_engine
        .list_root_publications(policy, query.limit, query.before)
        .await?;

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
                "section_count": p.section_count(),
                "relays": p.relays,
                "signed": p.signed,
                "forked": p.forked
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

/// Query parameters for `GET /publications/:pubkey/:d_tag`.
///
/// `depth` controls how many levels of nested 30040 indexes are eagerly
/// resolved. NKBIP-01 publications are hierarchical: an index can reference
/// other indexes. Depth 0 loads only this index and its own sections; depth N
/// recurses N levels of nesting (sections are leaves and never consume a
/// level). Defaults to 2 — enough to render a publication and one level of
/// its sub-publications without a click.
#[derive(Debug, Deserialize)]
pub struct GetPublicationQuery {
    pub policy: Option<String>,
    pub depth: Option<usize>,
}

/// Default eager-expansion depth for `get_publication_handler`.
const DEFAULT_PUBLICATION_DEPTH: usize = 2;
/// Upper bound on `depth` so a crafted request can't trigger an unbounded
/// recursive fetch. The recursive loader is cycle-guarded, but a legitimately
/// deep publication could still fan out a huge number of relay requests.
const MAX_PUBLICATION_DEPTH: usize = 12;

/// GET /api/v1/publications/:pubkey/:d_tag
///
/// Get a publication with its table of contents. The TOC is a recursive tree:
/// each entry carries `depth`, `is_publication` (30040 vs 30041), and — for
/// resolved sections within the depth horizon — `content`. Nested 30040 indexes
/// appear as entries the reader can refocus into.
pub async fn get_publication_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
    axum::extract::Query(query): axum::extract::Query<GetPublicationQuery>,
) -> Result<impl IntoResponse, EngineError> {
    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let depth = query
        .depth
        .unwrap_or(DEFAULT_PUBLICATION_DEPTH)
        .min(MAX_PUBLICATION_DEPTH);

    debug!(
        "Get publication request: {}:{} policy={:?} depth={}",
        params.pubkey, params.d_tag, policy, depth
    );

    // Validate hex pubkey format
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    // Recursive depth-N load: resolves nested 30040 indexes and fully loads
    // every section within `depth`. `build_toc` then recurses over the filled
    // `nested` tree, so the TOC is the indented N-level view.
    let publication = pub_engine
        .load_publication_tree(&addr, depth, policy)
        .await?;
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
            "depth": depth,
            "section_count": publication.sections.len()
        })),
    ))
}

/// GET /api/v1/publications/:pubkey/:d_tag/stream
///
/// SSE variant of `get_publication_handler`: instead of returning the whole
/// TOC in one response, it streams one `PubLoadEvent` per node as the
/// recursive loader resolves it, ending with a `done` event. The client builds
/// the tree incrementally and shows a true per-event load counter. Closing the
/// connection drops the channel receiver, which aborts the engine-side loader.
pub async fn stream_publication_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
    axum::extract::Query(query): axum::extract::Query<GetPublicationQuery>,
) -> Result<axum::response::Response, EngineError> {
    use axum::response::sse::{Event as SseHttpEvent, KeepAlive, Sse};
    use axum::response::IntoResponse;
    use futures::stream::StreamExt;

    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let depth = query
        .depth
        .unwrap_or(DEFAULT_PUBLICATION_DEPTH)
        .min(MAX_PUBLICATION_DEPTH);

    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }
    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    debug!(
        "Stream publication: {}:{} policy={:?} depth={}",
        params.pubkey, params.d_tag, policy, depth
    );

    // The loader runs in a spawned task and sends events into this channel.
    // When the SSE response is dropped (client disconnects) the unfold below
    // drops `rx`; the loader's `send`s then fail and the recursion unwinds.
    let (tx, rx) = tokio::sync::mpsc::channel::<crate::publication::PubLoadEvent>(64);
    let engine_for_task = engine.clone();
    tokio::spawn(async move {
        let pub_engine = PublicationEngine::new(&engine_for_task);
        pub_engine
            .stream_publication_tree(&addr, depth, policy, tx)
            .await;
    });

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(ev) => {
                let payload = serde_json::to_string(&ev).unwrap_or_default();
                let sse_event = SseHttpEvent::default().data(payload);
                Some((Ok::<SseHttpEvent, std::convert::Infallible>(sse_event), rx))
            }
            None => None,
        }
    });

    let mut resp = Sse::new(stream.boxed())
        .keep_alive(KeepAlive::default())
        .into_response();
    // See `fetch_events_handler` — `no-transform` stops intermediaries from
    // buffering the event stream.
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-transform"),
    );
    Ok(resp)
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

    let mut publication = pub_engine
        .load_publication(&addr, FetchPolicy::LocalFirst)
        .await?;
    let loaded_count = pub_engine
        .load_sections(&mut publication, FetchPolicy::LocalFirst)
        .await?;

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

    let section = publication
        .sections
        .get(params.index)
        .ok_or_else(|| EngineError::InvalidFilter("Section index out of bounds".into()))?;

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

    let addr = NAddr::new(
        crate::publication::KIND_PUBLICATION_SECTION,
        &params.pubkey,
        &params.d_tag,
    );
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

    let mut publication = pub_engine
        .load_publication(&addr, FetchPolicy::LocalOnly)
        .await?;
    pub_engine.load_sections(&mut publication, policy).await?;

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
            if !path.is_file() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let supported = [
                "pdf", "docx", "epub", "html", "htm", "txt", "md", "org", "adoc", "asciidoc", "rst",
            ];
            if !supported.contains(&ext.as_str()) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let modified = entry
                .metadata()
                .ok()
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
        b.get("modified")
            .and_then(|v| v.as_u64())
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
        return Err(EngineError::InvalidFilter(format!(
            "File not found: {}",
            req.filename
        )));
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

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| EngineError::Database(format!("Multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            filename = field.file_name().unwrap_or("upload").to_string();
            file_bytes = field
                .bytes()
                .await
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
    let content = event
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
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
    let _guard = crate::query::ndb_query_lock();
    let txn = nostrdb::Transaction::new(ndb).ok()?;
    let filter = nostrdb::FilterBuilder::new()
        .kinds([0])
        .authors([pubkey_bytes].iter())
        .limit(1)
        .build();
    let results = ndb.query(&txn, &[filter], 1).ok()?;
    let qr = results.first()?;
    let note = ndb.get_note_by_key(&txn, qr.note_key).ok()?;
    crate::query::note_to_json_pub(&note, &txn).ok()
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

/// POST /api/v1/profiles/fetch — batch-fetch profiles from relays.
///
/// By default only pubkeys not already in nostrdb are queried. Set
/// `force` to refetch every listed pubkey unconditionally — used by the
/// reader's "Refresh discussions" button and the inline profile-refresh
/// control to pick up renamed / newly-seen authors. A `force` fetch is
/// an explicit user action, so it also bypasses offline mode: the user
/// pressing refresh is the okay to reach relays.
#[derive(Debug, Deserialize)]
pub struct FetchProfilesRequest {
    pub pubkeys: Vec<String>,
    #[serde(default)]
    pub force: bool,
}

pub async fn fetch_profiles_handler(
    State(engine): State<AppState>,
    Json(req): Json<FetchProfilesRequest>,
) -> Result<Json<Value>, EngineError> {
    // Profiles can live on any configured relay — query the union of
    // every set so an empty `general` set can't silently disable this.
    let mut relays = engine.relay_config().all_urls();
    let mut fetched = 0;

    // When force is set, query every listed pubkey unconditionally so a
    // newer kind 0 supersedes the cached one. Otherwise only pubkeys
    // missing from nostrdb are queried — the default avoids hammering
    // relays on every reader-buffer open.
    let targets: Vec<&str> = req
        .pubkeys
        .iter()
        .filter(|pk| pk.len() == 64 && (req.force || query_profile(engine.ndb(), pk).is_none()))
        .map(|pk| pk.as_str())
        .collect();

    if targets.is_empty() {
        return Ok(Json(json!({ "fetched": 0, "total": req.pubkeys.len() })));
    }

    // A forced profile fetch is an explicit user action — gate it.
    // Declined → report nothing fetched (cached profiles are kept).
    let op = if req.force {
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Profile,
                format!(
                    "Fetch {} profile{}",
                    targets.len(),
                    if targets.len() == 1 { "" } else { "s" }
                ),
                describe_profile_steps(&targets),
                relays.clone(),
            )
            .await
        {
            Ok(o) => {
                relays = o.relays().to_vec();
                Some(o)
            }
            Err(_) => {
                return Ok(Json(json!({ "fetched": 0, "total": req.pubkeys.len() })));
            }
        }
    } else {
        None
    };

    // Batch fetch: one request per relay with ALL missing pubkeys
    let filter = json!({"kinds": [0], "authors": targets, "limit": targets.len()});
    for relay_url in &relays {
        match engine
            .tracked_fetch_with_options(
                relay_url,
                &[filter.clone()],
                FetchTrigger::ProfilePrefetch,
                req.force,
            )
            .await
        {
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

    if let Some(op) = op {
        op.complete(fetched);
    }
    Ok(Json(json!({
        "fetched": fetched,
        "total": req.pubkeys.len()
    })))
}

// ============================================================================
// Relay Config API Endpoints
// ============================================================================

/// Request to fetch from one or more relays in a single operation.
#[derive(Debug, Deserialize)]
pub struct FetchRelayRequest {
    /// Single relay (legacy single-target form). Prefer `relays`.
    #[serde(default)]
    pub relay: String,
    /// Relay set — fetched together under one confirm operation.
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<u64>,
    /// Pubkeys to fetch from (hex). Empty = no author filter.
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default = "default_fetch_limit")]
    pub limit: usize,
    /// Set true for explicit user-initiated fetches. In Confirm mode the
    /// engine gates the operation behind the confirm modal; background
    /// pollers must NOT set this.
    #[serde(default)]
    pub mode_confirm: bool,
    /// NIP-50 search string. When present, the REQ filter sent to the
    /// relay includes `"search": "<value>"` so NIP-50-supporting relays
    /// do free-text matching (typically against `content` for kind 1,
    /// and against profile name/display_name/nip05 for kind 0). Relays
    /// that don't support NIP-50 ignore the field per the spec.
    pub search: Option<String>,
}

fn default_fetch_limit() -> usize {
    200
}

/// POST /api/v1/fetch — fetch events from one or more relays
pub async fn fetch_relay_handler(
    State(engine): State<AppState>,
    Json(req): Json<FetchRelayRequest>,
) -> Result<Json<Value>, EngineError> {
    // Relay set: `relays` wins; fall back to the legacy single `relay`.
    let targets: Vec<String> = if !req.relays.is_empty() {
        req.relays.clone()
    } else if !req.relay.is_empty() {
        vec![req.relay.clone()]
    } else {
        return Err(EngineError::InvalidFilter("no relay specified".into()));
    };

    debug!(
        "Fetch from {} relay(s) kinds={:?} authors={} limit={} mode_confirm={}",
        targets.len(),
        req.kinds,
        req.authors.len(),
        req.limit,
        req.mode_confirm
    );

    let mut filter = json!({"limit": req.limit});
    if !req.kinds.is_empty() {
        filter["kinds"] = json!(req.kinds);
    }
    if !req.authors.is_empty() {
        filter["authors"] = json!(req.authors);
    }
    if let Some(s) = req.search.as_deref() {
        if !s.is_empty() {
            filter["search"] = json!(s);
        }
    }

    // One confirm operation covers the whole relay set — gate once.
    let op = if req.mode_confirm {
        let mut steps = Vec::new();
        if !req.kinds.is_empty() {
            steps.push(format!("kinds {:?}", req.kinds));
        }
        if !req.authors.is_empty() {
            steps.push(format!("{} author(s)", req.authors.len()));
        }
        if let Some(s) = req.search.as_deref().filter(|s| !s.is_empty()) {
            steps.push(format!("NIP-50 search: {s}"));
        }
        steps.push(format!("limit {}", req.limit));
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Custom,
                format!("Fetch from {} relay(s)", targets.len()),
                steps,
                targets.clone(),
            )
            .await
        {
            Ok(o) => Some(o),
            Err(_) => {
                return Ok(Json(json!({
                    "fetched": 0,
                    "relays": targets,
                    "kinds": req.kinds
                })));
            }
        }
    } else {
        None
    };

    let fetch_relays: Vec<String> = match &op {
        Some(o) => o.relays().to_vec(),
        None => targets,
    };

    let mut count = 0usize;
    for relay_url in &fetch_relays {
        match engine
            .tracked_fetch_with_options(
                relay_url,
                std::slice::from_ref(&filter),
                FetchTrigger::UserAction,
                req.mode_confirm,
            )
            .await
        {
            Ok(events) => count += events.len(),
            Err(e) => debug!("Fetch from {} failed: {}", relay_url, e),
        }
    }
    debug!("Fetched {} events from {} relay(s)", count, fetch_relays.len());
    if let Some(op) = op {
        op.complete(count);
    }

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
        "relays": fetch_relays,
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

        match engine
            .tracked_fetch(relay_url, &[filter], FetchTrigger::UserAction)
            .await
        {
            Ok(events) => {
                debug!(
                    "Fetched {} events for authors from {}",
                    events.len(),
                    relay_url
                );
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
pub async fn network_status_handler(State(engine): State<AppState>) -> Json<Value> {
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
    let mode: NetworkMode = req
        .mode
        .parse()
        .map_err(|e: String| EngineError::InvalidFilter(e))?;

    engine.set_network_mode(mode);

    // Persist to config.toml in a blocking task to avoid stalling the runtime
    if let Some(config_path) = engine.config_path() {
        let config_path = config_path.to_path_buf();
        let mode_str = mode.to_string();
        tokio::task::spawn_blocking(move || {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(mut doc) = content.parse::<toml::Table>() {
                    let network = doc
                        .entry("network")
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

/// GET /api/v1/network/fetch-events
///
/// Long-lived SSE stream of fetch-operation events (`intent`,
/// `progress`, `completed`, `failed`). In Confirm mode an `intent`
/// with `needs_confirmation` must be answered via `/network/fetch-confirm`.
pub async fn fetch_events_handler(State(engine): State<AppState>) -> axum::response::Response {
    use axum::response::sse::{Event as SseHttpEvent, KeepAlive, Sse};
    use axum::response::IntoResponse;
    use futures::stream::StreamExt;
    use tokio::sync::broadcast::error::RecvError;

    let rx = engine.subscribe_fetch_events();
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_default();
                    let sse_event = SseHttpEvent::default().data(payload);
                    return Some((Ok::<SseHttpEvent, std::convert::Infallible>(sse_event), rx));
                }
                // Subscriber fell behind — skip dropped events and keep going.
                Err(RecvError::Lagged(_)) => continue,
                // No more senders — end the stream.
                Err(RecvError::Closed) => return None,
            }
        }
    });

    let mut resp = Sse::new(stream.boxed())
        .keep_alive(KeepAlive::default())
        .into_response();
    // `no-transform` tells intermediaries (vite preview's compression
    // middleware, nginx, …) not to buffer/compress the stream — without
    // it the event-stream is held until the connection closes and the
    // browser's EventSource never sees an event.
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-transform"),
    );
    resp
}

#[derive(Debug, Serialize)]
pub struct FetchConfirmResponse {
    pub resolved: bool,
}

/// POST /api/v1/network/fetch-confirm
///
/// The UI's reply to a confirm intent — body is a `ConfirmDecision`
/// (`operation_id`, `approved`, optional `relays` override).
pub async fn fetch_confirm_handler(
    State(engine): State<AppState>,
    Json(decision): Json<crate::network::ConfirmDecision>,
) -> Json<FetchConfirmResponse> {
    let resolved = engine.resolve_fetch_confirm(decision).await;
    Json(FetchConfirmResponse { resolved })
}

// ============================================================================
// Relay Config API Endpoints
// ============================================================================

/// Request to add relay/author to config
#[derive(Debug, Deserialize)]
pub struct ConfigUpdateRequest {
    /// Add a relay URL to a set ("general", "publish", "fetch")
    pub add_relay: Option<AddRelay>,
    /// Remove a relay URL from a set ("general", "publish", "fetch")
    pub remove_relay: Option<AddRelay>,
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

/// POST /api/v1/config/update — apply UI-driven config edits.
///
/// Relay add/remove writes through to `<data_dir>/relays.json` (the
/// live working sets — never to `config.toml`, which carries only the
/// bootstrap `initial_relays` seed). Author add/remove still mutates
/// `config.toml` because the author list hasn't been migrated to a
/// state file yet.
///
/// Relay changes still require a restart to take effect for the running
/// engine instance — the in-memory copy is loaded once at startup.
pub async fn config_update_handler(
    State(engine): State<AppState>,
    Json(req): Json<ConfigUpdateRequest>,
) -> Result<Json<Value>, EngineError> {
    let mut changed = false;

    // Relay edits route through Engine::add_relay / remove_relay, which
    // mutate the live in-memory RelayConfig AND write through to
    // <data_dir>/relays.json — so edits take effect immediately for the
    // running engine instance, no restart needed.
    if let Some(add) = &req.add_relay {
        if engine.add_relay(&add.set, &add.url) {
            changed = true;
        }
    }
    if let Some(rm) = &req.remove_relay {
        if engine.remove_relay(&rm.set, &rm.url) {
            changed = true;
        }
    }

    // Author edits still flow to config.toml — they're a separate concern
    // and not part of this migration.
    if req.add_author.is_some() || req.remove_author.is_some() {
        let config_path = engine.config_path().ok_or_else(|| {
            EngineError::Config("No config file path set (use -c config.toml)".into())
        })?;

        let content = std::fs::read_to_string(config_path)
            .map_err(|e| EngineError::Config(format!("Failed to read config: {e}")))?;
        let mut doc: toml::Table = toml::from_str(&content)
            .map_err(|e| EngineError::Config(format!("Failed to parse config: {e}")))?;

        if let Some(author) = &req.add_author {
            let relay = doc
                .entry("relay")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()));
            if let toml::Value::Table(relay_table) = relay {
                let authors = relay_table
                    .entry("authors")
                    .or_insert_with(|| toml::Value::Array(Vec::new()));
                if let toml::Value::Array(arr) = authors {
                    let val = toml::Value::String(author.clone());
                    if !arr.contains(&val) {
                        arr.push(val);
                        changed = true;
                    }
                }
            }
        }

        if let Some(author) = &req.remove_author {
            if let Some(toml::Value::Table(relay_table)) = doc.get_mut("relay") {
                if let Some(toml::Value::Array(arr)) = relay_table.get_mut("authors") {
                    let before = arr.len();
                    arr.retain(|v| v.as_str() != Some(author));
                    if arr.len() != before {
                        changed = true;
                    }
                }
            }
        }

        if changed {
            let output = toml::to_string_pretty(&doc)
                .map_err(|e| EngineError::Config(format!("Failed to serialize config: {e}")))?;
            std::fs::write(config_path, &output)
                .map_err(|e| EngineError::Config(format!("Failed to write config: {e}")))?;
        }
    }

    Ok(Json(json!({
        "updated": changed,
        "message": if changed { "Config updated. Restart to apply relay changes." } else { "No changes needed." }
    })))
}

/// POST /api/v1/config/snapshot — write the live engine relay sets into
/// config.toml's `[relay] initial_relays` as a portable bootstrap seed.
///
/// Snapshotting is **only** for portability (moving config between
/// machines, sharing a starter config, restoring after a wipe). At
/// runtime, `<data_dir>/relays.json` is the source of truth for the
/// engine's working sets; this just freezes a copy into TOML so a
/// fresh boot with no relays.json file can seed from it.
///
/// The captured value is the **union** of general / fetch / publish
/// URLs — `initial_relays` seeds all three sets identically on first
/// boot, so the read/write distinction lives in relays.json, not here.
#[derive(Debug, Deserialize, Default)]
pub struct ConfigSnapshotRequest {
    /// Include the live relay set as `[relay] initial_relays` (default true).
    #[serde(default = "default_include_relays")]
    pub include_relays: bool,
    /// Optional editor settings — when present, written to `[editor]`.
    #[serde(default)]
    pub editor: Option<crate::config::EditorConfig>,
    /// Optional compose settings — when present, written to `[compose]`.
    #[serde(default)]
    pub compose: Option<crate::config::ComposeConfig>,
    /// Optional network default — when present, written to `[network] mode`.
    #[serde(default)]
    pub network_mode: Option<String>,
}

fn default_include_relays() -> bool {
    true
}

pub async fn config_snapshot_handler(
    State(engine): State<AppState>,
    body: Option<Json<ConfigSnapshotRequest>>,
) -> Result<Json<Value>, EngineError> {
    let req = body.map(|Json(r)| r).unwrap_or_default();

    let config_path = engine
        .config_path()
        .ok_or_else(|| EngineError::Config("No config file path set (use -c config.toml)".into()))?
        .to_path_buf();

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| EngineError::Config(format!("Failed to read config: {e}")))?;
    let mut doc: toml::Table = toml::from_str(&content)
        .map_err(|e| EngineError::Config(format!("Failed to parse config: {e}")))?;

    let mut wrote: Vec<&'static str> = Vec::new();
    let mut relay_count = 0usize;

    if req.include_relays {
        let rc = engine.relay_config();
        let mut seen = std::collections::HashSet::new();
        let mut urls: Vec<String> = Vec::new();
        for u in rc.fetch.urls.iter().chain(&rc.publish.urls).chain(&rc.general.urls) {
            if seen.insert(u.clone()) {
                urls.push(u.clone());
            }
        }
        relay_count = urls.len();
        let relay = doc
            .entry("relay")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(relay_table) = relay else {
            return Err(EngineError::Config(
                "[relay] in config.toml is not a table".into(),
            ));
        };
        let arr: Vec<toml::Value> = urls.iter().map(|u| toml::Value::String(u.clone())).collect();
        relay_table.insert("initial_relays".to_string(), toml::Value::Array(arr));
        wrote.push("initial_relays");
    }

    if let Some(editor) = &req.editor {
        let mut t = toml::Table::new();
        t.insert("line_numbers".into(), toml::Value::Boolean(editor.line_numbers));
        t.insert("vim_mode".into(), toml::Value::Boolean(editor.vim_mode));
        t.insert("insert_mode".into(), toml::Value::String(editor.insert_mode.clone()));
        doc.insert("editor".into(), toml::Value::Table(t));
        wrote.push("editor");
    }

    if let Some(compose) = &req.compose {
        let mut t = toml::Table::new();
        t.insert("default_mode".into(), toml::Value::String(compose.default_mode.clone()));
        t.insert("sync_mode".into(), toml::Value::String(compose.sync_mode.clone()));
        t.insert("button_labels".into(), toml::Value::String(compose.button_labels.clone()));
        doc.insert("compose".into(), toml::Value::Table(t));
        wrote.push("compose");
    }

    if let Some(mode) = &req.network_mode {
        let network = doc
            .entry("network")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(t) = network {
            t.insert("mode".into(), toml::Value::String(mode.clone()));
            wrote.push("network");
        }
    }

    if wrote.is_empty() {
        return Ok(Json(json!({
            "updated": false,
            "message": "Nothing to snapshot — pass at least one of include_relays/editor/compose/network_mode."
        })));
    }

    let output = toml::to_string_pretty(&doc)
        .map_err(|e| EngineError::Config(format!("Failed to serialize config: {e}")))?;
    std::fs::write(&config_path, &output)
        .map_err(|e| EngineError::Config(format!("Failed to write config: {e}")))?;

    Ok(Json(json!({
        "updated": true,
        "wrote": wrote,
        "relay_count": relay_count,
        "path": config_path.display().to_string(),
        "message": format!("Saved settings ({}) to {}", wrote.join(", "), config_path.display()),
    })))
}

/// GET /api/v1/settings — return editor/compose/network defaults from the
/// current config.toml so the web can hydrate state at boot instead of
/// starting on hard-coded defaults that diverge from the user's last save.
pub async fn settings_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let config_path = engine.config_path();
    let cfg = match config_path {
        Some(p) => crate::config::Config::from_file(p).unwrap_or_default(),
        None => crate::config::Config::default(),
    };
    Ok(Json(json!({
        "editor": {
            "line_numbers": cfg.editor.line_numbers,
            "vim_mode": cfg.editor.vim_mode,
            "insert_mode": cfg.editor.insert_mode,
        },
        "compose": {
            "default_mode": cfg.compose.default_mode,
            "sync_mode": cfg.compose.sync_mode,
            "button_labels": cfg.compose.button_labels,
        },
        "network": {
            "mode": cfg.network.mode,
        },
    })))
}

/// GET /api/v1/relays — get relay configuration
pub async fn relay_config_handler(State(engine): State<AppState>) -> Json<Value> {
    let rc = engine.relay_config();
    Json(json!({
        "general": { "urls": rc.general.urls, "kinds": rc.general.kinds },
        "publish": { "urls": rc.publish.urls, "kinds": rc.publish.kinds },
        "fetch": { "urls": rc.fetch.urls, "kinds": rc.fetch.kinds },
        "authors": rc.authors_hex(),
        "initial_relays": rc.initial_relays,
    }))
}

#[derive(Debug, Deserialize)]
pub struct RelayInfoQuery {
    pub url: String,
    /// `?refresh=true` bypasses the cache and forces a fresh fetch —
    /// the UI's retry button after a transient failure.
    #[serde(default)]
    pub refresh: bool,
}

/// GET /api/v1/relay/info?url=wss://… — return cached NIP-11 doc
/// for the relay (or kick off a fetch if missing/stale and return
/// `Loading`). See `docs/relay-classes-and-info-port.md` §4 for the
/// caching contract.
pub async fn relay_nip11_handler(
    State(engine): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RelayInfoQuery>,
) -> Json<Value> {
    let status = if q.refresh {
        engine.nip11_cache().refresh(&q.url).await
    } else {
        engine.nip11_cache().get(&q.url).await
    };
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
// Discussion counts (NIP-22 comments + NIP-84 highlights)
// ============================================================================

/// Request body for `POST /api/v1/discussions/counts`.
///
/// `addresses` are NIP-01 `a` tag values in `kind:pubkey:d-tag` form.
/// The handler returns, per address, how many kind-1111 comments and
/// kind-9802 highlights reference it.
#[derive(Debug, Deserialize)]
pub struct DiscussionCountsRequest {
    pub addresses: Vec<String>,
    #[serde(default)]
    pub policy: Option<String>,
    pub relays: Option<Vec<String>>,
    /// Bypass the offline-mode policy downgrade. Set true for explicit,
    /// user-initiated refreshes ("Refresh discussions") so the user can
    /// pull comments and highlights into the local DB without having to
    /// flip the global network mode online first.
    #[serde(default)]
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct DiscussionCount {
    pub comments: usize,
    pub highlights: usize,
}

#[derive(Debug, Serialize)]
pub struct DiscussionCountsResponse {
    pub counts: std::collections::HashMap<String, DiscussionCount>,
    pub source: crate::engine::QuerySource,
}

/// POST /api/v1/discussions/counts
///
/// Aggregate NIP-22 (kind 1111) comment counts and NIP-84 (kind 9802)
/// highlight counts for a batch of addressable events. Used by the
/// reader to show "this section has discussions" indicators without
/// fetching the events themselves.
pub async fn discussion_counts_handler(
    State(engine): State<AppState>,
    Json(mut req): Json<DiscussionCountsRequest>,
) -> Result<Json<DiscussionCountsResponse>, EngineError> {
    let mut policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    // De-duplicate and validate addresses up front so we never ask a
    // relay for the same `a` tag twice.
    let mut seen = std::collections::HashSet::new();
    let addresses: Vec<String> = req
        .addresses
        .into_iter()
        .filter(|a| !a.is_empty() && seen.insert(a.clone()))
        .collect();

    if addresses.is_empty() {
        return Ok(Json(DiscussionCountsResponse {
            counts: std::collections::HashMap::new(),
            source: crate::engine::QuerySource {
                local_count: 0,
                relay_count: 0,
            },
        }));
    }

    debug!(
        "Discussion counts: {} addresses, policy={:?}",
        addresses.len(),
        policy
    );

    // A discussion-counts refresh is a user-initiated fetch operation —
    // gate it. Declined → fall back to a local-only count.
    let mut op: Option<crate::network::FetchOperation> = None;
    if req.mode_confirm {
        let proposed = req
            .relays
            .clone()
            .unwrap_or_else(|| engine.relay_config().all_urls());
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Thread,
                format!("Refresh discussion counts ({} target(s))", addresses.len()),
                describe_discussion_steps(&[1111, 9802], &addresses, &[]),
                proposed,
            )
            .await
        {
            Ok(o) => {
                req.relays = Some(o.relays().to_vec());
                op = Some(o);
            }
            Err(_) => {
                req.mode_confirm = false;
                policy = FetchPolicy::LocalOnly;
            }
        }
    }

    // One combined relay REQ for both kinds keeps roundtrips minimal.
    let filters = vec![json!({
        "kinds": [1111, 9802],
        "#a": addresses,
    })];

    let response = engine
        .get_events_with_options(filters, policy, req.relays.as_deref(), req.mode_confirm)
        .await?;
    let fetched = response.events.len();

    let address_set: std::collections::HashSet<&str> =
        addresses.iter().map(String::as_str).collect();
    let mut counts: std::collections::HashMap<String, DiscussionCount> = addresses
        .iter()
        .map(|a| (a.clone(), DiscussionCount::default()))
        .collect();

    for event in &response.events {
        let kind = event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let Some(tags) = event.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
        // The same comment can carry both `a` (parent) and `A` (root) tags
        // pointing to the same addr. We bump the counter once per (event,
        // address) by collecting matched addresses first.
        let mut matched: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for tag in tags {
            let arr = match tag.as_array() {
                Some(a) if a.len() >= 2 => a,
                _ => continue,
            };
            let key = arr[0].as_str().unwrap_or("");
            if !matches!(key, "a" | "A") {
                continue;
            }
            if let Some(val) = arr[1].as_str() {
                if let Some(&hit) = address_set.get(val) {
                    matched.insert(hit);
                }
            }
        }
        for addr in matched {
            let entry = counts.entry(addr.to_string()).or_default();
            match kind {
                1111 => entry.comments += 1,
                9802 => entry.highlights += 1,
                _ => {}
            }
        }
    }

    if let Some(op) = op {
        op.complete(fetched);
    }
    Ok(Json(DiscussionCountsResponse {
        counts,
        source: response.source,
    }))
}

/// Request body for `POST /api/v1/discussions/list`.
///
/// The address / event-id sets travel in a JSON body, not the URL: a
/// deep publication tree can reference hundreds of section coordinates,
/// which overflows the request-line/header limit of a GET (HTTP 431).
/// `kinds` empty/missing is treated as `[1111, 9802]`.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct DiscussionsListRequest {
    pub addresses: Vec<String>,
    pub event_ids: Vec<String>,
    pub kinds: Vec<u64>,
    pub policy: Option<String>,
    pub relays: Vec<String>,
    pub limit: Option<usize>,
    pub since: Option<i64>,
    pub mode_confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscussionsListResponse {
    pub events: Vec<Value>,
    pub source: crate::engine::QuerySource,
    /// Server's view of when the result was computed (unix seconds).
    /// The web uses this as a `since` cursor for incremental refreshes.
    pub refreshed_at: i64,
}

/// A bare 32-byte event id: exactly 64 hex chars.
fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// An addressable coordinate: `kind:pubkey:d_tag` (d_tag may be empty).
fn is_addr_coord(s: &str) -> bool {
    let mut p = s.splitn(3, ':');
    matches!(
        (p.next(), p.next(), p.next()),
        (Some(kind), Some(pk), Some(_))
            if !kind.is_empty() && kind.bytes().all(|b| b.is_ascii_digit()) && is_hex64(pk)
    )
}

/// `e0557f939c5e…` — a hex id truncated for display.
fn short_id(s: &str) -> String {
    if s.len() > 12 {
        format!("{}…", &s[..12])
    } else {
        s.to_string()
    }
}

/// `30023:52b4a076…:ab008d4c` — an addressable coord with a short pubkey.
fn short_addr(s: &str) -> String {
    let p: Vec<&str> = s.splitn(3, ':').collect();
    if p.len() == 3 {
        let pk = if p[1].len() > 8 {
            format!("{}…", &p[1][..8])
        } else {
            p[1].to_string()
        };
        format!("{}:{}:{}", p[0], pk, p[2])
    } else {
        s.to_string()
    }
}

/// Confirm-modal step descriptions for a discussions fetch, derived from
/// the actual request — which kinds, against which targets — rather than
/// a canned template. The modal shows the user exactly what this call
/// will ask the relays for.
fn describe_discussion_steps(
    kinds: &[u64],
    addresses: &[String],
    event_ids: &[String],
) -> Vec<String> {
    // Name the kinds being requested, deduped, in request order.
    let mut what: Vec<&str> = Vec::new();
    for k in kinds {
        let name = match k {
            1111 => "comments",
            9802 => "highlights",
            _ => "events",
        };
        if !what.contains(&name) {
            what.push(name);
        }
    }
    let what = what.join(" & ");
    let kinds_str = kinds
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let kind_word = if kinds.len() > 1 { "kinds" } else { "kind" };

    let mut steps = Vec::new();
    if !event_ids.is_empty() {
        let targets: Vec<String> = event_ids.iter().map(|s| short_id(s)).collect();
        steps.push(format!(
            "Fetch {what} ({kind_word} {kinds_str}) replying to {}",
            targets.join(", ")
        ));
    }
    if !addresses.is_empty() {
        let targets: Vec<String> = addresses.iter().map(|s| short_addr(s)).collect();
        steps.push(format!(
            "Fetch {what} ({kind_word} {kinds_str}) referencing {}",
            targets.join(", ")
        ));
    }
    steps
}

/// Confirm-modal steps for a relay search, derived from the parsed
/// query — which kinds, author scope, and tag filters it carries —
/// rather than a fixed template.
fn describe_search_steps(query: &str) -> Vec<String> {
    let mut steps = Vec::new();
    if let Ok(q) = SearchQuery::parse(query) {
        if let Some(kinds) = q.kind_filter.as_ref().filter(|k| !k.is_empty()) {
            let ks = kinds
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let word = if kinds.len() > 1 { "kinds" } else { "kind" };
            steps.push(format!("Match {word} {ks}"));
        }
        if q.author_filter.is_some() {
            steps.push("Scoped to a specific author".to_string());
        }
        if !q.tag_filters.is_empty() {
            steps.push(format!("Filter by {} tag(s)", q.tag_filters.len()));
        }
    }
    steps.push("Query the relays (NIP-01 / NIP-50) and ingest matches".to_string());
    steps.push("Backfill author profiles (kind 0)".to_string());
    steps
}

/// Confirm-modal steps for a forced profile fetch — names the pubkeys
/// being requested (truncated, with an overflow count).
fn describe_profile_steps(pubkeys: &[&str]) -> Vec<String> {
    let shown: Vec<String> = pubkeys.iter().take(5).map(|p| short_id(p)).collect();
    let mut step = format!("Fetch kind-0 profile metadata for {}", shown.join(", "));
    if pubkeys.len() > 5 {
        step.push_str(&format!(" +{} more", pubkeys.len() - 5));
    }
    vec![step]
}

/// POST /api/v1/discussions/list
///
/// Returns the full set of NIP-22 (kind 1111) comments and NIP-84 (kind
/// 9802) highlights referencing the requested addresses or event ids.
/// Unlike `discussions/counts`, this endpoint returns whole events so
/// the web can thread them, render comment bodies, and overlay
/// highlights. The same call also warms nostrdb when `policy` admits
/// relay fetches, so subsequent `discussions/counts` queries hit local.
pub async fn discussions_list_handler(
    State(engine): State<AppState>,
    Json(mut req): Json<DiscussionsListRequest>,
) -> Result<Json<DiscussionsListResponse>, EngineError> {
    // Drop malformed refs. A relay URL or other junk in `#e`/`#a` is
    // dropped during nostr-filter parsing, degenerating the query into an
    // unconstrained `kinds:[1111]` dump of every comment on the network.
    // Validate here so a bad ref yields an empty result, not 500 events.
    let mut addresses = std::mem::take(&mut req.addresses);
    let mut event_ids = std::mem::take(&mut req.event_ids);
    let raw_addr = addresses.len();
    let raw_eids = event_ids.len();
    addresses.retain(|s| is_addr_coord(s));
    event_ids.retain(|s| is_hex64(s));
    if addresses.len() != raw_addr || event_ids.len() != raw_eids {
        warn!(
            "Discussions list: dropped {} malformed address(es) and {} malformed event id(s)",
            raw_addr - addresses.len(),
            raw_eids - event_ids.len()
        );
    }

    let kinds: Vec<u64> = if req.kinds.is_empty() {
        vec![1111, 9802]
    } else {
        std::mem::take(&mut req.kinds)
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if (addresses.is_empty() && event_ids.is_empty()) || kinds.is_empty() {
        return Ok(Json(DiscussionsListResponse {
            events: vec![],
            source: crate::engine::QuerySource {
                local_count: 0,
                relay_count: 0,
            },
            refreshed_at: now,
        }));
    }

    let mut policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };
    let limit = req.limit.unwrap_or(500);

    // A discussions pull is a user-initiated fetch operation — gate it.
    // In Auto mode this returns at once; in Confirm mode it blocks for
    // the modal. Declined → fall back to a local-only read.
    let relays_vec = std::mem::take(&mut req.relays);
    let mut op: Option<crate::network::FetchOperation> = None;
    if req.mode_confirm {
        let label = if event_ids.is_empty() {
            format!("Fetch discussions for {} target(s)", addresses.len())
        } else {
            "Pull comment thread".to_string()
        };
        let proposed = if relays_vec.is_empty() {
            engine.relay_config().all_urls()
        } else {
            relays_vec.clone()
        };
        match engine
            .begin_fetch_operation(
                crate::network::FetchPattern::Thread,
                label,
                describe_discussion_steps(&kinds, &addresses, &event_ids),
                proposed,
            )
            .await
        {
            Ok(o) => op = Some(o),
            Err(_) => {
                req.mode_confirm = false;
                policy = FetchPolicy::LocalOnly;
            }
        }
    }

    // Build the filter set. NIP-01 ANDs inside a filter and ORs across
    // filters, so to catch every referencing form we emit:
    //   - `#a` for top-level + addressable-parent refs
    //   - `#A` for nested replies (root scope only carries uppercase A)
    //   - `#e` when the caller passed event ids directly
    let mut filters: Vec<Value> = Vec::new();

    let make_base = || {
        let mut f = serde_json::Map::new();
        f.insert("kinds".to_string(), json!(kinds));
        f.insert("limit".to_string(), json!(limit));
        if let Some(since) = req.since {
            f.insert("since".to_string(), json!(since));
        }
        f
    };

    if !addresses.is_empty() {
        let mut lower = make_base();
        lower.insert("#a".to_string(), json!(addresses));
        filters.push(Value::Object(lower));

        let mut upper = make_base();
        upper.insert("#A".to_string(), json!(addresses));
        filters.push(Value::Object(upper));
    }
    if !event_ids.is_empty() {
        let mut by_e = make_base();
        by_e.insert("#e".to_string(), json!(event_ids));
        filters.push(Value::Object(by_e));
    }

    // The confirm modal can override the relay set; otherwise use the
    // request's relays (or the engine default, resolved inside the engine).
    let chosen_relays: Vec<String> = match &op {
        Some(o) => o.relays().to_vec(),
        None => relays_vec.clone(),
    };
    let relays_opt: Option<&[String]> = if chosen_relays.is_empty() {
        None
    } else {
        Some(chosen_relays.as_slice())
    };

    debug!(
        "Discussions list: {} addresses, {} event_ids, kinds={:?}, policy={:?}",
        addresses.len(),
        event_ids.len(),
        kinds,
        policy
    );

    let response = engine
        .get_events_with_options(filters, policy, relays_opt, req.mode_confirm)
        .await?;

    // The `#a` and `#A` filters overlap — a comment that tags the
    // address as both parent and root matches both, so it comes back
    // twice. Dedup by id before returning.
    let mut seen = std::collections::HashSet::new();
    let events: Vec<Value> = response
        .events
        .into_iter()
        .filter(|e| {
            e.get("id")
                .and_then(|v| v.as_str())
                .map(|id| seen.insert(id.to_string()))
                .unwrap_or(true)
        })
        .collect();

    if let Some(op) = op {
        op.complete(events.len());
    }
    Ok(Json(DiscussionsListResponse {
        events,
        source: response.source,
        refreshed_at: now,
    }))
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
    /// Reuse this index `d` tag instead of minting a fresh nanoid — set on
    /// republish so the 30040 replaces the existing one rather than forking.
    #[serde(default)]
    pub d_tag: Option<String>,
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
    /// Heading depth (2 = top-level `==` section, 3+ = nested). Drives the
    /// nested 30040/30041 emitter; absent/`2` keeps the flat graph.
    #[serde(default)]
    pub level: Option<u8>,
    /// Reuse this section `d` tag instead of minting — set on republish for
    /// sections matched (by `T`) to an existing publication, so the 30041
    /// replaces rather than forks.
    #[serde(default)]
    pub d_tag: Option<String>,
}

/// Response from publish endpoint
#[derive(Debug, Serialize)]
pub struct PublishResponse {
    pub publication_id: String,
    pub section_ids: Vec<String>,
    pub signed: bool,
    pub ingested: bool,
    pub broadcast_results: Option<Vec<BroadcastResult>>,
    /// Full event JSON (index first, then sections) so the client can
    /// offer a per-event / expand-all JSON inspector without refetching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<Value>>,
}

#[derive(Debug, Serialize)]
pub struct BroadcastResult {
    pub relay: String,
    pub success: bool,
    pub message: Option<String>,
    /// Event this result is for, so the client can group results into a
    /// per-event × per-relay grid instead of a flat list.
    pub event_id: String,
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
        EngineError::Config(
            "Publishing requires identity login or [identity] pubkey in config".into(),
        )
    })?;

    // Map request to ComposeState
    use crate::tree::state::TagEntry;
    let mut compose = ComposeState::new();
    compose.title = req.title;
    for (name, value) in &req.tags {
        compose.tags.push(TagEntry {
            name: name.clone(),
            value: value.clone(),
        });
    }
    compose.sections = req
        .sections
        .iter()
        .map(|s| {
            let mut sc = SectionCompose::default();
            sc.title = s.title.clone();
            sc.content = s.content.clone();
            sc.level = s.level.unwrap_or(2);
            sc.d_tag = s.d_tag.clone();
            sc.tags = s
                .tags
                .iter()
                .map(|(n, v)| TagEntry {
                    name: n.clone(),
                    value: v.clone(),
                })
                .collect();
            sc
        })
        .collect();
    compose.d_tag = req.d_tag.clone();

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
            &mut compose,
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
        let events = build_publication_events(&mut compose, &pubkey);
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
        debug!(
            "Publication {} was not persisted by nostrdb after ingest",
            pub_id
        );
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

        let (_, _, results) = crate::relay::publish_events_to_relays(&relays, &event_jsons).await;

        // Record relay provenance for every (event, relay) pair that
        // succeeded, so a freshly-published publication stops showing as
        // local-only without waiting to be re-fetched.
        let by_id: std::collections::HashMap<&str, &String> = section_events
            .iter()
            .chain(std::iter::once(&pub_event))
            .zip(event_jsons.iter())
            .filter_map(|(e, j)| e.get("id").and_then(|v| v.as_str()).map(|id| (id, j)))
            .collect();
        for r in &results {
            if r.success {
                if let Some(j) = by_id.get(r.event_id.as_str()) {
                    if let Err(e) = engine.record_event_relay(j, &r.relay_url) {
                        debug!("record relay metadata: {e}");
                    }
                }
            }
        }

        Some(
            results
                .into_iter()
                .map(|r| BroadcastResult {
                    relay: r.relay_url,
                    success: r.success,
                    message: r.message,
                    event_id: r.event_id,
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

    // Index first, then sections — the order the inspector lists them.
    let all_events: Vec<Value> = std::iter::once(pub_event.clone())
        .chain(section_events.iter().cloned())
        .collect();

    Ok(Json(PublishResponse {
        publication_id: pub_id,
        section_ids,
        signed: req.sign,
        ingested,
        broadcast_results,
        events: Some(all_events),
    }))
}

/// POST /api/v1/publish/preview — build the unsigned event graph for a
/// compose and return it without signing, ingesting, or broadcasting.
/// Feeds the composer's "inspect events as JSON" modal so the user can
/// see the exact 30040/30041 shape (nesting, d/T/title, a-tag wiring)
/// before publishing.
pub async fn publish_preview_handler(
    State(engine): State<AppState>,
    Extension(identity): Extension<IdentityAppState>,
    Json(req): Json<PublishRequest>,
) -> Result<Json<Value>, EngineError> {
    let pubkey = {
        let session = identity.lock().unwrap();
        session.pubkey().map(|s| s.to_string())
    }
    .or_else(|| engine.my_pubkey().map(|s| s.to_string()))
    .unwrap_or_else(|| "<preview>".to_string());

    use crate::tree::state::TagEntry;
    let mut compose = ComposeState::new();
    compose.title = req.title;
    for (name, value) in &req.tags {
        compose.tags.push(TagEntry {
            name: name.clone(),
            value: value.clone(),
        });
    }
    compose.sections = req
        .sections
        .iter()
        .map(|s| {
            let mut sc = SectionCompose::default();
            sc.title = s.title.clone();
            sc.content = s.content.clone();
            sc.level = s.level.unwrap_or(2);
            sc.d_tag = s.d_tag.clone();
            sc.tags = s
                .tags
                .iter()
                .map(|(n, v)| TagEntry {
                    name: n.clone(),
                    value: v.clone(),
                })
                .collect();
            sc
        })
        .collect();
    compose.d_tag = req.d_tag.clone();

    let (pub_event, section_events) = build_publication_events(&mut compose, &pubkey);
    let events: Vec<Value> = std::iter::once(pub_event)
        .chain(section_events.into_iter())
        .collect();
    Ok(Json(json!({ "events": events })))
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
            PublishBlockKind::Editable { content } => BlockKind::Editable { content, cursor: 0 },
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
        let (_, _, results) = crate::relay::publish_events_to_relays(&relays, &event_jsons).await;

        // Record relay provenance for each (event, relay) pair that succeeded.
        let by_id: std::collections::HashMap<&str, &String> = section_events
            .iter()
            .chain(std::iter::once(&pub_event))
            .zip(event_jsons.iter())
            .filter_map(|(e, j)| e.get("id").and_then(|v| v.as_str()).map(|id| (id, j)))
            .collect();
        for r in &results {
            if r.success {
                if let Some(j) = by_id.get(r.event_id.as_str()) {
                    if let Err(e) = engine.record_event_relay(j, &r.relay_url) {
                        debug!("record relay metadata: {e}");
                    }
                }
            }
        }

        Some(
            results
                .into_iter()
                .map(|r| BroadcastResult {
                    relay: r.relay_url,
                    success: r.success,
                    message: r.message,
                    event_id: r.event_id,
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

    let all_events: Vec<Value> = std::iter::once(pub_event.clone())
        .chain(section_events.iter().cloned())
        .collect();

    Ok(Json(PublishResponse {
        publication_id: pub_id,
        section_ids,
        signed: req.sign,
        ingested,
        broadcast_results,
        events: Some(all_events),
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
        let kinds: Vec<u64> = kinds_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if !kinds.is_empty() {
            filter["kinds"] = json!(kinds);
        }
    }
    if let Some(authors_str) = params.get("authors") {
        let authors: Vec<&str> = authors_str
            .split(',')
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
    let limit = params
        .get("limit")
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
        let kinds: Vec<u64> = kinds_str
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        if !kinds.is_empty() {
            filter["kinds"] = json!(kinds);
        }
    }
    if let Some(authors_str) = params.get("authors") {
        let authors: Vec<&str> = authors_str
            .split(',')
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
    let local_events = crate::query::query_local(engine.ndb(), &[filter]).unwrap_or_default();
    let total_events = local_events.len();

    // Count stale embeddings (indexed but no longer in nostrdb)
    let local_ids: std::collections::HashSet<&str> = local_events
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()))
        .collect();
    let stale_count = indexed_count.saturating_sub(
        index
            .all_ids()
            .iter()
            .filter(|id| local_ids.contains(id.as_str()))
            .count(),
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
    let dir = engine
        .claude_sessions_dir()
        .ok_or_else(|| EngineError::Config("Claude Code sessions directory not found".into()))?;
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
    let dir = engine
        .claude_sessions_dir()
        .ok_or_else(|| EngineError::Config("Claude Code sessions directory not found".into()))?;

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
            return Err(EngineError::NotFound(format!(
                "No session matching '{session_id}'"
            )));
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
    let dir = engine
        .claude_sessions_dir()
        .ok_or_else(|| EngineError::Config("Claude Code sessions directory not found".into()))?;
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
    let event = req
        .event
        .as_object()
        .ok_or_else(|| EngineError::Config("event must be a JSON object".into()))?;
    for field in [
        "id",
        "pubkey",
        "sig",
        "kind",
        "created_at",
        "tags",
        "content",
    ] {
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

    let event_json = serde_json::to_string(&req.event)
        .map_err(|e| EngineError::Config(format!("event serialize: {e}")))?;
    let results = crate::relay::publish_to_relays(&relays, &event_json).await;
    // Record relay provenance for each relay that accepted the event.
    for r in &results {
        if r.success {
            if let Err(e) = engine.record_event_relay(&event_json, &r.relay_url) {
                debug!("record relay metadata: {e}");
            }
        }
    }
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
) -> Result<
    axum::response::Sse<
        futures::stream::BoxStream<
            'static,
            Result<axum::response::sse::Event, std::convert::Infallible>,
        >,
    >,
    EngineError,
> {
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
                Some((Ok::<SseHttpEvent, std::convert::Infallible>(sse_event), rx))
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
                NoteRequest {
                    title: "A".into(),
                    content: "aaa".into(),
                },
                NoteRequest {
                    title: "B".into(),
                    content: "bbb".into(),
                },
            ],
        };
        let Json(resp) = chat_inject_context(State(state.clone()), Json(req)).await;
        assert_eq!(resp.context_count, 2);

        // Replace with single note
        let req = InjectContextRequest {
            notes: vec![NoteRequest {
                title: "C".into(),
                content: "ccc".into(),
            }],
        };
        let Json(resp) = chat_replace_context(State(state), Json(req)).await;
        assert_eq!(resp.context_count, 1);
    }
}
