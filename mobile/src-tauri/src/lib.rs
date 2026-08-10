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

fn engine_config(
    data_root: &std::path::Path,
) -> (nostr_engine::Config, std::path::PathBuf) {
    let config_path = data_root.join("config.toml");
    let mut config = nostr_engine::Config::load_or_default(Some(&config_path));
    config.database.data_dir = data_root.join("db").to_string_lossy().into_owned();
    config.documents.path = data_root.join("documents").to_string_lossy().into_owned();
    // No embedding backend is compiled into the mobile build (yet — the
    // ort-load-dynamic path lands later); keep the flag off so the engine
    // doesn't warn every boot.
    config.embedding.enabled = false;
    config.server.host = "127.0.0.1".into();
    // Ephemeral port: the kernel picks, we read it back from `start()`.
    config.server.port = 0;
    (config, config_path)
}

fn per_boot_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS randomness for the loopback token");
    hex::encode(bytes)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            let (config, config_path) = engine_config(&data_root);
            let token = per_boot_token();

            let server = tauri::async_runtime::block_on(nostr_engine::server::start(
                nostr_engine::server::ServeOptions {
                    config,
                    config_path,
                    loopback_token: Some(token.clone()),
                },
            ))?;
            tracing::info!("engine bound on {}", server.addr);

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
        .run(tauri::generate_context!())
        .expect("error while running tendrl mobile host");
}
