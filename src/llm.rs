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
//!
//! Two implementations: [`NoopProvider`] (echo, for plumbing tests) and
//! [`OpenAiCompatProvider`] (chat-completions wire format — what the paid
//! agent services in `providers.rs`, ppq.ai and routstr nodes, speak). The
//! former Anthropic Messages API provider was removed on 2026-09-22: the
//! in-app chat runs on agent services only; Claude reaches tendrl as an
//! *external* harness over MCP, never as a key the engine holds.

use crate::chat::{ChatRole, LLMMessage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

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
// OpenAiCompatProvider — chat-completions wire format (ppq.ai, routstr nodes,
// anything else OpenAI-shaped)
// ---------------------------------------------------------------------------

/// A provider speaking `POST {base}/chat/completions` with bearer auth.
///
/// The neutral types never change: tools become `function` entries, a
/// returned `tool_calls` list becomes [`ContentBlock::ToolUse`]s (arguments
/// arrive as a JSON *string* and are parsed here), and
/// [`AgentMessage::ToolResults`] go back as one `role: tool` message per
/// result. `reasoning` / `reasoning_content` on the reply becomes a
/// display-only [`ContentBlock::Thinking`]. No streaming yet.
pub struct OpenAiCompatProvider {
    name: &'static str,
    /// Up to and including `/v1` — `https://api.ppq.ai/v1`, `https://node/v1`.
    base_url: String,
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAiCompatProvider {
    pub fn new(name: &'static str, base_url: String, api_key: String, model: String) -> Self {
        Self {
            name,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Render the neutral history as chat-completions messages.
    pub fn to_wire_messages(messages: &[AgentMessage]) -> Vec<Value> {
        let mut out: Vec<Value> = Vec::new();
        for msg in messages {
            match msg {
                AgentMessage::System(s) => out.push(json!({ "role": "system", "content": s })),
                AgentMessage::User(s) => out.push(json!({ "role": "user", "content": s })),
                AgentMessage::Assistant(blocks) => {
                    let text: String = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    let calls: Vec<Value> = blocks
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::ToolUse { id, name, input } => Some(json!({
                                "id": id,
                                "type": "function",
                                "function": { "name": name, "arguments": input.to_string() }
                            })),
                            _ => None,
                        })
                        .collect();
                    let mut m = json!({ "role": "assistant" });
                    m["content"] = if text.is_empty() { Value::Null } else { Value::String(text) };
                    if !calls.is_empty() {
                        m["tool_calls"] = Value::Array(calls);
                    }
                    out.push(m);
                }
                AgentMessage::ToolResults(blocks) => {
                    for b in blocks {
                        if let ContentBlock::ToolResult { tool_use_id, content, is_error } = b {
                            let content = if *is_error {
                                format!("ERROR: {content}")
                            } else {
                                content.clone()
                            };
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                    }
                }
            }
        }
        out
    }

    pub fn to_wire_tools(tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }

    /// Parse `choices[0]` into blocks + a stop reason.
    pub fn parse_choice(choice: &Value) -> TurnOutput {
        let message = &choice["message"];
        let mut blocks: Vec<ContentBlock> = Vec::new();
        let reasoning = message["reasoning"]
            .as_str()
            .or_else(|| message["reasoning_content"].as_str())
            .unwrap_or("");
        if !reasoning.is_empty() {
            blocks.push(ContentBlock::Thinking {
                thinking: reasoning.to_string(),
            });
        }
        match &message["content"] {
            Value::String(s) if !s.is_empty() => blocks.push(ContentBlock::Text { text: s.clone() }),
            Value::Array(parts) => {
                let text: String = parts
                    .iter()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() {
                    blocks.push(ContentBlock::Text { text });
                }
            }
            _ => {}
        }
        let mut saw_tool_call = false;
        if let Some(calls) = message["tool_calls"].as_array() {
            for (i, c) in calls.iter().enumerate() {
                let id = c["id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{i}"));
                let name = c["function"]["name"].as_str().unwrap_or("").to_string();
                let input = match &c["function"]["arguments"] {
                    Value::String(s) if s.trim().is_empty() => json!({}),
                    Value::String(s) => serde_json::from_str::<Value>(s)
                        .unwrap_or_else(|_| json!({ "_unparsed_arguments": s })),
                    Value::Null => json!({}),
                    other => other.clone(),
                };
                saw_tool_call = true;
                blocks.push(ContentBlock::ToolUse { id, name, input });
            }
        }
        let stop_reason = match choice["finish_reason"].as_str() {
            _ if saw_tool_call => StopReason::ToolUse,
            Some("stop") | None => StopReason::EndTurn,
            Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            Some(other) => StopReason::Other(other.to_string()),
        };
        TurnOutput {
            content: blocks,
            stop_reason,
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for OpenAiCompatProvider {
    async fn run_turn(
        &self,
        messages: &[AgentMessage],
        tools: &[ToolDefinition],
        model: Option<&str>,
    ) -> Result<TurnOutput, LLMError> {
        let model = model.unwrap_or(&self.model);
        let wire = Self::to_wire_messages(messages);
        if !wire.iter().any(|m| m["role"] != "system") {
            return Err(LLMError::ProviderError(
                "No user/assistant messages to send".into(),
            ));
        }
        let mut body = json!({
            "model": model,
            "messages": wire,
            "max_tokens": 4096,
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(Self::to_wire_tools(tools));
            body["tool_choice"] = json!("auto");
        }
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| LLMError::RequestFailed(format!("Failed to read response: {e}")))?;
        let resp_body: Value = serde_json::from_str(&text)
            .unwrap_or_else(|_| json!({ "error": { "message": text.chars().take(300).collect::<String>() } }));
        if !status.is_success() {
            let err_msg = resp_body["error"]["message"]
                .as_str()
                .or_else(|| resp_body["detail"]["error"]["message"].as_str())
                .or_else(|| resp_body["detail"].as_str())
                .unwrap_or("Unknown error");
            let hint = if status.as_u16() == 402 {
                " (payment required — the account balance is too low for this request)"
            } else {
                ""
            };
            return Err(LLMError::ProviderError(format!("{}: {}{}", status, err_msg, hint)));
        }
        let choice = resp_body["choices"]
            .as_array()
            .and_then(|c| c.first())
            .ok_or_else(|| LLMError::ProviderError("No choices in response".into()))?;
        Ok(Self::parse_choice(choice))
    }

    fn name(&self) -> &str {
        self.name
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
