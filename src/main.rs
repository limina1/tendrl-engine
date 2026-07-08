//! nostr-engine HTTP server binary
//!
//! Run a standalone HTTP API for querying Nostr events.

use axum::{
    routing::{get, post, put},
    Router,
};
use clap::Parser;
use nostr_engine::{
    api, chat::ChatState, config::Config, engine::Engine, identity::IdentitySession, llm,
    spell, static_assets, tools,
};
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

    /// Do not open a browser on startup (for headless/server use)
    #[arg(long)]
    no_open: bool,

    /// Download the embedding model into a `models/` folder next to the binary,
    /// then exit. Pre-populates the portable bundle so it ships with the model —
    /// end users get embeddings with no first-run HuggingFace download.
    #[arg(long)]
    fetch_model: bool,
}

/// Restore a persisted assistant identity from the OS keyring into `session`.
/// Best-effort: a missing/unavailable keyring or a malformed blob just leaves
/// the assistant unset. An ncryptsec restores to a *locked* signer (pubkey
/// hinted so `by:assistant` scopes before unlock); a pubkey-only blob (from a
/// session-only nsec) restores scoping but not signing — re-paste the nsec to
/// sign again.
fn restore_assistant_identity(session: &api::IdentityAppState, keyring_available: bool) {
    if !keyring_available {
        return;
    }
    let Ok(blob) = nostr_engine::identity::IdentityKeyring::new().get_last_assistant() else {
        return;
    };
    #[derive(serde::Deserialize)]
    struct Persist {
        pubkey: Option<String>,
        ncryptsec: Option<String>,
    }
    let Ok(p) = serde_json::from_str::<Persist>(&blob) else {
        return;
    };
    let mut s = session.lock().unwrap();
    if let Some(nc) = p.ncryptsec.as_deref() {
        if s.login_ncryptsec(nc).is_ok() {
            if let Some(pk) = p.pubkey {
                s.set_pubkey_hint(pk);
            }
            info!("Restored assistant identity (locked) from keyring");
            return;
        }
    }
    if let Some(pk) = p.pubkey {
        s.set_pubkey_hint(pk);
        info!("Restored assistant pubkey (scope-only) from keyring");
    }
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

    tracing_subscriber::fmt().with_env_filter(filter).init();

    // Resolve the config path up front so load and save agree on ONE file.
    // Explicit `-c` wins; otherwise <data_dir>/config.toml, where data_dir is
    // `--data-dir` if given else the built-in default. This is what makes a
    // zero-config run persist *and* reload settings (network mode, etc.) —
    // previously boot loaded hardcoded defaults while saves went to disk
    // unread, so nothing round-tripped without `-c`.
    let config_path = args.config.clone().unwrap_or_else(|| {
        let dir = args
            .data_dir
            .clone()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(nostr_engine::config::default_data_dir);
        PathBuf::from(dir).join("config.toml")
    });

    // Load configuration from that path (falls back to defaults if absent).
    let mut config = Config::load_or_default(Some(&config_path));

    // CLI args override config file
    config.server.port = args.port;
    config.server.host = args.host;

    if let Some(data_dir) = args.data_dir {
        config.database.data_dir = data_dir.to_string_lossy().to_string();
    }

    // `--fetch-model`: pre-download the embedding model into a `models/` folder
    // next to the executable, then exit. The portable bundle ships that folder so
    // end users get embeddings with no first-run HuggingFace download. Runs before
    // any server/db setup.
    if args.fetch_model {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("models")))
            .unwrap_or_else(|| PathBuf::from("models"));
        info!(
            "Fetching embedding model '{}' into {}",
            config.embedding.model,
            dir.display()
        );
        nostr_engine::embedding::EmbeddingIndex::prefetch_model(&config.embedding.model, &dir)?;
        info!("Model cached at {} — ship this folder beside the binary.", dir.display());
        return Ok(());
    }

    info!("Starting nostr-engine v{}", env!("CARGO_PKG_VERSION"));
    info!("Data directory: {}", config.database.data_dir);

    // Pass the TOML-derived RelayConfig straight through. The working
    // URL sets are layered in by Engine::with_relay_config from
    // <data_dir>/relays.json (seeded from initial_relays on first boot).
    let relay_config = config.relay.clone();

    // Create the engine
    let data_path = PathBuf::from(&config.database.data_dir);
    let mut engine = Engine::with_relay_config(&data_path, &relay_config)?;
    // Same path we loaded from — settings persist to and reload from one file.
    engine.set_config_path(config_path);
    engine.set_documents_path(std::path::PathBuf::from(&config.documents.path));

    // --- Identity sessions (runtime, never config) -------------------------
    // Identity is no longer seeded from config. The user identity boots at the
    // saved signer source (config keeps only the non-secret `[identity] source`
    // preference, so the engine knows which signer to reattach to); the key is
    // provided at runtime via NIP-07 or a pasted ncryptsec. The assistant
    // identity is restored from the OS keyring if present. Both sessions are
    // wired into the engine so `by:me` / `by:assistant` resolve from the live
    // session rather than a config seed.
    let boot_source = config
        .identity
        .source
        .as_deref()
        .and_then(nostr_engine::identity::IdentitySource::from_config_str)
        .unwrap_or(nostr_engine::identity::IdentitySource::Engine);
    info!("Saved identity source: {}", boot_source.kind_str());
    let user_session: api::IdentityAppState =
        Arc::new(Mutex::new(IdentitySession::with_source(boot_source)));
    // Honor the saved auto-lock timeout (0 = never) from the first unlock on.
    user_session
        .lock()
        .unwrap()
        .set_timeout_minutes(config.identity.lock_timeout_minutes);

    let keyring_available = nostr_engine::identity::IdentityKeyring::new().is_available();
    if !keyring_available {
        tracing::warn!(
            "OS keyring unavailable — assistant identity will not persist across restarts"
        );
    }
    let assistant_session: api::IdentityAppState = Arc::new(Mutex::new(IdentitySession::new()));
    restore_assistant_identity(&assistant_session, keyring_available);

    engine.set_user_session(user_session.clone());
    engine.set_assistant_session(assistant_session.clone());

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

    // Set initial network mode from config. `mode_chosen` is false on a fresh
    // install, which raises the one-time "choose your default mode" modal in
    // the UI before any relay fetch.
    if let Ok(mode) = config.network.mode.parse::<nostr_engine::NetworkMode>() {
        engine.set_initial_network_mode(mode, config.network.mode_chosen);
        info!(
            "Network mode: {} (chosen: {})",
            mode, config.network.mode_chosen
        );
    }

    let state = Arc::new(engine);

    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Chat state + LLM provider
    let ai_policy = match config.ai.enabled_tools.clone() {
        Some(names) => tools::ToolPolicy::from_enabled(names),
        None => tools::ToolPolicy::default(),
    };
    let prompt_path = api::resolve_prompt_path(
        args.config.as_deref(),
        state.data_dir(),
        config.ai.system_prompt_path.as_deref(),
    );
    api::ensure_prompt_file(&prompt_path);
    info!("AI system prompt: {}", prompt_path.display());
    // Seed the chat's system prompt from prompt.md so the UI's System view
    // shows it on load and the agent uses it through the normal message path.
    let mut initial_chat = ChatState::new();
    initial_chat.system_prompt = api::read_system_prompt(&prompt_path);
    let chat_state = api::ChatAppState {
        chat: Arc::new(Mutex::new(initial_chat)),
        provider: llm::provider_from_config(&config.ai),
        engine: state.clone(),
        max_tool_turns: config.ai.max_tool_turns,
        policy: Arc::new(std::sync::RwLock::new(ai_policy)),
        system_prompt_path: prompt_path,
    };

    let chat_routes = Router::new()
        .route("/api/v1/chat", get(api::chat_get).delete(api::chat_reset))
        .route("/api/v1/chat/message", post(api::chat_send))
        .route("/api/v1/chat/agent", post(api::chat_agent_handler))
        .route(
            "/api/v1/chat/sessions",
            get(api::session_list).post(api::session_save),
        )
        .route(
            "/api/v1/chat/sessions/:id",
            get(api::session_load).delete(api::session_delete),
        )
        .route(
            "/api/v1/ai/settings",
            get(api::ai_settings_get).post(api::ai_settings_post),
        )
        .route(
            "/api/v1/ai/prompt",
            get(api::ai_prompt_get).put(api::ai_prompt_put),
        )
        .route(
            "/api/v1/chat/edit",
            post(api::chat_enter_edit).put(api::chat_exit_edit),
        )
        .route("/api/v1/chat/system", post(api::chat_set_system))
        .route(
            "/api/v1/chat/context",
            post(api::chat_inject_context).put(api::chat_replace_context),
        )
        .route("/api/v1/chat/load", put(api::chat_load_fragments))
        .with_state(chat_state);

    // Identity routes operate on the user session created above (boot source +
    // auto-lock timeout already applied). The session source honors
    // `[identity] source` so a nip07 user reattaches without the modeline
    // flickering to "engine" on every restart.
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
        .with_state(user_session.clone());

    // SigningController owns the external-signer registry and routes
    // signing through the active source. Constructed once and shared
    // across the sign / signer-register / signer-channel / sign-response
    // handlers.
    let signing_controller = nostr_engine::signing::SigningController::new(user_session.clone());

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

    // Assistant identity routes — a second session, paste-to-establish
    // (nsec/ncryptsec), persisted in the OS keyring. Publish-as-assistant is
    // intentionally NOT exposed here: the signing capability stays gated until
    // an explicit, confirmed publish path exists.
    let assistant_routes = Router::new()
        .route(
            "/api/v1/assistant-identity",
            get(api::assistant_identity_status_handler),
        )
        .route(
            "/api/v1/assistant-identity/login",
            post(api::assistant_identity_login_handler),
        )
        .route(
            "/api/v1/assistant-identity/unlock",
            post(api::assistant_identity_unlock_handler),
        )
        .route(
            "/api/v1/assistant-identity/logout",
            post(api::assistant_identity_logout_handler),
        )
        .with_state(api::AssistantIdentity {
            session: assistant_session.clone(),
            keyring_available,
        });

    // Config endpoint — only the data dir now. Identity pubkeys come from the
    // live /identity and /assistant-identity status endpoints, not config.
    let config_data_dir = state.data_dir().to_string_lossy().to_string();
    let config_handler = move || async move {
        axum::Json(serde_json::json!({
            // Expose the data dir so the Settings/Purge confirm can show
            // exactly which path is about to be wiped before the user clicks OK.
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
        // Nostrdown {{ }} reference resolution
        .route(
            "/api/v1/nostrdown/resolve",
            post(api::resolve_nostrdown_handler),
        )
        .route("/api/v1/nostrdown/parse", post(api::parse_nostrdown_handler))
        .route(
            "/api/v1/nostrdown/normalize",
            post(api::normalize_nostrdown_handler),
        )
        .route(
            "/api/v1/nostrdown/fetch-entity",
            post(api::fetch_nostrdown_entity_handler),
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
        .route(
            "/api/v1/config/snapshot",
            post(api::config_snapshot_handler),
        )
        .route("/api/v1/config/export", get(api::config_export_handler))
        .route("/api/v1/settings", get(api::settings_handler))
        .route("/api/v1/fetch", post(api::fetch_relay_handler))
        .route("/api/v1/fetch/authors", post(api::fetch_authors_handler))
        .route("/api/v1/fetch/sections", post(api::fetch_sections_handler))
        // Network mode
        .route("/api/v1/network/status", get(api::network_status_handler))
        .route("/api/v1/network/mode", post(api::set_network_mode_handler))
        .route(
            "/api/v1/network/reset-mode-choice",
            post(api::reset_mode_choice_handler),
        )
        .route(
            "/api/v1/network/fetch-events",
            get(api::fetch_events_handler),
        )
        .route(
            "/api/v1/network/fetch-confirm",
            post(api::fetch_confirm_handler),
        )
        // Ignore list + purge
        .route(
            "/api/v1/ignore",
            get(api::ignore_list_handler)
                .post(api::ignore_add_handler)
                .delete(api::ignore_remove_handler),
        )
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
        .route(
            "/api/v1/publish/preview",
            post(api::publish_preview_handler),
        )
        .route("/api/v1/publish/blocks", post(api::publish_blocks_handler))
        .route(
            "/api/v1/publish/blocks/preview",
            post(api::publish_blocks_preview_handler),
        )
        .route(
            "/api/v1/publish/republish-diff",
            post(api::republish_diff_handler),
        )
        .route(
            "/api/v1/relays/publish-diff",
            post(api::relay_list_diff_handler),
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
        .route(
            "/api/v1/claude-sessions",
            get(api::list_claude_sessions_handler),
        )
        .route(
            "/api/v1/claude-sessions/:id",
            get(api::get_claude_session_handler),
        )
        .route(
            "/api/v1/claude-sessions/:id/message",
            post(api::append_claude_session_handler),
        )
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
        .route("/api/v1/spell/inspect", post(spell::inspect_handler))
        .route("/api/v1/spell/execute", post(spell::execute_handler))
        .route("/api/v1/spell/list", post(spell::list_handler))
        .route("/api/v1/spell/compose", post(spell::compose_handler))
        .with_state(state.clone())
        .merge(chat_routes)
        .merge(identity_routes)
        .merge(signing_routes)
        .merge(assistant_routes)
        // Serve the embedded SPA for everything that is not an API route.
        .fallback(static_assets::static_handler)
        .layer(axum::Extension(user_session.clone()))
        .layer(axum::Extension(signing_controller.clone()))
        .layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)) // 50MB for JSONL import
        .layer(cors);

    // Background sync — fetch missing sections and embed new events
    if config.embedding.enabled {
        tokio::spawn(async move {
            // Let startup settle before the first background pass. The ONNX
            // model loads in-process on first embed (no sidecar to wait for).
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;

                if state.is_auto() {
                    match state.fetch_missing_sections().await {
                        Ok((_, missing, fetched)) => {
                            if fetched > 0 {
                                info!(
                                    "Background section fetch: {} fetched ({} were missing)",
                                    fetched, missing
                                );
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

    // Open the browser to the bundled UI once we're bound. The host is the bind
    // address (127.0.0.1 by default); for a non-loopback bind we skip it, since
    // that's a server deployment where popping a local browser makes no sense.
    if !args.no_open {
        let host = config.server.host.as_str();
        let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        if is_loopback {
            let url = format!("http://{}/", bind_addr);
            info!("Opening browser at {}", url);
            open_default_browser(&url);
        }
    }

    axum::serve(listener, app).await?;

    Ok(())
}

/// Open `url` in the user's **default** browser.
///
/// On Linux we route through `$BROWSER` (explicit user override) then
/// `xdg-open`, which honor the desktop's `x-scheme-handler/http` association —
/// i.e. whatever the user actually set as their default. The `webbrowser`
/// crate's Linux path can otherwise bypass that and launch a hardcoded browser
/// (the "wrong browser" testers reported). macOS/Windows defer to the crate,
/// whose `open` / `start` already respect the system default. If the platform
/// opener is missing or fails, fall back to the crate's heuristics.
fn open_default_browser(url: &str) {
    #[cfg(target_os = "linux")]
    {
        use std::process::{Command, Stdio};
        let spawn = |prog: &str| {
            Command::new(prog)
                .arg(url)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_ok()
        };
        // Explicit $BROWSER wins; otherwise the desktop default via xdg-open.
        if let Ok(b) = std::env::var("BROWSER") {
            if !b.is_empty() && spawn(&b) {
                return;
            }
        }
        if spawn("xdg-open") {
            return;
        }
        // Neither worked — fall through to the crate.
    }
    if let Err(e) = webbrowser::open(url) {
        tracing::warn!("Could not open browser ({e}); navigate to {url} manually");
    }
}
