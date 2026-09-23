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

impl ToolPolicy {
    /// Restore a saved allowlist without freezing the catalog: a tool the
    /// saved list never knew about (`known` lacks it — added by a later
    /// build) gets the default policy's answer for its category, while every
    /// tool the user did see keeps exactly the saved on/off.
    pub fn from_saved(enabled: &[String], known: Option<&[String]>) -> Self {
        let mut policy = Self::from_enabled(enabled.iter().cloned());
        if let Some(known) = known {
            let default = Self::default();
            for t in catalog() {
                if !known.iter().any(|k| k == t.name) && default.enabled.contains(t.name) {
                    policy.enabled.insert(t.name.to_string());
                }
            }
        }
        policy
    }
}

impl Default for ToolPolicy {
    /// Every tool enabled — including Publish (2026-09-22: the assistant has
    /// its own identity and publishes under it, never the user's; broadcasts
    /// still pass the Confirm-mode publish confirmation, and the AI Tools
    /// settings tab can switch any tool off).
    fn default() -> Self {
        let enabled = catalog()
            .iter()
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
            description: "Search Nostr events with the structured query DSL — kind (k:30040), author (by:me / by:assistant / by:npub… / by:name:…), any tag (d:slug, T:title-slug, t:topic, a:coord), time (since:/until:), exact text (\"phrase\"), or a note1/nevent1/naddr1 entity. `scope` picks where: \"local\" (default) searches the index already on this machine and never touches the network; \"relays\" queries the user's relays and pulls what it finds into the index — in Confirm network mode this pops a confirmation the user approves first (declined ⇒ approved:false, use local results). Search locally first; go to relays when local comes up empty or the user asks for fresh/complete/remote results. Returns compact handles; expand with view_publication or get_event.",
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
            description: "Fetch one event by its 64-hex id. Reads the local index; with from_relays: true it asks the user's relays for it (Confirm mode pops a confirmation first) and stores the result. Call this to read the full JSON of an event you found via search_events, or to pull a specific id the user gave you that is not local.",
            category: ToolCategory::Read,
            input_schema: schema_get_event,
        },
        ToolDef {
            name: "get_addressable",
            description: "Fetch one replaceable/addressable event by its coordinate (kind, pubkey, d-tag). Reads the local index; with from_relays: true it asks the user's relays (Confirm mode pops a confirmation first) and stores the newest version found.",
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
            name: "get_identities",
            description: "Who you are and who the user is. Returns the assistant's own Nostr identity (pubkey, npub, signer source such as an engine key or a NIP-46 bunker, kind-0 name when known) and the user's (pubkey, npub, whether they can sign right now). Call this before searching with `by:assistant` (your own publications) or `by:me` (the user's), or whenever you are asked what you are, whose key signs your publications, or whether you can publish.",
            category: ToolCategory::Read,
            input_schema: schema_empty,
        },
        ToolDef {
            name: "publish",
            description: "Broadcast events that ALREADY EXIST locally (64-hex ids — pass `ids` for a whole publication: the index plus its sections) to relays. This does NOT sign or create content; it is the separate broadcast step for something already signed (a snapshot the user made, or your own `sign_draft` / `publish_publication` with broadcast: false). In Confirm mode it pops one publish confirmation the user must approve. Only call it when the user has asked to broadcast.",
            category: ToolCategory::Publish,
            input_schema: schema_publish,
        },
        ToolDef {
            name: "publish_publication",
            description: "Publish content under YOUR OWN identity (the assistant's key — engine key or bunker — never the user's): sign a title + ordered sections as an NKBIP-01 publication (30040 index + 30041 sections), store it locally, and broadcast it to the publish relays. Set `kind` (30023 long-form, 30818 wiki, or another addressable kind) with `content` to publish one atomic event instead. Call this when the user explicitly asks you to publish something as yourself; use `propose_section` when they want it in THEIR composer. In Confirm network mode the broadcast pops a confirmation the user approves; `broadcast: false` signs and stores only (a local snapshot you can `publish` later by id).",
            category: ToolCategory::Publish,
            input_schema: schema_publish_publication,
        },
        ToolDef {
            name: "sign_draft",
            description: "Sign a saved draft AS THE ASSISTANT — its own key, never the user's — and store the signed events locally. This is a snapshot, not a broadcast: the publication appears in the feed marked local; sending it to relays is the separate `publish` step the user confirms. Give the `draft_id` returned by save_draft. Disabled by default.",
            category: ToolCategory::Publish,
            input_schema: schema_sign_draft,
        },
        ToolDef {
            name: "list_insertions",
            description: "List the NKBIP-01 insertions attached to a publication — reader-written or assistant-written kind-30041 sections anchored to it (post-its, inserted pages) that are NOT in its index, and kind-30040 bundles (a whole kasten attached to the publication; `children` lists its notes). Give the root coordinate (30040:pubkey:d). Returns each note's coordinate, title, author, its parent section and placement (attach / before / after). policy=local_only reads the store; local_first / fetch_always may pull from relays (Confirm-gated).",
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
            name: "propose_bundle",
            description: "Deliver a whole kasten as ONE insertion, signed AS THE ASSISTANT and kept local: give the unsigned NKBIP-01 events of a kasten build (leaves first, root 30040 last) and the publication's root coordinate. The engine binds the key, stamps the ROOT with the anchor and the `request` pointer, signs every event in order and stores them. This is the shape for a request whose `output` is `kasten`: the answer is the single root 30040 that contains the whole thing — its notes carry no anchor and point at nothing; the root points at the publication. Use propose_insertion for loose notes.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_propose_bundle,
        },
        ToolDef {
            name: "list_requests",
            description: "List the user's annotation requests — tasks they wrote in the reader against a place in a publication (\"write notes about this section\", \"compare with…\"). Each has an anchor (root publication + parent section), the task, scope hints (section / siblings / publication / neighbours / relays / web), an optional selected excerpt, and whether it is open or answered. Two sources are merged: local records (rq_…) and requests the user signed as kind-30041 annotations (`coordinate` set; ev_… when there is no record). A signed request counts as answered as soon as a local note points at it with `request`. Start here when asked to check requests. Answer one by reading the anchor (view_publication / get_addressable / get_context), researching per its scope, writing notes with propose_insertion anchored to the same parent — passing the request's `coordinate` as `request` on EVERY note — then answer_request for a record.",
            category: ToolCategory::Read,
            input_schema: schema_list_requests,
        },
        ToolDef {
            name: "list_styles",
            description: "List the user's saved writing styles — how they want notes written (e.g. \"equations first, then the same step as code\"; \"an annotated reading guide\"). Each has a coordinate, a title and the instructions. A request names one by coordinate; follow its instructions when answering and pass the coordinate as `style` to propose_insertion.",
            category: ToolCategory::Read,
            input_schema: schema_list_styles,
        },
        ToolDef {
            name: "answer_request",
            description: "Close an annotation request RECORD (rq_…): a short summary and the coordinates of the notes you wrote (30041:pk:d — and the 30040 bundle when you made a kasten). No event is signed: the notes that answer a signed request point at it themselves (`request` on propose_insertion), and a request with no record (ev_…) is closed by those notes alone. Call it once per record, after propose_insertion has stored the notes.",
            category: ToolCategory::ComposeWrite,
            input_schema: schema_answer_request,
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
        "get_identities" => get_identities(engine).await,
        "publish" => publish(engine, input).await,
        "publish_publication" => publish_publication(cx, input).await,
        "sign_draft" => sign_draft(cx, input).await,
        "list_insertions" => list_insertions(engine, input).await,
        "propose_insertion" => propose_insertion(cx, input).await,
        "propose_bundle" => propose_bundle(cx, input).await,
        "promote_insertion" => promote_insertion(cx, input).await,
        "list_requests" => list_requests(engine, input).await,
        "list_styles" => list_styles(engine, input).await,
        "answer_request" => answer_request(engine, input).await,
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
            "limit": { "type": "integer", "description": "Max results to return." },
            "scope": {
                "type": "string",
                "enum": ["local", "relays"],
                "description": "\"local\" (default): the on-device index only. \"relays\": query the user's relays too (Confirm mode asks the user first)."
            }
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
            "id": { "type": "string", "description": "64-hex event id." },
            "from_relays": { "type": "boolean", "description": "Also ask the user's relays when it is not local (Confirm mode asks the user first). Default false." }
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
            "d_tag": { "type": "string", "description": "The d-tag identifier (may be empty)." },
            "from_relays": { "type": "boolean", "description": "Also ask the user's relays (Confirm mode asks the user first). Default false." }
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
            "id": { "type": "string", "description": "64-hex id of one local, already-signed event to broadcast." },
            "ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Several 64-hex ids to broadcast together in one confirmation — e.g. a publication's index id followed by its section ids."
            },
            "relays": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional relay URLs to broadcast to (defaults to the configured publish relays)."
            }
        }
    })
}

