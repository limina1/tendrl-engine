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
    vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.nostr.band".to_string(),
    ]
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
}

impl Config {
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
