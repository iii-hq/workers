//! Embedded SPA bundle.
//!
//! `web/dist/` is included into the binary at compile time via
//! [`rust_embed::RustEmbed`]. The public handlers are:
//!
//! - [`index_handler`] — serves the SPA shell at `/`.
//! - [`manifest_handler`] and [`service_worker_handler`] — serve the
//!   installable-PWA metadata without long-lived caching.
//! - [`icon_handler`] — serves install icons with immutable caching.
//! - [`asset_handler`] — serves `/assets/<file>` with an
//!   `immutable` cache header (Vite emits content-hashed filenames).
//!
//! Anything else falls through to the router's 404. The SPA is hash-
//! routed, so client-side navigation never needs server help.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/web/dist"]
struct WebDist;

pub async fn index_handler() -> Response {
    serve_embedded("index.html", false)
}

pub async fn asset_handler(Path(path): Path<String>) -> Response {
    let key = format!("assets/{path}");
    serve_embedded(&key, true)
}
pub async fn manifest_handler() -> Response {
    let mut response = serve_embedded("manifest.webmanifest", false);
    if response.status() == StatusCode::OK {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/manifest+json"),
        );
    }
    response
}

pub async fn service_worker_handler() -> Response {
    let mut response = serve_embedded("sw.js", false);
    if response.status() == StatusCode::OK {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/javascript; charset=utf-8"),
        );
        // The worker lives at the Console mount root. Relative scope keeps it
        // valid when a reverse proxy mounts the application under a subpath.
        response.headers_mut().insert(
            header::HeaderName::from_static("service-worker-allowed"),
            HeaderValue::from_static("./"),
        );
    }
    response
}

pub async fn icon_handler(Path(path): Path<String>) -> Response {
    let key = format!("icons/{path}");
    serve_embedded(&key, true)
}

/// `/vendor/*` — the shared-dep shim modules for injected UI (static ESM
/// files from `web/public/vendor/`, copied into `web/dist/vendor/` by the
/// Vite build). Served `no-cache`: the shims are tiny and change with the
/// console's React version, so they must never inherit the immutable
/// header `/assets/*` uses.
pub async fn vendor_handler(Path(path): Path<String>) -> Response {
    let key = format!("vendor/{path}");
    let mut response = serve_embedded(&key, false);
    if response.status() == StatusCode::OK {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}

fn serve_embedded(key: &str, immutable: bool) -> Response {
    let Some(file) = WebDist::get(key) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    let mime = mime_guess::from_path(key).first_or_octet_stream();
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CONTENT_LENGTH, file.data.len())
        .body(Body::from(file.data.into_owned()))
        .unwrap_or_else(|_| {
            (StatusCode::INTERNAL_SERVER_ERROR, "response build failed").into_response()
        });

    if immutable {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(IMMUTABLE_CACHE),
        );
    } else {
        // index.html must NOT be cached aggressively; it references the
        // hashed asset filenames and changes every deploy.
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache, must-revalidate"),
        );
    }

    response
}

/// `true` if at least one file is embedded (i.e. `web/dist/` was non-
/// empty at build time). Used by integration tests to short-circuit
/// when the SPA bundle is missing.
pub fn has_bundle() -> bool {
    WebDist::iter().next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_has_index_html() {
        // The bundle is built by `build.rs`. If this fails locally,
        // run `pnpm install && pnpm build` inside `web/` once and
        // re-run `cargo test`.
        assert!(
            has_bundle(),
            "web/dist/ is empty — build.rs should have populated it"
        );
        assert!(
            WebDist::get("index.html").is_some(),
            "web/dist/index.html missing from the embed"
        );
        assert!(
            WebDist::get("manifest.webmanifest").is_some(),
            "web/dist/manifest.webmanifest missing from the embed"
        );
        assert!(
            WebDist::get("sw.js").is_some(),
            "web/dist/sw.js missing from the embed"
        );
        assert!(
            WebDist::get("icons/icon-192.png").is_some(),
            "web/dist/icons/icon-192.png missing from the embed"
        );
        assert!(
            WebDist::get("icons/icon-512.png").is_some(),
            "web/dist/icons/icon-512.png missing from the embed"
        );
    }
}
