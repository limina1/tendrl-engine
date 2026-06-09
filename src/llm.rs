//! LLM provider trait and implementations
//!
//! Defines the async interface for chat completion providers. Two layers:
//!
//! - The legacy single-shot `chat(messages) -> String` path (the original chat
//!   feature), preserved verbatim as a *provided* trait method.
//! - The tool-aware agentic path: `run_turn(messages, tools, model)` returning
//!   structured [`ContentBlock`]s and a [`StopReason`], so a server-side loop can
//!   drive tool-calling. See `docs/ai-tools-architecture.md`.
//!
//! Provider-neutral types ([`ContentBlock`], [`ToolDefinition`], [`AgentMessage`])
//! are declared once; each provider translates them to/from its wire format.
//! Auth is abstracted behind [`ClaudeCredential`] (api-key vs subscription
//! bearer), mirroring `signing.rs`'s pluggable `Signer`.

use crate::chat::{ChatRole, LLMMessage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::sync::Arc;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";

/// Errors from LLM providers
#[derive(Debug)]
pub enum LLMError {
    /// The HTTP/network request failed
    RequestFailed(String),
    /// The provider returned an error response
    ProviderError(String),
    /// No provider is configured
    NotConfigured(String),
}

impl fmt::Display for LLMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LLMError::RequestFailed(msg) => write!(f, "Request failed: {}", msg),
            LLMError::ProviderError(msg) => write!(f, "Provider error: {}", msg),
            LLMError::NotConfigured(msg) => write!(f, "Not configured: {}", msg),
        }
    }
}

impl std::error::Error for LLMError {}

// ---------------------------------------------------------------------------
// Provider-neutral agent types
// ---------------------------------------------------------------------------

/// One block of model output (or a tool result for display/persistence).
///
/// Serializes to a flat `{ "type": "...", ... }` object so the same schema is
/// shared by the Rust loop, the SSE payloads, and the web `ClaudeSessionBlock`
/// renderer (which reads `text` / `thinking` / `name` / `input` / `content`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Assistant text.
    Text { text: String },
    /// Reasoning summary (display only; dropped when echoing history back to
    /// the API since we don't capture thinking signatures).
    Thinking { thinking: String },
    /// The model wants a tool run.
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// The result of running a tool (fed back to the model on the next turn).
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// A message in the agentic conversation, grouped by role.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    System(String),
    User(String),
    /// An assistant turn: text / thinking / tool_use blocks.
    Assistant(Vec<ContentBlock>),
    /// Tool results answering the immediately-preceding assistant tool_use
    /// blocks. Serialized as a `user` message carrying `tool_result` blocks.
    ToolResults(Vec<ContentBlock>),
}

/// A tool advertised to the provider (vendor-neutral; the provider maps it to
/// its own wire shape).
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Why the model stopped this turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

/// The result of one assistant turn.
#[derive(Debug, Clone)]
pub struct TurnOutput {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
}

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// The auth header to send on a request — exactly one of these, never both
/// (sending both `x-api-key` and `Authorization` makes the API reject it).
#[derive(Debug, Clone)]
pub enum AuthHeader {
    ApiKey(String),
    Bearer(String),
}

/// A source of the current auth header for the Anthropic API. Mirrors the
/// pluggable `Signer` in `signing.rs` — leaves room for a future refreshing
/// source without changing `ClaudeProvider`.
#[async_trait::async_trait]
pub trait ClaudeCredential: Send + Sync {
    /// The header to send now.
    async fn header(&self) -> Result<AuthHeader, LLMError>;
    /// Called on a 401 so a refreshing impl can re-mint (v1 impls no-op; the
    /// UI surfaces a "re-supply token" prompt instead).
    async fn invalidate(&self);
}

/// Developer-platform API key (`ANTHROPIC_API_KEY`), sent as `x-api-key`.
pub struct ApiKeyCredential(pub String);

#[async_trait::async_trait]
impl ClaudeCredential for ApiKeyCredential {
    async fn header(&self) -> Result<AuthHeader, LLMError> {
        Ok(AuthHeader::ApiKey(self.0.clone()))
    }
    async fn invalidate(&self) {}
}

/// Hand-supplied subscription bearer (`ANTHROPIC_AUTH_TOKEN`, e.g. a
/// `claude setup-token` 1-year token), sent as `Authorization: Bearer`. No
/// refresh — on expiry the engine surfaces "re-supply ANTHROPIC_AUTH_TOKEN".
pub struct StaticBearerCredential(pub String);

