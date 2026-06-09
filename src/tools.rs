//! AI tool registry + dispatcher.
//!
//! A static catalog of tools the assistant can call, plus a `match`-based
//! async `dispatch`. Each tool is a thin language→code bridge over an existing
//! engine method — this module adds no new data logic. Tools return **compact
//! JSON handles** (id / kind / addr / title / snippet / score), not full event
//! JSON, so the model curates a working set by id then expands only what it
//! needs via the `view_*` / `get_*` tools.
//!
//! Permission model: [`definitions`] filters the catalog by [`ToolPolicy`]
//! *before* the provider call, so a disabled tool never reaches the model.
//! See `docs/ai-tools-architecture.md`.

use crate::engine::{Engine, FetchPolicy};
use crate::error::{EngineError, Result};
use crate::llm::ToolDefinition;
use crate::publication::{NAddr, Publication, PublicationEngine};
use crate::search::SearchQuery;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// Permission/gating class for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Local reads (view events / publications / KB).
    Read,
    /// Local search + embeddings.
    Search,
    /// Relay fetches — gated by `NetworkMode::Confirm` inside the engine.
    Network,
    /// Curate the shared chat context (pull events/ideas into the reference set
    /// the user also sees and edits).
    Context,
    /// Propose/edit sections, save drafts.
    ComposeWrite,
    /// Sign + broadcast — always opt-in, always confirmed.
    Publish,
}

/// Static metadata for one tool.
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub category: ToolCategory,
    /// JSON Schema for the tool's input object.
    pub input_schema: fn() -> Value,
}

/// Which tools are permitted, and which categories require runtime approval.
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    /// Tool names the model may call.
    pub enabled: HashSet<String>,
    /// Categories whose calls must be approved at runtime (Network is gated by
    /// the engine's existing fetch-confirm flow; Publish by publish-confirm).
    pub require_approval: HashSet<ToolCategory>,
}

impl ToolPolicy {
    /// Build a policy from an explicit enabled-tool name list (unknown names
    /// are ignored at `definitions` time). Approval categories keep their
    /// defaults (Network + Publish).
    pub fn from_enabled(enabled: impl IntoIterator<Item = String>) -> Self {
        let require_approval = [ToolCategory::Network, ToolCategory::Publish]
            .into_iter()
            .collect();
        Self {
            enabled: enabled.into_iter().collect(),
            require_approval,
        }
    }
}

impl Default for ToolPolicy {
    /// "Everything but publish": all Read/Search/Network/ComposeWrite tools
    /// enabled; Publish tools opt-in.
    fn default() -> Self {
        let enabled = catalog()
            .iter()
            .filter(|t| t.category != ToolCategory::Publish)
            .map(|t| t.name.to_string())
            .collect();
        let require_approval = [ToolCategory::Network, ToolCategory::Publish]
            .into_iter()
            .collect();
        Self {
            enabled,
            require_approval,
        }
    }
}

