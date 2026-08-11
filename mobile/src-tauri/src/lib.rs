//! tendrl Android host.
//!
//! Boot sequence (all in the Tauri `setup` hook):
//!   1. Resolve the app-private data dir and build the engine `Config` there
//!      (fresh installs get sane on-device paths; `config.toml` persists user
//!      settings across runs exactly like desktop).
//!   2. Generate a per-boot loopback token — on Android `127.0.0.1` is
//!      reachable by every app on the device, so the engine 401s `/api/`
//!      requests that don't carry it.
//!   3. Start the engine in-process on an ephemeral loopback port
//!      (`server::start` returns once the port is bound).
//!   4. Open the WebView at the engine origin with `?shell=mobile` (mobile
//!      shell selection) and `auth_token=` (captured into a cookie by the
//!      SPA's boot module, then scrubbed from the URL).
//!
//! The WebView is same-origin with the API — the engine serves the embedded
//! SPA — so fetches and EventSources carry the token cookie automatically.

use tauri::Manager;

mod nip55;

/// Stable loopback port. The WebView origin (scheme+host+port) keys ALL of
/// the SPA's persisted state — localStorage (signer-app pubkey, watch npub,
/// shell prefs) and cookies. An ephemeral port meant a fresh origin every
/// launch, silently wiping that state (the NIP-55 boot re-attach never found
/// its persisted pubkey). Uncommon high port to dodge other apps; on bind
/// failure we fall back to ephemeral — state loss beats not launching.
const PREFERRED_PORT: u16 = 41347;

fn engine_config(
    data_root: &std::path::Path,
    port: u16,
) -> (nostr_engine::Config, std::path::PathBuf) {
    let config_path = data_root.join("config.toml");
    let mut config = nostr_engine::Config::load_or_default(Some(&config_path));
    config.database.data_dir = data_root.join("db").to_string_lossy().into_owned();
    config.documents.path = data_root.join("documents").to_string_lossy().into_owned();
    // Embeddings on: the index is cheap to open; the heavy part (the
    // one-time ~90 MB model download) only happens on an explicit,
    // labeled user action — the UI gates it on the status endpoint's
    // model_ready flag. Boot-time only (init_embedding runs pre-Arc), so
    // the host decides rather than the settings UI.
    config.embedding.enabled = true;
    // Explicit model cache under app data: the desktop fallback probes
    // current_exe()'s parent for a shipped models/ folder, which is
    // meaningless inside an APK.
    config.embedding.cache_dir = Some(data_root.join("models").to_string_lossy().into_owned());
    config.server.host = "127.0.0.1".into();
    config.server.port = port;
    (config, config_path)
}

/// Tell ort's load-dynamic to dlopen the ONNX Runtime by **soname**. With
/// modern packaging the AAR's libonnxruntime.so is not extracted to the
/// filesystem, but it lives in the app's linker namespace (same dir as our
/// host lib), so `dlopen("libonnxruntime.so")` resolves without a path —
/// the standard unextracted-lib pattern. (This is also ort's default on
/// Android; set explicitly for a clear log line and to skip ort's
/// current_exe()-relative probe.)
#[cfg(target_os = "android")]
fn set_ort_dylib_path() {
    std::env::set_var("ORT_DYLIB_PATH", "libonnxruntime.so");
    tracing::info!("ORT_DYLIB_PATH=libonnxruntime.so (resolved via app linker namespace)");
}

fn per_boot_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS randomness for the loopback token");
    hex::encode(bytes)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Android: stdout is a black hole — send tracing to logcat (tag
    // "tendrl", filter with `adb logcat -s tendrl`). Elsewhere keep the
    // stdout fmt subscriber for desktop-dev runs of the host.
    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        let filter = tracing_subscriber::EnvFilter::new(
            "info,nostr_engine=debug,tendrl_mobile_lib=debug",
        );
        match tracing_android::layer("tendrl") {
            Ok(layer) => {
                tracing_subscriber::registry().with(filter).with(layer).init();
            }
            Err(e) => eprintln!("failed to init logcat tracing: {e}"),
        }
    }
    #[cfg(not(target_os = "android"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nostr_engine=info".parse().unwrap())
                .add_directive("tendrl_mobile_lib=info".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(nip55::init())
        .setup(|app| {
            let data_root = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_root)?;
            #[cfg(target_os = "android")]
            set_ort_dylib_path();
            let token = per_boot_token();

            let (config, config_path) = engine_config(&data_root, PREFERRED_PORT);
            let server = match tauri::async_runtime::block_on(nostr_engine::server::start(
                nostr_engine::server::ServeOptions {
                    config,
                    config_path: config_path.clone(),
                    loopback_token: Some(token.clone()),
                },
            )) {
                Ok(server) => server,
                Err(e) => {
                    tracing::warn!(
                        "preferred port {PREFERRED_PORT} unavailable ({e}); falling back to an \
                         ephemeral port — SPA-persisted state (signer, prefs) won't carry over"
                    );
                    let (config, config_path) = engine_config(&data_root, 0);
                    tauri::async_runtime::block_on(nostr_engine::server::start(
                        nostr_engine::server::ServeOptions {
                            config,
                            config_path,
                            loopback_token: Some(token.clone()),
                        },
                    ))?
                }
            };
            tracing::info!("engine bound on {}", server.addr);
            // Foreground gate: the run-loop callback below flips this on
            // Suspended/Resumed so the 60s background loop sleeps while
            // the app is backgrounded.
            app.manage(BackgroundGate(server.background_paused.clone()));

            let url = format!(
                "http://127.0.0.1:{}/?shell=mobile&auth_token={}",
                server.addr.port(),
                token
            );
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse()?),
            )
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tendrl mobile host")
        .run(|app, event| {
            // Mobile lifecycle → background-loop gate. Desktop-dev builds of
            // the host never see these variants (cfg(mobile)).
            #[cfg(target_os = "android")]
            if let tauri::RunEvent::WindowEvent { event, .. } = &event {
                let paused = match event {
                    tauri::WindowEvent::Suspended => Some(true),
                    tauri::WindowEvent::Resumed => Some(false),
                    _ => None,
                };
                if let Some(paused) = paused {
                    if let Some(gate) = app.try_state::<BackgroundGate>() {
                        gate.0
                            .store(paused, std::sync::atomic::Ordering::Relaxed);
                        tracing::info!(
                            "background loop {}",
                            if paused { "paused (app suspended)" } else { "resumed" }
                        );
                    }
                }
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, event);
        });
}

/// The engine's background-loop pause switch, managed as Tauri state so the
/// run-loop lifecycle callback can reach it.
struct BackgroundGate(std::sync::Arc<std::sync::atomic::AtomicBool>);
