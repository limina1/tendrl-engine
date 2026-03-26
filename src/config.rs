//! Configuration parsing
//!
//! Handles TOML configuration file loading and defaults.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,
    /// Port to listen on
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3030
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// Database configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Path to nostrdb data directory
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
}

fn default_data_dir() -> String {
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        let path = Path::new(&data_home).join("nostr-engine").join("nostrdb");
        return path.to_string_lossy().to_string();
    }

    if let Some(home) = std::env::var_os("HOME") {
        let path = Path::new(&home)
            .join(".local")
            .join("share")
            .join("nostr-engine")
            .join("nostrdb");
        return path.to_string_lossy().to_string();
    }

    "./nostr_engine_data/nostrdb".to_string()
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
        }
    }
}

/// Relay configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    /// Default relays to fetch from
    #[serde(default = "default_relays")]
    pub default_relays: Vec<String>,
    /// Request timeout in milliseconds
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_relays() -> Vec<String> {
    crate::relay::DEFAULT_RELAYS
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_timeout_ms() -> u64 {
    15000
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            default_relays: default_relays(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

/// Identity configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentityConfig {
    /// User's public key (npub1... or hex)
    pub pubkey: Option<String>,
}

/// Embedding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Enable embedding index
    #[serde(default)]
    pub enabled: bool,
    /// Backend: "python" (sidecar) or "onnx" (in-process, requires --features onnx)
    #[serde(default = "default_embedding_backend")]
    pub backend: String,
    /// Python sidecar URL
    #[serde(default = "default_sidecar_url")]
    pub sidecar_url: String,
    /// Sentence transformer model name
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Embedding dimensions (must match model)
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
    /// Custom index path (defaults to {data_dir parent}/vectors.*)
    #[serde(default)]
    pub index_path: Option<String>,
    /// Automatically embed new events on ingest
    #[serde(default)]
    pub auto_embed: bool,
}

fn default_embedding_backend() -> String { "python".to_string() }
fn default_sidecar_url() -> String { "http://localhost:3031".to_string() }
fn default_embedding_model() -> String { "all-MiniLM-L6-v2".to_string() }
fn default_dimensions() -> usize { 384 }

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_embedding_backend(),
            sidecar_url: default_sidecar_url(),
            model: default_embedding_model(),
            dimensions: default_dimensions(),
            index_path: None,
            auto_embed: false,
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Server settings
    #[serde(default)]
    pub server: ServerConfig,
    /// Database settings
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Relay settings
    #[serde(default)]
    pub relay: RelayConfig,
    /// Identity settings
    #[serde(default)]
    pub identity: IdentityConfig,
    /// Embedding settings
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

impl Config {
    /// Resolve the configured pubkey to hex format (handles npub1... bech32)
    pub fn pubkey_hex(&self) -> Option<String> {
        let raw = self.identity.pubkey.as_deref()?;
        if raw.starts_with("npub1") {
            crate::identity::decode_npub(raw).ok()
        } else if raw.len() == 64 && hex::decode(raw).is_ok() {
            Some(raw.to_string())
        } else {
            None
        }
    }

    /// Load configuration from a TOML file
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| EngineError::Config(format!("Failed to parse config: {}", e)))?;
        Ok(config)
    }

    /// Load configuration from file if it exists, otherwise use defaults
    pub fn load_or_default(path: Option<&Path>) -> Self {
        if let Some(p) = path {
            if p.exists() {
                match Self::from_file(p) {
                    Ok(config) => return config,
                    Err(e) => {
                        tracing::warn!("Failed to load config from {:?}: {}", p, e);
                    }
                }
            }
        }
        Self::default()
    }

    /// Get the bind address as a string
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }
}
