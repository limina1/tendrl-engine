//! LLM provider trait and implementations
//!
//! Defines the async interface for chat completion providers.
//! Includes a NoopProvider for testing and ClaudeProvider for the
//! Anthropic Messages API.

use crate::chat::{ChatRole, LLMMessage};
use std::fmt;
use std::sync::Arc;

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

/// Trait for LLM chat completion providers
#[async_trait::async_trait]
pub trait LLMProvider: Send + Sync {
    /// Send a list of messages and receive a completion response
    async fn chat(&self, messages: Vec<LLMMessage>) -> Result<String, LLMError>;

    /// Human-readable name of this provider
    fn name(&self) -> &str;
}

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
    async fn chat(&self, messages: Vec<LLMMessage>) -> Result<String, LLMError> {
        if self.echo {
            let last_user = messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, crate::chat::ChatRole::User))
                .map(|m| format!("Echo: {}", m.content))
                .unwrap_or_else(|| "Echo: <no user message>".to_string());
            Ok(last_user)
        } else {
            Ok(self.response.clone())
        }
    }

    fn name(&self) -> &str {
        "noop"
    }
}

/// Provider that calls the Anthropic Messages API
pub struct ClaudeProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl ClaudeProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: "claude-haiku-4-5-20251001".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

#[async_trait::async_trait]
impl LLMProvider for ClaudeProvider {
    async fn chat(&self, messages: Vec<LLMMessage>) -> Result<String, LLMError> {
        // Split system prompt from conversation messages.
        // The Anthropic API takes `system` as a top-level param, not in the
        // messages array.
        let mut system_parts: Vec<String> = Vec::new();
        let mut api_messages: Vec<serde_json::Value> = Vec::new();

        for msg in &messages {
            match msg.role {
                ChatRole::System => {
                    system_parts.push(msg.content.clone());
                }
                ChatRole::User | ChatRole::Assistant => {
                    api_messages.push(serde_json::json!({
                        "role": msg.role.as_str(),
                        "content": msg.content,
                    }));
                }
            }
        }

        // Anthropic requires at least one message
        if api_messages.is_empty() {
            return Err(LLMError::ProviderError(
                "No user/assistant messages to send".into(),
            ));
        }

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": api_messages,
        });

        if !system_parts.is_empty() {
            body["system"] = serde_json::Value::String(system_parts.join("\n\n"));
        }

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LLMError::RequestFailed(e.to_string()))?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LLMError::RequestFailed(format!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            let err_msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown error");
            return Err(LLMError::ProviderError(format!(
                "{}: {}",
                status, err_msg
            )));
        }

        // Extract text from the content blocks
        let content = resp_body["content"]
            .as_array()
            .ok_or_else(|| LLMError::ProviderError("No content in response".into()))?;

        let text: String = content
            .iter()
            .filter_map(|block| {
                if block["type"].as_str() == Some("text") {
                    block["text"].as_str().map(String::from)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text)
    }

    fn name(&self) -> &str {
        "claude"
    }
}

/// Build a provider from environment configuration.
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