/// The full tool catalog (metadata only).
pub fn catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "search_events",
            description: "Search the local Nostr index with the structured query DSL. Call this whenever the user asks to find notes, sections, or events by kind, author, tag, time, or text. Returns compact handles (id/kind/addr/title/snippet); expand with view_publication or get_event.",
            category: ToolCategory::Search,
            input_schema: schema_search_events,
        },
        ToolDef {
            name: "semantic_search",
            description: "Find events by meaning (vector similarity) rather than exact text. Call this when the user describes a concept or topic and exact keywords are unlikely to match. Requires the embedding index to be enabled.",
            category: ToolCategory::Search,
            input_schema: schema_semantic_search,
        },
        ToolDef {
            name: "search_profiles",
            description: "Find Nostr profiles (kind 0 metadata) by name/nip05 partial. Call this to resolve a person the user names before filtering by author.",
            category: ToolCategory::Search,
            input_schema: schema_search_profiles,
        },
        ToolDef {
            name: "get_event",
            description: "Fetch one event by its 64-hex id from the local index. Call this to read the full JSON of an event you found via search_events.",
            category: ToolCategory::Read,
            input_schema: schema_get_event,
        },
        ToolDef {
            name: "get_addressable",
            description: "Fetch one replaceable/addressable event by its coordinate (kind, pubkey, d-tag) from the local index.",
            category: ToolCategory::Read,
            input_schema: schema_get_addressable,
        },
        ToolDef {
            name: "list_publications",
            description: "List the root NKBIP-01 publications (kind 30040 indexes) known locally, with title/summary/section-count. Call this to discover what publications exist before opening one.",
            category: ToolCategory::Read,
            input_schema: schema_list_publications,
        },
        ToolDef {
            name: "view_publication",
            description: "Open one publication by its address (naddr1… or kind:pubkey:d_tag) and return its sections with content. Call this to read a publication the user references.",
            category: ToolCategory::Read,
            input_schema: schema_view_publication,
        },
        ToolDef {
            name: "view_publication_tree",
            description: "Open a publication and recurse into nested publications to the given depth, returning the full structured tree with section content. Call this when the user wants the whole multi-level document, not just the top index.",
            category: ToolCategory::Read,
            input_schema: schema_view_publication_tree,
        },
        ToolDef {
            name: "list_section_versions",
            description: "List alternate versions of a section (same kind + d-tag, any author) by its address. Call this when the user asks about the history or variants of a section.",
            category: ToolCategory::Read,
            input_schema: schema_list_section_versions,
        },
        ToolDef {
            name: "fetch_from_relays",
            description: "Run a Nostr query against the network (relays), not just the local index. Use this when local search comes up empty or the user explicitly wants fresh/remote events. Takes the same query DSL as search_events but a plain Nostr filter — NOT semantic (~:) search. In Confirm network mode this pops a confirmation the user must approve before any relay is contacted; if they decline, the tool returns approved:false and you should fall back to local results.",
            category: ToolCategory::Network,
            input_schema: schema_fetch_from_relays,
        },
        ToolDef {
            name: "add_to_context",
            description: "Pull a reference into the shared chat context — the working set of notes the user also sees and can edit. Give an event `id`, an addressable `addr` (naddr1…/kind:pubkey:d_tag), or a raw `title`+`content` to capture an idea. Call this when something you found is worth keeping in front of you both for the rest of the conversation, instead of re-searching. The item appears in the user's Context panel.",
            category: ToolCategory::Context,
            input_schema: schema_add_to_context,
        },
        ToolDef {
            name: "propose_section",
            description: "Draft a new publication section and offer it to the user's composer. Call this when the user asks you to write, draft, or outline a section. The section is added to the composer for the user to review/edit — it is NOT published.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_propose_section,
        },
        ToolDef {
            name: "edit_section",
            description: "Propose revised content for an existing section (by address) and offer it to the composer alongside the original. Call this when the user asks you to rewrite or revise a section you can read. It does NOT modify the published section — the user applies it from the composer.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_edit_section,
        },
        ToolDef {
            name: "save_draft",
            description: "Assemble a title + sections into a local NKBIP-01 draft and persist it (unsigned, unpublished) so the user can resume it later. Call this when the user asks you to save the draft you've assembled.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_save_draft,
        },
        ToolDef {
            name: "publish",
            description: "Broadcast an event that ALREADY EXISTS locally (by its 64-hex id) to relays. This does NOT sign or create content — signing is a deliberate user action; this only performs the separate broadcast step for something already signed (e.g. a snapshot the user made). In Confirm mode it pops a publish confirmation the user must approve. Disabled by default; only call it when the user has explicitly asked to broadcast a specific local event.",
            category: ToolCategory::Publish,
            input_schema: schema_publish,
        },
    ]
}

/// Tool definitions filtered by policy, ready to hand to the provider.
/// Disabled tools are omitted entirely — the model can't attempt them.
pub fn definitions(policy: &ToolPolicy) -> Vec<ToolDefinition> {
    catalog()
        .iter()
        .filter(|t| policy.enabled.contains(t.name))
        .map(|t| ToolDefinition {
            name: t.name.to_string(),
            description: t.description.to_string(),
            input_schema: (t.input_schema)(),
        })
        .collect()
}

