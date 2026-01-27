//! nostr-engine HTTP server binary
//!
//! Run a standalone HTTP API for querying Nostr events.

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use nostr_engine::{api, config::Config, engine::Engine};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, Level};
use tracing_subscriber::EnvFilter;

/// Nostr Engine - Local Nostr backend with HTTP API
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "3030")]
    port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Path to nostrdb data directory
    #[arg(short, long)]
    data_dir: Option<PathBuf>,

    /// Path to configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter = if args.verbose {
        EnvFilter::from_default_env()
            .add_directive(Level::DEBUG.into())
            .add_directive("nostr_engine=debug".parse().unwrap())
    } else {
        EnvFilter::from_default_env()
            .add_directive(Level::INFO.into())
            .add_directive("nostr_engine=info".parse().unwrap())
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    // Load configuration
    let mut config = Config::load_or_default(args.config.as_deref());

    // CLI args override config file
    config.server.port = args.port;
    config.server.host = args.host;

    if let Some(data_dir) = args.data_dir {
        config.database.data_dir = data_dir.to_string_lossy().to_string();
    }

    info!("Starting nostr-engine v{}", env!("CARGO_PKG_VERSION"));
    info!("Data directory: {}", config.database.data_dir);
    info!("Default relays: {:?}", config.relay.default_relays);

    // Create the engine
    let data_path = PathBuf::from(&config.database.data_dir);
    let relay_refs: Vec<&str> = config.relay.default_relays.iter().map(|s| s.as_str()).collect();
    let engine = Engine::with_config(&data_path, &relay_refs, config.relay.timeout_ms)?;
    let state = Arc::new(engine);

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Build router
    let app = Router::new()
        // Core endpoints
        .route("/health", get(api::health_handler))
        .route("/api/v1/query", post(api::query_handler))
        .route("/api/v1/events/{id}", get(api::get_event_handler))
        .route(
            "/api/v1/addressable/{kind}/{pubkey}/{d_tag}",
            get(api::get_addressable_handler),
        )
        // Publication endpoints
        .route("/api/v1/publications", get(api::list_publications_handler))
        .route(
            "/api/v1/publications/{pubkey}/{d_tag}",
            get(api::get_publication_handler),
        )
        .route(
            "/api/v1/publications/{pubkey}/{d_tag}/sections",
            post(api::load_sections_handler),
        )
        .route(
            "/api/v1/publications/{pubkey}/{d_tag}/sections/{index}",
            get(api::get_section_handler),
        )
        .route(
            "/api/v1/sections/{pubkey}/{d_tag}/versions",
            get(api::get_section_versions_handler),
        )
        .layer(cors)
        .with_state(state);

    // Start server
    let bind_addr = config.bind_addr();
    info!("Listening on http://{}", bind_addr);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
