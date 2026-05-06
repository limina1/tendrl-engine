//! nostr-engine: A standalone local Nostr backend
//!
//! This crate provides a unified interface for querying Nostr events from
//! a local nostrdb database with optional relay backfill.
//!
//! # Features
//!
//! - **Local-first**: Query your local nostrdb first for fast responses
//! - **Relay backfill**: Automatically fetch from relays when data is missing
//! - **Configurable policies**: Choose between local_only, local_first, or fetch_always
//! - **REST API**: Run as a standalone HTTP service
//! - **Library usage**: Embed directly in your Rust application
//!
//! # Example
//!
//! ```rust,no_run
//! use nostr_engine::{Engine, FetchPolicy};
//! use std::path::Path;
//! use serde_json::json;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create engine with data directory
//!     let engine = Engine::new(Path::new("/tmp/nostr-engine-test"))?;
//!
//!     // Query for kind 1 notes
//!     let filter = json!({"kinds": [1], "limit": 10});
//!     let response = engine.get_events(
//!         vec![filter],
//!         FetchPolicy::LocalFirst,
//!         None
//!     ).await?;
//!
//!     println!("Found {} events", response.count);
//!     Ok(())
//! }
//! ```
//!
//! # Running as HTTP Service
//!
//! ```bash
//! nostr-engine --port 3030 --data-dir ~/.local/share/nostr-engine/nostrdb
//! ```
//!
//! Then query via HTTP:
//!
//! ```bash
//! curl -X POST http://localhost:3030/api/v1/query \
//!   -H "Content-Type: application/json" \
//!   -d '{"filters": [{"kinds": [1], "limit": 10}]}'
//! ```

pub mod api;
pub mod chat;
pub mod claude_sessions;
pub mod config;
pub mod drafts;
pub mod embedding;
pub mod engine;
pub mod error;
pub mod identity;
pub mod llm;
pub mod network;
pub mod nip11;
pub mod publication;
pub mod query;
pub mod relay;
pub mod search;
pub mod tree;
pub mod user_data;

// Re-export main types for convenience
pub use config::Config;
pub use engine::{Engine, FetchPolicy, QueryResponse, QuerySource};
pub use network::NetworkMode;
pub use error::{EngineError, Result};
pub use publication::{
    LoadStatus, NAddr, Publication, PublicationEngine, Section, SectionVersion, TocEntry,
    KIND_PUBLICATION_INDEX, KIND_PUBLICATION_SECTION,
};
