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

use crate::chat::ChatState;
use crate::context::{ContextEntry, ContextOrigin, ContextSource, ContextStore};
use std::sync::Mutex;

/// What a tool needs to run: the engine for reads, and the chat handle so the
/// context tools mutate the engine-owned set directly (no proposal round-trip
/// through the web). Phase 4's MCP endpoint builds the same value, so both
/// harnesses go through one `dispatch`.
#[derive(Clone)]
pub struct ToolCx {
    pub engine: Arc<Engine>,
    pub chat: Arc<Mutex<ChatState>>,
}

impl ToolCx {
    /// Persist the active set after a mutation. A failed write is logged, not
    /// fatal — the in-memory set is already updated and the next write retries.
    fn persist_context(&self, chat: &ChatState) {
        if let Err(e) = ContextStore::new(self.engine.data_dir())
            .and_then(|s| s.save_active(&chat.context))
        {
            tracing::warn!("context set: persist failed: {e}");
        }
    }
}

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
            name: "get_context",
            description: "List what is in the shared context set right now — each entry's id, title, source, who added it, and a snippet. The full text of every entry is already in your system message; pass `full: true` only when you need the exact current content of entries you are about to edit or remove.",
            category: ToolCategory::Context,
            input_schema: schema_get_context,
        },
        ToolDef {
            name: "remove_from_context",
            description: "Drop an entry from the shared context set by its `id` (from get_context). The user sees it leave their Context panel.",
            category: ToolCategory::Context,
            input_schema: schema_remove_from_context,
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
        ToolDef {
            name: "sign_draft",
            description: "Sign a saved draft AS THE ASSISTANT — its own key, never the user's — and store the signed events locally. This is a snapshot, not a broadcast: the publication appears in the feed marked local; sending it to relays is the separate `publish` step the user confirms. Give the `draft_id` returned by save_draft. Disabled by default.",
            category: ToolCategory::Publish,
            input_schema: schema_sign_draft,
        },
        ToolDef {
            name: "list_insertions",
            description: "List the NKBIP-01 insertions attached to a publication — reader-written or assistant-written kind-30041 sections anchored to it (post-its, inserted pages) that are NOT in its index. Give the root coordinate (30040:pubkey:d). Returns each note's coordinate, title, author, its parent section and placement (attach / before / after). policy=local_only reads the store; local_first / fetch_always may pull from relays (Confirm-gated).",
            category: ToolCategory::Network,
            input_schema: schema_list_insertions,
        },
        ToolDef {
            name: "propose_insertion",
            description: "Write a new note ATTACHED to a place in a publication, signed AS THE ASSISTANT with its own key (never the user's) and kept local. This is the right shape for 'write more notes from this section': the note is a kind-30041 section anchored to the section (or the index) you name, with an optional position claim (before / after) the curator can promote it into. It does NOT change the publication — the human folds it in later via promote_insertion or the reader. Sending it to relays is the separate `publish` step.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_propose_insertion,
        },
        ToolDef {
            name: "promote_insertion",
            description: "Fold an insertion into a publication's structure by republishing its index with the note's coordinate spliced in — NKBIP-01's own copy process, signed AS THE ASSISTANT. If the assistant authored the index this is a new version (same d-tag); otherwise it is a DERIVATIVE index under the assistant's key (p + E to the original). The insertion event itself is never re-authored. Kept local; broadcast is the separate `publish` step. Disabled by default.",
            category: ToolCategory::Publish,
            input_schema: schema_promote_insertion,
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
pub async fn dispatch(cx: &ToolCx, name: &str, input: Value) -> Result<Value> {
    let engine = &cx.engine;
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
        "add_to_context" => add_to_context(cx, input).await,
        "get_context" => get_context(cx, input).await,
        "remove_from_context" => remove_from_context(cx, input).await,
        "propose_section" => propose_section(input).await,
        "edit_section" => edit_section(engine, input).await,
        "save_draft" => save_draft(engine, input).await,
        "publish" => publish(engine, input).await,
        "sign_draft" => sign_draft(cx, input).await,
        "list_insertions" => list_insertions(engine, input).await,
        "propose_insertion" => propose_insertion(cx, input).await,
        "promote_insertion" => promote_insertion(cx, input).await,
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

fn schema_get_context() -> Value {
    json!({
        "type": "object",
        "properties": {
            "full": { "type": "boolean", "description": "Include each entry's full current content (default false: snippet only)." }
        }
    })
}

fn schema_remove_from_context() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Context entry id, as returned by get_context or add_to_context." }
        },
        "required": ["id"]
    })
}

fn schema_sign_draft() -> Value {
    json!({
        "type": "object",
        "properties": {
            "draft_id": { "type": "string", "description": "Draft id from save_draft." }
        },
        "required": ["draft_id"]
    })
}