/// Execute a tool by name. Returns the JSON the model sees as a tool result.
pub async fn dispatch(engine: &Arc<Engine>, name: &str, input: Value) -> Result<Value> {
    match name {
        "search_events" => search_events(engine, input).await,
        "semantic_search" => semantic_search(engine, input).await,
        "search_profiles" => tool_search_profiles(engine, input).await,
        "get_event" => get_event(engine, input).await,
        "get_addressable" => get_addressable(engine, input).await,
        "list_publications" => list_publications(engine, input).await,
        "view_publication" => view_publication(engine, input).await,
        "view_publication_tree" => view_publication_tree(engine, input).await,
        "list_section_versions" => list_section_versions(engine, input).await,
        "fetch_from_relays" => fetch_from_relays(engine, input).await,
        "add_to_context" => add_to_context(engine, input).await,
        "propose_section" => propose_section(input).await,
        "edit_section" => edit_section(engine, input).await,
        "save_draft" => save_draft(engine, input).await,
        "publish" => publish(engine, input).await,
        other => Err(EngineError::BadRequest(format!("unknown tool: {other}"))),
    }
}

// ---------------------------------------------------------------------------
// Input schemas
// ---------------------------------------------------------------------------

fn schema_search_events() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search DSL. Examples: 'k:30040' (kind), 'by:name:alice', '\"exact phrase\"', '~:concept' (semantic), 'has:title', 'since:1700000000', or an naddr1…/nevent1…/note1… entity. Combine with spaces."
            },
            "limit": { "type": "integer", "description": "Max results to return." }
        },
        "required": ["query"]
    })
}

fn schema_semantic_search() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Natural-language concept to match by meaning." },
            "k": { "type": "integer", "description": "Number of nearest results (default 10)." }
        },
        "required": ["query"]
    })
}

fn schema_search_profiles() -> Value {
    json!({
        "type": "object",
        "properties": {
            "term": { "type": "string", "description": "Name or nip05 partial to match." }
        },
        "required": ["term"]
    })
}

fn schema_get_event() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "64-hex event id." }
        },
        "required": ["id"]
    })
}

fn schema_get_addressable() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": { "type": "integer", "description": "Event kind (e.g. 30040)." },
            "pubkey": { "type": "string", "description": "Author pubkey (64-hex)." },
            "d_tag": { "type": "string", "description": "The d-tag identifier (may be empty)." }
        },
        "required": ["kind", "pubkey"]
    })
}

fn schema_list_publications() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit": { "type": "integer", "description": "Max publications to list (default 50)." }
        }
    })
}

fn schema_address_only(desc: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "addr": { "type": "string", "description": desc }
        },
        "required": ["addr"]
    })
}

fn schema_view_publication() -> Value {
    schema_address_only("Publication address: naddr1… or kind:pubkey:d_tag.")
}

fn schema_view_publication_tree() -> Value {
    json!({
        "type": "object",
        "properties": {
            "addr": { "type": "string", "description": "Publication address: naddr1… or kind:pubkey:d_tag." },
            "depth": { "type": "integer", "description": "How many nesting levels to recurse (default 3, max 5)." }
        },
        "required": ["addr"]
    })
}

fn schema_list_section_versions() -> Value {
    schema_address_only("Section address: naddr1… or kind:pubkey:d_tag.")
}

fn schema_fetch_from_relays() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Nostr query DSL (same as search_events): 'k:30040', 'by:name:alice', '\"exact phrase\"', 'has:title', an naddr1…/nevent1…/note1… entity, etc. Do NOT use ~: semantic search here — relays only understand plain filters."
            },
            "limit": { "type": "integer", "description": "Max results to return." }
        },
        "required": ["query"]
    })
}

fn schema_publish() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "64-hex id of a local, already-signed event to broadcast." },
            "relays": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional relay URLs to broadcast to (defaults to the configured publish relays)."
            }
        },
        "required": ["id"]
    })
}

fn schema_add_to_context() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "64-hex event id to capture from the local index." },
            "addr": { "type": "string", "description": "Addressable coordinate to capture: naddr1… or kind:pubkey:d_tag." },
            "title": { "type": "string", "description": "Title for the context item. Required when giving raw content; optional override otherwise." },
            "content": { "type": "string", "description": "Raw content to capture as an idea (use instead of id/addr)." }
        }
    })
}

fn schema_propose_section() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "Section heading." },
            "content": { "type": "string", "description": "Section body (Markdown/Org/AsciiDoc/plain text)." },
            "level": { "type": "integer", "description": "Heading depth: 2 = top-level (default), 3+ = nested." }
        },
        "required": ["content"]
    })
}

