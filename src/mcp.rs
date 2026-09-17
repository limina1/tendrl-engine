//! The engine speaks MCP — Streamable HTTP transport on `/mcp`.
//!
//! One catalog: the same `tools::catalog()` / `tools::dispatch()` the in-app
//! agent loop uses, filtered by the same live `ToolPolicy` (Settings → AI
//! tools), so Claude Code, Codex and the chat pane call identical functions.
//! Confirm-mode gating needs nothing extra — every relay fetch and every
//! broadcast a tool performs raises its intent inside the engine, exactly as
//! an in-app call does.
//!
//! Transport (MCP spec 2025-03-26 / 2025-06-18, stateless): the client POSTs
//! JSON-RPC messages (single or batch) to `/mcp`; requests get a JSON body
//! back, notifications and client responses get `202 Accepted`. No
//! server-initiated stream, so `GET /mcp` is `405` and no `Mcp-Session-Id` is
//! issued. Auth: a bearer token — `[mcp] token` in config, else one minted
//! once into `<data_dir>/mcp-token` — or the mobile host's loopback token.
//!
//! Design: `docs/zettel/feat-mcp.org`; plan Phase 4.

use crate::api::ChatAppState;
use crate::tools;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Extension, Json, Router,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Protocol versions this server speaks, newest first. `initialize` echoes
/// the client's version when it is one of these, else offers the newest.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub const TOKEN_FILE: &str = "mcp-token";

/// What a request must carry to reach the tools.
#[derive(Debug, Clone)]
pub struct McpAuth {
    pub token: String,
    /// The mobile host's per-boot loopback token also passes.
    pub loopback: Option<String>,
    /// Where the token lives on disk (`None` when it came from config).
    pub token_path: Option<PathBuf>,
    /// The URL a harness connects to.
    pub url: String,
}

impl McpAuth {
    pub fn accepts(&self, headers: &HeaderMap) -> bool {
        let bearer = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
            .map(str::trim);
        let x_token = headers.get("x-tendrl-token").and_then(|h| h.to_str().ok());
        let presented = bearer.or(x_token);
        match presented {
            Some(t) => t == self.token || self.loopback.as_deref() == Some(t),
            None => false,
        }
    }

    /// The one-liner that registers this engine in Claude Code.
    pub fn claude_mcp_add(&self) -> String {
        format!(
            "claude mcp add --transport http tendrl {} --header \"Authorization: Bearer {}\"",
            self.url, self.token
        )
    }
}

/// The token: `configured` wins; else the one in `<data_dir>/mcp-token`,
/// minted on first boot (0600 on Unix). Returns the token and its file path.
pub fn load_or_mint_token(
    data_dir: &Path,
    configured: Option<&str>,
) -> std::io::Result<(String, Option<PathBuf>)> {
    if let Some(t) = configured.map(str::trim).filter(|t| !t.is_empty()) {
        return Ok((t.to_string(), None));
    }
    let path = data_dir.join(TOKEN_FILE);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return Ok((t, Some(path)));
        }
    }
    let token = mint_token();
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(&path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok((token, Some(path)))
}

fn mint_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `/mcp` (the transport) and `GET /api/v1/mcp` (how to connect).
pub fn routes(state: ChatAppState, auth: Arc<McpAuth>) -> Router {
    Router::new()
        .route(
            "/mcp",
            axum::routing::post(mcp_post)
                .get(|| async { (StatusCode::METHOD_NOT_ALLOWED, "no server-initiated stream") })
                .delete(|| async { StatusCode::NO_CONTENT }),
        )
        .route("/api/v1/mcp", get(mcp_info))
        .layer(Extension(auth))
        .with_state(state)
}

/// GET /api/v1/mcp — the connection details for a harness. The token is
/// included: the desktop API has no auth of its own, so this reveals nothing
/// a local caller could not already do through `/api/v1`.
async fn mcp_info(
    State(state): State<ChatAppState>,
    Extension(auth): Extension<Arc<McpAuth>>,
) -> Json<Value> {
    let policy = state.policy.read().unwrap().clone();
    let enabled: Vec<&str> = tools::catalog()
        .iter()
        .filter(|t| policy.enabled.contains(t.name))
        .map(|t| t.name)
        .collect();
    Json(json!({
        "url": auth.url,
        "token": auth.token,
        "token_path": auth.token_path,
        "protocol_versions": PROTOCOL_VERSIONS,
        "enabled_tools": enabled,
        "claude_mcp_add": auth.claude_mcp_add(),
    }))
}