#[async_trait::async_trait]
impl ClaudeCredential for StaticBearerCredential {
    async fn header(&self) -> Result<AuthHeader, LLMError> {
        Ok(AuthHeader::Bearer(self.0.clone()))
    }
    async fn invalidate(&self) {}
}

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// Trait for LLM chat/agent providers.
#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    /// Run one assistant turn, optionally with tools available. The caller
    /// feeds tool results back as [`AgentMessage::ToolResults`] and calls again
    /// until `stop_reason == EndTurn`.
    async fn run_turn(
        &self,
        messages: &[AgentMessage],
        tools: &[ToolDefinition],
        model: Option<&str>,
    ) -> Result<TurnOutput, LLMError>;

    /// Human-readable name of this provider
    fn name(&self) -> &str;

    /// Legacy single-shot completion. Provided in terms of `run_turn` (no
    /// tools) so existing callers keep their exact contract.
    async fn chat(&self, messages: Vec<LLMMessage>) -> Result<String, LLMError> {
        let agent: Vec<AgentMessage> = messages
            .into_iter()
            .map(|m| match m.role {
                ChatRole::System => AgentMessage::System(m.content),
                ChatRole::User => AgentMessage::User(m.content),
                ChatRole::Assistant => {
                    AgentMessage::Assistant(vec![ContentBlock::Text { text: m.content }])
                }
            })
            .collect();
        let out = self.run_turn(&agent, &[], None).await?;
        Ok(out
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""))
    }
}

// ---------------------------------------------------------------------------
// NoopProvider
// ---------------------------------------------------------------------------

/// A no-op provider for testing that returns a fixed response or echoes input
pub struct NoopProvider {
    response: String,
    echo: bool,
}

impl NoopProvider {
    /// Create a provider that always returns the given fixed response
    pub fn new(response: String) -> Self {
        Self {
            response,
            echo: false,
        }
    }

