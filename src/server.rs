//! Embeddable engine boot: the full HTTP server (engine + routers +
//! background sync) behind a single library entry point.
//!
//! The `tendrl-engine` binary is a thin CLI wrapper over [`start`]; an
//! embedding host (the Tauri Android shell) calls it directly with
//! `config.server.port = 0` to bind an ephemeral loopback port and reads
//! the real port back from [`RunningServer::addr`].

use axum::{
    routing::{get, post, put},
    Router,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::{
    api, chat::ChatState, config::Config, engine::Engine, identity::IdentitySession, llm, spell,
    static_assets, tools,
};

/// Everything [`start`] needs. The config must already be fully resolved —
/// CLI/host overrides applied — because load and save must agree on one file.
pub struct ServeOptions {
    pub config: Config,
    /// The path `config` was loaded from; settings persist back to it.
    pub config_path: PathBuf,
    /// Per-boot secret gating `/api/` requests. On Android the loopback
    /// interface is shared by every app on the device, so "it's local" is not
    /// a trust boundary — the Tauri host generates a token each boot and
    /// injects it into the WebView's initial URL; anything on the port without
    /// it gets 401. `None` (the desktop default) adds no middleware at all.
    pub loopback_token: Option<String>,
}

/// A started server. `start` returns only after the listener is bound, so
/// `addr` is immediately connectable (requests queue in the accept backlog
/// even before the serve task is first polled).
pub struct RunningServer {
    /// The actual bound address — with `port = 0` this carries the
    /// kernel-assigned ephemeral port.
    pub addr: SocketAddr,
    /// The serve task; await it to run until shutdown.
    pub handle: tokio::task::JoinHandle<std::io::Result<()>>,
    /// Pause switch for the 60s background loop (section fetch + embed
    /// sync). Mobile hosts flip it on Suspended/Resumed so a backgrounded
    /// app doesn't keep waking the CPU; ticks resume where they left off.
    /// Inert when embeddings are disabled (the loop doesn't exist) and on
    /// desktop (nothing flips it).
    pub background_paused: Arc<std::sync::atomic::AtomicBool>,
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
    let Ok(blob) = crate::identity::IdentityKeyring::new().get_last_assistant() else {
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

/// Build the engine and every router, bind the listener, and spawn the serve
/// loop. Returns once the port is bound.
/// Does `req` carry the loopback token? Non-`/api/` paths (the embedded SPA,
/// `/health`) always pass — the app shell itself is not secret, and the boot
/// URL is what *delivers* the token. Accepted carriers, in order: the
/// `X-Tendrl-Token` header, a `tendrl_token` cookie (set once by the SPA from
/// the boot query param — rides on every same-origin fetch *and* EventSource
/// with no per-call-site code), and an `auth_token` query param as the
/// belt-and-braces fallback for contexts where cookies fail.
fn request_has_token(req: &axum::extract::Request, token: &str) -> bool {
    if !req.uri().path().starts_with("/api/") {
        return true;
    }
    if let Some(h) = req.headers().get("x-tendrl-token") {
        if h.to_str().map(|v| v == token).unwrap_or(false) {
            return true;
        }
    }
    if let Some(c) = req.headers().get(axum::http::header::COOKIE) {
        if let Ok(s) = c.to_str() {
            let found = s.split(';').any(|kv| {
                kv.trim()
                    .strip_prefix("tendrl_token=")
                    .map(|v| v == token)
                    .unwrap_or(false)
            });
            if found {
                return true;
            }
        }
    }
    if let Some(q) = req.uri().query() {
        let found = q.split('&').any(|kv| {
            kv.strip_prefix("auth_token=")
                .map(|v| v == token)
                .unwrap_or(false)
        });
        if found {
            return true;
        }
    }
    false
}

/// Wrap `app` in the loopback-token guard.
fn apply_loopback_guard(app: Router, token: String) -> Router {
    use axum::response::IntoResponse;
    let token = Arc::new(token);
    app.layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let token = token.clone();
            async move {
                if request_has_token(&req, &token) {
                    next.run(req).await
                } else {
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        axum::Json(serde_json::json!({
                            "error": { "message": "missing or invalid loopback token" }
                        })),
                    )
                        .into_response()
                }
            }
        },
    ))
}