fn schema_empty() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

fn schema_publish_publication() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string", "description": "Publication (or atomic event) title." },
            "sections": {
                "type": "array",
                "description": "Ordered sections for the 30040/30041 path (ignored when `kind` is set).",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "content": { "type": "string" },
                        "level": { "type": "integer", "description": "Heading depth: 2 = top-level section, 3+ = nested." },
                        "tags": {
                            "type": "array",
                            "items": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2 },
                            "description": "Optional [name, value] tag pairs for this section."
                        }
                    },
                    "required": ["content"]
                }
            },
            "tags": {
                "type": "array",
                "items": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 2 },
                "description": "Optional [name, value] tag pairs on the index (or the atomic event), e.g. [[\"t\", \"nostr\"]]."
            },
            "kind": { "type": "integer", "description": "Publish ONE atomic event of this addressable kind (30023 long-form, 30818 wiki, …) with `content` as its body, instead of a section graph." },
            "content": { "type": "string", "description": "Body of the atomic event (only with `kind`)." },
            "d_tag": { "type": "string", "description": "Reuse this d tag to replace an earlier version instead of creating a new publication." },
            "broadcast": { "type": "boolean", "description": "Send to the publish relays after signing (default true). false = sign + store locally only." },
            "relays": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional relay URLs (defaults to the configured publish relays)."
            }
        },
        "required": ["title"]
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
            "context": { "type": "string", "description": "Optional excerpt of the parent this note responds to." },
            "replace": { "type": "string", "description": "Coordinate (30041:pk:d) of YOUR earlier note to replace — the new event reuses its d-tag, so it is a new version, not a second note." },
            "request": { "type": "string", "description": "The `coordinate` of the annotation request this note answers (30041:pk:d, from list_requests). Pass it on every note written for a request — it is what marks the request answered and lets the reader thread the note under it." },
            "style": { "type": "string", "description": "Coordinate of the style event (30078:pk:d) this note follows — copy it from the request's `style`. Stamped as a `style` tag so everything written in that style is one search." },
            "links": { "type": "array", "items": { "type": "string" }, "description": "Web URLs the note cites, one `r` tag each — the note's outbound links live in its tags, so a tag-walking reader lists them without parsing the prose." },
            "tags": { "type": "array", "items": { "type": "array", "items": { "type": "string" } }, "description": "Extra tag rows, e.g. [[\"a\", \"30041:pk:d\"]] for a cited coordinate. Anchor, d, title and request rows are set by the engine and cannot be overridden here." }
        },
        "required": ["root", "title", "content"]
    })
}

