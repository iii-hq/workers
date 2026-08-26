//! HTTP server: routes `/`, `/assets/*`, `/ws` (WebSocket proxy), and —
//! when injectable UI is enabled — `/ui`, `/ui/*`, and `/vendor/*`.
//!
//! Binds `0.0.0.0:<http_port>` so the worker is reachable from
//! containers/LAN by default; tighten with a reverse proxy when
//! exposing publicly.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{FromRef, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex};
use tokio::task::{AbortHandle, JoinHandle};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::ui_assets::UiRegistry;
use crate::{assets, proxy};

/// Grace period before a superseded listener is hard-aborted. Graceful
/// shutdown is the primary path; the abort only bounds how long a stuck
/// connection can keep the old port occupied after a configuration rebind.
/// This matches the HTTP worker's listener-rebind strategy.
const OLD_SERVER_HARD_STOP_GRACE: Duration = Duration::from_secs(2);

/// Control handle for the currently running HTTP listener.
pub struct ServerControl {
    pub graceful: oneshot::Sender<()>,
    pub abort: AbortHandle,
    pub local_addr: SocketAddr,
    pub join: JoinHandle<()>,
}

/// Shared slot holding the current listener. A configuration change replaces
/// this only after the new port has bound successfully.
pub type ServerControlCell = Arc<Mutex<Option<ServerControl>>>;

/// Handle returned once the initial listener is bound and serving.
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    pub control: ServerControlCell,
}

impl ServerHandle {
    /// The address currently serving Console. This changes after a live port
    /// rebind and becomes `None` after shutdown.
    pub async fn current_addr(&self) -> Option<SocketAddr> {
        self.control.lock().await.as_ref().map(|c| c.local_addr)
    }

    /// Gracefully stop the current listener and wait for it to finish.
    pub async fn shutdown(self) {
        if let Some(control) = self.control.lock().await.take() {
            let _ = control.graceful.send(());
            let _ = control.join.await;
        }
    }
}

/// Router state. `ui: None` means injectable UI is disabled (config kill
/// switch) — the `/ui` and `/vendor` routes are not mounted at all.
#[derive(Clone)]
pub struct AppState {
    pub engine_url: Arc<String>,
    pub namespace: Option<String>,
    pub ui: Option<Arc<UiRegistry>>,
}

impl AppState {
    pub fn new(
        engine_url: Arc<String>,
        namespace: Option<String>,
        ui: Option<Arc<UiRegistry>>,
    ) -> Self {
        Self {
            engine_url,
            namespace,
            ui,
        }
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
        .route("/runtime", get(runtime_handler))
        .route("/ws", get(proxy::ws_proxy));

    if state.ui.is_some() {
        router = router
            .route("/ui", get(ui_manifest_handler))
            .route("/ui/*path", get(ui_asset_handler))
            .route("/vendor/*path", get(assets::vendor_handler));
    }

    router
        .fallback(not_found)
        .with_state(state)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static("frame-ancestors 'none'; object-src 'none'; base-uri 'none'"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

/// `GET /runtime` — connection settings that cannot be baked into the SPA.
/// A browser has no process environment, so it needs the console worker to
/// tell it which namespace its proxied engine connection must join.
async fn runtime_handler(State(state): State<AppState>) -> Response {
    let mut response = Json(serde_json::json!({
        "namespace": state.namespace,
    }))
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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
    let handle = start(http_port, state).await?;

    if let Some(tx) = bound {
        let _ = tx.send(handle.local_addr);
    }

    let _ = shutdown.await;
    handle.shutdown().await;
    Ok(())
}

/// Bind the initial listener and spawn its server task.
pub async fn start(http_port: u16, state: AppState) -> anyhow::Result<ServerHandle> {
    let listener = bind_listener(http_port).await?;
    let local_addr = listener
        .local_addr()
        .with_context(|| "reading bound listener local_addr")?;
    let control = spawn_server(listener, state);
    Ok(ServerHandle {
        local_addr,
        control: Arc::new(Mutex::new(Some(control))),
    })
}

/// Bind `0.0.0.0:<http_port>` without changing any live server state. Rebinds
/// use this bind-new-before-stop-old step so a failed port change leaves the
/// existing Console listener untouched.
pub async fn bind_listener(http_port: u16) -> anyhow::Result<TcpListener> {
    let addr: SocketAddr = (std::net::Ipv4Addr::UNSPECIFIED, http_port).into();
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding TCP listener on {addr}"))
}

/// Spawn a server over an already-bound listener. The same shared app state is
/// reused when configuration moves the listener to a different port.
pub fn spawn_server(listener: TcpListener, state: AppState) -> ServerControl {
    let local_addr = listener
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    let (graceful_tx, graceful_rx) = oneshot::channel::<()>();
    let app = router(state);

    let join = tokio::spawn(async move {
        tracing::info!(addr = %local_addr, "console http listening");
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = graceful_rx.await;
            })
            .await
        {
            tracing::error!(%error, "console http server exited with error");
        }
        tracing::info!(addr = %local_addr, "console http stopped");
    });

    let abort = join.abort_handle();
    ServerControl {
        graceful: graceful_tx,
        abort,
        local_addr,
        join,
    }
}

/// Gracefully drain a superseded listener, with a bounded hard-stop fallback.
/// The replacement listener is already serving before this is called.
pub fn stop_old_server(old: ServerControl) {
    let _ = old.graceful.send(());
    let abort = old.abort;
    tokio::spawn(async move {
        tokio::time::sleep(OLD_SERVER_HARD_STOP_GRACE).await;
        abort.abort();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn state_with_ui() -> (AppState, Arc<UiRegistry>) {
        let ui = Arc::new(UiRegistry::default());
        (
            AppState::new(
                Arc::new("ws://127.0.0.1:1".to_string()),
                Some("project-a".to_string()),
                Some(ui.clone()),
            ),
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
    async fn runtime_exposes_the_worker_namespace_without_caching() {
        let (state, _ui) = state_with_ui();
        let response = get_response(router(state), "/runtime", &[]).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value, serde_json::json!({ "namespace": "project-a" }));
    }

    #[tokio::test]
    async fn kill_switch_removes_ui_routes() {
        let state = AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), None, None);
        let response = get_response(router(state.clone()), "/ui", &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = get_response(router(state), "/vendor/react.js", &[]).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn every_http_response_carries_defensive_browser_headers() {
        let state = AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), None, None);
        for uri in ["/", "/missing"] {
            let response = get_response(router(state.clone()), uri, &[]).await;
            assert_eq!(
                response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
                Some(&HeaderValue::from_static("nosniff"))
            );
            assert_eq!(
                response.headers().get(header::X_FRAME_OPTIONS),
                Some(&HeaderValue::from_static("DENY"))
            );
            assert_eq!(
                response.headers().get("referrer-policy"),
                Some(&HeaderValue::from_static("no-referrer"))
            );
            assert_eq!(
                response.headers().get("cross-origin-resource-policy"),
                Some(&HeaderValue::from_static("same-origin"))
            );
            assert_eq!(
                response.headers().get("content-security-policy"),
                Some(&HeaderValue::from_static(
                    "frame-ancestors 'none'; object-src 'none'; base-uri 'none'"
                ))
            );
        }
    }
}
