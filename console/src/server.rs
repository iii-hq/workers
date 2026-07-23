//! HTTP server: routes `/`, `/assets/*`, `/ws` (WebSocket proxy), and —
//! when injectable UI is enabled — `/ui`, `/ui/*`, and `/vendor/*`.
//!
//! Binds `0.0.0.0:<http_port>` so the worker is reachable from
//! containers/LAN by default; tighten with a reverse proxy when
//! exposing publicly.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::ui_assets::UiRegistry;
use crate::{assets, proxy};

/// Router state. `ui: None` means injectable UI is disabled (config kill
/// switch) — the `/ui` and `/vendor` routes are not mounted at all.
#[derive(Clone)]
pub struct AppState {
    pub engine_url: Arc<String>,
    pub ui: Option<Arc<UiRegistry>>,
}

impl AppState {
    pub fn new(engine_url: Arc<String>, ui: Option<Arc<UiRegistry>>) -> Self {
        Self { engine_url, ui }
    }
}

/// The `/ws` proxy only needs the engine URL.
impl FromRef<AppState> for Arc<String> {
    fn from_ref(state: &AppState) -> Self {
        state.engine_url.clone()
    }
}

/// Build the router. Exposed for tests so they can drive it without
/// binding a real socket.
pub fn router(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/", get(assets::index_handler))
        .route("/assets/*path", get(assets::asset_handler))
        .route("/ws", get(proxy::ws_proxy));

    if state.ui.is_some() {
        router = router
            .route("/ui", get(ui_manifest_handler))
            .route("/ui/*path", get(ui_asset_handler))
            .route("/vendor/*path", get(assets::vendor_handler));
    }

    router.fallback(not_found).with_state(state)
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

/// `Cache-Control` for every injected-UI response: mutable content, so the
/// HTTP cache must revalidate — the `?v=<hash>` query is what actually
/// busts the browser's module map.
const NO_CACHE: &str = "no-cache";

/// `GET /ui` — the manifest JSON (same shape as `console::ui-manifest`),
/// curl-friendly debugging.
async fn ui_manifest_handler(State(state): State<AppState>) -> Response {
    let Some(ui) = &state.ui else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let body = serde_json::json!({
        "disabled": false,
        "assets": ui.manifest(),
        "workers": ui.workers(),
    });
    let mut response = Json(body).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(NO_CACHE));
    response
}

/// `GET /ui/*path` — current bytes for a registered asset, from the
/// in-memory registry. `no-cache` + `ETag: "<hash>"` (304 on
/// `If-None-Match`).
async fn ui_asset_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(ui) = &state.ui else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let Some((content, content_type, hash)) = ui.serve(&path) else {
        return (StatusCode::NOT_FOUND, "unknown ui asset").into_response();
    };
    let etag = format!("\"{hash}\"");

    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok());
    if if_none_match == Some(etag.as_str()) {
        return Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(header::ETAG, &etag)
            .header(header::CACHE_CONTROL, NO_CACHE)
            .body(axum::body::Body::empty())
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, NO_CACHE)
        .header(header::ETAG, etag)
        .body(axum::body::Body::from(content))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// Bind and serve until `shutdown` resolves. The `bound` channel is
/// fired once the listener is actually bound — useful for tests that
/// need the OS-assigned port when `http_port` is `0`.
pub async fn serve(
    http_port: u16,
    state: AppState,
    shutdown: oneshot::Receiver<()>,
    bound: Option<oneshot::Sender<SocketAddr>>,
) -> anyhow::Result<()> {
    let app = router(state);

    let addr: SocketAddr = (std::net::Ipv4Addr::UNSPECIFIED, http_port).into();
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding TCP listener on {addr}"))?;
    let local = listener
        .local_addr()
        .with_context(|| "reading bound listener local_addr")?;
    tracing::info!(addr = %local, "console http listening");

    if let Some(tx) = bound {
        let _ = tx.send(local);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.await;
        })
        .await
        .context("axum::serve loop exited with error")?;

    tracing::info!("console http stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn state_with_ui() -> (AppState, Arc<UiRegistry>) {
        let ui = Arc::new(UiRegistry::default());
        (
            AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), Some(ui.clone())),
            ui,
        )
    }

    async fn get_response(router: Router, uri: &str, headers: &[(&str, &str)]) -> Response {
        let mut builder = axum::http::Request::builder().uri(uri);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        let request = builder.body(axum::body::Body::empty()).unwrap();
        router.oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn ui_asset_serves_with_etag_and_304() {
        let (state, ui) = state_with_ui();
        crate::ui_assets::test_support::insert_script(&ui, "state/page.js", "export {}", "t1");
        let hash = crate::ui_assets::content_hash("export {}");

        let response = get_response(router(state.clone()), "/ui/state/page.js", &[]).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
        assert_eq!(
            response.headers().get(header::ETAG).unwrap(),
            &format!("\"{hash}\"")
        );
        assert!(response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/javascript"));

        let etag = format!("\"{hash}\"");
        let response = get_response(
            router(state),
            "/ui/state/page.js",
            &[("if-none-match", etag.as_str())],
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn ui_unknown_asset_404s() {
        let (state, _ui) = state_with_ui();
        let response = get_response(router(state), "/ui/nope/missing.js", &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ui_manifest_shape() {
        let (state, ui) = state_with_ui();
        crate::ui_assets::test_support::insert_script(&ui, "state/page.js", "export {}", "t1");
        let response = get_response(router(state), "/ui", &[]).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["disabled"], serde_json::json!(false));
        assert_eq!(v["assets"][0]["path"], "state/page.js");
        assert_eq!(v["assets"][0]["kind"], "script");
    }

    #[tokio::test]
    async fn kill_switch_removes_ui_routes() {
        let state = AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), None);
        let response = get_response(router(state.clone()), "/ui", &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = get_response(router(state), "/vendor/react.js", &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
