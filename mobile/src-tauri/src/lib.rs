//! tendrl Android host.
//!
//! Boot sequence:
//!   1. (setup hook) Resolve the app-private data dir, per-boot loopback
//!      token, and open the WebView IMMEDIATELY on the bundled splash
//!      (`frontendDist`'s `index.html`). The engine used to boot via
//!      `block_on` on the Android main thread *before* any UI existed — a
//!      slow first boot (LMDB map, HNSW/model load) was indistinguishable
//!      from a hang and burned ANR budget.
//!   2. (spawned task) Build the engine `Config` (fresh installs get sane
//!      on-device paths; `config.toml` persists user settings across runs
//!      exactly like desktop; the LMDB mapsize is capped for mobile) and
//!      `server::start` on the loopback port. The token gates `/api/`:
//!      `127.0.0.1` is reachable by every app on the device.
//!   3. Once the port is bound, `navigate()` the WebView to the engine
//!      origin with `?shell=mobile` (mobile shell selection) and
//!      `auth_token=` (captured into a cookie by the SPA's boot module,
//!      then scrubbed from the URL). On a failed boot, the error is
//!      eval()'d into the splash instead of a silent hang. SPA-persisted
//!      state is keyed to the destination origin, so the splash→engine
//!      navigation touches none of it.
//!
//! The WebView ends same-origin with the API — the engine serves the
//! embedded SPA — so fetches and EventSources carry the token cookie
//! automatically.

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
    // Cap the LMDB map for mobile: nostrdb's default is a 32 GiB
    // address-space reservation per boot — harmless on desktop, bad VSS
    // optics under the low-memory killer. 4 GiB is far beyond any plausible
    // on-device store; a user-set config.toml value wins. If a store ever
    // outgrows it, ingest fails with MDB_MAP_FULL — raise the knob.
    config.database.mapsize_mb.get_or_insert(4096);
    // Embeddings on: the index is cheap to open; the heavy part (the
    // one-time ~90 MB model download) only happens on an explicit,
    // labeled user action — the UI gates it on the status endpoint's
    // model_ready flag. Boot-time only (init_embedding runs pre-Arc), so
    // the host decides rather than the settings UI.
    config.embedding.enabled = true;
    // Embed in trickles: an uncapped backlog pass saturates the cores for
    // minutes on-device (thermals + battery + a starved WebView). 64 per
    // 60 s tick clears a 600-section backlog in ~10 min of background time;
    // a user-set config.toml value wins.
    config.embedding.max_per_tick.get_or_insert(64);
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
            {
                // hf-hub's ApiBuilder::new() constructs Cache::default(),
                // which does dirs::home_dir().expect("Cache directory cannot
                // be found") — and Android has no $HOME, so the model
                // download panics before fastembed's explicit cache_dir
                // override can apply. Give home_dir something to return;
                // the override still decides where the model actually lands
                // (<data>/models).
                std::env::set_var("HOME", &data_root);
                set_ort_dylib_path();
            }
            let token = per_boot_token();

            // WebView FIRST, on the bundled splash — the engine boot below
            // runs in a spawned task, off the main thread, so a slow first
            // boot shows a spinner instead of eating the ANR window.
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .build()?;

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let (config, config_path) = engine_config(&data_root, PREFERRED_PORT);
                let started = match nostr_engine::server::start(
                    nostr_engine::server::ServeOptions {
                        config,
                        config_path,
                        loopback_token: Some(token.clone()),
                    },
                )
                .await
                {
                    Ok(server) => Ok(server),
                    Err(e) => {
                        tracing::warn!(
                            "preferred port {PREFERRED_PORT} unavailable ({e}); falling back \
                             to an ephemeral port — SPA-persisted state (signer, prefs) won't \
                             carry over"
                        );
                        let (config, config_path) = engine_config(&data_root, 0);
                        nostr_engine::server::start(nostr_engine::server::ServeOptions {
                            config,
                            config_path,
                            loopback_token: Some(token.clone()),
                        })
                        .await
                    }
                };
                match started {
                    Ok(server) => {
                        tracing::info!("engine bound on {}", server.addr);
                        // Foreground gate: the run-loop callback flips this on
                        // Suspended/Resumed so the 60s background loop sleeps
                        // while the app is backgrounded. Managed late (post-
                        // bind) — the run-loop reads it via try_state and
                        // tolerates its absence in the boot window.
                        handle.manage(BackgroundGate(server.background_paused.clone()));
                        let url = format!(
                            "http://127.0.0.1:{}/?shell=mobile&auth_token={}",
                            server.addr.port(),
                            token
                        );
                        match url.parse::<tauri::Url>() {
                            Ok(target) => {
                                // The Android WebView is created asynchronously
                                // while the engine boots in parallel — and the
                                // engine wins by a wide margin (observed: bound
                                // in 18 ms on a warm store). navigate() only
                                // queues an event-loop message; if the webview
                                // doesn't exist yet the message is DROPPED
                                // silently and the splash never leaves. Retry
                                // until the webview actually reports the
                                // engine origin.
                                let mut navigated = false;
                                for attempt in 1..=50u32 {
                                    if let Err(e) = window.navigate(target.clone()) {
                                        tracing::warn!("navigate attempt {attempt}: {e}");
                                    }
                                    tokio::time::sleep(std::time::Duration::from_millis(200))
                                        .await;
                                    if let Ok(current) = window.url() {
                                        if current.host_str() == target.host_str()
                                            && current.port() == target.port()
                                        {
                                            tracing::info!(
                                                "webview on the engine origin \
                                                 (attempt {attempt})"
                                            );
                                            navigated = true;
                                            break;
                                        }
                                    }
                                }
                                if !navigated {
                                    tracing::error!(
                                        "webview never reached the engine origin"
                                    );
                                    let _ = window.eval(
                                        "document.getElementById('status').textContent = \
                                         'Engine is up but the app failed to load — \
                                          reopen the app.';",
                                    );
                                }
                            }
                            Err(e) => tracing::error!("engine URL failed to parse: {e}"),
                        }
                    }
                    Err(e) => {
                        tracing::error!("engine failed to start: {e}");
                        // Surface it on the splash — a visible failure beats
                        // the old panic-in-setup (and beats a forever-spinner).
                        let msg = format!("Engine failed to start: {e}")
                            .replace('\\', " ")
                            .replace('"', "'")
                            .replace('\n', " ");
                        let _ = window.eval(&format!(
                            "document.getElementById('status').textContent = \"{msg}\";\
                             document.querySelector('.ring').style.display = 'none';"
                        ));
                    }
                }
            });
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
