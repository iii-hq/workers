//! axum server wiring: binds the listener, assembles the router (catch-all
//! handler + CORS + timeout + concurrency limit), and runs it with graceful
//! shutdown.
//!
//! Ported from the in-engine `iii-http` `RestApiWorker` (`api_core.rs`):
//! `build_cors_layer`, the `TimeoutLayer` / `ConcurrencyLimitLayer` stack and
//! the `axum::serve` + `with_graceful_shutdown` task. Reduced to a single
//! static router (no hot-router): all requests hit `dynamic_handler`, which
//! matches routes from the shared [`RouteTable`] at request time, so adding or
//! removing routes needs no router rebuild.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    Extension, Router,
    http::{Method, StatusCode},
};
use iii_sdk::IIIClient;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
};

use crate::config::RestApiConfig;
use crate::handler::{self, AppState};
use crate::trigger::RouteTable;

/// Sentinel meaning "any origin" in CORS config.
const ALLOW_ORIGIN_ANY: &str = "*";

/// A running server: its bound address, the task handle, and a shutdown
/// trigger.
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    pub join: JoinHandle<()>,
    pub shutdown: oneshot::Sender<()>,
}

/// Build the CORS layer from config. No config → permissive (any
/// origin/method). With config: an empty list or the `*` sentinel allows any;
/// otherwise the listed origins/methods are used. Headers are always `Any`.
/// Mirrors the engine's `build_cors_layer`.
pub fn build_cors_layer(config: &RestApiConfig) -> CorsLayer {
    let Some(cors_config) = &config.cors else {
        return CorsLayer::permissive();
    };

    let mut cors = CorsLayer::new();

    let has_any_sentinel = cors_config
        .allowed_origins
        .iter()
        .any(|o| o == ALLOW_ORIGIN_ANY);

    if cors_config.allowed_origins.is_empty() || has_any_sentinel {
        cors = cors.allow_origin(Any);
    } else {
        let origins: Vec<_> = cors_config
            .allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        cors = cors.allow_origin(origins);
    }

    if cors_config.allowed_methods.is_empty() {
        cors = cors.allow_methods(Any);
    } else {
        let methods: Vec<Method> = cors_config
            .allowed_methods
            .iter()
            .filter_map(|m| m.parse().ok())
            .collect();
        cors = cors.allow_methods(methods);
    }

    cors.allow_headers(Any)
}

/// Bind on `config.host:config.port` (port 0 → OS-assigned ephemeral),
/// assemble the router, and spawn the serving task. Returns the resolved local
/// address (so callers can reach an ephemeral port) plus the task/shutdown
/// handles.
pub async fn serve(
    routes: Arc<RwLock<RouteTable>>,
    iii: Arc<IIIClient>,
    config: RestApiConfig,
) -> anyhow::Result<ServerHandle> {
    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    let cors = build_cors_layer(&config);
    let timeout = TimeoutLayer::with_status_code(
        StatusCode::GATEWAY_TIMEOUT,
        Duration::from_millis(config.default_timeout),
    );
    let concurrency = ConcurrencyLimitLayer::new(config.concurrency_request_limit);

    let state = Arc::new(AppState {
        routes,
        iii,
        config,
    });

    // A fallback-only router: every request (matched against our own route
    // table inside the handler) flows through `dynamic_handler`.
    let app = Router::new()
        .fallback(handler::dynamic_handler)
        .layer(Extension(state))
        .layer(cors)
        .layer(timeout)
        .layer(concurrency);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        tracing::info!(address = %local_addr, "iii-http listening");
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        });
        if let Err(e) = server.await {
            tracing::error!(error = %e, "iii-http server exited with error");
        }
    });

    Ok(ServerHandle {
        local_addr,
        join,
        shutdown: shutdown_tx,
    })
}
