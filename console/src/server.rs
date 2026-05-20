//! HTTP server: routes `/`, `/assets/*`, and `/ws` (WebSocket proxy).
//!
//! Binds `0.0.0.0:<http_port>` so the worker is reachable from
//! containers/LAN by default; tighten with a reverse proxy when
//! exposing publicly.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::{assets, proxy};

/// Build the router with the engine URL injected as state for the
/// `/ws` proxy. Exposed for tests so they can drive it without binding
/// a real socket.
pub fn router(engine_url: Arc<String>) -> Router {
    Router::new()
        .route("/", get(assets::index_handler))
        .route("/assets/*path", get(assets::asset_handler))
        .route("/ws", get(proxy::ws_proxy))
        .fallback(not_found)
        .with_state(engine_url)
}

async fn not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "not found")
}

/// Bind and serve until `shutdown` resolves. The `bound` channel is
/// fired once the listener is actually bound — useful for tests that
/// need the OS-assigned port when `http_port` is `0`.
pub async fn serve(
    http_port: u16,
    engine_url: Arc<String>,
    shutdown: oneshot::Receiver<()>,
    bound: Option<oneshot::Sender<SocketAddr>>,
) -> anyhow::Result<()> {
    let app = router(engine_url);

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
