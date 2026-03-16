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

    let mut query = SearchQuery::parse(&req.query)
        .map_err(|e| EngineError::InvalidFilter(e.to_string()))?;

    if let Some(limit) = req.limit {
        query.limit = Some(limit);
    }

    // Resolve by:me to actual pubkey
    if let Some(AuthorFilter::CurrentUser) = &query.author_filter {
        if let Some(ref pk) = req.my_pubkey {
            query.author_filter = Some(AuthorFilter::Pubkeys(vec![pk.clone()]));
        } else {
            return Err(EngineError::InvalidFilter(
                "by:me requires my_pubkey in request".to_string(),
            ));
        }
    }

    let policy = match &req.policy {
        Some(p) => p.parse()?,
        None => FetchPolicy::default(),
    };

    let response = engine
        .search(&query, policy, req.relays.as_deref())
        .await?;

    Ok(Json(response))
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
