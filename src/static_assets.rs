//! Serve the bundled SvelteKit SPA out of the engine binary.
//!
//! The web frontend builds to a fully static bundle (`@sveltejs/adapter-static`
//! with `fallback: index.html`) under `web/build/`. We embed that directory
//! into the binary with `rust-embed` and serve it as the Axum `.fallback()`
//! handler — everything that is not an API route resolves here. Because the SPA
//! calls the backend with same-origin relative URLs (`/api/v1/...`), serving it
//! from the engine's own origin needs no frontend changes.
//!
//! Routing rules:
//! - `/` → `index.html`
//! - an existing asset path → that asset, with its content type
//! - anything else → `index.html` (200), so client-side routes resolve on
//!   deep-link / hard-refresh (SPA fallback)
//!
//! In debug builds `rust-embed` reads from disk at request time, so `cargo run`
//! picks up `web/build/` changes without a recompile; release builds bake the
//! assets into the binary.

use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/build/"]
struct WebAssets;

/// Axum fallback handler serving the embedded SPA.
pub async fn static_handler(uri: Uri) -> Response {
    // Strip the leading '/' and treat the root as index.html.
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match WebAssets::get(path) {
        Some(content) => serve(path, content),
        // SPA fallback: unknown non-asset path → let the client router handle it.
        None => match WebAssets::get("index.html") {
            Some(content) => serve("index.html", content),
            None => (StatusCode::NOT_FOUND, "web UI not bundled").into_response(),
        },
    }
}

fn serve(path: &str, content: rust_embed::EmbeddedFile) -> Response {
    let mime = content.metadata.mimetype();

    // SvelteKit emits content-hashed assets under `_app/` — safe to cache hard.
    // index.html (and other unhashed paths) must stay revalidated.
    let cache = if path.starts_with("_app/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(content.data))
        .unwrap()
}