fn schema_edit_section() -> Value {
    json!({
        "type": "object",
        "properties": {
            "addr": { "type": "string", "description": "Address of the section to revise: naddr1… or kind:pubkey:d_tag." },
            "content": { "type": "string", "description": "The proposed replacement content." }
        },
        "required": ["addr", "content"]
    })
}

fn schema_save_draft() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "Publication title." },
            "sections": {
                "type": "array",
                "description": "Ordered sections.",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["content"]
                }
            }
        },
        "required": ["title", "sections"]
    })
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn search_events(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let q = require_str(&input, "query")?;
    let mut query = SearchQuery::parse(q)
        .map_err(|e| EngineError::BadRequest(format!("invalid query: {e}")))?;
    if let Some(limit) = input.get("limit").and_then(Value::as_u64) {
        query.limit = Some(limit as usize);
    }
    let resp = engine.search(&query, FetchPolicy::LocalOnly, None).await?;
    let results: Vec<Value> = resp.results.iter().map(compact_result).collect();
    Ok(json!({ "count": resp.count, "results": results }))
}

async fn semantic_search(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    if engine.embedding_index().is_none() {
        return Err(EngineError::BadRequest(
            "semantic_search: embedding index is not enabled".into(),
        ));
    }
    let q = require_str(&input, "query")?;
    let cleaned = q.replace('"', " ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Err(EngineError::BadRequest(
            "semantic_search: empty query".into(),
        ));
    }
    let k = input.get("k").and_then(Value::as_u64).unwrap_or(10);
    let dsl = format!("~:\"{cleaned}\":{k}");
    let query = SearchQuery::parse(&dsl)
        .map_err(|e| EngineError::BadRequest(format!("invalid semantic query: {e}")))?;
    let resp = engine.search(&query, FetchPolicy::LocalOnly, None).await?;
    let results: Vec<Value> = resp.results.iter().map(compact_result).collect();
    Ok(json!({ "count": resp.count, "results": results }))
}

async fn tool_search_profiles(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let term = require_str(&input, "term")?;
    let profiles = engine.search_profiles(term).await;
    Ok(json!({ "count": profiles.len(), "profiles": profiles }))
}

async fn get_event(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let id = require_str(&input, "id")?;
    let event = engine.get_by_id(id, FetchPolicy::LocalOnly).await?;
    Ok(json!({ "found": event.is_some(), "event": event }))
}

async fn get_addressable(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let kind = input
        .get("kind")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::BadRequest("get_addressable: missing 'kind'".into()))?;
    let pubkey = require_str(&input, "pubkey")?;
    let d_tag = input.get("d_tag").and_then(Value::as_str).unwrap_or("");
    let event = engine
        .get_addressable(kind, pubkey, d_tag, FetchPolicy::LocalOnly)
        .await?;
    Ok(json!({ "found": event.is_some(), "event": event }))
}

async fn list_publications(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let pe = PublicationEngine::new(engine);
    let pubs = pe
        .list_root_publications(FetchPolicy::LocalOnly, limit, None)
        .await?;
    let publications: Vec<Value> = pubs.iter().map(compact_pub_summary).collect();
    Ok(json!({ "count": publications.len(), "publications": publications }))
}

async fn view_publication(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let addr = parse_address(require_str(&input, "addr")?)?;
    let pe = PublicationEngine::new(engine);
    let pub_ = pe
        .load_publication_tree(&addr, 1, FetchPolicy::LocalOnly)
        .await?;
    Ok(compact_pub_full(&pub_, 1))
}

async fn view_publication_tree(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let addr = parse_address(require_str(&input, "addr")?)?;
    let depth = input
        .get("depth")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .min(5) as usize;
    let pe = PublicationEngine::new(engine);
    let pub_ = pe
        .load_publication_tree(&addr, depth, FetchPolicy::LocalOnly)
        .await?;
    Ok(compact_pub_full(&pub_, depth))
}

async fn list_section_versions(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let addr = parse_address(require_str(&input, "addr")?)?;
    let pe = PublicationEngine::new(engine);
    let versions = pe
        .find_section_versions(&addr, FetchPolicy::LocalOnly)
        .await?;
    let out: Vec<Value> = versions
        .iter()
        .map(|v| json!({ "author": v.author, "created_at": v.created_at, "version": v.version }))
        .collect();
    Ok(json!({ "count": out.len(), "versions": out }))
}

