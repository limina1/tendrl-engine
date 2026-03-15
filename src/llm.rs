//! LLM provider trait and implementations
//!
//! Defines the async interface for chat completion providers.
//! Includes a NoopProvider for testing.

use crate::chat::LLMMessage;
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
