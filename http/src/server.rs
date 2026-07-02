//! axum server wiring: binds the listener, assembles the router (catch-all
//! handler + CORS + timeout + concurrency limit), and runs it with graceful
//! shutdown.
//!
//! Ported from the in-engine `iii-http` `RestApiWorker` (`api_core.rs`):
//! `build_cors_layer`, the `TimeoutLayer` / `ConcurrencyLimitLayer` stack and
//! the `axum::serve` + `with_graceful_shutdown` task. Routes are dynamic: every
//! request hits `dynamic_handler`, which matches routes from the shared
//! [`RouteTable`] at request time, so adding or removing routes needs no router
//! rebuild.
//!
//! ## Hot-swappable layers (Phase A)
//!
//! The CORS / outer-timeout / concurrency tower layers are baked into the axum
//! `Router` at build time, so they cannot change per-request. To let them
//! hot-reload on a **same-address** config change without dropping the listener,
//! the router is wrapped in a [`HotRouter`] — a `tower::Service` holding an
//! `Arc<RwLock<Router>>`. Each request clones the current `Router`, injects the
//! shared [`AppState`] extension, and dispatches. Swapping `*router.write()`
//! (via [`rebuild_layers`]) makes the next request use freshly-built layers.
//! This mirrors the engine's `HotRouter` (`hot_router.rs`), with the engine's
//! injected `engine` extension replaced by our `AppState`.
//!
//! Address (host/port) rebind is deliberately NOT handled here — that is a
//! separate later phase; host/port changes remain restart-only.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::{
    Extension, Router,
    body::Body,
    http::{Method, Request, Response, StatusCode},
    serve::IncomingStream,
};
use futures::Future;
use iii_sdk::IIIClient;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, oneshot};
use tokio::task::JoinHandle;
use tower::Service;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    cors::{Any, CorsLayer},
    timeout::TimeoutLayer,
};

use crate::config::RestApiConfig;
use crate::configuration::ConfigCell;
use crate::handler::{self, AppState};
use crate::trigger::RouteTable;

/// Sentinel meaning "any origin" in CORS config.
const ALLOW_ORIGIN_ANY: &str = "*";

/// Shared, swappable axum `Router`. Held behind an `RwLock` so a config change
/// can rebuild the layered router (see [`rebuild_layers`]) while the listener
/// keeps running; the [`HotRouter`] reads it per request.
pub type RouterCell = Arc<RwLock<Router>>;

/// A running server: its bound address, the task handle, the swappable router
/// cell (so a same-address config change can rebuild its layers), and a
/// shutdown trigger.
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    pub router: RouterCell,
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

/// Build the layered axum `Router` from a config snapshot: the fallback-only
/// `dynamic_handler` wrapped in the CORS, outer-timeout, and concurrency tower
/// layers. The [`AppState`] extension is intentionally NOT added here — the
/// [`HotRouter`] injects it per request (mirroring the engine injecting its
/// `engine` extension), so the router itself carries only the reloadable layers.
pub fn build_router(snapshot: &RestApiConfig) -> Router {
    let cors = build_cors_layer(snapshot);
    let timeout = TimeoutLayer::with_status_code(
        StatusCode::GATEWAY_TIMEOUT,
        Duration::from_millis(snapshot.default_timeout),
    );
    let concurrency = ConcurrencyLimitLayer::new(snapshot.concurrency_request_limit);

    Router::new()
        .fallback(handler::dynamic_handler)
        .layer(cors)
        .layer(timeout)
        .layer(concurrency)
}

/// Rebuild the layered router from `snapshot` and swap it into `router`. The
/// next request served by the [`HotRouter`] picks up the new CORS / timeout /
/// concurrency layers; the listener is untouched. Called on a same-address
/// config change (see [`crate::configuration::on_config_change`]).
pub async fn rebuild_layers(router: &RouterCell, snapshot: &RestApiConfig) {
    *router.write().await = build_router(snapshot);
}

/// A `tower::Service` that dispatches each request through the current
/// [`RouterCell`], injecting the shared [`AppState`] as an axum `Extension` per
/// call. Cloning the router per request is what makes a mid-flight swap
/// (`rebuild_layers`) safe: in-flight requests keep the router they cloned,
/// new requests see the swapped one. Ported from the engine's `HotRouter`.
#[derive(Clone)]
pub struct HotRouter {
    pub inner: RouterCell,
    pub state: Arc<AppState>,
}

impl Service<Request<Body>> for HotRouter {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let router_arc = self.inner.clone();
        let state = self.state.clone();
        Box::pin(async move {
            let router_clone = {
                let router_guard = router_arc.read().await;
                router_guard.clone()
            };

            let router_with_extension = router_clone.layer(Extension(state));
            let mut router_service = router_with_extension.into_service();

            match Service::call(&mut router_service, req).await {
                Ok(response) => Ok(response),
                Err(infallible) => match infallible {},
            }
        })
    }
}

/// The `MakeService` handed to `axum::serve`: yields a clone of the shared
/// [`HotRouter`] for each accepted connection. Ported from the engine.
pub struct MakeHotRouterService {
    pub router: HotRouter,
}

impl<'a> Service<IncomingStream<'a>> for MakeHotRouterService {
    type Response = HotRouter;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: IncomingStream<'a>) -> Self::Future {
        let router = self.router.clone();
        Box::pin(async move { Ok(router) })
    }
}

/// Bind on `config.host:config.port` (port 0 → OS-assigned ephemeral), assemble
/// the [`HotRouter`], and spawn the serving task. Returns the resolved local
/// address (so callers can reach an ephemeral port), the swappable router cell,
/// and the task/shutdown handles.
///
/// The CORS, outer timeout, and concurrency tower layers are built from a
/// snapshot taken here at serve time, but — unlike a static router — they can be
/// rebuilt live via [`rebuild_layers`] on a same-address config change (see
/// [`crate::configuration`]). The shared [`ConfigCell`] is handed to the
/// handler, which reads `middleware` and `default_timeout` from it per-request,
/// so those fields hot-reload too.
pub async fn serve(
    routes: Arc<RwLock<RouteTable>>,
    iii: Arc<IIIClient>,
    config: ConfigCell,
) -> anyhow::Result<ServerHandle> {
    let snapshot: Arc<RestApiConfig> = config.read().await.clone();

    let addr = format!("{}:{}", snapshot.host, snapshot.port);
    let listener = TcpListener::bind(&addr).await?;
    let local_addr = listener.local_addr()?;

    let state = Arc::new(AppState {
        routes,
        iii,
        config,
    });

    let router: RouterCell = Arc::new(RwLock::new(build_router(&snapshot)));
    let hot_router = HotRouter {
        inner: router.clone(),
        state,
    };

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        tracing::info!(address = %local_addr, "iii-http listening");
        let server = axum::serve(listener, MakeHotRouterService { router: hot_router })
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
        if let Err(e) = server.await {
            tracing::error!(error = %e, "iii-http server exited with error");
        }
    });

    Ok(ServerHandle {
        local_addr,
        router,
        join,
        shutdown: shutdown_tx,
    })
}