fn schema_propose_bundle() -> Value {
    json!({
        "type": "object",
        "properties": {
            "root": { "type": "string", "description": "The publication the bundle answers or annotates: 30040:pubkey:d. A bundle attaches to the whole publication, never to one section." },
            "events": { "type": "array", "items": { "type": "object" }, "description": "The unsigned NKBIP-01 events of the kasten in dependency order — leaves first, the root 30040 LAST — exactly as a kasten build emits them (`__PUBKEY__` where the signing key goes). Each needs a `d` tag; kinds 30040 / 30041 only." },
            "request": { "type": "string", "description": "The `coordinate` of the annotation request this bundle answers (30041:pk:d, from list_requests). Goes on the ROOT only — the single 30040 is the answer." },
            "context": { "type": "string", "description": "Optional excerpt of the publication the bundle responds to." },
            "tags": { "type": "array", "items": { "type": "array", "items": { "type": "string" } }, "description": "Extra rows for the root, e.g. link tags." }
        },
        "required": ["root", "events"]
    })
}

fn schema_list_requests() -> Value {
    json!({
        "type": "object",
        "properties": {
            "root": { "type": "string", "description": "Optional publication coordinate to restrict to." },
            "status": { "type": "string", "enum": ["open", "answered", "all"], "description": "Default open." }
        }
    })
}

fn schema_list_styles() -> Value {
    json!({ "type": "object", "properties": {} })
}