fn schema_list_insertions() -> Value {
    json!({
        "type": "object",
        "properties": {
            "root": { "type": "string", "description": "Publication coordinate, 30040:pubkey:d (or a standalone note's 30041:pubkey:d)." },
            "authors": { "type": "array", "items": { "type": "string" }, "description": "Optional 64-hex pubkeys to restrict to." },
            "policy": { "type": "string", "enum": ["local_only", "local_first", "fetch_always"], "description": "Default local_first." }
        },
        "required": ["root"]
    })
}

fn schema_propose_insertion() -> Value {
    json!({
        "type": "object",
        "properties": {
            "root": { "type": "string", "description": "The publication this note belongs to: 30040:pubkey:d." },
            "parent": { "type": "string", "description": "The section (30041:pubkey:d) or index it attaches to. Omit = the index itself (front matter / appendix)." },
            "placement": { "type": "string", "enum": ["attach", "before", "after"], "description": "attach (default) = a margin note with no place in the reading order; before/after = claims a slot next to the parent." },
            "title": { "type": "string", "description": "Section title (required by NKBIP-01)." },
            "content": { "type": "string", "description": "Section body." },
            "format": { "type": "string", "enum": ["markdown", "org", "asciidoc", "plain"], "description": "Content mode. Default markdown." },
            "context": { "type": "string", "description": "Optional excerpt of the parent this note responds to." }
        },
        "required": ["root", "title", "content"]
    })
}