    /// Create a provider that echoes the last user message as "Echo: {msg}"
    pub fn echo() -> Self {
        Self {
            response: String::new(),
            echo: true,
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for NoopProvider {
    async fn run_turn(
        &self,
        messages: &[AgentMessage],
        _tools: &[ToolDefinition],
        _model: Option<&str>,
    ) -> Result<TurnOutput, LLMError> {
        let text = if self.echo {
            messages
                .iter()
                .rev()
                .find_map(|m| match m {
                    AgentMessage::User(s) => Some(format!("Echo: {}", s)),
                    _ => None,
                })
                .unwrap_or_else(|| "Echo: <no user message>".to_string())
        } else {
            self.response.clone()
        };
        Ok(TurnOutput {
            content: vec![ContentBlock::Text { text }],
            stop_reason: StopReason::EndTurn,
        })
    }

    fn name(&self) -> &str {
        "noop"
    }
}

// ---------------------------------------------------------------------------
// ClaudeProvider (Anthropic Messages API)
// ---------------------------------------------------------------------------

/// Provider that calls the Anthropic Messages API
pub struct ClaudeProvider {
    credential: Arc<dyn ClaudeCredential>,
    model: String,
    client: reqwest::Client,
}

impl ClaudeProvider {
    /// Construct from a raw API key (back-compat). Wraps an `ApiKeyCredential`.
    pub fn new(api_key: String) -> Self {
        Self::with_credential(Arc::new(ApiKeyCredential(api_key)))
    }

    /// Construct from any credential source (api-key or subscription bearer).
    pub fn with_credential(credential: Arc<dyn ClaudeCredential>) -> Self {
        Self {
            credential,
            model: "claude-haiku-4-5-20251001".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

/// Map an assistant-side block to its Anthropic JSON shape. Thinking blocks are
/// dropped (no signature to echo); tool_result blocks never belong here.
fn block_to_assistant_json(b: &ContentBlock) -> Option<Value> {
    match b {
        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentBlock::ToolUse { id, name, input } => {
            Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
        }
        ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => None,
    }
}

/// Map a tool-result block to its Anthropic JSON shape (lives in a user turn).
fn block_to_tool_result_json(b: &ContentBlock) -> Option<Value> {
    match b {
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        })),
        _ => None,
    }
}

/// Parse one Anthropic response content block into a [`ContentBlock`].
fn parse_content_block(block: &Value) -> Option<ContentBlock> {
    match block["type"].as_str()? {
        "text" => Some(ContentBlock::Text {
            text: block["text"].as_str().unwrap_or("").to_string(),
        }),
        "thinking" => Some(ContentBlock::Thinking {
            thinking: block["thinking"].as_str().unwrap_or("").to_string(),
        }),
        "tool_use" => Some(ContentBlock::ToolUse {
            id: block["id"].as_str().unwrap_or("").to_string(),
            name: block["name"].as_str().unwrap_or("").to_string(),
            input: block.get("input").cloned().unwrap_or(Value::Null),
        }),
        _ => None,
    }
}

#[async_trait::async_trait]
impl LLMProvider for ClaudeProvider {
    async fn run_turn(
        &self,
        messages: &[AgentMessage],
        tools: &[ToolDefinition],
        model: Option<&str>,
    ) -> Result<TurnOutput, LLMError> {
        let model = model.unwrap_or(&self.model);

        // The Anthropic API takes `system` as a top-level param, not in the
        // messages array.
        let mut system_parts: Vec<String> = Vec::new();
        let mut api_messages: Vec<Value> = Vec::new();

        for msg in messages {
            match msg {
                AgentMessage::System(s) => system_parts.push(s.clone()),
                AgentMessage::User(s) => api_messages.push(json!({ "role": "user", "content": s })),
                AgentMessage::Assistant(blocks) => {
                    let content: Vec<Value> =
                        blocks.iter().filter_map(block_to_assistant_json).collect();
                    api_messages.push(json!({ "role": "assistant", "content": content }));
                }
                AgentMessage::ToolResults(blocks) => {
                    let content: Vec<Value> = blocks
                        .iter()
                        .filter_map(block_to_tool_result_json)
                        .collect();
                    api_messages.push(json!({ "role": "user", "content": content }));
                }
            }
        }

        // Anthropic requires at least one message
        if api_messages.is_empty() {
            return Err(LLMError::ProviderError(
                "No user/assistant messages to send".into(),
            ));
        }

        // NOTE: we deliberately do NOT send `temperature`/`top_p`/`top_k` —
        // they are removed on Opus 4.8/4.7 (a 400), and omitting them is valid
        // on every current model. `thinking` is likewise omitted (valid
        // everywhere); revisit if adaptive thinking is wanted for the loop.
        let mut body = json!({
            "model": model,
            "max_tokens": 4096,
            "messages": api_messages,
        });
        if !system_parts.is_empty() {
            body["system"] = Value::String(system_parts.join("\n\n"));
        }
        if !tools.is_empty() {
            body["tools"] =
                serde_json::to_value(tools).map_err(|e| LLMError::ProviderError(e.to_string()))?;
        }

        let mut req = self
            .client
            .post(ANTHROPIC_URL)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");
        match self.credential.header().await? {
            AuthHeader::ApiKey(k) => req = req.header("x-api-key", k),
            AuthHeader::Bearer(t) => req = req.header("authorization", format!("Bearer {t}")),
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        let status = resp.status();
        if status.as_u16() == 401 {
            self.credential.invalidate().await;
        }
        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| LLMError::RequestFailed(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            let err_msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error");
            return Err(LLMError::ProviderError(format!("{}: {}", status, err_msg)));
        }

        let content = resp_body["content"]
            .as_array()
            .ok_or_else(|| LLMError::ProviderError("No content in response".into()))?;
        let blocks: Vec<ContentBlock> = content.iter().filter_map(parse_content_block).collect();

        let stop_reason = match resp_body["stop_reason"].as_str() {
            Some("end_turn") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some(other) => StopReason::Other(other.to_string()),
            None => StopReason::EndTurn,
        };

        Ok(TurnOutput {
            content: blocks,
            stop_reason,
        })
    }

    fn name(&self) -> &str {
        "claude"
    }
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Build a provider from the `[ai]` config block. Secrets come from the
/// environment, never the config file. Falls back to the echo provider when no
/// credential is available, so a missing key degrades gracefully.
pub fn provider_from_config(ai: &crate::config::AiConfig) -> Arc<dyn LLMProvider> {
    match ai.provider.as_str() {
        "noop" => {
            tracing::info!("LLM provider: noop (echo)");
            Arc::new(NoopProvider::echo())
        }
        _ => {
            let credential: Option<Arc<dyn ClaudeCredential>> = match ai.auth.as_str() {
                "oauth" => std::env::var("ANTHROPIC_AUTH_TOKEN")
                    .ok()
                    .map(|t| Arc::new(StaticBearerCredential(t)) as Arc<dyn ClaudeCredential>),
                _ => std::env::var("ANTHROPIC_API_KEY")
                    .ok()
                    .map(|k| Arc::new(ApiKeyCredential(k)) as Arc<dyn ClaudeCredential>),
            };
            match credential {
                Some(c) => {
                    tracing::info!(
                        "LLM provider: claude (model: {}, auth: {})",
                        ai.model,
                        ai.auth
                    );
                    Arc::new(ClaudeProvider::with_credential(c).with_model(ai.model.clone()))
                }
                None => {
                    tracing::warn!(
                        "LLM provider: echo (no credential for auth={}); set {}",
                        ai.auth,
                        if ai.auth == "oauth" {
                            "ANTHROPIC_AUTH_TOKEN"
                        } else {
                            "ANTHROPIC_API_KEY"
                        }
                    );
                    Arc::new(NoopProvider::echo())
                }
            }
        }
    }
}

/// Build a provider from environment configuration (legacy entry point).
///
/// If `ANTHROPIC_API_KEY` is set, returns a ClaudeProvider.
/// The model can be overridden with `ANTHROPIC_MODEL`.
/// Otherwise falls back to the echo NoopProvider.
pub fn provider_from_env() -> Arc<dyn LLMProvider> {
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        let mut provider = ClaudeProvider::new(api_key);
        if let Ok(model) = std::env::var("ANTHROPIC_MODEL") {
            provider = provider.with_model(model);
        }
        tracing::info!("LLM provider: claude (model: {})", provider.model);
        Arc::new(provider)
    } else {
        tracing::info!("LLM provider: echo (no ANTHROPIC_API_KEY set)");
        Arc::new(NoopProvider::echo())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{ChatRole, LLMMessage};

    #[tokio::test]
    async fn test_noop_fixed_response() {
        let provider = NoopProvider::new("fixed answer".into());
        let messages = vec![LLMMessage {
            role: ChatRole::User,
            content: "anything".into(),
        }];
        let result = provider.chat(messages).await.unwrap();
        assert_eq!(result, "fixed answer");
    }

    #[tokio::test]
    async fn test_noop_echo_mode() {
        let provider = NoopProvider::echo();
        let messages = vec![
            LLMMessage {
                role: ChatRole::System,
                content: "system prompt".into(),
            },
            LLMMessage {
                role: ChatRole::User,
                content: "hello world".into(),
            },
        ];
        let result = provider.chat(messages).await.unwrap();
        assert_eq!(result, "Echo: hello world");
    }

    #[tokio::test]
    async fn test_noop_echo_no_user_message() {
        let provider = NoopProvider::echo();
        let messages = vec![LLMMessage {
            role: ChatRole::System,
            content: "system only".into(),
        }];
        let result = provider.chat(messages).await.unwrap();
        assert_eq!(result, "Echo: <no user message>");
    }

    #[tokio::test]
    async fn test_noop_run_turn_no_tools() {
        let provider = NoopProvider::echo();
        let msgs = vec![
            AgentMessage::System("sys".into()),
            AgentMessage::User("ping".into()),
        ];
        let out = provider.run_turn(&msgs, &[], None).await.unwrap();
        assert_eq!(out.stop_reason, StopReason::EndTurn);
        assert!(matches!(
            out.content.first(),
            Some(ContentBlock::Text { text }) if text == "Echo: ping"
        ));
    }

    #[test]
    fn test_content_block_serialization() {
        // The flat {type, ...} shape the web ClaudeSessionBlock renderer reads.
        let b = ContentBlock::ToolUse {
            id: "tu_1".into(),
            name: "search_events".into(),
            input: json!({"query": "k:30040"}),
        };
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["type"], "tool_use");
        assert_eq!(v["name"], "search_events");
        assert_eq!(v["input"]["query"], "k:30040");

        let r = ContentBlock::ToolResult {
            tool_use_id: "tu_1".into(),
            content: "ok".into(),
            is_error: false,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["content"], "ok");
    }

    #[test]
    fn test_llm_error_display() {
        let err = LLMError::RequestFailed("timeout".into());
        assert_eq!(format!("{}", err), "Request failed: timeout");

        let err = LLMError::ProviderError("rate limit".into());
        assert_eq!(format!("{}", err), "Provider error: rate limit");

        let err = LLMError::NotConfigured("no API key".into());
        assert_eq!(format!("{}", err), "Not configured: no API key");
    }
}
