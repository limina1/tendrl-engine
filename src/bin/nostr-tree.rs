//! nostr-tree: TUI for navigating NKBIP-01 publications
//!
//! A vim-style terminal interface for browsing and manipulating
//! Nostr publications (kind 30040/30041).

use clap::Parser;
use nostr_engine::engine::{Engine, FetchPolicy};
use nostr_engine::tree::tui::TuiApp;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "nostr-tree")]
#[command(about = "TUI for navigating NKBIP-01 publications")]
struct Args {
    /// Path to nostrdb data directory
    #[arg(short, long, default_value = "~/.local/share/nostr-engine/nostrdb")]
    data_dir: String,

    /// Fetch policy: local_only, local_first, or fetch_always
    #[arg(short, long, default_value = "local_first")]
    policy: String,

    /// Relays to fetch from (can be specified multiple times)
    #[arg(short, long)]
    relay: Vec<String>,

    /// Enable debug logging to file
    #[arg(long)]
    debug: bool,

    /// Purge the database before starting (deletes all cached data)
    #[arg(long)]
    purge_db: bool,

    /// Skip confirmation prompt for --purge-db
    #[arg(long, short = 'y')]
    yes: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Setup logging if debug mode
    if args.debug {
        let file_appender = tracing_appender::rolling::daily("/tmp", "nostr-tree.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::registry()
            .with(fmt::layer().with_writer(non_blocking))
            .with(EnvFilter::from_default_env().add_directive("nostr_engine=debug".parse()?))
            .init();
    }

    // Expand tilde in path
    let data_dir = expand_tilde(&args.data_dir);

    // Handle --purge-db flag
    if args.purge_db {
        if data_dir.exists() {
            if !args.yes {
                print!("This will delete all data in {:?}. Continue? [y/N] ", data_dir);
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            println!("Purging database at {:?}...", data_dir);
            std::fs::remove_dir_all(&data_dir)?;
            println!("Database purged.");
        } else {
            println!("Database directory does not exist, nothing to purge.");
        }
    }

    // Parse fetch policy
    let policy: FetchPolicy = args.policy.parse()?;

    // Create engine
    let engine = if args.relay.is_empty() {
        Engine::new(&data_dir)?
    } else {
        let relays: Vec<&str> = args.relay.iter().map(|s| s.as_str()).collect();
        Engine::with_config(&data_dir, &relays, 15000)?
    };

    let engine = Arc::new(engine);

    // Create and run TUI
    let mut app = TuiApp::new(engine, policy);

    // Load initial publications
    app.load_initial().await?;

    // Run the event loop
    app.run().await?;

    Ok(())
}

fn expand_tilde(path: &str) -> PathBuf {
    if path.starts_with("~/") {
        if let Some(home) = dirs_home() {
            return home.join(&path[2..]);
        }
    }
    PathBuf::from(path)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
