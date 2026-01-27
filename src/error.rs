//! Error types for nostr-engine
//!
//! Provides unified error handling for database, relay, and API operations.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// Main error type for nostr-engine
#[derive(Error, Debug)]
pub enum EngineError {
    /// Database-related errors
    #[error("Database error: {0}")]
    Database(String),

    /// Relay connection or communication errors
    #[error("Relay error: {0}")]
    Relay(String),

    /// JSON serialization/deserialization errors
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Invalid filter format
    #[error("Invalid filter: {0}")]
    InvalidFilter(String),

    /// Invalid hex string
    #[error("Invalid hex: {0}")]
    InvalidHex(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic error wrapper
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for EngineError {
    fn from(err: anyhow::Error) -> Self {
        EngineError::Other(err.to_string())
    }
}

impl From<hex::FromHexError> for EngineError {
    fn from(err: hex::FromHexError) -> Self {
        EngineError::InvalidHex(err.to_string())
    }
}

/// Axum response implementation for API error responses
impl IntoResponse for EngineError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match &self {
            EngineError::Database(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "database_error", msg.clone()),
            EngineError::Relay(msg) => (StatusCode::BAD_GATEWAY, "relay_error", msg.clone()),
            EngineError::Serialization(err) => (StatusCode::BAD_REQUEST, "serialization_error", err.to_string()),
            EngineError::InvalidFilter(msg) => (StatusCode::BAD_REQUEST, "invalid_filter", msg.clone()),
            EngineError::InvalidHex(msg) => (StatusCode::BAD_REQUEST, "invalid_hex", msg.clone()),
            EngineError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "config_error", msg.clone()),
            EngineError::Io(err) => (StatusCode::INTERNAL_SERVER_ERROR, "io_error", err.to_string()),
            EngineError::Other(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "error", msg.clone()),
        };

        let body = Json(json!({
            "error": {
                "type": error_type,
                "message": message
            }
        }));

        (status, body).into_response()
    }
}

/// Result type alias for nostr-engine operations
pub type Result<T> = std::result::Result<T, EngineError>;
