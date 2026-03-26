//! nostr-engine HTTP server binary
//!
//! Run a standalone HTTP API for querying Nostr events.

use axum::{
    routing::{get, post},
    Router,
};
use clap::Parser;
use nostr_engine::{api, chat::ChatState, config::Config, engine::Engine, llm};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
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

    // Resolve relay config (backwards-compat default_relays → sets)
    let relay_config = config.relay.resolved();
    info!("Relays — general: {:?}, fetch: {:?}, publish: {:?}",
        relay_config.general.urls, relay_config.fetch.urls, relay_config.publish.urls);

    let my_pubkey = config.pubkey_hex();
    if let Some(ref pk) = my_pubkey {
        info!("Identity pubkey: {}...{}", &pk[..8], &pk[pk.len()-8..]);
    }

    // Create the engine
    let data_path = PathBuf::from(&config.database.data_dir);
    let mut engine = Engine::with_relay_config(&data_path, &relay_config)?;
    engine.set_my_pubkey(my_pubkey.clone());

    // Initialize embedding index if enabled
    if config.embedding.enabled {
        if let Err(e) = engine.init_embedding(&config.embedding) {
            tracing::warn!("Failed to initialize embedding index: {}", e);
        }
    }

    let state = Arc::new(engine);

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Chat state + LLM provider
    let chat_state = api::ChatAppState {
        chat: Arc::new(Mutex::new(ChatState::new())),
        provider: llm::provider_from_env(),
    };

    let chat_routes = Router::new()
        .route("/api/v1/chat", get(api::chat_get).delete(api::chat_reset))
        .route("/api/v1/chat/message", post(api::chat_send))
        .route(
            "/api/v1/chat/edit",
            post(api::chat_enter_edit).put(api::chat_exit_edit),
        )
        .route("/api/v1/chat/system", post(api::chat_set_system))
        .route("/api/v1/chat/context", post(api::chat_inject_context).put(api::chat_replace_context))
        .with_state(chat_state);

    // Config endpoint (returns pubkey etc. to the frontend)
    let config_pubkey = my_pubkey.clone();
    let config_handler = move || async move {
        axum::Json(serde_json::json!({
            "my_pubkey": config_pubkey
        }))
    };

    // Build router
    let app = Router::new()
        // Core endpoints
        .route("/health", get(api::health_handler))
        .route("/api/v1/config", get(config_handler))
        .route("/api/v1/query", post(api::query_handler))
        .route("/api/v1/events/:id", get(api::get_event_handler))
        .route(
            "/api/v1/addressable/:kind/:pubkey/:d_tag",
            get(api::get_addressable_handler),
        )
        // Search endpoint
        .route("/api/v1/search", post(api::search_handler))
        // Relay config
        .route("/api/v1/relays", get(api::relay_config_handler))
        // Ignore list + purge
        .route("/api/v1/ignore", get(api::ignore_list_handler).post(api::ignore_add_handler).delete(api::ignore_remove_handler))
        .route("/api/v1/purge", post(api::purge_handler))
        // Publish endpoint
        .route("/api/v1/publish", post(api::publish_handler))
        // Embedding endpoints
        .route("/api/v1/embed/status", get(api::embed_status_handler))
        .route("/api/v1/embed/sync", post(api::embed_sync_handler))
        .route("/api/v1/embed/reindex", post(api::embed_reindex_handler))
        // Publication endpoints
        .route("/api/v1/publications", get(api::list_publications_handler))
        .route(
            "/api/v1/publications/:pubkey/:d_tag",
            get(api::get_publication_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/sections/metadata",
            post(api::load_sections_metadata_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/sections",
            post(api::load_sections_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/sections/:index",
            get(api::get_section_handler),
        )
        .route(
            "/api/v1/sections/:pubkey/:d_tag/versions",
            get(api::get_section_versions_handler),
        )
        .with_state(state)
        .merge(chat_routes)
        .layer(cors);

    // Serve static files from web/build/ if it exists (production SPA)
    let web_dir = std::path::Path::new("web/build");
    let app = if web_dir.exists() {
        info!("Serving web UI from web/build/");
        app.fallback_service(
            ServeDir::new(web_dir)
                .not_found_service(ServeFile::new(web_dir.join("index.html"))),
        )
    } else {
        app
    };

    // Start server
    let bind_addr = config.bind_addr();
    info!("Listening on http://{}", bind_addr);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