/// POST /mcp — one JSON-RPC message or a batch.
async fn mcp_post(
    State(state): State<ChatAppState>,
    Extension(auth): Extension<Arc<McpAuth>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !auth.accepts(&headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": { "message": "missing or invalid MCP token" } })),
        )
            .into_response();
    }
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::OK,
                Json(rpc_error(Value::Null, -32700, format!("parse error: {e}"))),
            )
                .into_response();
        }
    };
    let (messages, batch) = match parsed {
        Value::Array(items) => (items, true),
        other => (vec![other], false),
    };
    let mut responses = Vec::new();
    for msg in messages {
        if let Some(r) = handle_message(&state, msg).await {
            responses.push(r);
        }
    }
    if responses.is_empty() {
        return StatusCode::ACCEPTED.into_response();
    }
    let body = if batch {
        Value::Array(responses)
    } else {
        responses.into_iter().next().unwrap_or(Value::Null)
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn rpc_error(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// Route one JSON-RPC message. A request yields a response; a notification
/// or a client-side response yields nothing.
pub async fn handle_message(state: &ChatAppState, msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str);
    let id = msg.get("id").cloned();
    match (method, id) {
        // A response from the client to something we never sent — ignore.
        (None, _) => None,
        (Some(m), None) => {
            handle_notification(m);
            None
        }
        (Some(m), Some(id)) => {
            let params = msg.get("params").cloned().unwrap_or(json!({}));
            Some(handle_request(state, id, m, params).await)
        }
    }
}

fn handle_notification(method: &str) {
    match method {
        "notifications/initialized" => tracing::info!("MCP: client initialized"),
        "notifications/cancelled" => {}
        other => tracing::debug!("MCP: ignoring notification {other}"),
    }
}

async fn handle_request(state: &ChatAppState, id: Value, method: &str, params: Value) -> Value {
    match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            let client = params
                .get("clientInfo")
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            tracing::info!("MCP: initialize from {client} (protocol {version})");
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "tendrl-engine", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => rpc_result(id, json!({})),
        "tools/list" => {
            let defs = {
                let policy = state.policy.read().unwrap();
                tools::definitions(&policy)
            };
            let list: Vec<Value> = defs
                .into_iter()
                .map(|d| {
                    json!({
                        "name": d.name,
                        "description": d.description,
                        "inputSchema": d.input_schema,
                    })
                })
                .collect();
            rpc_result(id, json!({ "tools": list }))
        }
        "tools/call" => {
            let name = match params.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => return rpc_error(id, -32602, "tools/call needs `name`".into()),
            };
            let enabled = state.policy.read().unwrap().enabled.contains(&name);
            let known = tools::catalog().iter().any(|t| t.name == name);
            if !known {
                return rpc_error(id, -32602, format!("unknown tool `{name}`"));
            }
            if !enabled {
                return rpc_error(
                    id,
                    -32602,
                    format!("tool `{name}` is not enabled (Settings → AI tools)"),
                );
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            let cx = tools::ToolCx {
                engine: state.engine.clone(),
                chat: state.chat.clone(),
            };
            match tools::dispatch(&cx, &name, args).await {
                Ok(out) => {
                    let text = serde_json::to_string_pretty(&out).unwrap_or_default();
                    let mut result = json!({
                        "content": [{ "type": "text", "text": text }],
                        "isError": false,
                    });
                    if out.is_object() {
                        result["structuredContent"] = out;
                    }
                    rpc_result(id, result)
                }
                Err(e) => rpc_result(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": e.to_string() }],
                        "isError": true,
                    }),
                ),
            }
        }
        "prompts/list" => rpc_result(id, json!({ "prompts": [] })),
        "resources/list" => rpc_result(id, json!({ "resources": [] })),
        "resources/templates/list" => rpc_result(id, json!({ "resourceTemplates": [] })),
        "logging/setLevel" => rpc_result(id, json!({})),
        other => rpc_error(id, -32601, format!("method not found: {other}")),
    }
}

