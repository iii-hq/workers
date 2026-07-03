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
//! ## Address rebind (Phase B)
//!
//! A host/port change rebinds the TCP listener without dropping requests: the
//! NEW address is bound first (a failed bind keeps the old server untouched),
//! the new server is spawned over the SAME [`RouterCell`] + [`AppState`] (so it
//! serves identical routes/layers), and only THEN is the OLD server gracefully
//! shut down (its listener + idle keep-alive connections closed) with a hard
//! abort as a safety net. The currently-running server is tracked in a shared
//! [`ServerControlCell`] so a config-change task can replace it. This mirrors
//! the engine's `spawn_server` / `apply_config` address-change branch.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::{
    body::Body,
    http::{Method, Request, Response, StatusCode},
    serve::IncomingStream,
    Extension, Router,
};
use futures::Future;
use iii_sdk::IIIClient;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Mutex, RwLock};
use tokio::task::{AbortHandle, JoinHandle};
use tower::limit::ConcurrencyLimitLayer;
use tower::Service;
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

/// Grace period after a rebind before the old server task is hard-aborted, as a
/// safety net for a connection that refuses to drain. Graceful shutdown (closing
/// the old listener + idle keep-alive connections) is the primary mechanism;
/// this only bounds how long a stuck connection can pin the old address.
/// Mirrors the engine's `OLD_SERVER_HARD_STOP_GRACE`.
const OLD_SERVER_HARD_STOP_GRACE: Duration = Duration::from_secs(2);

/// Shared, swappable axum `Router`. Held behind an `RwLock` so a config change
/// can rebuild the layered router (see [`rebuild_layers`]) while the listener
/// keeps running; the [`HotRouter`] reads it per request.
pub type RouterCell = Arc<RwLock<Router>>;

/// Control handle for the CURRENTLY-running server task: its graceful-shutdown
/// sender, its abort handle (safety-net hard stop), the bound address, and the
/// task join handle. A host/port rebind (see
/// [`crate::configuration::on_config_change`]) spawns a new server, swaps this
/// into the shared [`ServerControlCell`], and gracefully stops the old one.
pub struct ServerControl {
    pub graceful: oneshot::Sender<()>,
    pub abort: AbortHandle,
    pub local_addr: SocketAddr,
    pub join: JoinHandle<()>,
}

/// Shared, replaceable handle to the current [`ServerControl`]. `None` once the
/// server has been shut down (the control is `take`n). Behind an async `Mutex`
/// so the config-change task can atomically swap in a rebound server.
pub type ServerControlCell = Arc<Mutex<Option<ServerControl>>>;

/// A running server: its (initial) bound address, the swappable router cell (so
/// a same-address config change can rebuild its layers), the shared
/// [`HotRouter`] (reused to spawn a rebound server on the same routes/state),
/// and the replaceable [`ServerControlCell`].
pub struct ServerHandle {
    pub local_addr: SocketAddr,
    pub router: RouterCell,
    pub hot_router: HotRouter,
    pub control: ServerControlCell,
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

    let control = spawn_server(listener, hot_router.clone());

    Ok(ServerHandle {
        local_addr,
        router,
        hot_router,
        control: Arc::new(Mutex::new(Some(control))),
    })
}

/// Spawn the axum serving task over an already-bound `listener`, serving via a
/// clone of the shared [`HotRouter`] (same [`RouterCell`] + [`AppState`], so it
/// behaves identically to the original server). Returns a [`ServerControl`]: a
/// graceful-shutdown sender (gates `with_graceful_shutdown`), the task abort
/// handle, the bound address, and the join handle.
///
/// Shared by the initial [`serve`] and by host/port rebinds
/// (see [`crate::configuration::on_config_change`]). Graceful shutdown closes
/// the listener AND its open connections (idle keep-alive included), which is
/// what actually frees the address; a bare `abort()` would only drop the
/// listener. Mirrors the engine's `spawn_server`.
pub fn spawn_server(listener: TcpListener, hot_router: HotRouter) -> ServerControl {
    let local_addr = hot_router_local_addr(&listener);
    let (graceful_tx, graceful_rx) = oneshot::channel::<()>();

    let join = tokio::spawn(async move {
        tracing::info!(address = %local_addr, "iii-http listening");
        let server = axum::serve(listener, MakeHotRouterService { router: hot_router })
            .with_graceful_shutdown(async move {
                // A dropped sender resolves the receiver too — both mean "stop".
                let _ = graceful_rx.await;
            });
        if let Err(e) = server.await {
            tracing::error!(error = %e, "iii-http server exited with error");
        }
    });

    let abort = join.abort_handle();
    ServerControl {
        graceful: graceful_tx,
        abort,
        local_addr,
        join,
    }
}

/// Resolve a listener's bound address, falling back to an unspecified addr if
/// the OS query fails (only used for logging / the reported `local_addr`).
fn hot_router_local_addr(listener: &TcpListener) -> SocketAddr {
    listener
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)))
}

/// Gracefully stop a superseded server after a rebind: signal its graceful
/// shutdown (closes the old listener + open connections so the old address is
/// fully freed), then hard-abort it after [`OLD_SERVER_HARD_STOP_GRACE`] as a
/// safety net. The join handle is detached — draining happens in the background.
/// Mirrors the engine's post-rebind old-server teardown.
pub fn stop_old_server(old: ServerControl) {
    let _ = old.graceful.send(());
    let abort = old.abort;
    tokio::spawn(async move {
        tokio::time::sleep(OLD_SERVER_HARD_STOP_GRACE).await;
        abort.abort();
    });
}
