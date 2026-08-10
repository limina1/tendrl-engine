//! nostr-engine HTTP server binary
//!
//! Thin CLI wrapper over `nostr_engine::server::start` — argument parsing,
//! logging, config resolution, and the desktop browser-open live here; the
//! engine boot itself (routers, background sync, bind + serve) is in the
//! library so embedding hosts (the Tauri Android shell) share it.

use clap::Parser;
use nostr_engine::config::Config;
use nostr_engine::server::{self, ServeOptions};
use std::path::PathBuf;
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

    let host = config.server.host.clone();

    let server = server::start(ServeOptions {
        config,
        config_path,
    })
    .await?;

    // Open the browser to the bundled UI once we're bound. The host is the bind
    // address (127.0.0.1 by default); for a non-loopback bind we skip it, since
    // that's a server deployment where popping a local browser makes no sense.
    if !args.no_open {
        let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
        if is_loopback {
            let url = format!("http://{}/", server.addr);
            info!("Opening browser at {}", url);
            open_default_browser(&url);
        }
    }

    server.handle.await??;

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