/// Run a plain Nostr query against relays. Gated through the SAME flow as a
/// user-initiated relay search: `begin_fetch_operation` emits an Intent that,
/// in Confirm mode, blocks on the FetchConfirmModal until the user approves (or
/// declines → `FetchCancelled`). Auto mode proceeds immediately.
async fn fetch_from_relays(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let q = require_str(&input, "query")?;
    let mut query = SearchQuery::parse(q)
        .map_err(|e| EngineError::BadRequest(format!("invalid query: {e}")))?;
    if let Some(limit) = input.get("limit").and_then(Value::as_u64) {
        query.limit = Some(limit as usize);
    }

    let relays = engine.relay_config().all_urls();
    let op = match engine
        .begin_fetch_operation(
            crate::network::FetchPattern::Search,
            format!("AI relay search: {}", truncate(q, 60)),
            vec![format!("Query relays for: {q}")],
            relays,
        )
        .await
    {
        Ok(o) => o,
        Err(_) => {
            return Ok(json!({
                "approved": false,
                "note": "The user declined the relay fetch; use local results instead.",
            }));
        }
    };

    let chosen = op.relays().to_vec();
    let resp = engine
        .search_with_options(&query, FetchPolicy::FetchAlways, Some(&chosen), true)
        .await?;
    op.complete(resp.count);

    let results: Vec<Value> = resp.results.iter().map(compact_result).collect();
    Ok(json!({
        "approved": true,
        "count": resp.count,
        "local_count": resp.local_count,
        "relay_count": resp.relay_count,
        "results": results,
    }))
}

/// Capture a reference into the shared chat context. Resolves an event (by id
/// or addressable coordinate) to title+content, or accepts a raw title+content
/// "idea". Like the compose tools, this only *emits* a structured context item;
/// the web folds it into its pool and syncs it to the engine (boundary rule:
/// Rust derives the item from events, the web owns the working-set view state).
async fn add_to_context(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let title_override = input.get("title").and_then(Value::as_str);

    // Resolve the source: addressable coordinate, then event id, then raw idea.
    let (source_addr, source_id, resolved_title, content) =
        if let Some(addr_str) = input.get("addr").and_then(Value::as_str) {
            let addr = parse_address(addr_str)?;
            let ev = engine
                .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, FetchPolicy::LocalOnly)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!("no local event for address {addr_str}"))
                })?;
            let (title, body) = event_title_content(&ev);
            (Some(addr), None, title, body)
        } else if let Some(id) = input.get("id").and_then(Value::as_str) {
            let ev = engine
                .get_by_id(id, FetchPolicy::LocalOnly)
                .await?
                .ok_or_else(|| EngineError::NotFound(format!("no local event with id {id}")))?;
            let (title, body) = event_title_content(&ev);
            (None, Some(id.to_string()), title, body)
        } else if let Some(raw) = input.get("content").and_then(Value::as_str) {
            (None, None, None, raw.to_string())
        } else {
            return Err(EngineError::BadRequest(
                "add_to_context: provide one of 'id', 'addr', or 'content'".into(),
            ));
        };

    let title = title_override
        .map(str::to_string)
        .or(resolved_title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| truncate(content.trim(), 48));

    // `source_addr` matches the web `NAddr` shape ({kind, pubkey, d_tag}) so the
    // pool can dedupe an item the user may have added from search.
    Ok(json!({
        "type": "context",
        "title": title,
        "content": content,
        "snippet": truncate(content.trim(), 200),
        "source_event_id": source_id,
        "source_addr": source_addr.map(|a| json!({
            "kind": a.kind,
            "pubkey": a.pubkey,
            "d_tag": a.d_tag,
        })),
    }))
}

/// Propose a new section. Pure structuring of the model's draft — the web
/// folds the result into the composer for the user to review (boundary rule:
/// Rust emits structured section data; the web decides how to apply it).
async fn propose_section(input: Value) -> Result<Value> {
    let title = input
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let content = require_str(&input, "content")?.to_string();
    let level = input.get("level").and_then(Value::as_u64).unwrap_or(2);
    Ok(json!({
        "type": "section",
        "title": title,
        "content": content,
        "level": level,
    }))
}

