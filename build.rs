//! Build script — guarantees `web/build/` exists for `rust-embed`.
//!
//! `rust-embed`'s derive macro resolves `#[folder = "web/build/"]` at
//! compile time and errors if the folder is missing. `web/build/` is a
//! gitignored artifact produced by `pnpm -C web build`, so a fresh checkout
//! has no folder and a plain `cargo build`/`cargo check` would fail before the
//! frontend is ever built.
//!
//! To keep the Rust build self-sufficient, we create the folder with a
//! placeholder `index.html` when it is absent. The real SPA (via
//! `scripts/build-bundle.sh`) overwrites it; if you only ran `cargo build`,
//! the served page tells you how to produce the real bundle.

use std::fs;
use std::path::Path;

fn main() {
    let build_dir = Path::new("web/build");
    let index = build_dir.join("index.html");

    if !index.exists() {
        fs::create_dir_all(build_dir).expect("create web/build placeholder dir");
        fs::write(
            &index,
            "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>tendrl-engine</title></head><body>\
<p>Web UI not built. Run <code>scripts/build-bundle.sh</code> \
(or <code>pnpm -C web build</code>) to bundle the SPA.</p>\
</body></html>",
        )
        .expect("write web/build placeholder index.html");
    }

    // Re-run if the bundle is rebuilt so release binaries pick up fresh assets.
    //
    // A shallow `rerun-if-changed=web/build` only fires when a *top-level* entry
    // is added/removed — it misses content edits to index.html and changes to the
    // hashed bundles under `_app/immutable/`. That left the portable build (a
    // separate `target/portable` dir compiled in Docker) silently re-embedding a
    // STALE SPA: the bundle build picked up changes via the scripts' `touch
    // web/build`, but the portable build did not. Walk the tree and watch every
    // file + directory so any `pnpm build` reliably re-embeds.
    emit_rerun_recursive(build_dir);
}

/// Emit `cargo:rerun-if-changed` for `dir` and everything under it. Watching
/// each file (not just the directory) means a content edit to an existing asset
/// invalidates the embed; watching directories catches add/remove of the hashed
/// bundles vite emits with fresh filenames each build.
fn emit_rerun_recursive(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            emit_rerun_recursive(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
