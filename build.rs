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
    println!("cargo:rerun-if-changed=web/build");
}