fn schema_answer_request() -> Value {
    json!({
        "type": "object",
        "properties": {
            "request_id": { "type": "string", "description": "The request record id (rq_…) from list_requests. An ev_… request has no record — it is answered by notes carrying `request`." },
            "summary": { "type": "string", "description": "One paragraph: what you wrote and where it came from." },
            "coordinates": { "type": "array", "items": { "type": "string" }, "description": "Coordinates of the notes you stored (from propose_insertion / promote_insertion)." }
        },
        "required": ["request_id", "summary"]
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
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("local");
    match scope {
        "relays" => relay_search(engine, q, &query).await,
        "local" => {
            let resp = engine.search(&query, FetchPolicy::LocalOnly, None).await?;
            let results: Vec<Value> = resp.results.iter().map(compact_result).collect();
            Ok(json!({ "scope": "local", "count": resp.count, "results": results }))
        }
        other => Err(EngineError::BadRequest(format!(
            "search_events: scope must be \"local\" or \"relays\" (got {other:?})"
        ))),
    }
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

fn wants_relays(input: &Value) -> bool {
    input
        .get("from_relays")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The relay set for a targeted fetch: the configured relays plus any relay
/// hints local events attach to this id / coordinate.
fn relays_for_target(engine: &Arc<Engine>, target: &str) -> Vec<String> {
    let mut relays = engine.relay_config().all_urls();
    for hint in engine.relay_hints_for_target(target) {
        if !relays.contains(&hint) {
            relays.push(hint);
        }
    }
    relays
}

async fn get_event(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let id = require_str(&input, "id")?;
    let local = engine.get_by_id(id, FetchPolicy::LocalOnly).await?;
    if local.is_some() || !wants_relays(&input) {
        return Ok(json!({ "found": local.is_some(), "source": "local", "event": local }));
    }
    // Not local and the model asked for relays: open a gated fetch operation
    // (the Confirm-mode intent modal), then fetch with the approved relays.
    let op = match engine
        .begin_fetch_operation(
            crate::network::FetchPattern::Event,
            format!("AI fetch event {}", truncate(id, 12)),
            vec![format!("Fetch event {id} by id")],
            relays_for_target(engine, id),
        )
        .await
    {
        Ok(o) => o,
        Err(_) => {
            return Ok(json!({
                "found": false,
                "approved": false,
                "note": "The user declined the relay fetch; the event is not in the local index.",
            }));
        }
    };
    let chosen = op.relays().to_vec();
    let event = engine
        .get_by_id_with_options(id, FetchPolicy::FetchAlways, Some(&chosen), true)
        .await?;
    op.complete(usize::from(event.is_some()));
    Ok(json!({ "found": event.is_some(), "source": "relays", "approved": true, "event": event }))
}

async fn get_addressable(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let kind = input
        .get("kind")
        .and_then(Value::as_u64)
        .ok_or_else(|| EngineError::BadRequest("get_addressable: missing 'kind'".into()))?;
    let pubkey = require_str(&input, "pubkey")?;
    let d_tag = input.get("d_tag").and_then(Value::as_str).unwrap_or("");
    let local = engine
        .get_addressable(kind, pubkey, d_tag, FetchPolicy::LocalOnly)
        .await?;
    if !wants_relays(&input) {
        return Ok(json!({ "found": local.is_some(), "source": "local", "event": local }));
    }
    // Addressable events can have a newer version on relays even when a
    // local copy exists, so from_relays always asks (gated) and keeps the
    // newest. Same intent flow as any user-initiated fetch.
    let coord = format!("{kind}:{pubkey}:{d_tag}");
    let op = match engine
        .begin_fetch_operation(
            crate::network::FetchPattern::Publication,
            format!("AI fetch {} ({})", kind, truncate(d_tag, 24)),
            vec![format!("Fetch coordinate {coord}")],
            relays_for_target(engine, &coord),
        )
        .await
    {
        Ok(o) => o,
        Err(_) => {
            return Ok(json!({
                "found": local.is_some(),
                "source": "local",
                "approved": false,
                "note": "The user declined the relay fetch; showing the local copy if any.",
                "event": local,
            }));
        }
    };
    let chosen = op.relays().to_vec();
    let event = engine
        .get_addressable_with_options(kind, pubkey, d_tag, FetchPolicy::FetchAlways, Some(&chosen), true)
        .await?;
    op.complete(usize::from(event.is_some()));
    let event = event.or(local);
    Ok(json!({ "found": event.is_some(), "source": "relays", "approved": true, "event": event }))
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

/// Run a query against relays — the SAME flow as a user-initiated relay
/// search: `begin_fetch_operation` emits an Intent that, in Confirm mode,
/// blocks on the FetchConfirmModal until the user approves (or declines →
/// `approved: false`). Auto mode proceeds immediately. Tag filters (`d:`,
/// `T:`, `t:`, `a:`…) travel in the DSL like any other clause.
async fn relay_search(engine: &Arc<Engine>, q: &str, query: &SearchQuery) -> Result<Value> {
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
                "scope": "relays",
                "approved": false,
                "count": 0,
                "results": [],
                "note": "The user declined the relay fetch; use local results instead.",
            }));
        }
    };

    let chosen = op.relays().to_vec();
    let resp = engine
        .search_with_options(query, FetchPolicy::FetchAlways, Some(&chosen), true)
        .await?;
    op.complete(resp.count);

    let results: Vec<Value> = resp.results.iter().map(compact_result).collect();
    Ok(json!({
        "scope": "relays",
        "approved": true,
        "relays": chosen,
        "count": resp.count,
        "local_count": resp.local_count,
        "relay_count": resp.relay_count,
        "results": results,
    }))
}