fn schema_promote_insertion() -> Value {
    json!({
        "type": "object",
        "properties": {
            "index": { "type": "string", "description": "The index to copy: 30040:pubkey:d." },
            "insertion": { "type": "string", "description": "The note to splice in: its 30041:pubkey:d coordinate or 64-hex id." },
            "slot": { "type": "integer", "description": "Override the position (0-based, in the index's a-list). Default: what the note's placement claims." }
        },
        "required": ["index", "insertion"]
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
        .list_root_publications(FetchPolicy::LocalOnly, limit, None, false, None)
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
/// Resolve a context request — `{id}` | `{addr}` | `{title?, content}` — into
/// an entry with title, content, tags and (for events) the captured version.
/// Shared by the `add_to_context` tool and `POST /api/v1/context/entries`, so
/// a user pill and an assistant call produce identical entries.
pub async fn resolve_context_entry(
    engine: &Arc<Engine>,
    input: &Value,
    origin: ContextOrigin,
) -> Result<ContextEntry> {
    let title_override = input.get("title").and_then(Value::as_str);

    // Resolve the source: addressable coordinate, then event id, then raw idea.
    let (source, resolved_title, content, tags) =
        if let Some(addr_str) = input.get("addr").and_then(Value::as_str) {
            let addr = parse_address(addr_str)?;
            let ev = engine
                .get_addressable(addr.kind, &addr.pubkey, &addr.d_tag, FetchPolicy::LocalOnly)
                .await?
                .ok_or_else(|| {
                    EngineError::NotFound(format!("no local event for address {addr_str}"))
                })?;
            let (title, body) = event_title_content(&ev);
            let source = ContextSource::Addr {
                kind: addr.kind,
                pubkey: addr.pubkey,
                d_tag: addr.d_tag,
                event_id: ev.get("id").and_then(Value::as_str).map(str::to_string),
            };
            (source, title, body, event_tags(&ev))
        } else if let Some(id) = input.get("id").and_then(Value::as_str) {
            let ev = engine
                .get_by_id(id, FetchPolicy::LocalOnly)
                .await?
                .ok_or_else(|| EngineError::NotFound(format!("no local event with id {id}")))?;
            let (title, body) = event_title_content(&ev);
            (
                ContextSource::Event { id: id.to_string() },
                title,
                body,
                event_tags(&ev),
            )
        } else if let Some(raw) = input.get("content").and_then(Value::as_str) {
            (ContextSource::Literal, None, raw.to_string(), Vec::new())
        } else {
            return Err(EngineError::BadRequest(
                "provide one of 'id', 'addr', or 'content'".into(),
            ));
        };

    let title = title_override
        .map(str::to_string)
        .or(resolved_title)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| truncate(content.trim(), 48));

    Ok(ContextEntry::new(source, title, content, tags, origin))
}

fn event_tags(ev: &Value) -> Vec<Vec<String>> {
    ev.get("tags")
        .and_then(Value::as_array)
        .map(|ts| {
            ts.iter()
                .filter_map(Value::as_array)
                .map(|t| t.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .collect()
        })
        .unwrap_or_default()
}

/// Pull a reference into the shared context set. Writes the engine-owned set
/// directly and persists it; the web just re-reads. Returns a compact handle
/// (`added: false` means the source was already there).
async fn add_to_context(cx: &ToolCx, input: Value) -> Result<Value> {
    let entry = resolve_context_entry(&cx.engine, &input, ContextOrigin::Assistant).await?;
    let title = entry.title.clone();
    let snippet = truncate(entry.content.trim(), 200);
    let (id, added, count) = {
        let mut chat = cx.chat.lock().unwrap();
        let (id, added) = chat.context.add(entry);
        if added {
            cx.persist_context(&chat);
        }
        (id, added, chat.context.len())
    };
    Ok(json!({
        "type": "context",
        "id": id,
        "added": added,
        "title": title,
        "snippet": snippet,
        "count": count,
    }))
}

/// Compact handles for every entry in the set; `full: true` adds content.
async fn get_context(cx: &ToolCx, input: Value) -> Result<Value> {
    let full = input.get("full").and_then(Value::as_bool).unwrap_or(false);
    let chat = cx.chat.lock().unwrap();
    let entries: Vec<Value> = chat
        .context
        .entries
        .iter()
        .map(|e| {
            let mut v = json!({
                "id": e.id,
                "title": e.title,
                "source": e.source,
                "origin": e.origin,
                "modified": e.modified(),
                "snippet": truncate(e.content.trim(), 200),
            });
            if full {
                v["content"] = json!(e.content);
            }
            v
        })
        .collect();
    Ok(json!({ "count": entries.len(), "entries": entries }))
}

async fn remove_from_context(cx: &ToolCx, input: Value) -> Result<Value> {
    let id = require_str(&input, "id")?;
    let mut chat = cx.chat.lock().unwrap();
    let removed = chat.context.remove(id);
    if removed {
        cx.persist_context(&chat);
    }
    Ok(json!({ "removed": removed, "count": chat.context.len() }))
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
            slot_coord: None,
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

/// Sign a saved draft with the assistant's own key and ingest the events
/// (local-until-broadcast). Never the user's key: `Engine::assistant_signer`
/// resolves the assistant session only. Decision D11.
async fn sign_draft(cx: &ToolCx, input: Value) -> Result<Value> {
    use crate::publication::compose::ComposeState;
    let draft_id = require_str(&input, "draft_id")?;
    let store = crate::drafts::DraftStore::new(cx.engine.data_dir())
        .map_err(|e| EngineError::Other(format!("draft store: {e}")))?;
    let draft = store
        .load_draft(draft_id)
        .map_err(|e| EngineError::NotFound(format!("draft {draft_id}: {e}")))?;
    let mut compose: ComposeState = (&draft.compose_state).into();
    let signer = cx
        .engine
        .assistant_signing()
        .ok_or_else(|| EngineError::Auth("no assistant identity — set one in Settings".into()))?;
    let pubkey = signer
        .active_pubkey()
        .await
        .ok_or_else(|| EngineError::Auth("no assistant identity — set one in Settings".into()))?;
    let (pub_event, section_events) =
        crate::publication::build_signed_publication_events_via_signer(
            &mut compose,
            &pubkey,
            signer,
        )
        .await
        .map_err(|e| EngineError::Other(format!("sign: {e}")))?;
    let resp = crate::api::finalize_publish(
        &cx.engine,
        Some(pub_event),
        section_events,
        false,
        None,
        true,
    )
    .await?;
    let v = serde_json::to_value(resp.0)?;
    Ok(json!({
        "signed_as": "assistant",
        "pubkey": pubkey,
        "publication_id": v["publication_id"],
        "section_ids": v["section_ids"],
        "ingested": v["ingested"],
        "note": "Signed and stored locally. Use `publish` with the publication_id to broadcast.",
    }))
}

/// The assistant's own signer + pubkey, or a clear "set one up" error.
async fn assistant_signer(
    engine: &Arc<Engine>,
) -> Result<(String, crate::signing::SigningController)> {
    let signer = engine
        .assistant_signing()
        .ok_or_else(|| EngineError::Auth("no assistant identity — set one in Settings".into()))?;
    let pubkey = signer
        .active_pubkey()
        .await
        .ok_or_else(|| EngineError::Auth("no assistant identity — set one in Settings".into()))?;
    Ok((pubkey, signer.clone()))
}

/// The `#A` discovery pass as a tool — compact handles.
async fn list_insertions(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let root = require_str(&input, "root")?.to_string();
    let authors: Vec<String> = input
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let policy = input
        .get("policy")
        .and_then(Value::as_str)
        .unwrap_or("local_first")
        .to_string();
    let resp = crate::api::insertions::load_insertions(
        engine,
        crate::api::insertions::InsertionsLoadRequest {
            root,
            authors,
            policy: Some(policy),
            ..Default::default()
        },
    )
    .await?;
    let items: Vec<Value> = resp
        .insertions
        .iter()
        .map(|i| {
            let ev = &i.event;
            let title = ev
                .get("tags")
                .and_then(Value::as_array)
                .and_then(|tags| {
                    tags.iter().find_map(|t| {
                        let a = t.as_array()?;
                        (a.first()?.as_str()? == "title").then(|| a.get(1)?.as_str().map(String::from))?
                    })
                })
                .unwrap_or_default();
            let content = ev.get("content").and_then(Value::as_str).unwrap_or("");
            let snippet: String = content.chars().take(160).collect();
            json!({
                "coordinate": i.coordinate,
                "id": ev.get("id"),
                "title": title,
                "author": ev.get("pubkey"),
                "created_at": ev.get("created_at"),
                "parent": i.anchor.parent,
                "top_level": i.anchor.is_top_level(),
                "placement": i.anchor.placement,
                "snippet": snippet,
            })
        })
        .collect();
    Ok(json!({
        "root": input.get("root"),
        "count": items.len(),
        "insertions": items,
        "query": resp.query,
        "source": resp.source,
    }))
}

/// Write an anchored note as the assistant — local snapshot, no broadcast.
async fn propose_insertion(cx: &ToolCx, input: Value) -> Result<Value> {
    let anchor = crate::api::insertions::AnchorRequest {
        root: require_str(&input, "root")?.to_string(),
        parent: input.get("parent").and_then(Value::as_str).map(String::from),
        parent_event_id: None,
        placement: input.get("placement").and_then(Value::as_str).map(String::from),
    };
    let body = crate::api::insertions::InsertionBody {
        title: require_str(&input, "title")?.to_string(),
        content: require_str(&input, "content")?.to_string(),
        format: input.get("format").and_then(Value::as_str).map(String::from),
        context: input.get("context").and_then(Value::as_str).map(String::from),
        tags: Vec::new(),
    };
    let (mut template, resolved) =
        crate::api::insertions::insertion_template(&cx.engine, &anchor, &body).await?;
    let (pubkey, signer) = assistant_signer(&cx.engine).await?;
    template.pubkey = Some(pubkey.clone());
    let signed = signer
        .sign(template)
        .await
        .map_err(|e| EngineError::Other(format!("sign: {e}")))?;
    let coordinate = crate::publication::insertion::event_coordinate(&signed).unwrap_or_default();
    let resp = crate::api::finalize_publish(&cx.engine, None, vec![signed.clone()], false, None, true)
        .await?;
    let v = serde_json::to_value(resp.0)?;
    Ok(json!({
        "signed_as": "assistant",
        "pubkey": pubkey,
        "id": signed.get("id"),
        "coordinate": coordinate,
        "anchor": resolved,
        "ingested": v["ingested"],
        "note": "Attached and stored locally — not in the index. `publish` broadcasts it; promote_insertion (or the reader's promote) folds it into the structure.",
    }))
}

/// Republish an index with an insertion spliced in, as the assistant.
async fn promote_insertion(cx: &ToolCx, input: Value) -> Result<Value> {
    let req = crate::api::insertions::ForkRequest {
        index: require_str(&input, "index")?.to_string(),
        insertion: require_str(&input, "insertion")?.to_string(),
        slot: input.get("slot").and_then(Value::as_u64).map(|s| s as usize),
        sign_as: Some(crate::api::SignAs::Assistant),
        broadcast: false,
        relays: None,
        policy: None,
    };
    let (pubkey, signer) = assistant_signer(&cx.engine).await?;
    let preview = crate::api::insertions::fork_template(&cx.engine, &req, &pubkey).await?;
    let mut template = preview.event;
    template.pubkey = Some(pubkey.clone());
    let signed = signer
        .sign(template)
        .await
        .map_err(|e| EngineError::Other(format!("sign: {e}")))?;
    let coordinate = crate::publication::insertion::event_coordinate(&signed).unwrap_or_default();
    let resp = crate::api::finalize_publish(&cx.engine, Some(signed.clone()), vec![], false, None, true)
        .await?;
    let v = serde_json::to_value(resp.0)?;
    Ok(json!({
        "signed_as": "assistant",
        "pubkey": pubkey,
        "id": signed.get("id"),
        "coordinate": coordinate,
        "derivative": preview.derivative,
        "slot": preview.slot,
        "after": preview.after,
        "copied_tags": preview.copied_tags,
        "ingested": v["ingested"],
        "note": if preview.derivative {
            "A derivative index under the assistant's key (p + E name the original). Local until `publish`."
        } else {
            "A new version of the assistant's own index. Local until `publish`."
        },
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
    let (accepted, rejected, results) = crate::relay::publish_events_to_relays(
        &chosen,
        &[event_json.clone()],
        &engine.relay_auth(),
    )
    .await;
    for r in &results {
        let status = crate::api::publish_result_status(r);
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