/// Propose revised content for an existing section, returning the original
/// alongside the proposal. Does not mutate anything — the web applies it.
async fn edit_section(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let addr = parse_address(require_str(&input, "addr")?)?;
    let content = require_str(&input, "content")?.to_string();
    let original = engine
        .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, FetchPolicy::LocalOnly)
        .await?;
    let (title, original_content) = match original.as_ref() {
        Some(ev) => event_title_content(ev),
        None => (None, String::new()),
    };
    Ok(json!({
        "type": "section",
        "addr": addr.to_a_tag(),
        "title": title,
        "original_content": original_content,
        "content": content,
        "level": 2,
    }))
}

/// Assemble {title, sections} into a `ComposeState` and persist it as a local
/// draft via `DraftStore` (unsigned, unpublished). Returns the draft id.
async fn save_draft(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    use crate::publication::compose::{ComposeState, SectionCompose};

    let title = require_str(&input, "title")?.to_string();
    let sections = input
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| EngineError::BadRequest("save_draft: 'sections' array required".into()))?;

    let mut compose = ComposeState {
        title,
        ..Default::default()
    };
    for s in sections {
        let content = s
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                EngineError::BadRequest("save_draft: each section needs 'content'".into())
            })?
            .to_string();
        compose.sections.push(SectionCompose {
            title: s
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            content,
            tags: Vec::new(),
            level: 2,
            d_tag: None,
            tag_mode: false,
            current_tag_name: String::new(),
            current_tag_value: String::new(),
        });
    }

    let store = crate::drafts::DraftStore::new(engine.data_dir())
        .map_err(|e| EngineError::Other(format!("draft store: {e}")))?;
    let draft_id = store
        .save_draft(&mut compose)
        .map_err(|e| EngineError::Other(format!("save draft: {e}")))?;

    Ok(json!({
        "saved": true,
        "draft_id": draft_id,
        "d_tag": compose.d_tag,
        "section_count": compose.sections.len(),
    }))
}

/// Broadcast an already-signed local event to relays — the deliberate
/// "broadcast" step, decoupled from signing (which stays a user action). Gated
/// through `begin_publish_operation` (PublishConfirmModal in Confirm mode). On
/// success, records relay provenance so the feed pill flips local → published.
async fn publish(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let id = require_str(&input, "id")?;
    let event = engine
        .get_by_id(id, FetchPolicy::LocalOnly)
        .await?
        .ok_or_else(|| EngineError::NotFound(format!("no local event with id {id}")))?;
    let event_json =
        serde_json::to_string(&event).map_err(|e| EngineError::Other(format!("serialize: {e}")))?;

    let relays = input
        .get("relays")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| engine.publish_relays());

    let op = match engine
        .begin_publish_operation(
            format!(
                "AI broadcast {} to {} relay(s)",
                truncate(id, 12),
                relays.len()
            ),
            relays.clone(),
            vec![id.to_string()],
            None,
            None,
        )
        .await
    {
        Ok(o) => o,
        Err(_) => {
            return Ok(json!({
                "published": false,
                "note": "The user declined to broadcast this event.",
            }));
        }
    };

    let chosen = op.relays().to_vec();
    for url in &chosen {
        op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
    }
    let (accepted, rejected, results) =
        crate::relay::publish_events_to_relays(&chosen, &[event_json.clone()]).await;
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
            // Best-effort provenance — failure only delays the feed pill flip.
            let _ = engine.record_event_relay(&event_json, &r.relay_url);
        }
    }
    op.complete(accepted);

    let out: Vec<Value> = results
        .iter()
        .map(|r| json!({ "relay": r.relay_url, "success": r.success, "message": r.message }))
        .collect();
    Ok(json!({
        "published": accepted > 0,
        "accepted": accepted,
        "rejected": rejected,
        "results": out,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_str<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| EngineError::BadRequest(format!("missing required string field '{key}'")))
}

/// Extract `(title, content)` from an event JSON: `content` is the event body,
/// `title` is the value of the first `["title", …]` tag if present.
fn event_title_content(ev: &Value) -> (Option<String>, String) {
    let content = ev
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = ev.get("tags").and_then(Value::as_array).and_then(|tags| {
        tags.iter().find_map(|t| {
            let arr = t.as_array()?;
            if arr.first()?.as_str()? == "title" {
                arr.get(1)?.as_str().map(String::from)
            } else {
                None
            }
        })
    });
    (title, content)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n).collect();
        out.push('…');
        out
    }
}

