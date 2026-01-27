//! nostr-tree: TUI for navigating NKBIP-01 publications
//!
//! A vim-style terminal interface for browsing and manipulating
//! Nostr publications (kind 30040/30041).

use clap::Parser;
use nostr_engine::engine::{Engine, FetchPolicy};
use nostr_engine::identity::IdentityKeyring;
use nostr_engine::relay;
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

    /// Also clear identity from OS keyring when purging
    #[arg(long)]
    purge_identity: bool,

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

    // Parse fetch policy early (needed for purge warning)
    let policy: FetchPolicy = args.policy.parse()?;

    // Handle --purge-db flag
    if args.purge_db {
        let has_data = data_dir.exists();

        if !args.yes {
            print!("This will delete all cached data in {:?}", data_dir);
            if args.purge_identity {
                print!(" and clear identity from OS keyring");
            }
            print!(". Continue? [y/N] ");
            io::stdout().flush()?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Aborted.");
                return Ok(());
            }
        }

        if has_data {
            println!("Purging database at {:?}...", data_dir);
            std::fs::remove_dir_all(&data_dir)?;
            println!("Database purged.");
        } else {
            println!("Database directory does not exist, nothing to purge.");
        }

        if args.purge_identity {
            println!("Clearing identity from OS keyring...");
            let keyring = IdentityKeyring::new();
            let _ = keyring.clear_last_identity();
            println!("Identity cleared.");
        }

        // Warn user if policy will re-fetch
        if policy != FetchPolicy::LocalOnly {
            println!();
            println!("Note: Using '{}' policy - events will be re-fetched from relays.", args.policy);
            println!("      Use '--policy local_only' to start with an empty feed.");
        }
    }

    // Determine relays to use
    let relays = if args.relay.is_empty() {
        // Check for local relay and prepend if available
        relay::get_relays_with_local().await
    } else {
        args.relay.clone()
    };

    // Create engine
    let relay_refs: Vec<&str> = relays.iter().map(|s| s.as_str()).collect();
    let engine = Engine::with_config(&data_dir, &relay_refs, 15000)?;

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