/// `fetch_from_relays`: kept as an explicit alias of `search_events` with
/// scope "relays" (older prompts and transcripts name it).
async fn fetch_from_relays(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let q = require_str(&input, "query")?;
    let mut query = SearchQuery::parse(q)
        .map_err(|e| EngineError::BadRequest(format!("invalid query: {e}")))?;
    if let Some(limit) = input.get("limit").and_then(Value::as_u64) {
        query.limit = Some(limit as usize);
    }
    relay_search(engine, q, &query).await
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
        "note": "Signed and stored locally. Use `publish` with ids: [publication_id, ...section_ids] to broadcast the whole publication in one confirmation.",
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
/// `links` (URLs → `r` rows) and `tags` (explicit rows) from a tool input.
/// Rows that would collide with what the engine sets are dropped.
fn link_tags(input: &Value) -> Vec<Vec<String>> {
    const RESERVED: &[&str] = &["d", "title", "T", "A", "K", "P", "a", "k", "p", "e", "insert", "request", "m", "format"];
    let mut out: Vec<Vec<String>> = Vec::new();
    if let Some(links) = input.get("links").and_then(Value::as_array) {
        for l in links.iter().filter_map(Value::as_str) {
            let l = l.trim();
            if l.starts_with("http://") || l.starts_with("https://") {
                out.push(vec!["r".to_string(), l.to_string()]);
            }
        }
    }
    if let Some(rows) = input.get("tags").and_then(Value::as_array) {
        for row in rows.iter().filter_map(Value::as_array) {
            let cells: Vec<String> = row.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if cells.len() >= 2 && !cells[0].is_empty() && !RESERVED.contains(&cells[0].as_str()) {
                out.push(cells);
            }
        }
    }
    out
}

/// A whole kasten as ONE insertion: sign every event of a built tree as the
/// assistant and attach its root 30040 to the publication.
async fn propose_bundle(cx: &ToolCx, input: Value) -> Result<Value> {
    let events: Vec<Value> = input
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| EngineError::BadRequest("propose_bundle: `events` (array of unsigned events, root last) is required".into()))?;
    let req = crate::api::insertions::BundleRequest {
        anchor: crate::api::insertions::AnchorRequest {
            root: require_str(&input, "root")?.to_string(),
            ..Default::default()
        },
        events,
        request: input.get("request").and_then(Value::as_str).map(String::from),
        context: input.get("context").and_then(Value::as_str).map(String::from),
        tags: link_tags(&input),
        sign_as: None,
        broadcast: false,
        relays: None,
    };
    let (pubkey, signer) = assistant_signer(&cx.engine).await?;
    let (templates, resolved) = crate::api::insertions::bundle_templates(&cx.engine, &req, &pubkey).await?;
    let mut signed = Vec::with_capacity(templates.len());
    for t in templates {
        signed.push(
            signer
                .sign(t)
                .await
                .map_err(|e| EngineError::Other(format!("sign: {e}")))?,
        );
    }
    let publish = crate::api::insertions::store_bundle(&cx.engine, signed.clone(), false, None).await?;
    let coordinates: Vec<String> = signed
        .iter()
        .filter_map(crate::publication::insertion::event_coordinate)
        .collect();
    let root = signed.last().cloned().unwrap_or(Value::Null);
    let v = serde_json::to_value(publish)?;
    Ok(json!({
        "signed_as": "assistant",
        "pubkey": pubkey,
        "id": root.get("id"),
        "coordinate": crate::publication::insertion::event_coordinate(&root),
        "coordinates": coordinates,
        "count": signed.len(),
        "anchor": resolved,
        "request": req.request,
        "ingested": v["ingested"],
        "note": "Stored locally as one bundle: the root 30040 is the insertion (anchored to the publication, carrying `request`); its notes are plain NKBIP-01 events under it. `publish` broadcasts; the curator promotes the root as a sub-index.",
    }))
}

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
        tags: {
            let mut tags: Vec<Vec<String>> = input
                .get("style")
                .and_then(Value::as_str)
                .map(|c| vec![vec!["style".to_string(), c.to_string()]])
                .unwrap_or_default();
            tags.extend(link_tags(&input));
            tags
        },
        replace: input.get("replace").and_then(Value::as_str).map(String::from),
        request: input.get("request").and_then(Value::as_str).map(String::from),
    };
    let (mut template, resolved) =
        crate::api::insertions::insertion_template(&cx.engine, &anchor, &body).await?;
    let (pubkey, signer) = assistant_signer(&cx.engine).await?;
    crate::api::insertions::check_replace_owner(&body, &pubkey)?;
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
        "request": body.request,
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