pub async fn start(opts: ServeOptions) -> anyhow::Result<RunningServer> {
    let ServeOptions {
        config,
        config_path,
        loopback_token,
    } = opts;

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
    engine.set_config_path(config_path.clone());
    engine.set_documents_path(PathBuf::from(&config.documents.path));

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
        .and_then(crate::identity::IdentitySource::from_config_str)
        .unwrap_or(crate::identity::IdentitySource::Engine);
    info!("Saved identity source: {}", boot_source.kind_str());
    let user_session: api::IdentityAppState =
        Arc::new(Mutex::new(IdentitySession::with_source(boot_source)));
    // Honor the saved auto-lock timeout (0 = never) from the first unlock on.
    user_session
        .lock()
        .unwrap()
        .set_timeout_minutes(config.identity.lock_timeout_minutes);

    let keyring_available = crate::identity::IdentityKeyring::new().is_available();
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
    let claude_dir = crate::claude_sessions::resolve_claude_dir(&cwd);
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
    if let Ok(mode) = config.network.mode.parse::<crate::NetworkMode>() {
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
        Some(&config_path),
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
            "/api/v1/identity/login-npub",
            post(api::identity_npub_login_handler),
        )
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
    let signing_controller = crate::signing::SigningController::new(user_session.clone());

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
        .route(
            "/api/v1/discussions/comment",
            post(api::discussion_comment_handler),
        )
        .route(
            "/api/v1/discussions/highlight",
            post(api::discussion_highlight_handler),
        )
        .route(
            "/api/v1/discussions/highlight/preview",
            post(api::discussion_highlight_preview_handler),
        )
        .route(
            "/api/v1/discussions/delete",
            post(api::discussion_delete_handler),
        )
        .route("/api/v1/spell/inspect", post(spell::inspect_handler))
        .route("/api/v1/spell/execute", post(spell::execute_handler))
        .route("/api/v1/spell/list", post(spell::list_handler))
        .route("/api/v1/spell/compose", post(spell::compose_handler))
        .route("/api/v1/spell/book", post(spell::book_handler))
        .route(
            "/api/v1/spell/book/template",
            post(spell::book_template_handler),
        )
        .route("/api/v1/spell/book/save", post(spell::book_save_handler))
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

    // Loopback auth: added only when a token is configured, so the desktop
    // flow (no token) is byte-identical.
    let app = match loopback_token {
        Some(token) => {
            info!("Loopback token auth enabled for /api");
            apply_loopback_guard(app, token)
        }
        None => app,
    };

    // Background sync — fetch missing sections and embed new events
    let background_paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if config.embedding.enabled {
        let state = state.clone();
        let paused = background_paused.clone();
        tokio::spawn(async move {
            // Let startup settle before the first background pass. The ONNX
            // model loads in-process on first embed (no sidecar to wait for).
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;

                // Foreground gate: skip the whole pass while the host says
                // we're backgrounded (mobile). Cheap check per tick.
                if paused.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }

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

    // Bind before returning so the caller's `addr` is immediately usable.
    let bind_addr = config.bind_addr();
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let addr = listener.local_addr()?;
    info!("Listening on http://{}", addr);

    let handle = tokio::spawn(async move { axum::serve(listener, app).await });

    Ok(RunningServer {
        addr,
        handle,
        background_paused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const TOKEN: &str = "sekrit";

    /// A tiny router standing in for the real app: one /api route, one open
    /// route — enough to exercise the guard without booting an Engine.
    fn guarded_app() -> Router {
        let app = Router::new()
            .route("/api/v1/ping", get(|| async { "pong" }))
            .route("/health", get(|| async { "ok" }));
        apply_loopback_guard(app, TOKEN.to_string())
    }

    async fn status_of(req: Request<Body>) -> StatusCode {
        guarded_app().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn api_without_token_is_401() {
        let req = Request::get("/api/v1/ping").body(Body::empty()).unwrap();
        assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn api_with_header_token_passes() {
        let req = Request::get("/api/v1/ping")
            .header("x-tendrl-token", TOKEN)
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn api_with_cookie_token_passes() {
        let req = Request::get("/api/v1/ping")
            .header("cookie", format!("foo=bar; tendrl_token={TOKEN}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn api_with_query_token_passes() {
        let req = Request::get(format!("/api/v1/ping?auth_token={TOKEN}"))
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(req).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn api_with_wrong_token_is_401() {
        let req = Request::get("/api/v1/ping")
            .header("x-tendrl-token", "wrong")
            .header("cookie", "tendrl_token=wrong")
            .body(Body::empty())
            .unwrap();
        assert_eq!(status_of(req).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn non_api_paths_stay_open() {
        let req = Request::get("/health").body(Body::empty()).unwrap();
        assert_eq!(status_of(req).await, StatusCode::OK);
    }
}
