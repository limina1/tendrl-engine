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

/// Optional query for the addressable handler — lets the web force a
/// `fetch_always` round-trip (per-section "fetch from relays" button in
/// the reader outline) instead of always-local-first.
#[derive(Debug, Deserialize, Default)]
pub struct AddressableQuery {
    pub policy: Option<String>,
}

/// GET /api/v1/addressable/:kind/:pubkey/:d_tag
///
/// Get an addressable event by kind, pubkey, and d-tag
pub async fn get_addressable_handler(
    State(engine): State<AppState>,
    Path(params): Path<AddressablePath>,
    axum::extract::Query(query): axum::extract::Query<AddressableQuery>,
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

    let policy = match &query.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::LocalFirst,
    };

    let event = engine
        .get_addressable(params.kind, &params.pubkey, &params.d_tag, policy)
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

/// POST /api/v1/encode body — the inverse of `DecodeRequest`. Tagged by `kind`
/// so the shape mirrors the `Decoded` enum the decode endpoint returns.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EncodeRequest {
    Npub {
        pubkey: String,
    },
    Note {
        event_id: String,
    },
    Nevent {
        event_id: String,
        #[serde(default)]
        relays: Vec<String>,
        #[serde(default)]
        author: Option<String>,
        #[serde(default)]
        kind_int: Option<u32>,
    },
    Naddr {
        kind_int: u32,
        pubkey: String,
        d_tag: String,
        #[serde(default)]
        relays: Vec<String>,
    },
    /// Convenience: encode a raw `kind:pubkey:d_tag` `a`-tag coordinate to `naddr`.
    Atag {
        a_tag: String,
        #[serde(default)]
        relays: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct EncodeResponse {
    pub encoded: String,
}

/// One section's text + the highlights to place in it. `key` is an opaque
/// caller-chosen identifier (typically the section address) echoed back in the
/// response so a batch can be unpacked.
#[derive(Debug, Deserialize)]
pub struct ResolveHighlightsItem {
    pub key: String,
    pub content: String,
    #[serde(default)]
    pub highlights: Vec<crate::discussions::Highlight>,
}

/// POST /api/v1/highlights/resolve body. Batched so the reader resolves every
/// visible section's highlights in a single round trip.
#[derive(Debug, Deserialize)]
pub struct ResolveHighlightsRequest {
    #[serde(default)]
    pub items: Vec<ResolveHighlightsItem>,
}

#[derive(Debug, Serialize)]
pub struct ResolveHighlightsResponse {
    /// `key` → resolved non-overlapping spans (UTF-16 offsets), one entry per
    /// requested item. The web slices the section text by these and renders
    /// `<mark>`s; focus is applied client-side by comparing span ids.
    pub spans: std::collections::HashMap<String, Vec<crate::discussions::HighlightSpan>>,
}

/// POST /api/v1/highlights/resolve
///
/// Resolve NIP-84 highlight positions within section text — the engine-side
/// replacement for the web's former `computeHighlightSegments`. Pure transform
/// (no engine state); batched over sections. Focus stays a frontend concern.
pub async fn resolve_highlights_handler(
    Json(req): Json<ResolveHighlightsRequest>,
) -> Result<Json<ResolveHighlightsResponse>, EngineError> {
    let spans = req
        .items
        .into_iter()
        .map(|item| {
            let resolved =
                crate::discussions::resolve_highlight_spans(&item.content, &item.highlights);
            (item.key, resolved)
        })
        .collect();
    Ok(Json(ResolveHighlightsResponse { spans }))
}

/// POST /api/v1/encode
///
/// Encode structured fields into a NIP-19 bech32 identifier — the inverse of
/// `/decode`. NIP-19 derivation (encode + decode) lives in Rust so every
/// frontend gets identical, spec-correct output without its own bech32 impl.
pub async fn encode_handler(
    Json(req): Json<EncodeRequest>,
) -> Result<Json<EncodeResponse>, EngineError> {
    let encoded = match req {
        EncodeRequest::Npub { pubkey } => nip19::encode_npub(&pubkey),
        EncodeRequest::Note { event_id } => nip19::encode_note(&event_id),
        EncodeRequest::Nevent {
            event_id,
            relays,
            author,
            kind_int,
        } => nip19::encode_nevent(&event_id, &relays, author.as_deref(), kind_int),
        EncodeRequest::Naddr {
            kind_int,
            pubkey,
            d_tag,
            relays,
        } => nip19::encode_naddr(kind_int, &pubkey, &d_tag, &relays),
        EncodeRequest::Atag { a_tag, relays } => nip19::naddr_from_a_tag(&a_tag, &relays),
    }
    .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;
    Ok(Json(EncodeResponse { encoded }))
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

        // Build a structured RequestSummary so the confirm modal
        // renders the DSL sentence + structured filter block instead
        // of a flat opaque "Search relays: <full naddr>" label. The
        // parse may fail (e.g. malformed input) — in that case we
        // fall back to summary: None and the modal renders the legacy
        // flat view. Compound (`|`) queries get parsed below and
        // contribute one filter per branch.
        let summary = build_search_summary(&req.query, &relays, &engine);
        let label = short_search_label(&req.query);

        match engine
            .begin_fetch_operation_with_summary(
                crate::network::FetchPattern::Search,
                label,
                describe_search_steps(&req.query),
                relays,
                summary,
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

    // Persist any user-introduced search relays into the fetch set so they
    // stick for the session + across restarts (mirrors to config.toml too).
    // No-op for relays already known — only brand-new ones are added.
    if let Some(relays) = override_relays.as_deref() {
        engine.persist_discovered_relays(relays);
    }

    // Check for compound query (contains |)
    if req.query.contains('|') {
        let compound = SearchQuery::parse_compound(&req.query)
            .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

        let mut all_results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        let mut total_local = 0;
        let mut total_relay = 0;
        let mut any_relays_queried = false;
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
                .search_with_options(
                    &branch,
                    policy,
                    override_relays.as_deref(),
                    req.mode_confirm,
                )
                .await?;

            total_local += resp.local_count;
            total_relay += resp.relay_count;
            any_relays_queried |= resp.relays_queried;

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
            relays_queried: any_relays_queried,
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
            // Resolve "me" from the logged-in user session (engine secret or
            // NIP-07 signer), with the request's my_pubkey as a client hint.
            let pk = req.my_pubkey.clone().or_else(|| engine.my_pubkey());
            if let Some(pk) = pk {
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk]));
            } else {
                return Err(EngineError::InvalidFilter(
                    "by:me requires a logged-in identity".to_string(),
                ));
            }
        }
        Some(AuthorFilter::AssistantUser) => {
            // Resolve from the live assistant identity session.
            if let Some(pk) = engine.assistant_pubkey() {
                query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk]));
            } else {
                return Err(EngineError::InvalidFilter(
                    "by:assistant requires a logged-in assistant identity".to_string(),
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
    /// Include the broad "general feed" — recent kind-30040 from ALL authors,
    /// not just the scoped by:user(s). Default off (scoped). Logged out the
    /// engine fetches broad regardless (there's no one to scope to).
    #[serde(default)]
    pub general: bool,
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
        .list_root_publications(policy, query.limit, query.before, query.general)
        .await?;

    // "local" = a signed snapshot the user created that hasn't (successfully)
    // been broadcast to any relay yet. Tracked engine-side; the feed renders a
    // "local" pill off it. Tracker absence (e.g. IO error) just yields false.
    let tracker = local_pub_tracker(&engine).ok();

    // Reverse a-tag lookup: which publications contain each of these. A feed
    // row that is nested under an off-window parent shows up here as a false
    // root, so this is what lets the UI badge it as "part of N". Local-only.
    let coords: Vec<crate::publication::NAddr> =
        publications.iter().map(|p| p.addr.clone()).collect();
    let containing = pub_engine
        .containing_publications(&coords)
        .await
        .unwrap_or_default();

    // Convert to summary format
    let summaries: Vec<Value> = publications
        .iter()
        .map(|p| {
            let local = tracker
                .as_ref()
                .map(|t| t.is_local(&p.addr.to_a_tag()))
                .unwrap_or(false);
            let contained_in = containing
                .get(&p.addr.to_a_tag())
                .cloned()
                .unwrap_or_default();
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
                "forked": p.forked,
                "local": local,
                "contained_in": contained_in
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

/// POST /api/v1/publish/republish-diff body. One section is `{title, content}`
/// — the same minimal shape the publish path consumes.
#[derive(Debug, Deserialize)]
pub struct RepublishDiffRequest {
    pub title: String,
    #[serde(default)]
    pub sections: Vec<RepublishDiffSectionInput>,
}

#[derive(Debug, Deserialize)]
pub struct RepublishDiffSectionInput {
    pub title: String,
    #[serde(default)]
    pub content: String,
}

/// POST /api/v1/publish/republish-diff
///
/// Detect that a same-title publication of the user's already exists and return
/// a section-level diff (matched / added / removed by title slug) so the UI can
/// offer "replace" — reusing the existing 30040/30041 identifiers — instead of
/// forking with fresh d-tags. Returns `null` when there's no match or no
/// signed-in identity (the normal first-publish case). Fail-open: a lookup
/// error resolves to `null`, never an error status, so it can't block a publish.
pub async fn republish_diff_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<RepublishDiffRequest>,
) -> Result<Json<Option<crate::publication::RepublishDiff>>, EngineError> {
    let Some(my_pubkey) = active_or_config_pubkey(&engine, &signing).await else {
        return Ok(Json(None));
    };
    let sections: Vec<crate::publication::RepublishSectionInput> = req
        .sections
        .into_iter()
        .map(|s| crate::publication::RepublishSectionInput {
            title: s.title,
            content: s.content,
        })
        .collect();
    let pub_engine = PublicationEngine::new(&engine);
    let diff = pub_engine
        .detect_republish_diff(&my_pubkey, &req.title, &sections)
        .await
        .unwrap_or(None); // fail-open — never block a publish on a lookup error
    Ok(Json(diff))
}

/// GET /api/v1/publications/:pubkey/:d_tag/stream
///
/// POST /api/v1/publications/:pubkey/:d_tag/backfill?depth=N
///
/// Batch-fetch the publication's missing 30041 sections + nested
/// 30040 indexes from relays. In confirm mode pops ONE modal listing
/// the addresses about to be requested (rather than one modal per
/// section). Auto mode fires silently and the activity toast tracks
/// per-relay progress.
#[derive(Debug, Deserialize)]
pub struct BackfillQuery {
    /// How many tree levels to walk when collecting missing children.
    /// Defaults to the same DEFAULT_PUBLICATION_DEPTH used by
    /// get_publication_handler so the backfill horizon matches what
    /// the reader displays.
    pub depth: Option<usize>,
}

pub async fn backfill_publication_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
    axum::extract::Query(query): axum::extract::Query<BackfillQuery>,
) -> Result<Json<Value>, EngineError> {
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    let depth = query
        .depth
        .unwrap_or(DEFAULT_PUBLICATION_DEPTH)
        .min(MAX_PUBLICATION_DEPTH);
    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);

    let (requested, fetched) = pub_engine
        .backfill_publication_sections(&addr, depth)
        .await?;

    Ok(Json(json!({
        "requested": requested,
        "fetched": fetched,
        "depth": depth,
    })))
}

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

    // Parse in-process (no sidecar)
    let parsed = crate::document::parse_document(&req.filename, &file_bytes)?;
    Ok(Json(serde_json::to_value(parsed)?))
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

    // Parse in-process (no sidecar)
    let parsed = crate::document::parse_document(&filename, &file_bytes)?;
    let resp = serde_json::to_value(parsed)?;

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
    // Single source of truth for kind-0 parsing (the inline twin was deleted).
    // A non-JSON content yields all-None — same as the former `unwrap_or({})`.
    let meta = crate::user_data::Metadata::from_event_content(content, 0).unwrap_or_default();
    json!({
        "pubkey": pubkey,
        "name": meta.name,
        "display_name": meta.display_name,
        "picture": meta.picture,
        "about": meta.about,
        "nip05": meta.nip05,
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

    // Phase 4: profile lookups now walk the indexer composition —
    // primary (read [∪ indexer.default]) → indexer.fallback if zero.
    // fetch_with_composition opens the SSE op (carrying the
    // RequestSummary so the toast renders the structured query),
    // streams per-relay RelayStatus events, and emits Completed at
    // the end. The whole chain shows up as one toast with phases
    // visible in the expanded modal.
    let composition = engine.compose_discovery_phases("indexer");
    let filter = json!({"kinds": [0], "authors": targets, "limit": targets.len()});

    let events = engine
        .fetch_with_composition(
            &composition,
            &[filter],
            format!(
                "Fetch {} profile{}",
                targets.len(),
                if targets.len() == 1 { "" } else { "s" }
            ),
            crate::network::FetchPattern::Profile,
            req.force,
        )
        .await;

    let fetched = events.len();

    // Brief wait for nostrdb to process ingested events
    if fetched > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    Ok(Json(json!({
        "fetched": fetched,
        "total": req.pubkeys.len()
    })))
}

/// POST /api/v1/pull-user-data — fetch the user's relay-list events
/// (kind 10002 NIP-65, 10007 search, 10086 indexer, 10088 broadcast,
/// 30002 NIP-51 named sets) through the indexer composition.
///
/// Phase 4.1 wiring: instead of the previous "hit `initial_relays`
/// directly" path, this routes through `engine.fetch_with_composition`
/// so the read → indexer.default → indexer.fallback chain handles the
/// "kind 10002 isn't cached locally but lives on purplepag.es" case
/// automatically. Emits an SSE op with the structured RequestSummary
/// so the activity toast shows the formal-language query and
/// per-relay status; per-phase fallback shows up as ordered toasts.
#[derive(Debug, Deserialize)]
pub struct PullUserDataRequest {
    /// Hex pubkey of the user whose relay-list events to fetch.
    /// (Typically the logged-in identity, but any author is allowed —
    /// "pull alice's named sets" is a planned secondary use case.)
    pub pubkey: String,
    /// Whether this is user-initiated (default true). Confirm-mode
    /// gates the operation behind the modal when true.
    #[serde(default = "default_pull_confirm")]
    pub mode_confirm: bool,
}

fn default_pull_confirm() -> bool {
    true
}

pub async fn pull_user_data_handler(
    State(engine): State<AppState>,
    Json(req): Json<PullUserDataRequest>,
) -> Result<Json<Value>, EngineError> {
    if req.pubkey.len() != 64 || hex::decode(&req.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }

    // Kinds we pull as relay-list payloads. The 100xx are Amethyst-
    // defined PrivateTagArrayEvents; 30002 is NIP-51 named sets. The
    // web parses each kind appropriately after the fetch lands in
    // local nostrdb.
    let kinds = [10002u64, 10007, 10086, 10088, 30002];

    let composition = engine.compose_discovery_phases("indexer");
    // One filter for the 100xx replaceable kinds (1 event per
    // {author, kind}) and one for kind 30002 addressable (many per
    // author). We send them as a single REQ subscription with two
    // filters so relays can stream both in one round trip.
    let filter_replaceable = json!({
        "kinds": [10002, 10007, 10086, 10088],
        "authors": [&req.pubkey],
        "limit": 4,
    });
    let filter_addressable = json!({
        "kinds": [30002],
        "authors": [&req.pubkey],
        "limit": 64,
    });

    let events = engine
        .fetch_with_composition(
            &composition,
            &[filter_replaceable, filter_addressable],
            format!("Pull relay lists for @{}", short_id(&req.pubkey)),
            crate::network::FetchPattern::Profile,
            req.mode_confirm,
        )
        .await;

    let fetched = events.len();

    // Brief wait for nostrdb to ingest before the web reads back.
    if fetched > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    Ok(Json(json!({
        "fetched": fetched,
        "kinds": kinds,
        "author": req.pubkey,
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

    // A relay the user brought into this fetch should stick: add any new ones
    // to the working fetch set (persists to relays.json + config.toml).
    engine.persist_discovered_relays(&fetch_relays);

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
    debug!(
        "Fetched {} events from {} relay(s)",
        count,
        fetch_relays.len()
    );
    if let Some(op) = op {
        op.complete(count);
    }

    // Trigger background embedding sync for newly fetched events
    if count > 0 && engine.auto_embed() && engine.embedding_index().is_some() {
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

    let mut filter = json!({ "limit": 200, "authors": authors });
    if !kinds.is_empty() {
        filter["kinds"] = json!(kinds);
    }

    // Route through fetch_with_composition — one op fanned out across
    // all fetch relays — so Confirm mode emits a single approvable
    // intent instead of silently returning `fetched: 0` (the dead-button
    // bug: plain tracked_fetch short-circuits to Ok(vec![]) with no
    // intent when not in Auto mode). This endpoint is only hit by an
    // explicit user action, so mode_confirm = true.
    let composition = crate::network::CompositionShape {
        phases: vec![crate::network::PhaseStage {
            label: "primary".into(),
            members: vec![(crate::network::Phase::Read, rc.fetch.urls.clone())],
            start_delay_ms: 0,
        }],
    };
    let events = engine
        .fetch_with_composition(
            &composition,
            &[filter],
            format!("Fetch {} configured author(s)", authors.len()),
            crate::network::FetchPattern::Custom,
            true,
        )
        .await;
    let total_fetched = events.len();

    // Trigger background embedding sync for newly fetched events
    if total_fetched > 0 && engine.auto_embed() && engine.embedding_index().is_some() {
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
    if fetched > 0 && engine.auto_embed() && engine.embedding_index().is_some() {
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
                        // Record that the user has explicitly chosen a mode so
                        // the first-run modal never re-appears on this machine.
                        table.insert("mode_chosen".into(), toml::Value::Boolean(true));
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
    /// Create an empty named set (NIP-51 kind 30002 candidate).
    pub create_named_set: Option<CreateNamedSet>,
    /// Delete a named set by d_tag.
    pub delete_named_set: Option<String>,
    /// Rename a named set's title.
    pub rename_named_set: Option<RenameNamedSet>,
    /// Add a relay URL to a named set.
    pub add_to_named_set: Option<NamedSetMember>,
    /// Remove a relay URL from a named set.
    pub remove_from_named_set: Option<NamedSetMember>,
    /// Toggle the `exclusive` flag for a discovery class
    /// (`"search"` or `"indexer"`). When ON, the engine bypasses read
    /// relays for that class's lookup type and uses the class's
    /// `.default` / `.fallback` sets only.
    pub set_exclusive: Option<SetExclusive>,
    /// `true` → merge the engine's well-known indexer/search default
    /// URLs (`crate::relay::DEFAULT_INDEXERS` / `DEFAULT_SEARCH`)
    /// into the current `default` tiers. Idempotent. Lets existing
    /// users opt into the same set a fresh install gets.
    pub restore_discovery_defaults: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SetExclusive {
    /// `"search"` or `"indexer"`.
    pub class: String,
    pub value: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateNamedSet {
    pub d_tag: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameNamedSet {
    pub d_tag: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct NamedSetMember {
    pub d_tag: String,
    pub url: String,
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
    if let Some(create) = &req.create_named_set {
        if engine.create_named_set(&create.d_tag, &create.title) {
            changed = true;
        }
    }
    if let Some(d_tag) = &req.delete_named_set {
        if engine.delete_named_set(d_tag) {
            changed = true;
        }
    }
    if let Some(rename) = &req.rename_named_set {
        if engine.rename_named_set(&rename.d_tag, &rename.title) {
            changed = true;
        }
    }
    if let Some(m) = &req.add_to_named_set {
        if engine.add_to_named_set(&m.d_tag, &m.url) {
            changed = true;
        }
    }
    if let Some(m) = &req.remove_from_named_set {
        if engine.remove_from_named_set(&m.d_tag, &m.url) {
            changed = true;
        }
    }
    if let Some(ex) = &req.set_exclusive {
        if engine.set_discovery_exclusive(&ex.class, ex.value) {
            changed = true;
        }
    }
    if req.restore_discovery_defaults.unwrap_or(false) {
        let added = engine.merge_discovery_defaults();
        if added > 0 {
            changed = true;
        }
    }

    // Author edits still flow to config.toml — they're a separate concern
    // and not part of this migration.
    if req.add_author.is_some() || req.remove_author.is_some() {
        let config_path = engine.config_path().ok_or_else(|| {
            EngineError::Config("No config file path set (use -c config.toml)".into())
        })?;

        let content = match std::fs::read_to_string(config_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(EngineError::Config(format!("Failed to read config: {e}"))),
        };
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
        "message": if changed { "Relay config updated." } else { "No changes needed." }
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
    /// Optional identity source — when present, written to
    /// `[identity] source`. Values: `"engine"` / `"nip07"`.
    #[serde(default)]
    pub identity_source: Option<String>,
    /// Optional engine auto-lock timeout in minutes — when present,
    /// written to `[identity] lock_timeout_minutes`. `0` = never.
    #[serde(default)]
    pub identity_lock_timeout_minutes: Option<u64>,
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

    // Tolerate a not-yet-created config (zero-config run): start from an empty
    // table and let the write below create the file.
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(EngineError::Config(format!("Failed to read config: {e}"))),
    };
    let mut doc: toml::Table = toml::from_str(&content)
        .map_err(|e| EngineError::Config(format!("Failed to parse config: {e}")))?;

    let mut wrote: Vec<&'static str> = Vec::new();
    let mut relay_count = 0usize;

    if req.include_relays {
        let rc = engine.relay_config();
        let mut seen = std::collections::HashSet::new();
        let mut urls: Vec<String> = Vec::new();
        for u in rc
            .fetch
            .urls
            .iter()
            .chain(&rc.publish.urls)
            .chain(&rc.general.urls)
        {
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
        let arr: Vec<toml::Value> = urls
            .iter()
            .map(|u| toml::Value::String(u.clone()))
            .collect();
        relay_table.insert("initial_relays".to_string(), toml::Value::Array(arr));
        wrote.push("initial_relays");
    }

    if let Some(editor) = &req.editor {
        let mut t = toml::Table::new();
        t.insert(
            "line_numbers".into(),
            toml::Value::Boolean(editor.line_numbers),
        );
        t.insert("vim_mode".into(), toml::Value::Boolean(editor.vim_mode));
        t.insert(
            "insert_mode".into(),
            toml::Value::String(editor.insert_mode.clone()),
        );
        doc.insert("editor".into(), toml::Value::Table(t));
        wrote.push("editor");
    }

    if let Some(compose) = &req.compose {
        let mut t = toml::Table::new();
        t.insert(
            "default_mode".into(),
            toml::Value::String(compose.default_mode.clone()),
        );
        t.insert(
            "sync_mode".into(),
            toml::Value::String(compose.sync_mode.clone()),
        );
        t.insert(
            "button_labels".into(),
            toml::Value::String(compose.button_labels.clone()),
        );
        doc.insert("compose".into(), toml::Value::Table(t));
        wrote.push("compose");
    }

    if let Some(mode) = &req.network_mode {
        let network = doc
            .entry("network")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(t) = network {
            t.insert("mode".into(), toml::Value::String(mode.clone()));
            // Saving settings is an explicit choice — never re-prompt.
            t.insert("mode_chosen".into(), toml::Value::Boolean(true));
            wrote.push("network");
        }
    }

    if let Some(source) = &req.identity_source {
        let identity = doc
            .entry("identity")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(t) = identity {
            t.insert("source".into(), toml::Value::String(source.clone()));
            wrote.push("identity.source");
        }
    }

    if let Some(minutes) = req.identity_lock_timeout_minutes {
        let identity = doc
            .entry("identity")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(t) = identity {
            t.insert(
                "lock_timeout_minutes".into(),
                toml::Value::Integer(minutes as i64),
            );
            wrote.push("identity.lock_timeout_minutes");
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

/// GET /api/v1/config/export — download the active config.toml as a portable
/// file. The encrypted engine key (`[identity] ncryptsec`) is **stripped**: the
/// user authenticates via a pasted ncryptsec or NIP-07, so the stored key is
/// not part of a portable/backup copy and must never leave the machine in one.
///
/// Source is the on-disk config the engine reads/writes; if nothing has been
/// saved yet, the serialized defaults are exported so the download is never
/// blank. (Live relay-set edits live in relays.json — snapshot first if you
/// want them folded into the exported `[relay] initial_relays` seed.)
pub async fn config_export_handler(
    State(engine): State<AppState>,
) -> Result<impl IntoResponse, EngineError> {
    let content = match engine.config_path() {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(EngineError::Config(format!("Failed to read config: {e}"))),
        },
        None => String::new(),
    };

    // Empty (nothing saved yet) → export defaults so the file is meaningful.
    let mut doc: toml::Table = if content.trim().is_empty() {
        let defaults = toml::to_string_pretty(&crate::config::Config::default())
            .map_err(|e| EngineError::Config(format!("Failed to serialize defaults: {e}")))?;
        toml::from_str(&defaults)
            .map_err(|e| EngineError::Config(format!("Failed to parse defaults: {e}")))?
    } else {
        toml::from_str(&content)
            .map_err(|e| EngineError::Config(format!("Failed to parse config: {e}")))?
    };

    // Redact the secret.
    if let Some(toml::Value::Table(identity)) = doc.get_mut("identity") {
        identity.remove("ncryptsec");
    }

    let output = toml::to_string_pretty(&doc)
        .map_err(|e| EngineError::Config(format!("Failed to serialize config: {e}")))?;

    let mut headers = axum::http::HeaderMap::new();
    headers.insert("content-type", "application/toml".parse().unwrap());
    headers.insert(
        "content-disposition",
        "attachment; filename=\"config.toml\"".parse().unwrap(),
    );
    Ok((headers, output))
}

/// GET /api/v1/settings — return editor/compose/network defaults from the
/// current config.toml so the web can hydrate state at boot instead of
/// starting on hard-coded defaults that diverge from the user's last save.
pub async fn settings_handler(State(engine): State<AppState>) -> Result<Json<Value>, EngineError> {
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
        "identity": {
            "source": cfg.identity.source,
            "lock_timeout_minutes": cfg.identity.lock_timeout_minutes,
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
        "broadcast": { "urls": rc.broadcast.urls, "kinds": rc.broadcast.kinds },
        // Discovery classes split into default/fallback tiers. Add/remove
        // them through /config/update with the dotted set names
        // `search.default`, `search.fallback`, `indexer.default`,
        // `indexer.fallback`.
        "search": {
            "default": rc.search.default,
            "fallback": rc.search.fallback,
        },
        "indexer": {
            "default": rc.indexer.default,
            "fallback": rc.indexer.fallback,
        },
        "exclusive": {
            "search": rc.exclusive.search,
            "indexer": rc.exclusive.indexer,
        },
        "named_sets": rc.named_sets,
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
///
/// Confirm mode: a plain `get` must not produce outbound HTTP — screen
/// renders prime NIP-11 for every visible relay, which would silently
/// ping each relay host. So without `refresh` we only `peek` the cache.
/// `?refresh=true` is an explicit per-relay user click, which carries
/// its own consent (same convention as `force` on profile fetches).
pub async fn relay_nip11_handler(
    State(engine): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<RelayInfoQuery>,
) -> Json<Value> {
    let status = if q.refresh {
        engine.nip11_cache().refresh(&q.url).await
    } else if engine.is_auto() {
        engine.nip11_cache().get(&q.url).await
    } else {
        engine.nip11_cache().peek(&q.url).await
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

/// POST /api/v1/purge — delete nostrdb and re-exec the engine process.
///
/// Schedules a background task that deletes `data.mdb` + `lock.mdb`
/// from the data directory, then re-execs the current binary with
/// the same argv via `CommandExt::exec` (Unix). The HTTP response
/// returns immediately so the web can show a "purging…" toast and
/// reconnect to the fresh engine in ~1 second.
///
/// What's preserved: `relays.json` (relay state file, lives in the
/// same data_dir but isn't touched), `config.toml`, the embedding
/// index files. What's purged: the LMDB database holding events,
/// profiles, blocks, ingest queue.
pub async fn purge_handler(
    State(engine): State<AppState>,
) -> Result<Json<serde_json::Value>, EngineError> {
    let data_dir = engine.data_dir().to_path_buf();

    // Schedule the destructive work AFTER this response gets sent.
    // The browser sees a clean 200 with the message, then the engine
    // tears down and re-execs — the SSE channels and any in-flight
    // requests die cleanly with connection-closed rather than a
    // mid-response crash.
    tokio::spawn(async move {
        // Give the HTTP layer a beat to flush this response.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Delete the LMDB files. On Linux, unlink succeeds even while
        // the running engine still has the files mapped — the inode
        // stays alive until the engine exits, but the directory entry
        // is removed so the fresh post-exec engine sees an empty dir.
        for name in ["data.mdb", "lock.mdb"] {
            let p = data_dir.join(name);
            if let Err(e) = std::fs::remove_file(&p) {
                tracing::warn!("purge: failed to remove {}: {}", p.display(), e);
            }
        }

        // Re-exec the current binary with the original argv. On Unix
        // this replaces the process image: same PID, same parent,
        // fresh memory + fresh file handles. `exec()` only returns on
        // failure.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let exe = match std::env::current_exe() {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!("purge: can't resolve current exe: {}", e);
                    std::process::exit(1);
                }
            };
            let args: Vec<_> = std::env::args().skip(1).collect();
            let err = std::process::Command::new(exe).args(&args).exec();
            tracing::error!(
                "purge: exec failed: {} — aborting so a process supervisor can restart",
                err
            );
            std::process::exit(1);
        }
        #[cfg(not(unix))]
        {
            tracing::error!("purge: in-process re-exec only supported on Unix");
            std::process::exit(1);
        }
    });

    let data_dir_display = engine
        .data_dir()
        .to_path_buf()
        .to_string_lossy()
        .to_string();
    Ok(Json(json!({
        "message": "Purging the local cache and re-execing the engine. Reconnect in ~1 second.",
        "data_dir": data_dir_display,
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

/// Tally NIP-22 comments (kind 1111) and NIP-84 highlights (kind 9802) per
/// referenced address. An event is counted once per address it tags via `a`
/// or `A` — a comment carrying both the parent `a` and root `A` for the same
/// coordinate counts once. Every requested address appears in the result, at
/// zero if nothing references it.
///
/// Callers must pass events already deduplicated by id (the `#a`/`#A` filters
/// overlap), otherwise an event matched by both filters is tallied twice. This
/// is the single source of truth for both `discussions/counts` and the
/// `counts` ride-along on `discussions/list`.
fn tally_discussion_counts(
    events: &[Value],
    addresses: &[String],
) -> std::collections::HashMap<String, DiscussionCount> {
    let address_set: std::collections::HashSet<&str> =
        addresses.iter().map(String::as_str).collect();
    let mut counts: std::collections::HashMap<String, DiscussionCount> = addresses
        .iter()
        .map(|a| (a.clone(), DiscussionCount::default()))
        .collect();

    for event in events {
        let kind = event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
        let Some(tags) = event.get("tags").and_then(|v| v.as_array()) else {
            continue;
        };
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
    counts
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

    // Single `#a` filter here (no `#A`), so no cross-filter duplicates to
    // dedup before tallying. The `discussions/list` ride-along dedups first.
    let counts = tally_discussion_counts(&response.events, &addresses);

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
    /// When true, the response also carries the NIP-22 thread forest built
    /// engine-side (`threads_by_address` for address queries, the flat
    /// `threads` for event-id queries). The web consumes that instead of
    /// threading the flat `events` itself.
    pub threaded: bool,
}

#[derive(Debug, Serialize)]
pub struct DiscussionsListResponse {
    pub events: Vec<Value>,
    /// NIP-22 comment / NIP-84 highlight tallies per requested address,
    /// computed engine-side over the same (deduped) event set. Rides along so
    /// the reader can drive its discussion badges without re-deriving counts
    /// from the events client-side, and without a second `discussions/counts`
    /// round trip. Empty when the caller queried by event id only.
    pub counts: std::collections::HashMap<String, DiscussionCount>,
    pub source: crate::engine::QuerySource,
    /// Server's view of when the result was computed (unix seconds).
    /// The web uses this as a `since` cursor for incremental refreshes.
    pub refreshed_at: i64,
    /// NIP-22 thread forest grouped by requested address (kind-1111 comments
    /// only), present when `threaded` was set on an address query. Each address
    /// maps to its root threads; the reader renders these directly. Omitted
    /// (not just empty) when not requested or when the query was event-id-only.
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub threads_by_address: std::collections::HashMap<String, Vec<crate::discussions::ThreadNode>>,
    /// Flat NIP-22 thread forest over all kind-1111 comments, present when
    /// `threaded` was set on an *event-id* query (no address to group by).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<crate::discussions::ThreadNode>,
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

/// Short, screen-friendly label for the confirm modal's title. The
/// previous form used the full query string, which produced a 500+ char
/// header when the query was a NIP-19 entity (naddr/nevent/etc).
/// Truncate to a head/tail snippet so the header stays one line.
fn short_search_label(query: &str) -> String {
    let q = query.trim();
    if q.len() <= 48 {
        return format!("Search · {q}");
    }
    let head: String = q.chars().take(14).collect();
    let tail: String = q
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("Search · {head}…{tail}")
}

/// Build a structured RequestSummary for a search query. Returns
/// None when the query fails to parse — caller falls back to the
/// legacy flat-relay modal view. Composition is a single primary
/// "read" stage since search.rs's existing per-class routing isn't
/// wired through this handler yet (planned follow-up).
fn build_search_summary(
    query: &str,
    relays: &[String],
    _engine: &Engine,
) -> Option<crate::network::RequestSummary> {
    use crate::network::{
        nip_filter_from_json, CompositionShape, Phase, PhaseStage, RequestSummary,
    };

    // Try compound parse first — a `|` query splits into multiple
    // branches that each contribute a NIP-01 filter (or several).
    let filter_jsons: Vec<serde_json::Value> = if query.contains('|') {
        SearchQuery::parse_compound(query)
            .ok()?
            .branches
            .into_iter()
            .flat_map(|b| b.to_nip01_filters())
            .collect()
    } else {
        SearchQuery::parse(query).ok()?.to_nip01_filters()
    };
    if filter_jsons.is_empty() {
        return None;
    }

    let composition = CompositionShape {
        phases: vec![PhaseStage {
            label: "primary".into(),
            members: vec![(Phase::Read, relays.to_vec())],
            start_delay_ms: 0,
        }],
    };
    let nip_filters: Vec<_> = filter_jsons.iter().map(nip_filter_from_json).collect();
    let mut summary = RequestSummary {
        filters: nip_filters,
        composition,
        dsl: String::new(),
    };
    summary.dsl = summary.to_dsl();
    Some(summary)
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
            counts: std::collections::HashMap::new(),
            source: crate::engine::QuerySource {
                local_count: 0,
                relay_count: 0,
            },
            refreshed_at: now,
            threads_by_address: std::collections::HashMap::new(),
            threads: vec![],
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

    // Tally over the deduped event set (the `#a`/`#A` filters overlap, so an
    // event matched by both must count once). Addresses-only; an event-id-only
    // query carries no address coordinates, so counts stays empty there.
    let counts = tally_discussion_counts(&events, &addresses);

    // Thread engine-side when asked. Address queries group by section
    // (`threads_by_address`); an event-id-only query has no address to group
    // by, so it gets the flat `threads` forest. The web consumes these instead
    // of threading the flat `events` itself.
    let (threads_by_address, threads) = if req.threaded {
        if !addresses.is_empty() {
            (
                crate::discussions::group_threads_by_address(&events, &addresses),
                Vec::new(),
            )
        } else {
            let comments: Vec<Value> = events
                .iter()
                .filter(|e| e.get("kind").and_then(|v| v.as_u64()) == Some(1111))
                .cloned()
                .collect();
            (
                std::collections::HashMap::new(),
                crate::discussions::build_thread(&comments),
            )
        }
    } else {
        (std::collections::HashMap::new(), Vec::new())
    };

    if let Some(op) = op {
        op.complete(events.len());
    }
    Ok(Json(DiscussionsListResponse {
        events,
        counts,
        source: response.source,
        refreshed_at: now,
        threads_by_address,
        threads,
    }))
}

// ============================================================================
// Drafts API — local unsigned-publication storage (DraftStore)
// ============================================================================
//
// Drafts persist the full compose state (title, tags, sections + their content,
// levels, and stable d-tags) to `<data_dir>/drafts/` so an in-progress
// publication survives a browser refresh / engine restart, can be listed and
// resumed, and is reachable from any frontend — unlike the in-memory compose
// buffer or an unsigned event dumped into the feed. Storage + (de)serialization
// live in `src/drafts.rs`; these handlers are the thin HTTP surface.

use crate::drafts::{DraftError, DraftStore};

fn draft_store(engine: &Engine) -> std::result::Result<DraftStore, EngineError> {
    DraftStore::new(engine.data_dir()).map_err(draft_err)
}

fn draft_err(e: DraftError) -> EngineError {
    match e {
        DraftError::NotFound(id) => EngineError::NotFound(format!("draft: {id}")),
        DraftError::Io(e) => EngineError::Io(e),
        DraftError::Json(e) => EngineError::Serialization(e),
    }
}

/// Resolve "my" pubkey for owner-scoped lookups (republish detection, diff vs
/// published). Prefers the active identity — engine session OR external NIP-07/
/// NIP-46 signer — over the config `[identity] pubkey`, because `my_pubkey()` is
/// only set from config at startup and stays `None` for a session/NIP-07 login.
async fn active_or_config_pubkey(
    engine: &Engine,
    signing: &crate::signing::SigningController,
) -> Option<String> {
    if let Some(pk) = signing.active_pubkey().await {
        return Some(pk);
    }
    engine.my_pubkey().map(|s| s.to_string())
}

/// Tracker of locally-created publications not yet pushed to a relay — drives
/// the feed's "local / not broadcast" pill. Marked local when a signed snapshot
/// is ingested without (successful) broadcast, marked published once it lands.
fn local_pub_tracker(
    engine: &Engine,
) -> std::result::Result<crate::drafts::LocalPublicationTracker, EngineError> {
    crate::drafts::LocalPublicationTracker::new(engine.data_dir()).map_err(draft_err)
}

/// The `kind:pubkey:d_tag` coordinate of a (replaceable) event, matching
/// `NAddr::to_a_tag`. `None` if the event carries no `d` tag.
fn event_a_tag(event: &Value) -> Option<String> {
    let kind = event.get("kind").and_then(|v| v.as_u64())?;
    let pubkey = event.get("pubkey").and_then(|v| v.as_str())?;
    let d = event
        .get("tags")
        .and_then(|v| v.as_array())?
        .iter()
        .find_map(|t| {
            let arr = t.as_array()?;
            if arr.first()?.as_str()? == "d" {
                arr.get(1)?.as_str()
            } else {
                None
            }
        })?;
    Some(format!("{kind}:{pubkey}:{d}"))
}

#[derive(Debug, Deserialize)]
pub struct DraftSectionRequest {
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    /// Heading depth (2 = top-level; 3+ nests at publish time). Default 2.
    #[serde(default)]
    pub level: Option<u8>,
    /// Stable section d-tag to preserve on resume / republish (minted if absent).
    #[serde(default)]
    pub d_tag: Option<String>,
}

/// POST /api/v1/drafts body. Mirrors the compose payload `PublishRequest`
/// carries, minus signing/broadcast — a draft is never signed.
#[derive(Debug, Deserialize)]
pub struct SaveDraftRequest {
    pub title: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    #[serde(default)]
    pub sections: Vec<DraftSectionRequest>,
    /// Existing publication d-tag to preserve addressable identity on resume.
    #[serde(default)]
    pub d_tag: Option<String>,
}

fn compose_from_draft_request(req: SaveDraftRequest) -> ComposeState {
    let mut compose = ComposeState::new();
    compose.title = req.title;
    compose.d_tag = req.d_tag;
    compose.tags = req
        .tags
        .into_iter()
        .map(|(name, value)| crate::publication::compose::TagEntry { name, value })
        .collect();
    compose.sections = req
        .sections
        .into_iter()
        .map(|s| SectionCompose {
            title: s.title,
            content: s.content,
            level: s.level.unwrap_or(2),
            d_tag: s.d_tag,
            tags: s
                .tags
                .into_iter()
                .map(|(name, value)| crate::publication::compose::TagEntry { name, value })
                .collect(),
            ..Default::default()
        })
        .collect();
    compose
}

/// POST /api/v1/drafts
///
/// Save a draft snapshot from the current compose state. Each save is a new
/// `<d-tag>-<millis>-<seq>` snapshot (versions never overwrite); returns the
/// `draft_id` and the publication `d_tag`. Thread that `d_tag` back onto later
/// saves to keep them versions of the same article — its absence mints a fresh
/// nanoid, i.e. a new article (which may share a title with another).
pub async fn save_draft_handler(
    State(engine): State<AppState>,
    Json(req): Json<SaveDraftRequest>,
) -> Result<Json<Value>, EngineError> {
    let store = draft_store(&engine)?;
    let mut compose = compose_from_draft_request(req);
    // The d-tag (nanoid) is the article's identity; the title is just a label.
    // Versions group by d-tag, NOT title — two articles may share a title
    // (different d-tags) and one article may be renamed (same d-tag, new title).
    // So we never merge by title: a threaded `req.d_tag` versions that article;
    // its absence mints a fresh nanoid (a new article). The web threads the
    // session d-tag across saves and restores it when a draft is resumed.
    let draft_id = store.save_draft(&mut compose).map_err(draft_err)?;
    let d_tag = compose.d_tag.clone().unwrap_or_default();
    Ok(Json(json!({ "draft_id": draft_id, "d_tag": d_tag })))
}

/// GET /api/v1/drafts — draft summaries, newest first (no event bodies).
pub async fn list_drafts_handler(
    State(engine): State<AppState>,
) -> Result<Json<Value>, EngineError> {
    let store = draft_store(&engine)?;
    let drafts = store.list_drafts().map_err(draft_err)?;
    let summaries: Vec<Value> = drafts
        .iter()
        .map(|d| {
            json!({
                "draft_id": d.draft_id,
                "title": d.title,
                // Publication identity — saves of one publication share this, so
                // the web groups them into a version list. Legacy drafts without
                // a stored d-tag fall back to the title slug so they still group.
                "d_tag": d.compose_state.d_tag.clone()
                    .unwrap_or_else(|| crate::publication::compose::ComposeState::generate_d_tag(&d.title)),
                "created_at": d.created_at,
                "modified_at": d.modified_at,
                "section_count": d.section_events.len(),
            })
        })
        .collect();
    Ok(Json(
        json!({ "drafts": summaries, "count": summaries.len() }),
    ))
}

/// GET /api/v1/drafts/:id — full draft, including the compose state to resume.
pub async fn get_draft_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<crate::drafts::DraftPublication>, EngineError> {
    let store = draft_store(&engine)?;
    let draft = store.load_draft(&id).map_err(draft_err)?;
    Ok(Json(draft))
}

/// DELETE /api/v1/drafts/:id
pub async fn delete_draft_handler(
    State(engine): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, EngineError> {
    let store = draft_store(&engine)?;
    store.delete_draft(&id).map_err(draft_err)?;
    Ok(Json(json!({ "deleted": id })))
}

/// POST /api/v1/drafts/diff body — two draft ids of the same publication.
#[derive(Debug, Deserialize)]
pub struct DraftDiffRequest {
    /// The older version being viewed.
    pub from_id: String,
    /// The version to compare against (typically the latest).
    pub to_id: String,
}

/// POST /api/v1/drafts/diff
///
/// Diff two draft snapshots (`from_id` → `to_id`) — the 30040 title/tag changes
/// plus the contained 30041 sections (matched by title slug, annotated
/// matched/added/removed with content/tag/level diffs). Drives the composer's
/// per-version "what changed vs latest" view.
pub async fn draft_diff_handler(
    State(engine): State<AppState>,
    Json(req): Json<DraftDiffRequest>,
) -> Result<Json<crate::drafts::VersionDiff>, EngineError> {
    let store = draft_store(&engine)?;
    let from = store.load_draft(&req.from_id).map_err(draft_err)?;
    let to = store.load_draft(&req.to_id).map_err(draft_err)?;
    let diff = crate::drafts::diff_draft_versions(&from.compose_state, &to.compose_state);
    Ok(Json(diff))
}

/// Build a `DraftComposeState` from a save/compose request — the "current"
/// (possibly unsaved) working state, for diffing against the published version.
fn request_to_draft_compose(req: &SaveDraftRequest) -> crate::drafts::DraftComposeState {
    use crate::drafts::{DraftComposeState, DraftSectionCompose, DraftTagEntry};
    DraftComposeState {
        title: req.title.clone(),
        d_tag: req.d_tag.clone(),
        tags: req
            .tags
            .iter()
            .map(|(n, v)| DraftTagEntry {
                name: n.clone(),
                value: v.clone(),
            })
            .collect(),
        sections: req
            .sections
            .iter()
            .map(|s| DraftSectionCompose {
                title: s.title.clone(),
                content: s.content.clone(),
                tags: s
                    .tags
                    .iter()
                    .map(|(n, v)| DraftTagEntry {
                        name: n.clone(),
                        value: v.clone(),
                    })
                    .collect(),
                level: s.level.unwrap_or(2),
                d_tag: s.d_tag.clone(),
            })
            .collect(),
    }
}

/// Flatten a loaded (signed) publication into the same `DraftComposeState` shape
/// so it can be diffed against the current compose. Sections are the 30041
/// leaves at `2 + nesting depth`; tags are the non-structural ("custom") tags on
/// each event — the ones a user actually edits.
fn publication_to_draft_compose(
    pub_: &crate::publication::Publication,
) -> crate::drafts::DraftComposeState {
    use crate::drafts::{DraftComposeState, DraftSectionCompose, DraftTagEntry};
    const STRUCTURAL: &[&str] = &["d", "title", "T", "a", "A", "e", "auto-update", "alt"];

    fn custom_tags(ev: Option<&Value>) -> Vec<DraftTagEntry> {
        let Some(ev) = ev else {
            return Vec::new();
        };
        ev.get("tags")
            .and_then(|v| v.as_array())
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| {
                        let arr = t.as_array()?;
                        let name = arr.first()?.as_str()?;
                        if STRUCTURAL.contains(&name) {
                            return None;
                        }
                        Some(DraftTagEntry {
                            name: name.to_string(),
                            value: arr
                                .get(1)
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn walk(pub_: &crate::publication::Publication, level: u8, out: &mut Vec<DraftSectionCompose>) {
        for s in &pub_.sections {
            out.push(DraftSectionCompose {
                title: s.title.clone().unwrap_or_default(),
                content: s.content.clone().unwrap_or_default(),
                tags: custom_tags(s.event.data()),
                level,
                d_tag: Some(s.addr.d_tag.clone()),
            });
        }
        for nested in &pub_.nested {
            walk(nested, level + 1, out);
        }
    }

    let mut sections = Vec::new();
    walk(pub_, 2, &mut sections);
    DraftComposeState {
        title: pub_.title.clone().unwrap_or_default(),
        d_tag: Some(pub_.addr.d_tag.clone()),
        tags: custom_tags(pub_.index.data()),
        sections,
    }
}

/// Find the user's last *published* (signed) version of an article — by d-tag
/// when the compose carries one, else by title slug (republish-diff style).
/// Returns the loaded tree, or `None` if nothing's been published.
async fn find_published_publication(
    pub_engine: &PublicationEngine<'_>,
    my_pubkey: &str,
    title: &str,
    d_tag: Option<&str>,
) -> Result<Option<crate::publication::Publication>, EngineError> {
    if let Some(dtag) = d_tag {
        let addr = NAddr::new(KIND_PUBLICATION_INDEX, my_pubkey, dtag);
        return Ok(pub_engine
            .load_publication_tree(&addr, DEFAULT_PUBLICATION_DEPTH, FetchPolicy::LocalOnly)
            .await
            .ok());
    }
    let slug = ComposeState::generate_d_tag(title);
    let pubs = pub_engine
        .list_root_publications(FetchPolicy::LocalOnly, 50, None, false)
        .await?;
    let Some(m) = pubs
        .into_iter()
        .filter(|p| {
            p.author_pubkey == my_pubkey
                && p.title
                    .as_deref()
                    .is_some_and(|t| ComposeState::generate_d_tag(t) == slug)
        })
        .max_by_key(|p| p.created_at)
    else {
        return Ok(None);
    };
    Ok(pub_engine
        .load_publication_tree(&m.addr, DEFAULT_PUBLICATION_DEPTH, FetchPolicy::LocalOnly)
        .await
        .ok())
}

/// POST /api/v1/publish/diff
///
/// Diff the *current* compose (the live working state in the request) against
/// the last *published* (signed) version of that article — so the composer can
/// show "what's different from what I published" on demand. Returns
/// `{published:false}` if nothing's been published, else `{published:true, diff,
/// existingAddr}`. The diff direction is published → current.
pub async fn diff_published_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<SaveDraftRequest>,
) -> Result<Json<Value>, EngineError> {
    let Some(my_pubkey) = active_or_config_pubkey(&engine, &signing).await else {
        return Ok(Json(json!({ "published": false })));
    };
    let pub_engine = PublicationEngine::new(&engine);
    let published =
        find_published_publication(&pub_engine, &my_pubkey, &req.title, req.d_tag.as_deref())
            .await?;
    let Some(published_pub) = published.filter(|p| p.index.is_loaded()) else {
        return Ok(Json(json!({ "published": false })));
    };
    let from = publication_to_draft_compose(&published_pub);
    let to = request_to_draft_compose(&req);
    let diff = crate::drafts::diff_draft_versions(&from, &to);
    Ok(Json(json!({
        "published": true,
        "diff": diff,
        "existingAddr": published_pub.addr,
    })))
}

// ============================================================================
// Publish API Endpoints
// ============================================================================

use crate::publication::build_publication_events;
use crate::publication::compose::{
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

/// Map a signing error from the publish path to its HTTP error. Shared by
/// both publish handlers so they can't drift on identity semantics.
fn map_publish_sign_error(e: crate::signing::SigningError) -> EngineError {
    match e {
        crate::signing::SigningError::Locked => {
            EngineError::Locked("Identity is locked — unlock with password first".into())
        }
        crate::signing::SigningError::SignerNotConnected => EngineError::Auth(
            "External signer not connected — open a tab with the signer extension".into(),
        ),
        other => EngineError::Other(format!("Cannot sign: {other}")),
    }
}

/// Resolve the active signing pubkey, or the standard "no identity" 401.
async fn require_active_pubkey(
    signing: &crate::signing::SigningController,
) -> Result<String, EngineError> {
    signing.active_pubkey().await.ok_or_else(|| {
        EngineError::Auth(
            "No identity configured (engine source needs login; nip07 needs a connected signer)"
                .into(),
        )
    })
}

/// Shared tail of both publish handlers: ingest the signed events into
/// nostrdb, optionally broadcast to relays (confirm-gated, with relay
/// provenance), update the local/published feed-pill state, kick off an
/// embedding sync, and assemble the response. The two handlers differ
/// only in how they build the compose state and which signer fn they
/// call; everything after `(pub_event, section_events)` lives here so the
/// ingest/broadcast/response logic can't drift between them.
async fn finalize_publish(
    engine: &AppState,
    pub_event: Value,
    section_events: Vec<Value>,
    broadcast: bool,
    relays: Option<Vec<String>>,
    signed: bool,
) -> Result<Json<PublishResponse>, EngineError> {
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

    // Ingest into local nostrdb (async queue), then wait + verify.
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
    if !ingested {
        debug!(
            "Publication {} was not persisted by nostrdb after ingest",
            pub_id
        );
    }

    // Broadcast to relays if requested.
    let broadcast_results = if broadcast {
        let relays = relays.unwrap_or_else(|| engine.publish_relays().to_vec());
        let event_jsons: Vec<String> = section_events
            .iter()
            .chain(std::iter::once(&pub_event))
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        let event_ids: Vec<String> = section_events
            .iter()
            .chain(std::iter::once(&pub_event))
            .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect();

        // Formal-language summary — publication root + N sections.
        let summary = crate::network::RequestSummary {
            filters: vec![],
            composition: crate::network::CompositionShape {
                phases: vec![crate::network::PhaseStage {
                    label: "primary".into(),
                    members: vec![(crate::network::Phase::Publish, relays.clone())],
                    start_delay_ms: 0,
                }],
            },
            dsl: format!(
                "pub k:30040,30041 ({} events) via:publish",
                event_jsons.len()
            ),
        };
        let manifest = crate::network::PublishManifest::from_events(
            section_events.iter().chain(std::iter::once(&pub_event)),
        );

        let op = engine
            .begin_publish_operation(
                format!(
                    "Publishing {} events to {} relay(s)",
                    event_jsons.len(),
                    relays.len()
                ),
                relays.clone(),
                event_ids,
                Some(summary),
                Some(manifest),
            )
            .await
            .ok();

        if let Some(op) = op {
            let chosen = op.relays().to_vec();
            for url in &chosen {
                op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
            }

            let (_, _, results) =
                crate::relay::publish_events_to_relays(&chosen, &event_jsons).await;

            // One Accepted/Rejected per (relay, event) — the UI's expanded
            // toast renders them grouped by relay.
            for r in &results {
                let status = if r.success {
                    crate::network::RelayStatusValue::Accepted
                } else {
                    crate::network::RelayStatusValue::Rejected {
                        msg: r.message.clone().unwrap_or_default(),
                    }
                };
                op.relay_status(r.relay_url.clone(), status);
            }
            let successful_relays = results.iter().filter(|r| r.success).count();
            op.complete(successful_relays);

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
        }
    } else {
        None
    };

    // Feed-pill state: a signed snapshot that didn't (successfully) reach any
    // relay is "local"; once it lands it's published. Keyed by the 30040 coord.
    track_local_publication(engine, &pub_event, &broadcast_results);

    // Trigger background embedding sync so new events are searchable immediately.
    if ingested && engine.auto_embed() && engine.embedding_index().is_some() {
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
        signed,
        ingested,
        broadcast_results,
        events: Some(all_events),
    }))
}

/// POST /api/v1/publish — create a publication (draft or signed)
pub async fn publish_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<PublishRequest>,
) -> Result<impl IntoResponse, EngineError> {
    // Map request to ComposeState
    use crate::publication::compose::TagEntry;
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

    // Publishing writes a SIGNED snapshot to the db. Unsigned working drafts
    // are saved via POST /api/v1/drafts — we never ingest placeholder-sig
    // events. The signature is the snapshot; nostrdb keeps every version.
    if !req.sign {
        return Err(EngineError::BadRequest(
            "Publishing writes a signed snapshot; save an unsigned working draft via POST /api/v1/drafts instead."
                .into(),
        ));
    }

    // Sign through the SigningController — engine in-process for the engine
    // source, or the registered external signer (NIP-07/NIP-46) over the SSE
    // back-channel. The handler is unaware of the source.
    let active_pubkey = require_active_pubkey(&signing).await?;
    let (pub_event, section_events) =
        crate::publication::build_signed_publication_events_via_signer(
            &mut compose,
            &active_pubkey,
            &signing,
        )
        .await
        .map_err(map_publish_sign_error)?;

    finalize_publish(
        &engine,
        pub_event,
        section_events,
        req.broadcast,
        req.relays,
        req.sign,
    )
    .await
}

/// Mark the publication's 30040 coordinate local or published in the
/// `LocalPublicationTracker`, based on whether the broadcast leg landed on any
/// relay. Shared by both publish handlers. Best-effort — never fails a publish.
fn track_local_publication(
    engine: &Engine,
    pub_event: &Value,
    broadcast_results: &Option<Vec<BroadcastResult>>,
) {
    let Some(a_tag) = event_a_tag(pub_event) else {
        return;
    };
    let Ok(tracker) = local_pub_tracker(engine) else {
        return;
    };
    let broadcast_ok = broadcast_results
        .as_ref()
        .map(|rs| rs.iter().any(|r| r.success))
        .unwrap_or(false);
    let _ = if broadcast_ok {
        tracker.mark_published(&a_tag)
    } else {
        tracker.mark_local(&a_tag)
    };
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

    use crate::publication::compose::TagEntry;
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
    /// Reuse this 30041 d-tag instead of minting a fresh nanoid — set when
    /// re-publishing so the section replaces rather than forks. Ignored for
    /// imported blocks (they emit no event).
    #[serde(default)]
    pub d_tag: Option<String>,
    #[serde(flatten)]
    pub kind: PublishBlockKind,
}

#[derive(Debug, Deserialize)]
pub struct PublishBlocksRequest {
    pub title: String,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    /// Reuse this index d-tag instead of minting a fresh nanoid — set when
    /// re-publishing an existing publication.
    #[serde(default)]
    pub d_tag: Option<String>,
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

/// Convert the block-publish request payload into a `ComposeBlockState` —
/// shared by the publish and preview handlers so both build the exact same
/// event graph.
fn compose_block_state_from_request(
    title: String,
    tags: Vec<(String, String)>,
    d_tag: Option<String>,
    blocks: Vec<PublishBlockEntry>,
    source_publication_addr: Option<NAddrPayload>,
    source_publication_event_id: Option<String>,
) -> ComposeBlockState {
    use crate::publication::compose::TagEntry;
    let mut state = ComposeBlockState::new();
    state.title = title;
    state.d_tag = d_tag;
    for (name, value) in tags {
        state.tags.push(TagEntry { name, value });
    }
    state.source_publication_addr = source_publication_addr.map(|n| n.into_naddr());
    state.source_publication_event_id = source_publication_event_id;

    for (block_id, entry) in blocks.into_iter().enumerate() {
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
            d_tag: entry.d_tag,
        });
    }
    state
}

/// POST /api/v1/publish/blocks — publish a block-based draft.
pub async fn publish_blocks_handler(
    State(engine): State<AppState>,
    Extension(signing): Extension<crate::signing::SigningController>,
    Json(req): Json<PublishBlocksRequest>,
) -> Result<impl IntoResponse, EngineError> {
    // Publishing writes a SIGNED snapshot; unsigned working drafts go to
    // POST /api/v1/drafts. No placeholder-sig events ever enter the db.
    if !req.sign {
        return Err(EngineError::BadRequest(
            "Publishing writes a signed snapshot; save an unsigned working draft via POST /api/v1/drafts instead."
                .into(),
        ));
    }

    let mut state = compose_block_state_from_request(
        req.title,
        req.tags,
        req.d_tag,
        req.blocks,
        req.source_publication_addr,
        req.source_publication_event_id,
    );

    // Sign through the SigningController — the SAME path publish_handler uses:
    // engine in-process for the engine source, or the registered external
    // signer (NIP-07 / NIP-46) over the SSE back-channel. (A prior version
    // inlined an engine-host-only secret chain, so NIP-07 users got unsigned
    // block events — fixed, see docs/bugs.org.)
    let active_pubkey = require_active_pubkey(&signing).await?;
    let (pub_event, section_events) =
        crate::publication::build_signed_block_publication_events_via_signer(
            &mut state,
            &active_pubkey,
            &signing,
        )
        .await
        .map_err(map_publish_sign_error)?;

    finalize_publish(
        &engine,
        pub_event,
        section_events,
        req.broadcast,
        req.relays,
        req.sign,
    )
    .await
}

/// Provenance descriptor attached to a preview entry: the original's
/// coordinate plus a locally-resolved kind-0 author name (null when no
/// profile is cached). `found` is only present for linked entries and says
/// whether the original event itself is in the local store.
fn preview_original_info(ndb: &nostrdb::Ndb, addr: &NAddr, found: Option<bool>) -> Value {
    let author_name = query_profile(ndb, &addr.pubkey).and_then(|ev| {
        let content = ev.get("content")?.as_str()?.to_string();
        let meta = crate::user_data::Metadata::from_event_content(&content, 0)?;
        meta.display_name().map(String::from)
    });
    let mut info = json!({
        "addr": addr.to_a_tag(),
        "kind": addr.kind,
        "pubkey": addr.pubkey,
        "author_name": author_name,
    });
    if let Some(found) = found {
        info["found"] = json!(found);
    }
    info
}

/// POST /api/v1/publish/blocks/preview — build the would-be event graph for
/// a block-based compose without signing/ingesting/broadcasting.
///
/// Unlike the flat /publish/preview, this mirrors what /publish/blocks will
/// actually emit: imported blocks produce no new event (the 30040 references
/// the original coordinate) and forked blocks carry `fork`-marker tags. Each
/// entry is annotated with its provenance so the UI can banner it:
///
/// ```json
/// { "events": [ {
///     "status":  "new" | "forked" | "linked",
///     "title":   "<block title — publication title for the index>",
///     "event":   { … } | null,      // linked: the exact original event,
///                                   // null when it isn't cached locally
///     "original": { "addr", "kind", "pubkey", "author_name", "found"? }
/// } ] }
/// ```
///
/// The index entry comes first (status `forked` when the draft was seeded
/// from an existing publication), then one entry per block in order.
pub async fn publish_blocks_preview_handler(
    State(engine): State<AppState>,
    Extension(identity): Extension<IdentityAppState>,
    Json(req): Json<PublishBlocksRequest>,
) -> Result<Json<Value>, EngineError> {
    let pubkey = {
        let session = identity.lock().unwrap();
        session.pubkey().map(|s| s.to_string())
    }
    .or_else(|| engine.my_pubkey().map(|s| s.to_string()))
    .unwrap_or_else(|| "<preview>".to_string());

    let mut state = compose_block_state_from_request(
        req.title,
        req.tags,
        req.d_tag,
        req.blocks,
        req.source_publication_addr,
        req.source_publication_event_id,
    );

    let (pub_event, section_events) =
        crate::publication::build_block_publication_events(&mut state, &pubkey, None);

    let mut entries: Vec<Value> = Vec::with_capacity(state.blocks.len() + 1);
    entries.push(match &state.source_publication_addr {
        Some(addr) => json!({
            "status": "forked",
            "title": state.title,
            "event": pub_event,
            "original": preview_original_info(engine.ndb(), addr, None),
        }),
        None => json!({ "status": "new", "title": state.title, "event": pub_event }),
    });

    let mut sections = section_events.into_iter();
    for block in &state.blocks {
        entries.push(match &block.kind {
            BlockKind::Editable { .. } => json!({
                "status": "new",
                "title": block.title,
                "event": sections.next(),
            }),
            BlockKind::Forked { original_addr, .. } => json!({
                "status": "forked",
                "title": block.title,
                "event": sections.next(),
                "original": preview_original_info(engine.ndb(), original_addr, None),
            }),
            BlockKind::Imported { source_addr, .. } => {
                // Show the exact event the 30040 will reference — straight
                // slotted in, not re-published.
                let original = crate::query::query_addressable(
                    engine.ndb(),
                    source_addr.kind,
                    &source_addr.pubkey,
                    &source_addr.d_tag,
                )
                .ok()
                .flatten();
                let found = original.is_some();
                json!({
                    "status": "linked",
                    "title": block.title,
                    "event": original,
                    "original": preview_original_info(engine.ndb(), source_addr, Some(found)),
                })
            }
        });
    }

    Ok(Json(json!({ "events": entries })))
}

/// True iff the event carries a real (non-placeholder) signature.
fn is_signed_event(e: &Value) -> bool {
    e.get("sig")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty() && !s.chars().all(|c| c == '0'))
        .unwrap_or(false)
}

/// Collect every signed, loaded event in a publication tree — the 30040 index,
/// its 30041 sections, and the same for nested publications. Unsigned/unloaded
/// entries are skipped (only real snapshots get broadcast).
fn collect_signed_events(pub_: &crate::publication::Publication, out: &mut Vec<Value>) {
    if let Some(idx) = pub_.index.data() {
        if is_signed_event(idx) {
            out.push(idx.clone());
        }
    }
    for s in &pub_.sections {
        if let Some(ev) = s.event.data() {
            if is_signed_event(ev) {
                out.push(ev.clone());
            }
        }
    }
    for nested in &pub_.nested {
        collect_signed_events(nested, out);
    }
}

/// Optional body for `broadcast_publication_handler`. When `relays` is
/// absent or empty the engine's publish set is used; a caller (e.g. the
/// event modal's per-event Broadcast, which targets the aggregator
/// "broadcast" set) may override the destination for this one operation.
#[derive(Debug, Default, Deserialize)]
pub struct BroadcastPubRequest {
    #[serde(default)]
    pub relays: Option<Vec<String>>,
}

/// POST /api/v1/publications/:pubkey/:d_tag/broadcast
///
/// Push an already-signed publication — its 30040 index plus every loaded,
/// signed 30041 section, recursing through nested 30040 indices — to the
/// publish relays (or a caller-supplied set) in one operation. Always sends
/// the *whole* structure, never a bare index. The separate "broadcast a local
/// snapshot" step: no re-signing, no new versions. Records relay provenance
/// and clears the "local" pill (marks the publication published) once a relay
/// accepts. The `PublishIntent` it raises carries a manifest describing the
/// full tree (index + section counts, nested flag, per-event list).
pub async fn broadcast_publication_handler(
    State(engine): State<AppState>,
    Path(params): Path<PublicationPath>,
    body: Option<Json<BroadcastPubRequest>>,
) -> Result<Json<Value>, EngineError> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    if params.pubkey.len() != 64 || hex::decode(&params.pubkey).is_err() {
        return Err(EngineError::InvalidHex(
            "Pubkey must be a 64-character hex string".to_string(),
        ));
    }
    let addr = NAddr::new(KIND_PUBLICATION_INDEX, &params.pubkey, &params.d_tag);
    let pub_engine = PublicationEngine::new(&engine);
    let publication = pub_engine
        .load_publication_tree(&addr, DEFAULT_PUBLICATION_DEPTH, FetchPolicy::LocalOnly)
        .await?;

    let mut events: Vec<Value> = Vec::new();
    collect_signed_events(&publication, &mut events);
    if events.is_empty() {
        return Err(EngineError::NotFound(
            "No signed events to broadcast — sign the draft first.".to_string(),
        ));
    }

    let relays = match req.relays {
        Some(r) if !r.is_empty() => r,
        _ => engine.publish_relays().to_vec(),
    };
    if relays.is_empty() {
        return Err(EngineError::BadRequest(
            "No relays to broadcast to — pass `relays` or set some as 'publish' in the relays buffer.".into(),
        ));
    }
    let event_jsons: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect();
    let event_ids: Vec<String> = events
        .iter()
        .filter_map(|e| e.get("id").and_then(|v| v.as_str()).map(String::from))
        .collect();

    let summary = crate::network::RequestSummary {
        filters: vec![],
        composition: crate::network::CompositionShape {
            phases: vec![crate::network::PhaseStage {
                label: "primary".into(),
                members: vec![(crate::network::Phase::Broadcast, relays.clone())],
                start_delay_ms: 0,
            }],
        },
        dsl: format!("pub k:30040,30041 ({} events) via:broadcast", events.len()),
    };
    let manifest = crate::network::PublishManifest::from_events(events.iter());

    let op = engine
        .begin_publish_operation(
            format!(
                "Broadcasting publication ({} events) to {} relay(s)",
                events.len(),
                relays.len()
            ),
            relays.clone(),
            event_ids,
            Some(summary),
            Some(manifest),
        )
        .await
        .map_err(|_| EngineError::BadRequest("Broadcast cancelled by user".into()))?;

    let chosen = op.relays().to_vec();
    for url in &chosen {
        op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
    }
    let (_, _, results) = crate::relay::publish_events_to_relays(&chosen, &event_jsons).await;
    for r in &results {
        let status = if r.success {
            crate::network::RelayStatusValue::Accepted
        } else {
            crate::network::RelayStatusValue::Rejected {
                msg: r.message.clone().unwrap_or_default(),
            }
        };
        op.relay_status(r.relay_url.clone(), status);
    }
    let successful = results.iter().filter(|r| r.success).count();
    op.complete(successful);

    // Record relay provenance per (event, relay) success so the events stop
    // reading as local-only; then flip the publication's pill to published.
    let by_id: std::collections::HashMap<&str, &String> = events
        .iter()
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
    if successful > 0 {
        if let Ok(tracker) = local_pub_tracker(&engine) {
            let _ = tracker.mark_published(&addr.to_a_tag());
        }
    }

    let total = results.len();
    let broadcast_results: Vec<BroadcastResult> = results
        .into_iter()
        .map(|r| BroadcastResult {
            relay: r.relay_url,
            success: r.success,
            message: r.message,
            event_id: r.event_id,
        })
        .collect();
    Ok(Json(json!({
        "successful": successful,
        "total": total,
        "event_count": events.len(),
        "broadcast_results": broadcast_results,
    })))
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
                "embedding_available": false,
                "model": null,
                "embed_kinds": engine.embed_kinds(),
                "available_kinds": crate::embedding::DEFAULT_EMBED_KINDS.to_vec(),
                "auto_embed": engine.auto_embed(),
            })));
        }
    };

    let index = emb.read().await;
    let embedding_available = index.health_check().await.is_ok();
    let model = index.model().to_string();
    let indexed_count = index.len();

    // The user-configurable set of kinds we embed, plus the full menu the UI
    // offers as checkboxes (the canonical allow-list).
    let embed_kinds = engine.embed_kinds();
    let available_kinds = crate::embedding::DEFAULT_EMBED_KINDS.to_vec();

    // Count embeddable events in nostrdb (content kinds only; skip 30040 index events)
    let filter = serde_json::json!({"kinds": embed_kinds, "limit": 100000});
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
        "embedding_available": embedding_available,
        "model": model,
        "embed_kinds": embed_kinds,
        "available_kinds": available_kinds,
        "auto_embed": engine.auto_embed(),
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

/// Body for `POST /api/v1/embed/config`. Both fields are optional so the UI
/// can update the kind set and the auto-embed toggle independently.
#[derive(Deserialize)]
pub struct EmbedConfigRequest {
    /// The kinds to embed (deduped; custom kinds allowed). Omit to leave
    /// unchanged.
    pub kinds: Option<Vec<u16>>,
    /// Whether retrieval + publishing auto-embed. Omit to leave unchanged.
    pub auto_embed: Option<bool>,
}

/// POST /api/v1/embed/config — update the embed-kinds set and/or the
/// auto-embed toggle.
///
/// Persists to `config.toml` and applies in-memory so the next sync/reindex
/// and the retrieval/publish hooks honor it without a restart. Returns the
/// refreshed status so the UI can re-render in one round trip.
pub async fn embed_config_handler(
    State(engine): State<AppState>,
    Json(req): Json<EmbedConfigRequest>,
) -> Result<Json<Value>, EngineError> {
    if let Some(kinds) = req.kinds {
        engine.set_embed_kinds(kinds)?;
    }
    if let Some(auto) = req.auto_embed {
        engine.set_auto_embed(auto)?;
    }
    embed_status_handler(State(engine)).await
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
    /// Engine handle, for tool dispatch in the agent loop.
    pub engine: AppState,
    /// Hard cap on tool round-trips per agent turn (from `[ai] max_tool_turns`).
    pub max_tool_turns: usize,
    /// Live tool policy (which tools the agent may call). Read by the agent
    /// loop, written by the AI Tools settings endpoint.
    pub policy: Arc<std::sync::RwLock<crate::tools::ToolPolicy>>,
    /// Path to the editable Markdown system prompt, re-read each agent turn.
    pub system_prompt_path: std::path::PathBuf,
}

/// A single fragment in the API response
#[derive(Debug, Serialize)]
pub struct FragmentResponse {
    pub id: usize,
    pub role: String,
    pub content: String,
    /// Structured agent blocks (text/thinking/tool_use/tool_result) when this
    /// fragment came from the tool-calling loop. Omitted for plain fragments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<crate::llm::ContentBlock>>,
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
            blocks: f.blocks.clone(),
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

/// GET /api/v1/chat — get current conversation state. The system prompt is
/// refreshed from prompt.md so the UI always reflects the file (and survives a
/// reset / on-disk edit).
pub async fn chat_get(State(state): State<ChatAppState>) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    chat.system_prompt = read_system_prompt(&state.system_prompt_path);
    Json(build_chat_response(&chat))
}

/// DELETE /api/v1/chat — reset conversation (keeps the prompt.md system prompt).
pub async fn chat_reset(State(state): State<ChatAppState>) -> Json<ChatResponse> {
    let mut chat = state.chat.lock().unwrap();
    *chat = ChatState::new();
    chat.system_prompt = read_system_prompt(&state.system_prompt_path);
    Json(build_chat_response(&chat))
}

// --- Saved chat sessions (tendrl's own, under <data_dir>/sessions/) ---

/// Request to save the current chat (optional explicit title; auto-named otherwise).
#[derive(Debug, Deserialize, Default)]
pub struct SaveSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    /// When set, overwrite the existing session with this id (preserving its
    /// original `created_at`) instead of minting a new one. The web sends back
    /// the id it got from a prior save/load so re-saving updates in place.
    #[serde(default)]
    pub id: Option<String>,
}

/// POST /api/v1/chat/sessions — save the current conversation to a file.
pub async fn session_save(
    State(state): State<ChatAppState>,
    body: Option<Json<SaveSessionRequest>>,
) -> std::result::Result<Json<Value>, EngineError> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    let store = crate::sessions::SessionStore::new(state.engine.data_dir())?;

    let (fragments, context) = {
        let chat = state.chat.lock().unwrap();
        let frags = chat
            .fragments
            .iter()
            .map(|f| crate::sessions::SavedFragment {
                role: f.role.as_str().to_string(),
                content: f.content.clone(),
                blocks: f.blocks.clone(),
            })
            .collect::<Vec<_>>();
        let ctx = chat
            .injected_context
            .iter()
            .map(|n| crate::sessions::SavedNote {
                title: n.title.clone(),
                content: n.content.clone(),
            })
            .collect::<Vec<_>>();
        (frags, ctx)
    };

    if fragments.is_empty() {
        return Err(EngineError::BadRequest(
            "nothing to save — the chat is empty".into(),
        ));
    }

    let title = req
        .title
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| crate::sessions::derive_title(&fragments));
    let now = crate::sessions::now_millis();

    // Overwrite an existing session when the caller supplies its id, keeping
    // the original creation time; otherwise mint a fresh, time-prefixed id.
    let (id, created_at) = match req.id.filter(|s| !s.trim().is_empty()) {
        Some(existing) => {
            let created = store.load(&existing).map(|s| s.created_at).unwrap_or(now);
            (existing, created)
        }
        None => (format!("{}-{}", now, crate::sessions::slug(&title)), now),
    };
    let session = crate::sessions::SavedSession {
        id: id.clone(),
        title: title.clone(),
        created_at,
        modified_at: now,
        fragments,
        context,
    };
    store.save(&session)?;
    Ok(Json(json!({ "id": id, "title": title })))
}

/// GET /api/v1/chat/sessions — list saved sessions (newest first).
pub async fn session_list(
    State(state): State<ChatAppState>,
) -> std::result::Result<Json<Value>, EngineError> {
    let store = crate::sessions::SessionStore::new(state.engine.data_dir())?;
    let sessions = store.list()?;
    Ok(Json(
        json!({ "count": sessions.len(), "sessions": sessions }),
    ))
}

/// GET /api/v1/chat/sessions/:id — load a saved session into the live chat.
pub async fn session_load(
    State(state): State<ChatAppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> std::result::Result<Json<ChatResponse>, EngineError> {
    let store = crate::sessions::SessionStore::new(state.engine.data_dir())?;
    let saved = store.load(&id)?;

    let mut chat = state.chat.lock().unwrap();
    *chat = ChatState::new();
    for f in &saved.fragments {
        let role = ChatRole::from_str(&f.role).unwrap_or(ChatRole::User);
        chat.push_raw_fragment(role, f.content.clone(), f.blocks.clone());
    }
    if !saved.context.is_empty() {
        chat.inject_context(
            saved
                .context
                .iter()
                .map(|n| InjectedNote {
                    addr: None,
                    title: n.title.clone(),
                    content: n.content.clone(),
                })
                .collect(),
        );
    }
    chat.system_prompt = read_system_prompt(&state.system_prompt_path);
    Ok(Json(build_chat_response(&chat)))
}

/// DELETE /api/v1/chat/sessions/:id — delete a saved session.
pub async fn session_delete(
    State(state): State<ChatAppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> std::result::Result<Json<Value>, EngineError> {
    let store = crate::sessions::SessionStore::new(state.engine.data_dir())?;
    store.delete(&id)?;
    Ok(Json(json!({ "deleted": true })))
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

/// Build one SSE event for the agent stream: `{ "type": kind, "data": {...} }`.
fn agent_sse(kind: &str, data: Value) -> axum::response::sse::Event {
    let payload = json!({ "type": kind, "data": data }).to_string();
    axum::response::sse::Event::default().data(payload)
}

/// Concatenate the text blocks of an assistant turn (plain-text fallback).
fn assistant_text(blocks: &[crate::llm::ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            crate::llm::ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// POST /api/v1/chat/agent — run a tool-calling agent turn, streaming the
/// transcript over SSE.
///
/// The loop runs server-side: call the provider → emit text/thinking/tool_call
/// blocks → execute any tools via `tools::dispatch` → feed results back →
/// repeat until the model ends its turn or the tool-turn cap is hit. Relay
/// fetches and publishes gate themselves inside the engine (the existing
/// fetch-confirm / publish-confirm flows), so this stream is display-only.
///
/// SSE event shape: `{ "type": "text"|"thinking"|"tool_call"|"tool_result"|"done"|"error", "data": {...} }`.
pub async fn chat_agent_handler(
    State(state): State<ChatAppState>,
    Json(req): Json<SendMessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{KeepAlive, Sse};
    use axum::response::IntoResponse;
    use futures::stream::StreamExt;

    let stream = async_stream::stream! {
        use crate::llm::{AgentMessage, ContentBlock};

        // Seed the loop from history + the new user turn. Guard is dropped
        // before any await/yield (std Mutex must never cross those). The system
        // prompt is refreshed from prompt.md each turn (live on-disk edits).
        let mut messages = {
            let mut chat = state.chat.lock().unwrap();
            chat.system_prompt = read_system_prompt(&state.system_prompt_path);
            chat.push_user(req.content.clone());
            chat.generating = true;
            chat.to_agent_messages()
        };


        // Snapshot the live policy (guard dropped before any await/yield).
        let tools = {
            let policy = state.policy.read().unwrap();
            crate::tools::definitions(&policy)
        };

        let started = tokio::time::Instant::now();
        let budget = std::time::Duration::from_secs(600);
        let mut turn: usize = 0;

        loop {
            turn += 1;
            if turn > state.max_tool_turns {
                yield agent_sse("error", json!({ "message": "max tool turns exceeded" }));
                break;
            }
            if started.elapsed() > budget {
                yield agent_sse("error", json!({ "message": "agent time budget exceeded" }));
                break;
            }

            let out = match state.provider.run_turn(&messages, &tools, None).await {
                Ok(o) => o,
                Err(e) => {
                    yield agent_sse("error", json!({ "message": e.to_string() }));
                    break;
                }
            };

            // Stream this turn's blocks for display.
            for b in &out.content {
                match b {
                    ContentBlock::Text { text } => {
                        yield agent_sse("text", json!({ "text": text }));
                    }
                    ContentBlock::Thinking { thinking } => {
                        yield agent_sse("thinking", json!({ "thinking": thinking }));
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        yield agent_sse("tool_call", json!({ "id": id, "name": name, "input": input }));
                    }
                    ContentBlock::ToolResult { .. } => {}
                }
            }

            // Persist the assistant turn (with its blocks) for cross-turn history.
            {
                let mut chat = state.chat.lock().unwrap();
                chat.push_agent_fragment(
                    crate::chat::ChatRole::Assistant,
                    assistant_text(&out.content),
                    out.content.clone(),
                );
            }
            messages.push(AgentMessage::Assistant(out.content.clone()));

            // Collect tool calls this turn.
            let tool_uses: Vec<(String, String, Value)> = out
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, name, input } => {
                        Some((id.clone(), name.clone(), input.clone()))
                    }
                    _ => None,
                })
                .collect();

            if tool_uses.is_empty() {
                break; // end of turn
            }

            // Execute each tool and stream its result.
            let mut results: Vec<ContentBlock> = Vec::new();
            for (id, name, input) in tool_uses {
                let (content, is_error) =
                    match crate::tools::dispatch(&state.engine, &name, input).await {
                        Ok(v) => (v.to_string(), false),
                        Err(e) => (json!({ "error": e.to_string() }).to_string(), true),
                    };
                yield agent_sse(
                    "tool_result",
                    json!({ "tool_use_id": id, "name": name, "content": content, "is_error": is_error }),
                );
                results.push(ContentBlock::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error,
                });
            }

            // Persist tool results as a user-role fragment carrying the blocks.
            {
                let mut chat = state.chat.lock().unwrap();
                chat.push_agent_fragment(crate::chat::ChatRole::User, String::new(), results.clone());
            }
            messages.push(AgentMessage::ToolResults(results));
        }

        {
            let mut chat = state.chat.lock().unwrap();
            chat.generating = false;
        }
        yield agent_sse("done", json!({}));
    };

    let sse_stream = stream.map(Ok::<_, std::convert::Infallible>);
    let mut resp = Sse::new(sse_stream.boxed())
        .keep_alive(KeepAlive::default())
        .into_response();
    resp.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-transform"),
    );
    resp
}

/// Default contents written to the system-prompt file when it doesn't exist.
/// Kept in sync with the repo's `prompt.md`.
pub const DEFAULT_SYSTEM_PROMPT: &str = "# tendrl assistant\n\nYou are an AI writing assistant embedded in **tendrl**, a local-first Nostr\nknowledge base. You help the user read, organize, and compose NKBIP-01\npublications (kind 30040 indexes referencing kind 30041 sections) and other\nNostr events.\n\n## Working with the corpus\n\nYou have tools to search and read the user's **local** index, view publications\nand their nested trees, inspect section versions, and resolve profiles. Prefer\nreading the actual events over guessing. Curate a working set by id with\n`search_events` / `semantic_search`, then expand only what you need with\n`view_publication` / `get_event`.\n\n## Writing\n\nWhen the user asks you to draft or revise, use `propose_section` (or\n`edit_section`) so the result lands in their **composer** for review — it is not\npublished. Use `save_draft` only when they ask you to save. Never claim\nsomething is published unless the user explicitly published it.\n\n## Style\n\nBe concise and concrete. Reference events by their title or address when it\nhelps. Ask before doing anything destructive or anything that reaches the\nnetwork or signs an event.\n\n## Boundaries\n\n- Keep the user's data local. Never expose private notes, drafts, or keys outside\n  this workspace.\n- Be confident with local actions — reading, searching, organizing. Be cautious\n  with anything that leaves the machine: relay fetches, broadcasts, signing.\n- Don't publish, broadcast, or sign on the user's behalf without an explicit\n  request.\n- When writing would put words in another person's voice, stay neutral unless the\n  user asks you to match a specific style.\n";

/// Resolve the system-prompt file path. Relative paths resolve against the
/// config file's directory (or `data_dir` when there is no config file).
pub fn resolve_prompt_path(
    config_path: Option<&std::path::Path>,
    data_dir: &std::path::Path,
    configured: Option<&str>,
) -> std::path::PathBuf {
    let name = configured.unwrap_or("prompt.md");
    let p = std::path::Path::new(name);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    let base = config_path.and_then(|c| c.parent()).unwrap_or(data_dir);
    base.join(p)
}

/// Create the system-prompt file with the default template if it's missing.
pub fn ensure_prompt_file(path: &std::path::Path) {
    if !path.exists() {
        match std::fs::write(path, DEFAULT_SYSTEM_PROMPT) {
            Ok(()) => tracing::info!("created default AI system prompt at {}", path.display()),
            Err(e) => tracing::warn!("could not create system prompt {}: {e}", path.display()),
        }
    }
}

/// Read the system-prompt file, returning `None` when absent or blank.
pub fn read_system_prompt(path: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// GET /api/v1/ai/prompt — current system-prompt file contents + path.
pub async fn ai_prompt_get(State(state): State<ChatAppState>) -> Json<Value> {
    let content = std::fs::read_to_string(&state.system_prompt_path).unwrap_or_default();
    Json(json!({
        "content": content,
        "path": state.system_prompt_path.display().to_string(),
    }))
}

/// Request body for writing the system prompt.
#[derive(Debug, Deserialize)]
pub struct PromptUpdate {
    pub content: String,
}

/// PUT /api/v1/ai/prompt — overwrite the system-prompt file and update the
/// live system prompt the chat UI shows + the agent uses.
pub async fn ai_prompt_put(
    State(state): State<ChatAppState>,
    Json(req): Json<PromptUpdate>,
) -> std::result::Result<Json<Value>, EngineError> {
    std::fs::write(&state.system_prompt_path, &req.content)?;
    {
        let trimmed = req.content.trim();
        let mut chat = state.chat.lock().unwrap();
        chat.system_prompt = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    Ok(Json(json!({
        "saved": true,
        "path": state.system_prompt_path.display().to_string(),
    })))
}

/// Request body for updating AI settings (all fields optional).
#[derive(Debug, Deserialize, Default)]
pub struct AiSettingsRequest {
    #[serde(default)]
    pub enabled_tools: Option<Vec<String>>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Shape the AI settings response: config-backed provider/model + the tool
/// catalog annotated with each tool's live enablement.
fn build_ai_settings_response(
    engine: &crate::engine::Engine,
    policy: &crate::tools::ToolPolicy,
) -> Value {
    let cfg = engine
        .config_path()
        .and_then(|p| crate::config::Config::from_file(p).ok())
        .unwrap_or_default();
    let tools: Vec<Value> = crate::tools::catalog()
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "category": t.category,
                "enabled": policy.enabled.contains(t.name),
            })
        })
        .collect();
    json!({
        "provider": cfg.ai.provider,
        "model": cfg.ai.model,
        "max_tool_turns": cfg.ai.max_tool_turns,
        "tools": tools,
    })
}

/// GET /api/v1/ai/settings — current provider/model/auth + per-tool enablement.
pub async fn ai_settings_get(State(state): State<ChatAppState>) -> Json<Value> {
    let policy = state.policy.read().unwrap().clone();
    Json(build_ai_settings_response(&state.engine, &policy))
}

/// POST /api/v1/ai/settings — update enabled tools (live, immediate) and
/// persist provider/model/auth/enabled_tools to config.toml (provider/model/
/// auth apply on next boot; the tool allowlist also restores the policy on boot).
pub async fn ai_settings_post(
    State(state): State<ChatAppState>,
    Json(req): Json<AiSettingsRequest>,
) -> std::result::Result<Json<Value>, EngineError> {
    if let Some(names) = req.enabled_tools.clone() {
        *state.policy.write().unwrap() = crate::tools::ToolPolicy::from_enabled(names);
    }
    persist_ai_config(&state.engine, &req)?;
    let policy = state.policy.read().unwrap().clone();
    Ok(Json(build_ai_settings_response(&state.engine, &policy)))
}

/// Persist the provided AI settings into the `[ai]` block of config.toml,
/// preserving any keys not being changed.
fn persist_ai_config(
    engine: &crate::engine::Engine,
    req: &AiSettingsRequest,
) -> std::result::Result<(), EngineError> {
    let path = engine
        .config_path()
        .ok_or_else(|| EngineError::Config("no config path to persist AI settings".into()))?;
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml::Table =
        toml::from_str(&content).map_err(|e| EngineError::Config(format!("parse config: {e}")))?;
    let mut ai_tbl = doc
        .get("ai")
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();
    if let Some(v) = &req.provider {
        ai_tbl.insert("provider".into(), toml::Value::String(v.clone()));
    }
    if let Some(v) = &req.model {
        ai_tbl.insert("model".into(), toml::Value::String(v.clone()));
    }
    if let Some(names) = &req.enabled_tools {
        let arr = names
            .iter()
            .map(|n| toml::Value::String(n.clone()))
            .collect();
        ai_tbl.insert("enabled_tools".into(), toml::Value::Array(arr));
    }
    doc.insert("ai".into(), toml::Value::Table(ai_tbl));
    std::fs::write(
        path,
        toml::to_string_pretty(&doc).map_err(|e| EngineError::Config(e.to_string()))?,
    )?;
    Ok(())
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

/// POST /api/v1/chat/system — set the system prompt (also persists it to the
/// prompt.md file so the chat System view and the AI Tools prompt editor stay
/// in sync).
pub async fn chat_set_system(
    State(state): State<ChatAppState>,
    Json(req): Json<SystemPromptRequest>,
) -> Json<ChatResponse> {
    let _ = std::fs::write(&state.system_prompt_path, &req.prompt);
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

// ---------------------------------------------------------------------------
// Assistant identity — a SECOND identity, established by pasting a key (nsec
// plaintext or ncryptsec). Full signer; persisted in the OS keyring (pubkey
// always, encrypted ncryptsec optionally) — never config, never a raw nsec at
// rest. Drives `by:assistant` / feed scoping via the engine's live session.
// ---------------------------------------------------------------------------

/// Assistant identity state: a dedicated session plus whether the OS keyring is
/// usable for persistence on this host (surfaced in status so the UI can warn
/// that a key won't survive a restart).
#[derive(Clone)]
pub struct AssistantIdentity {
    pub session: IdentityAppState,
    pub keyring_available: bool,
}

#[derive(Debug, Deserialize)]
pub struct AssistantLoginRequest {
    /// `nsec1…` (plaintext → live full signer) or `ncryptsec1…` (encrypted →
    /// locked, needs a subsequent /unlock).
    pub key: String,
}

/// What we persist for the assistant in the OS keyring: the public pubkey (so
/// `by:assistant` survives a restart) and, only if the user pasted an
/// ncryptsec, the encrypted key (so signing can be restored after unlock). A
/// raw nsec is NEVER written here (ncryptsec-only at rest).
#[derive(Debug, Default, Serialize, Deserialize)]
struct AssistantPersist {
    #[serde(skip_serializing_if = "Option::is_none")]
    pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ncryptsec: Option<String>,
}

/// Best-effort keyring write of the assistant blob. No-op when the keyring is
/// unavailable; logs (never fails the request) on error.
fn persist_assistant(asst: &AssistantIdentity, blob: &AssistantPersist) {
    if !asst.keyring_available {
        return;
    }
    match serde_json::to_string(blob) {
        Ok(json) => {
            if let Err(e) = crate::identity::IdentityKeyring::new().store_last_assistant(&json) {
                tracing::warn!("Could not persist assistant identity to keyring: {e}");
            }
        }
        Err(e) => tracing::warn!("Could not serialize assistant identity: {e}"),
    }
}

/// Build the assistant status with the keyring-availability flag attached.
fn assistant_status_json(asst: &AssistantIdentity) -> IdentityStatusResponse {
    let mut session = asst.session.lock().unwrap();
    let mut status = session.status();
    status.keyring_available = Some(asst.keyring_available);
    status
}

/// GET /api/v1/assistant-identity — current assistant identity status.
pub async fn assistant_identity_status_handler(
    State(asst): State<AssistantIdentity>,
) -> Json<IdentityStatusResponse> {
    Json(assistant_status_json(&asst))
}

/// POST /api/v1/assistant-identity/login — paste a key to establish the
/// assistant. `nsec1…` derives a live signer immediately; `ncryptsec1…` loads
/// locked and needs /unlock. Persists pubkey (+ encrypted key for ncryptsec)
/// to the keyring; a raw nsec is never persisted.
pub async fn assistant_identity_login_handler(
    State(asst): State<AssistantIdentity>,
    Json(req): Json<AssistantLoginRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    let key = req.key.trim();
    if key.starts_with("nsec1") {
        let pubkey = {
            let mut session = asst.session.lock().unwrap();
            session
                .login_nsec(key)
                .map_err(|e| EngineError::InvalidFilter(e.to_string()))?
        };
        persist_assistant(
            &asst,
            &AssistantPersist {
                pubkey: Some(pubkey),
                ncryptsec: None,
            },
        );
    } else if key.starts_with("ncryptsec1") {
        {
            let mut session = asst.session.lock().unwrap();
            session
                .login_ncryptsec(key)
                .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;
        }
        // Persist the encrypted key now; the pubkey is added on unlock.
        persist_assistant(
            &asst,
            &AssistantPersist {
                pubkey: None,
                ncryptsec: Some(key.to_string()),
            },
        );
    } else {
        return Err(EngineError::BadRequest(
            "assistant key must be an nsec1… or ncryptsec1…".into(),
        ));
    }
    Ok(Json(assistant_status_json(&asst)))
}

/// POST /api/v1/assistant-identity/unlock — decrypt an ncryptsec assistant.
pub async fn assistant_identity_unlock_handler(
    State(asst): State<AssistantIdentity>,
    Json(req): Json<UnlockRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    let password = req.password.clone();
    let session_arc = asst.session.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut session = session_arc.lock().unwrap();
        session.unlock(&password)
    })
    .await
    .map_err(|e| EngineError::Other(format!("Task join error: {e}")))?;
    match result {
        Ok(pubkey) => {
            let ncryptsec = asst.session.lock().unwrap().ncryptsec();
            persist_assistant(
                &asst,
                &AssistantPersist {
                    pubkey: Some(pubkey),
                    ncryptsec,
                },
            );
            Ok(Json(assistant_status_json(&asst)))
        }
        Err(e) => Err(EngineError::Auth(format!("Assistant unlock failed: {e}"))),
    }
}

/// POST /api/v1/assistant-identity/logout — clear the assistant identity and
/// its persisted keyring entry.
pub async fn assistant_identity_logout_handler(
    State(asst): State<AssistantIdentity>,
) -> Json<IdentityStatusResponse> {
    {
        let mut session = asst.session.lock().unwrap();
        session.logout();
    }
    if asst.keyring_available {
        let _ = crate::identity::IdentityKeyring::new().clear_last_assistant();
    }
    Json(assistant_status_json(&asst))
}

#[derive(Debug, Deserialize)]
pub struct UseSourceRequest {
    /// "engine" | "nip07"
    pub source: String,
    /// Required when source is nip07 (returned by /signer-register).
    #[serde(default)]
    pub signer_id: Option<String>,
    /// Hex pubkey of the external signer. When provided alongside an
    /// nip07 source, the session surfaces it as the active
    /// pubkey via /identity status.
    #[serde(default)]
    pub pubkey: Option<String>,
}

/// POST /api/v1/identity/use — switch the active signing source.
pub async fn identity_use_source_handler(
    State(identity): State<IdentityAppState>,
    Json(req): Json<UseSourceRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    use crate::identity::IdentitySource;
    // signer_id is REQUIRED for live nip07/nip46 registration via this
    // endpoint — the web only calls /identity/use after it has a
    // signer_id from /signer-register. Use the typed
    // `IdentitySource::from_config_str` path (signer_id = None) only
    // at engine boot from config.toml, not here.
    let new_source = match req.source.as_str() {
        "engine" => IdentitySource::Engine,
        "nip07" => {
            let signer_id = req.signer_id.ok_or_else(|| {
                EngineError::BadRequest(
                    "nip07 source requires a signer_id — register a signer first (no extension connected?)".into(),
                )
            })?;
            IdentitySource::Nip07 {
                signer_id: Some(signer_id),
            }
        }
        // "nip46" is intentionally unsupported — the Nip46 variant has no
        // bunker transport, so it would register a non-functional signer.
        // It falls through to the unknown-source error below. Re-add an
        // arm here (and in IdentitySource::from_config_str) when NIP-46
        // ships.
        other => {
            return Err(EngineError::BadRequest(format!("unknown source: {other}")));
        }
    };
    let mut session = identity.lock().unwrap();
    match (&new_source, req.pubkey.as_ref()) {
        (
            crate::identity::IdentitySource::Nip07 { .. }
            | crate::identity::IdentitySource::Nip46 { .. },
            Some(pk),
        ) => session.set_source_with_pubkey(new_source, pk.clone()),
        _ => session.set_source(new_source),
    }
    Ok(Json(session.status()))
}

#[derive(Debug, Deserialize)]
pub struct LockTimeoutRequest {
    /// Minutes of inactivity before the engine key auto-locks. `0` = never.
    pub minutes: u64,
}

/// POST /api/v1/identity/lock-timeout — set the engine auto-lock timeout
/// on the live session. Persisting it across restarts is a separate
/// concern: the Settings "Save" writes `[identity] lock_timeout_minutes`
/// via the config snapshot. Only the engine source holds a secret to
/// lock; for nip07/nip46 this just records the preference with no effect.
pub async fn identity_lock_timeout_handler(
    State(identity): State<IdentityAppState>,
    Json(req): Json<LockTimeoutRequest>,
) -> Result<Json<IdentityStatusResponse>, EngineError> {
    let mut session = identity.lock().unwrap();
    session.set_timeout_minutes(req.minutes);
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
            EngineError::Auth("No identity configured".into())
        }
        crate::signing::SigningError::SignerNotConnected => EngineError::Auth(
            "External signer not connected — open a tab with the signer extension".into(),
        ),
        other => EngineError::Other(format!("Sign failed: {other}")),
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
        .ok_or_else(|| EngineError::BadRequest("event must be a JSON object".into()))?;
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
            return Err(EngineError::BadRequest(format!(
                "event missing required field `{field}`"
            )));
        }
    }

    let relays = req
        .relays
        .unwrap_or_else(|| engine.publish_relays().to_vec());
    if relays.is_empty() {
        return Err(EngineError::BadRequest(
            "no relays configured (set [relays.publish] in config or pass `relays`)".into(),
        ));
    }

    let event_json = serde_json::to_string(&req.event)
        .map_err(|e| EngineError::Config(format!("event serialize: {e}")))?;

    // Build a RequestSummary so the toast/confirm modal render the
    // formal-language form. `pub k:<kind> via:broadcast` is the
    // canonical broadcast sentence.
    let event_id = event
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let kind = event.get("kind").and_then(|v| v.as_u64()).unwrap_or(0);
    let summary = crate::network::RequestSummary {
        filters: vec![],
        composition: crate::network::CompositionShape {
            phases: vec![crate::network::PhaseStage {
                label: "primary".into(),
                members: vec![(crate::network::Phase::Broadcast, relays.clone())],
                start_delay_ms: 0,
            }],
        },
        dsl: format!("pub k:{kind} via:broadcast"),
    };

    let manifest = crate::network::PublishManifest::from_events(std::iter::once(&req.event));

    // Open the publish op envelope so the UI sees the activity. If the
    // user declines in Confirm mode, this returns FetchCancelled.
    let op = match engine
        .begin_publish_operation(
            format!("Broadcasting kind {kind} to {} relay(s)", relays.len()),
            relays.clone(),
            vec![event_id.clone()],
            Some(summary),
            Some(manifest),
        )
        .await
    {
        Ok(op) => op,
        Err(_) => {
            return Err(EngineError::Config("Broadcast cancelled by user".into()));
        }
    };
    let chosen_relays = op.relays().to_vec();

    // Emit Connecting per relay before the fan-out fires. (Per-relay
    // status streaming during the call awaits a follow-up refactor of
    // `publish_to_relays`; for now we batch-emit Accepted/Rejected
    // after each result returns.)
    for url in &chosen_relays {
        op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
    }

    let results = crate::relay::publish_to_relays(&chosen_relays, &event_json).await;

    for r in &results {
        let status = if r.success {
            crate::network::RelayStatusValue::Accepted
        } else {
            crate::network::RelayStatusValue::Rejected {
                msg: r.message.clone().unwrap_or_default(),
            }
        };
        op.relay_status(r.relay_url.clone(), status);
        if r.success {
            if let Err(e) = engine.record_event_relay(&event_json, &r.relay_url) {
                debug!("record relay metadata: {e}");
            }
        }
    }
    let successful = results.iter().filter(|r| r.success).count();
    let total = results.len();
    op.complete(successful);

    // A separately-broadcast publication index is no longer local-only.
    if successful > 0 && kind == 30040 {
        if let Some(a_tag) = event_a_tag(&req.event) {
            if let Ok(tracker) = local_pub_tracker(&engine) {
                let _ = tracker.mark_published(&a_tag);
            }
        }
    }

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
mod discussion_tally_tests {
    use super::*;

    fn ev(kind: u64, tags: Value) -> Value {
        json!({ "kind": kind, "tags": tags })
    }

    const A1: &str = "30041:1111111111111111111111111111111111111111111111111111111111111111:s1";
    const A2: &str = "30041:2222222222222222222222222222222222222222222222222222222222222222:s2";

    #[test]
    fn buckets_by_kind_and_zero_fills() {
        let addrs = vec![A1.to_string(), A2.to_string()];
        let events = vec![
            ev(1111, json!([["a", A1]])),
            ev(1111, json!([["a", A1]])),
            ev(9802, json!([["a", A1]])),
        ];
        let counts = tally_discussion_counts(&events, &addrs);
        assert_eq!(counts[A1].comments, 2);
        assert_eq!(counts[A1].highlights, 1);
        // Unreferenced address is present at zero, not absent.
        assert_eq!(counts[A2].comments, 0);
        assert_eq!(counts[A2].highlights, 0);
    }

    #[test]
    fn a_and_a_uppercase_on_same_event_count_once() {
        let addrs = vec![A1.to_string()];
        // A nested reply tagging the same coord as both parent `a` and root `A`.
        let events = vec![ev(1111, json!([["a", A1], ["A", A1]]))];
        let counts = tally_discussion_counts(&events, &addrs);
        assert_eq!(
            counts[A1].comments, 1,
            "a+A on one event must not double-count"
        );
    }

    #[test]
    fn counts_uppercase_a_root_scope() {
        let addrs = vec![A1.to_string()];
        // Nested reply that only carries the uppercase root-scope `A` tag.
        let events = vec![ev(1111, json!([["A", A1]]))];
        let counts = tally_discussion_counts(&events, &addrs);
        assert_eq!(counts[A1].comments, 1);
    }

    #[test]
    fn one_event_referencing_two_addresses_bumps_both() {
        let addrs = vec![A1.to_string(), A2.to_string()];
        let events = vec![ev(9802, json!([["a", A1], ["a", A2]]))];
        let counts = tally_discussion_counts(&events, &addrs);
        assert_eq!(counts[A1].highlights, 1);
        assert_eq!(counts[A2].highlights, 1);
    }

    #[test]
    fn ignores_unrequested_addresses_and_other_kinds() {
        let addrs = vec![A1.to_string()];
        let events = vec![
            ev(1111, json!([["a", A2]])),         // address not requested
            ev(30023, json!([["a", A1]])),        // not a discussion kind
            ev(1111, json!([["e", "deadbeef"]])), // wrong tag type
        ];
        let counts = tally_discussion_counts(&events, &addrs);
        assert_eq!(counts[A1].comments, 0);
        assert_eq!(counts.len(), 1);
    }
}

#[cfg(test)]
mod chat_api_tests {
    use super::*;

    fn make_state() -> ChatAppState {
        use crate::llm::NoopProvider;
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        let engine = Arc::new(crate::engine::Engine::new(dir.path()).unwrap());
        std::mem::forget(dir); // keep the temp dir alive for the test process
        ChatAppState {
            chat: Arc::new(Mutex::new(ChatState::new())),
            provider: Arc::new(NoopProvider::echo()),
            engine,
            max_tool_turns: 25,
            policy: Arc::new(std::sync::RwLock::new(crate::tools::ToolPolicy::default())),
            system_prompt_path: prompt_path,
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