/// The user's annotation requests (local records).
async fn list_requests(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let root = input.get("root").and_then(Value::as_str);
    let status = input.get("status").and_then(Value::as_str).unwrap_or("open");
    let list = crate::api::requests::list_requests(engine, root, None, Some(status))?;
    let mut requests = Vec::with_capacity(list.len());
    for r in list {
        requests.push(crate::api::requests::request_with_style(engine, r).await);
    }
    Ok(json!({
        "count": requests.len(),
        "requests": requests,
        "note": "Answer one with propose_insertion (anchored to its parent, root = its root; pass its `coordinate` as `request` on every note so it is marked answered, pass its `style` so the notes carry it, and follow `style_instructions`) then answer_request for an rq_… record. When `output` is `kasten`, build the notes as a kasten and deliver it with ONE propose_bundle (root = its root, `request` on the bundle) — the single 30040 is the answer, not one note per event.",
    }))
}

/// The saved writing styles (local style events).
async fn list_styles(engine: &Arc<Engine>, _input: Value) -> Result<Value> {
    let styles = crate::api::styles::list_styles(engine).await?;
    Ok(json!({ "count": styles.len(), "styles": styles }))
}

/// Close a request with a summary and the written coordinates.
async fn answer_request(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let id = require_str(&input, "request_id")?.to_string();
    let summary = require_str(&input, "summary")?.to_string();
    let coordinates: Vec<String> = input
        .get("coordinates")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let r = crate::api::requests::answer_request(
        engine,
        &id,
        crate::api::requests::AnswerRequest {
            summary,
            coordinates,
            sign_as: Some(crate::api::SignAs::Assistant),
        },
    )
    .await?;
    Ok(serde_json::to_value(r)?)
}

/// Broadcast already-signed local events to relays — the deliberate
/// "broadcast" step, decoupled from signing. One `begin_publish_operation`
/// covers the whole batch (PublishConfirmModal in Confirm mode; Auto
/// proceeds). On success, records relay provenance so the feed pill flips
/// local → published.
async fn broadcast_ids(
    engine: &Arc<Engine>,
    ids: &[String],
    relays: Option<Vec<String>>,
    label: String,
) -> Result<Value> {
    if ids.is_empty() {
        return Err(EngineError::BadRequest("no event ids to broadcast".into()));
    }
    let mut event_jsons: Vec<String> = Vec::with_capacity(ids.len());
    for id in ids {
        let event = engine
            .get_by_id(id, FetchPolicy::LocalOnly)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("no local event with id {id}")))?;
        event_jsons.push(
            serde_json::to_string(&event)
                .map_err(|e| EngineError::Other(format!("serialize: {e}")))?,
        );
    }
    let relays = relays
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| engine.publish_relays());

    let op = match engine
        .begin_publish_operation(label, relays.clone(), ids.to_vec(), None, None)
        .await
    {
        Ok(o) => o,
        Err(_) => {
            return Ok(json!({
                "published": false,
                "note": "The user declined to broadcast.",
            }));
        }
    };

    let chosen = op.relays().to_vec();
    for url in &chosen {
        op.relay_status(url.clone(), crate::network::RelayStatusValue::Connecting);
    }
    let (accepted, rejected, results) =
        crate::relay::publish_events_to_relays(&chosen, &event_jsons, &engine.relay_auth())
            .await;
    for r in &results {
        let status = crate::api::publish_result_status(r);
        op.relay_status(r.relay_url.clone(), status);
        if r.success {
            // Best-effort provenance — failure only delays the feed pill flip.
            for ej in &event_jsons {
                let _ = engine.record_event_relay(ej, &r.relay_url);
            }
        }
    }
    op.complete(accepted);

    let out: Vec<Value> = results
        .iter()
        .map(|r| json!({ "relay": r.relay_url, "success": r.success, "message": r.message }))
        .collect();
    Ok(json!({
        "published": accepted > 0,
        "event_count": ids.len(),
        "accepted": accepted,
        "rejected": rejected,
        "results": out,
    }))
}

fn ids_from_input(input: &Value) -> Vec<String> {
    let mut ids: Vec<String> = input
        .get("ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if let Some(id) = input.get("id").and_then(Value::as_str) {
        if !ids.iter().any(|x| x == id) {
            ids.insert(0, id.to_string());
        }
    }
    ids
}

fn relays_from_input(input: &Value) -> Option<Vec<String>> {
    input.get("relays").and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>()
    })
}