fn compact_result(r: &crate::search::SearchResult) -> Value {
    json!({
        "id": r.event_id,
        "kind": r.kind,
        "addr": r.addr.as_ref().map(|a| a.to_a_tag()),
        "title": r.title,
        "snippet": truncate(&r.preview, 200),
        "author": r.author,
        "created_at": r.created_at,
        "score": r.semantic_score,
    })
}

fn compact_pub_summary(p: &Publication) -> Value {
    json!({
        "addr": p.addr.to_a_tag(),
        "title": p.title,
        "summary": p.summary,
        "author": p.author_pubkey,
        "author_name": p.author_name,
        "created_at": p.created_at,
        "section_count": p.sections.len(),
        "is_root": p.is_root,
    })
}

fn compact_pub_full(p: &Publication, depth: usize) -> Value {
    let sections: Vec<Value> = p
        .sections
        .iter()
        .map(|s| {
            json!({
                "addr": s.addr.to_a_tag(),
                "title": s.title,
                "position": s.position,
                "content": s.content,
            })
        })
        .collect();
    let nested: Vec<Value> = if depth > 0 {
        p.nested
            .iter()
            .map(|n| compact_pub_full(n, depth - 1))
            .collect()
    } else {
        Vec::new()
    };
    json!({
        "addr": p.addr.to_a_tag(),
        "title": p.title,
        "summary": p.summary,
        "author": p.author_pubkey,
        "sections": sections,
        "nested": nested,
    })
}

/// Parse a user/model-supplied publication or section address: either an
/// `naddr1…` bech32 entity or a `kind:pubkey:d_tag` coordinate.
fn parse_address(input: &str) -> Result<NAddr> {
    let s = crate::nip19::strip_nostr_prefix(input);
    if s.starts_with("naddr") {
        match crate::nip19::decode(s) {
            Ok(crate::nip19::Decoded::Naddr {
                kind_int,
                pubkey,
                d_tag,
                ..
            }) => Ok(NAddr::new(kind_int as u64, &pubkey, &d_tag)),
            Ok(_) => Err(EngineError::BadRequest(
                "expected an naddr address, got a different entity".into(),
            )),
            Err(e) => Err(EngineError::BadRequest(format!("bad naddr: {e:?}"))),
        }
    } else if let Some(addr) = NAddr::from_a_tag(s) {
        Ok(addr)
    } else {
        Err(EngineError::BadRequest(format!(
            "invalid address (expected naddr1… or kind:pubkey:d_tag): {s}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_nonempty_and_schemas_are_objects() {
        let cat = catalog();
        assert!(!cat.is_empty());
        for t in &cat {
            let schema = (t.input_schema)();
            assert_eq!(schema["type"], "object", "tool {} schema", t.name);
        }
    }

    #[test]
    fn default_policy_is_everything_but_publish() {
        let policy = ToolPolicy::default();
        // No Publish tools exist yet, but the filter must hold for the catalog.
        for t in catalog() {
            let enabled = policy.enabled.contains(t.name);
            assert_eq!(
                enabled,
                t.category != ToolCategory::Publish,
                "tool {} enablement",
                t.name
            );
        }
        // definitions() only emits enabled tools.
        let defs = definitions(&policy);
        assert_eq!(defs.len(), policy.enabled.len());
    }

    #[test]
    fn definitions_filters_disabled_tools() {
        let mut policy = ToolPolicy::default();
        policy.enabled.remove("search_events");
        let defs = definitions(&policy);
        assert!(!defs.iter().any(|d| d.name == "search_events"));
    }

    #[test]
    fn parse_address_accepts_coordinate() {
        let a = parse_address("30040:abcdef:my-pub").unwrap();
        assert_eq!(a.kind, 30040);
        assert_eq!(a.pubkey, "abcdef");
        assert_eq!(a.d_tag, "my-pub");
    }

    #[test]
    fn parse_address_rejects_garbage() {
        assert!(parse_address("not-an-address").is_err());
    }
}
