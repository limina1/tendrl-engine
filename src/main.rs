//! nostr-engine HTTP server binary
//!
//! Run a standalone HTTP API for querying Nostr events.

use axum::{
    routing::{get, post, put},
    Router,
};
use clap::Parser;
use nostr_engine::{api, chat::ChatState, config::Config, engine::Engine, identity::IdentitySession, llm};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

    // Pass the TOML-derived RelayConfig straight through. The working
    // URL sets are layered in by Engine::with_relay_config from
    // <data_dir>/relays.json (seeded from initial_relays on first boot).
    let relay_config = config.relay.clone();

    let my_pubkey = config.pubkey_hex();
    if let Some(ref pk) = my_pubkey {
        info!("Identity pubkey: {}...{}", &pk[..8], &pk[pk.len()-8..]);
    }
    let assistant_pubkey = config.assistant_pubkey_hex();
    if let Some(ref pk) = assistant_pubkey {
        info!("Assistant pubkey: {}...{}", &pk[..8], &pk[pk.len()-8..]);
    }

    // Create the engine
    let data_path = PathBuf::from(&config.database.data_dir);
    let mut engine = Engine::with_relay_config(&data_path, &relay_config)?;
    if let Some(ref config_path) = args.config {
        engine.set_config_path(config_path.clone());
    }
    engine.set_documents_path(std::path::PathBuf::from(&config.documents.path));
    engine.set_sidecar_url(config.embedding.sidecar_url.clone());
    engine.set_my_pubkey(my_pubkey.clone());
    engine.set_assistant_pubkey(assistant_pubkey.clone());

    // Resolve Claude Code sessions directory from cwd
    let cwd = std::env::current_dir().unwrap_or_default();
    let claude_dir = nostr_engine::claude_sessions::resolve_claude_dir(&cwd);
    if claude_dir.exists() {
        info!("Claude Code sessions: {}", claude_dir.display());
        engine.set_claude_sessions_dir(Some(claude_dir));
    }

    // Initialize embedding index if enabled
    if config.embedding.enabled {
        if let Err(e) = engine.init_embedding(&config.embedding) {
            tracing::warn!("Failed to initialize embedding index: {}", e);
        }
    }

    // Set initial network mode from config
    if let Ok(mode) = config.network.mode.parse::<nostr_engine::NetworkMode>() {
        engine.set_initial_network_mode(mode);
        info!("Network mode: {}", mode);
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
        .route("/api/v1/chat/load", put(api::chat_load_fragments))
        .with_state(chat_state);

    // Identity session state. Honor `config.toml [identity] source`
    // directly — IdentitySource::Nip07/Nip46 with signer_id: None
    // expresses "user's intent is nip07; live signer not connected
    // yet". The web's /identity/use call promotes signer_id to
    // Some(reg.signer_id) once the extension is registered. Without
    // this, every restart briefly reported source: "engine" (the
    // hard-coded default) which made the SettingsBuffer radio look
    // "stuck on engine" until the web finished its auto-reconnect.
    let boot_source = config
        .identity
        .source
        .as_deref()
        .and_then(nostr_engine::identity::IdentitySource::from_config_str)
        .unwrap_or(nostr_engine::identity::IdentitySource::Engine);
    info!("Saved identity source: {}", boot_source.kind_str());
    let identity_state: api::IdentityAppState =
        Arc::new(Mutex::new(IdentitySession::with_source(boot_source)));
    // Load a persisted engine key (config.toml `[identity] ncryptsec`)
    // into a *locked* session so the UI prompts for just the password
    // instead of a re-paste. The key is scrypt-encrypted; unlocking
    // still needs the password the engine never stores.
    if let Some(nc) = config.identity.ncryptsec.as_deref().filter(|s| !s.is_empty()) {
        match identity_state.lock().unwrap().login_ncryptsec(nc) {
            Ok(()) => info!("Loaded persisted engine key (locked)"),
            Err(e) => tracing::warn!("Ignoring invalid persisted [identity] ncryptsec: {e}"),
        }
    }
    // Apply the saved auto-lock timeout (0 = never) to the live session
    // so the engine honours `[identity] lock_timeout_minutes` from the
    // first unlock onward, not just after the web re-sends it.
    identity_state
        .lock()
        .unwrap()
        .set_timeout_minutes(config.identity.lock_timeout_minutes);

    let identity_routes = Router::new()
        .route("/api/v1/identity", get(api::identity_status_handler))
        .route("/api/v1/identity/login", post(api::identity_login_handler))
        .route(
            "/api/v1/identity/unlock",
            post(api::identity_unlock_handler),
        )
        .route("/api/v1/identity/lock", post(api::identity_lock_handler))
        .route(
            "/api/v1/identity/lock-timeout",
            post(api::identity_lock_timeout_handler),
        )
        .route(
            "/api/v1/identity/logout",
            post(api::identity_logout_handler),
        )
        .route(
            "/api/v1/identity/use",
            post(api::identity_use_source_handler),
        )
        .with_state(identity_state.clone());

    // SigningController owns the external-signer registry and routes
    // signing through the active source. Constructed once and shared
    // across the sign / signer-register / signer-channel / sign-response
    // handlers.
    let signing_controller =
        nostr_engine::signing::SigningController::new(identity_state.clone());

    let signing_routes = Router::new()
        .route("/api/v1/identity/sign", post(api::identity_sign_handler))
        .route(
            "/api/v1/identity/signer-register",
            post(api::signer_register_handler),
        )
        .route(
            "/api/v1/identity/signer-channel",
            get(api::signer_channel_handler),
        )
        .route(
            "/api/v1/identity/sign-response",
            post(api::sign_response_handler),
        )
        .with_state(signing_controller.clone());

    // Config endpoint (returns pubkey etc. to the frontend)
    let config_pubkey = my_pubkey.clone();
    let config_assistant = assistant_pubkey.clone();
    let config_data_dir = state.data_dir().to_string_lossy().to_string();
    let config_handler = move || async move {
        axum::Json(serde_json::json!({
            "my_pubkey": config_pubkey,
            "assistant_pubkey": config_assistant,
            // Expose the data dir so the Settings/Purge confirm can
            // show exactly which path is about to be wiped before the
            // user clicks OK.
            "data_dir": config_data_dir,
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
        // NIP-19 decode
        .route("/api/v1/decode", post(api::decode_handler))
        // NIP-19 encode
        .route("/api/v1/encode", post(api::encode_handler))
        // NIP-84 highlight resolution
        .route(
            "/api/v1/highlights/resolve",
            post(api::resolve_highlights_handler),
        )
        // Documents
        .route("/api/v1/documents", get(api::list_documents_handler))
        .route("/api/v1/documents/parse", post(api::parse_document_handler))
        .route("/api/v1/import", post(api::import_document_handler))
        // Profile + relay config + fetch
        .route("/api/v1/profile/:pubkey", get(api::profile_handler))
        .route("/api/v1/profiles/fetch", post(api::fetch_profiles_handler))
        .route("/api/v1/pull-user-data", post(api::pull_user_data_handler))
        .route("/api/v1/relays", get(api::relay_config_handler))
        .route("/api/v1/relay/info", get(api::relay_nip11_handler))
        .route("/api/v1/config/update", post(api::config_update_handler))
        .route("/api/v1/config/snapshot", post(api::config_snapshot_handler))
        .route("/api/v1/settings", get(api::settings_handler))
        .route(
            "/api/v1/identity/persist-key",
            post(api::identity_persist_key_handler),
        )
        .route("/api/v1/fetch", post(api::fetch_relay_handler))
        .route("/api/v1/fetch/authors", post(api::fetch_authors_handler))
        .route("/api/v1/fetch/sections", post(api::fetch_sections_handler))
        // Network mode
        .route("/api/v1/network/status", get(api::network_status_handler))
        .route("/api/v1/network/mode", post(api::set_network_mode_handler))
        .route(
            "/api/v1/network/fetch-events",
            get(api::fetch_events_handler),
        )
        .route(
            "/api/v1/network/fetch-confirm",
            post(api::fetch_confirm_handler),
        )
        // Ignore list + purge
        .route("/api/v1/ignore", get(api::ignore_list_handler).post(api::ignore_add_handler).delete(api::ignore_remove_handler))
        .route("/api/v1/purge", post(api::purge_handler))
        // Export, publish & ingest endpoints
        .route("/api/v1/export", get(api::export_handler))
        .route("/api/v1/export/manifest", get(api::export_manifest_handler))
        // Drafts — local unsigned-publication storage (DraftStore)
        .route(
            "/api/v1/drafts",
            post(api::save_draft_handler).get(api::list_drafts_handler),
        )
        .route("/api/v1/drafts/diff", post(api::draft_diff_handler))
        .route(
            "/api/v1/drafts/:id",
            get(api::get_draft_handler).delete(api::delete_draft_handler),
        )
        .route("/api/v1/publish", post(api::publish_handler))
        .route("/api/v1/publish/preview", post(api::publish_preview_handler))
        .route("/api/v1/publish/blocks", post(api::publish_blocks_handler))
        .route(
            "/api/v1/publish/republish-diff",
            post(api::republish_diff_handler),
        )
        .route("/api/v1/publish/diff", post(api::diff_published_handler))
        .route("/api/v1/broadcast", post(api::broadcast_handler))
        .route("/api/v1/ingest", post(api::ingest_handler))
        // Embedding endpoints
        .route("/api/v1/embed/status", get(api::embed_status_handler))
        .route("/api/v1/embed/sync", post(api::embed_sync_handler))
        .route("/api/v1/embed/reindex", post(api::embed_reindex_handler))
        .route("/api/v1/embed/config", post(api::embed_config_handler))
        // Claude Code sessions
        .route("/api/v1/claude-sessions", get(api::list_claude_sessions_handler))
        .route("/api/v1/claude-sessions/:id", get(api::get_claude_session_handler))
        .route("/api/v1/claude-sessions/:id/message", post(api::append_claude_session_handler))
        // Publication endpoints
        .route("/api/v1/publications", get(api::list_publications_handler))
        .route(
            "/api/v1/publications/:pubkey/:d_tag",
            get(api::get_publication_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/stream",
            get(api::stream_publication_handler),
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
            "/api/v1/publications/:pubkey/:d_tag/backfill",
            post(api::backfill_publication_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/broadcast",
            post(api::broadcast_publication_handler),
        )
        .route(
            "/api/v1/publications/:pubkey/:d_tag/sections/:index",
            get(api::get_section_handler),
        )
        .route(
            "/api/v1/sections/:pubkey/:d_tag/versions",
            get(api::get_section_versions_handler),
        )
        .route(
            "/api/v1/discussions/counts",
            post(api::discussion_counts_handler),
        )
        .route(
            "/api/v1/discussions/list",
            post(api::discussions_list_handler),
        )
        .with_state(state.clone())
        .merge(chat_routes)
        .merge(identity_routes)
        .merge(signing_routes)
        .layer(axum::Extension(identity_state.clone()))
        .layer(axum::Extension(signing_controller.clone()))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)) // 50MB for JSONL import
        .layer(cors);

    // Background sync — fetch missing sections and embed new events
    if config.embedding.enabled {
        tokio::spawn(async move {
            // Wait for sidecar to start up
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;

                if state.is_auto() {
                    match state.fetch_missing_sections().await {
                        Ok((_, missing, fetched)) => {
                            if fetched > 0 {
                                info!("Background section fetch: {} fetched ({} were missing)", fetched, missing);
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Background section fetch error: {}", e);
                        }
                    }
                }

                // Embed any unembedded events (runs regardless of online/offline,
                // but only when auto-embed is on — otherwise embedding is manual).
                if state.auto_embed() && state.embedding_index().is_some() {
                    match state.sync_embeddings().await {
                        Ok(status) => {
                            if status.indexed_count < status.total_events {
                                info!(
                                    "Background embed sync: {}/{} indexed",
                                    status.indexed_count, status.total_events
                                );
                            }
                        }
                        Err(e) => {
                            tracing::debug!("Background embed sync error: {}", e);
                        }
                    }
                }
            }
        });
    }

    // Start server
    let bind_addr = config.bind_addr();
    info!("Listening on http://{}", bind_addr);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