fn tag_pairs(v: Option<&Value>) -> Vec<crate::publication::compose::TagEntry> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let t = t.as_array()?;
                    Some(crate::publication::compose::TagEntry {
                        name: t.first()?.as_str()?.to_string(),
                        value: t.get(1)?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `publish`: broadcast one id or a batch of ids.
async fn publish(engine: &Arc<Engine>, input: Value) -> Result<Value> {
    let ids = ids_from_input(&input);
    if ids.is_empty() {
        return Err(EngineError::BadRequest(
            "pass `id` (one event) or `ids` (several) to broadcast".into(),
        ));
    }
    let relays = relays_from_input(&input);
    let label = if ids.len() == 1 {
        format!("AI broadcast {}", truncate(&ids[0], 12))
    } else {
        format!("AI broadcast {} events ({}…)", ids.len(), truncate(&ids[0], 12))
    };
    broadcast_ids(engine, &ids, relays, label).await
}

/// `publish_publication`: sign as the assistant (its own key / bunker — never
/// the user's), ingest, and broadcast in one call. The signing half mirrors
/// `POST /api/v1/publish` with `sign_as: assistant`; the broadcast half is
/// the same Confirm-gated batch as `publish`.
#[allow(clippy::field_reassign_with_default)] // mirrors api::publish_handler's SectionCompose build
async fn publish_publication(cx: &ToolCx, input: Value) -> Result<Value> {
    use crate::publication::compose::{ComposeState, SectionCompose};
    let engine = &cx.engine;
    let title = require_str(&input, "title")?.to_string();
    let broadcast = input
        .get("broadcast")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let relays = relays_from_input(&input);
    let d_tag = input.get("d_tag").and_then(Value::as_str).map(String::from);

    let signer = engine.assistant_signing().ok_or_else(|| {
        EngineError::Auth(
            "no assistant identity — the user sets one (engine key or bunker) in Settings → assistant identity"
                .into(),
        )
    })?;
    let pubkey = signer.active_pubkey().await.ok_or_else(|| {
        EngineError::Auth("assistant identity is not unlocked / connected".into())
    })?;

    let kind = input
        .get("kind")
        .and_then(Value::as_u64)
        .filter(|k| *k != crate::publication::KIND_PUBLICATION_INDEX);
    let (pub_event, section_events) = if let Some(kind) = kind {
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let tags = tag_pairs(input.get("tags"));
        let ev = crate::publication::build_signed_atomic_event_via_signer(
            kind as u32,
            &title,
            &content,
            &tags,
            d_tag.as_deref(),
            &pubkey,
            signer,
        )
        .await
        .map_err(|e| EngineError::Other(format!("sign: {e}")))?;
        (ev, vec![])
    } else {
        let sections = input
            .get("sections")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if sections.is_empty() {
            return Err(EngineError::BadRequest(
                "a publication needs at least one section (or set `kind` + `content` for one atomic event)"
                    .into(),
            ));
        }
        let mut compose = ComposeState::new();
        compose.title = title.clone();
        compose.tags = tag_pairs(input.get("tags"));
        compose.d_tag = d_tag.clone();
        compose.sections = sections
            .iter()
            .map(|sec| {
                let mut sc = SectionCompose::default();
                sc.title = sec
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                sc.content = sec
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                sc.level = sec.get("level").and_then(Value::as_u64).unwrap_or(2) as u8;
                sc.tags = tag_pairs(sec.get("tags"));
                sc
            })
            .collect();
        crate::publication::build_signed_publication_events_via_signer(&mut compose, &pubkey, signer)
            .await
            .map_err(|e| EngineError::Other(format!("sign: {e}")))?
    };

    let resp = crate::api::finalize_publish(engine, Some(pub_event), section_events, false, None, true)
        .await?;
    let v = serde_json::to_value(resp.0)?;
    let publication_id = v["publication_id"].as_str().unwrap_or_default().to_string();
    let mut ids = vec![publication_id.clone()];
    ids.extend(
        v["section_ids"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default(),
    );

    let broadcast_result = if broadcast {
        Some(
            broadcast_ids(
                engine,
                &ids,
                relays,
                format!("AI publish \"{}\" ({} events)", truncate(&title, 40), ids.len()),
            )
            .await?,
        )
    } else {
        None
    };
    let published = broadcast_result
        .as_ref()
        .and_then(|b| b["published"].as_bool())
        .unwrap_or(false);
    Ok(json!({
        "signed_as": "assistant",
        "pubkey": pubkey,
        "publication_id": publication_id,
        "section_ids": v["section_ids"],
        "event_ids": ids,
        "stored_locally": v["ingested"],
        "published": published,
        "broadcast": broadcast_result,
        "note": if broadcast {
            if published { "Signed with your own key and broadcast." } else { "Signed and stored locally; the broadcast did not reach a relay (declined or rejected) — it can be retried with `publish`." }
        } else {
            "Signed and stored locally only (local snapshot). Use `publish` with event_ids to broadcast."
        },
    }))
}

/// `get_identities`: the assistant's own identity and the user's, so the
/// model knows what it is, whose key signs its publications, and how to
/// search for either (`by:assistant` / `by:me`).
async fn get_identities(engine: &Arc<Engine>) -> Result<Value> {
    fn npub(hex: &str) -> Option<String> {
        crate::identity::encode_npub(hex).ok()
    }
    async fn kind0_name(engine: &Arc<Engine>, pubkey: &str) -> Option<String> {
        let filter = json!({ "kinds": [0], "authors": [pubkey], "limit": 1 });
        let resp = engine
            .get_events_with_options(vec![filter], FetchPolicy::LocalOnly, None, false)
            .await
            .ok()?;
        let ev = resp.events.first()?;
        let content = ev.get("content").and_then(Value::as_str)?;
        let meta = crate::user_data::Metadata::from_event_content(content, 0)?;
        meta.display_name.or(meta.name)
    }

    let assistant_pubkey = engine.assistant_pubkey();
    let assistant_source = engine.assistant_signing().map(|c| match c.current_source() {
        crate::identity::IdentitySource::Engine => "engine key (stored in tendrl)".to_string(),
        crate::identity::IdentitySource::Nip46 { .. } => "NIP-46 bunker (remote signer)".to_string(),
        crate::identity::IdentitySource::Nip55 { .. } => "NIP-55 signer app (Amber)".to_string(),
        crate::identity::IdentitySource::Nip07 { .. } => "NIP-07 browser extension".to_string(),
    });
    let assistant_name = match &assistant_pubkey {
        Some(pk) => kind0_name(engine, pk).await,
        None => None,
    };
    let user_pubkey = engine.my_pubkey();
    let user_name = match &user_pubkey {
        Some(pk) => kind0_name(engine, pk).await,
        None => None,
    };
    let can_publish = assistant_pubkey.is_some();
    Ok(json!({
        "assistant": {
            "pubkey": assistant_pubkey,
            "npub": assistant_pubkey.as_deref().and_then(npub),
            "name": assistant_name,
            "signer": assistant_source,
            "can_publish": can_publish,
            "search": "by:assistant",
            "note": if can_publish {
                "This is YOU. Events you publish (publish_publication / sign_draft) are signed with this key and appear under by:assistant. The user's key is never used for your publications."
            } else {
                "You have no signing identity yet, so you cannot publish. The user can give you one (an engine key or a bunker) in Settings → assistant identity."
            },
        },
        "user": {
            "pubkey": user_pubkey,
            "npub": user_pubkey.as_deref().and_then(npub),
            "name": user_name,
            "logged_in": user_pubkey.is_some(),
            "search": "by:me",
            "note": "The person you are helping. Their publications are under by:me. You never sign as them — drafts for their composer go through propose_section.",
        },
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
    fn saved_allowlist_does_not_freeze_out_new_tools() {
        // The user saved a list before `list_requests` existed and had
        // turned `semantic_search` off deliberately.
        let known: Vec<String> = catalog()
            .iter()
            .map(|t| t.name.to_string())
            .filter(|n| n != "list_requests" && n != "answer_request")
            .collect();
        let enabled: Vec<String> = known
            .iter()
            .filter(|n| *n != "semantic_search" && !n.starts_with("publish"))
            .cloned()
            .collect();
        let p = ToolPolicy::from_saved(&enabled, Some(&known));
        assert!(!p.enabled.contains("semantic_search"), "an explicit off stays off");
        assert!(p.enabled.contains("list_requests"), "a tool the list never saw gets its default");
        assert!(p.enabled.contains("answer_request"));
        assert!(!p.enabled.contains("publish"), "Publish-category defaults stay off");
        // No known list (older config): behaves exactly like from_enabled.
        let q = ToolPolicy::from_saved(&enabled, None);
        assert!(!q.enabled.contains("list_requests"));
    }

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
    fn default_policy_enables_every_tool_and_gates_publish() {
        let policy = ToolPolicy::default();
        for t in catalog() {
            assert!(policy.enabled.contains(t.name), "tool {} enablement", t.name);
        }
        assert!(policy.require_approval.contains(&ToolCategory::Publish));
        assert!(policy.require_approval.contains(&ToolCategory::Network));
        assert!(catalog().iter().any(|t| t.name == "publish_publication"));
        assert!(catalog().iter().any(|t| t.name == "get_identities"));
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