const INSTRUCTIONS: &str = "tendrl is a local-first reader, writer and reference manager on Nostr \
(NKBIP-01 publications: kind 30040 indexes of 30041 sections). Start with get_context — the \
user's curated context set — and search_events (the query DSL). Anything you write is signed \
with the assistant's own key, never the user's: propose_insertion attaches a note to a place in \
a publication (pending until the human promotes it); save_draft + sign_draft make a local \
publication. Relay fetches and broadcasts may pause for the user's confirmation in the app.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ChatAppState;
    use crate::chat::ChatState;
    use crate::llm::NoopProvider;
    use std::sync::Mutex;

    fn make_state() -> ChatAppState {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("prompt.md");
        let engine = Arc::new(crate::engine::Engine::new(dir.path()).unwrap());
        std::mem::forget(dir);
        ChatAppState {
            chat: Arc::new(Mutex::new(ChatState::new())),
            provider: Arc::new(NoopProvider::echo()),
            engine,
            max_tool_turns: 25,
            policy: Arc::new(std::sync::RwLock::new(tools::ToolPolicy::default())),
            system_prompt_path: prompt_path,
        }
    }

    #[tokio::test]
    async fn initialize_echoes_a_supported_version_and_offers_newest_otherwise() {
        let s = make_state();
        let r = handle_message(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","clientInfo":{"name":"t"}}}),
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(r["result"]["serverInfo"]["name"], "tendrl-engine");
        let r = handle_message(
            &s,
            json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}),
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[tokio::test]
    async fn notifications_and_client_responses_yield_nothing() {
        let s = make_state();
        assert!(handle_message(&s, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await.is_none());
        assert!(handle_message(&s, json!({"jsonrpc":"2.0","id":9,"result":{}})).await.is_none());
        let r = handle_message(&s, json!({"jsonrpc":"2.0","id":3,"method":"nope"})).await.unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn tools_list_is_the_policy_filtered_catalog() {
        let s = make_state();
        let r = handle_message(&s, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await.unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"search_events"));
        assert!(names.contains(&"get_context"));
        assert!(names.contains(&"propose_insertion"));
        assert!(!names.contains(&"publish"), "Publish tools are off by default");
        assert!(r["result"]["tools"][0]["inputSchema"].is_object());
        // Disable everything but one: the list follows the live policy.
        *s.policy.write().unwrap() = tools::ToolPolicy::from_enabled(vec!["get_context".into()]);
        let r = handle_message(&s, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await.unwrap();
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tools_call_dispatches_and_reports_errors_in_band() {
        let s = make_state();
        // add_to_context writes the engine-owned set; get_context reads it back.
        let r = handle_message(
            &s,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"add_to_context","arguments":{"title":"Style","content":"short sentences"}}}),
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["isError"], false);
        assert!(r["result"]["structuredContent"].is_object());
        let r = handle_message(
            &s,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_context"}}),
        )
        .await
        .unwrap();
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("short sentences"));
        // A tool failure is a result with isError, not a JSON-RPC error.
        let r = handle_message(
            &s,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_event","arguments":{}}}),
        )
        .await
        .unwrap();
        assert_eq!(r["result"]["isError"], true);
        // Unknown and disabled tools are protocol errors.
        let r = handle_message(&s, json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"zzz"}})).await.unwrap();
        assert_eq!(r["error"]["code"], -32602);
        let r = handle_message(&s, json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"publish","arguments":{"id":"x"}}})).await.unwrap();
        assert!(r["error"]["message"].as_str().unwrap().contains("not enabled"));
    }

    #[test]
    fn token_is_minted_once_and_config_wins() {
        let dir = tempfile::tempdir().unwrap();
        let (a, pa) = load_or_mint_token(dir.path(), None).unwrap();
        let (b, _) = load_or_mint_token(dir.path(), None).unwrap();
        assert_eq!(a, b, "second boot reads the same token");
        assert_eq!(a.len(), 48);
        assert_eq!(pa.unwrap(), dir.path().join(TOKEN_FILE));
        let (c, pc) = load_or_mint_token(dir.path(), Some(" secret ")).unwrap();
        assert_eq!(c, "secret");
        assert!(pc.is_none());
    }

    #[test]
    fn auth_accepts_bearer_x_token_or_loopback() {
        let auth = McpAuth {
            token: "abc".into(),
            loopback: Some("loop".into()),
            token_path: None,
            url: "http://127.0.0.1:3030/mcp".into(),
        };
        let mut h = HeaderMap::new();
        assert!(!auth.accepts(&h));
        h.insert(axum::http::header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert!(auth.accepts(&h));
        h.insert(axum::http::header::AUTHORIZATION, "Bearer nope".parse().unwrap());
        assert!(!auth.accepts(&h));
        h.remove(axum::http::header::AUTHORIZATION);
        h.insert("x-tendrl-token", "loop".parse().unwrap());
        assert!(auth.accepts(&h));
        assert!(auth.claude_mcp_add().contains("--transport http tendrl http://127.0.0.1:3030/mcp"));
    }
}
